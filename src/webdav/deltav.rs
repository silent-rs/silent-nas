use super::{WebDavHandler, constants::*};
use http_body_util::BodyExt;
use quick_xml::events::Event;
use quick_xml::Reader;
use silent::prelude::*;
use tokio::fs;

impl WebDavHandler {
    /// VERSION-CONTROL - 启用版本控制（简化为标记属性）
    pub(super) async fn handle_version_control(&self, path: &str) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let mut props = self.props.write().await;
        let entry = props.entry(path).or_default();
        entry.insert("dav:version-controlled".to_string(), "true".to_string());
        Ok(Response::empty())
    }

    /// REPORT - 支持 sync-collection（简化）与版本列表
    pub(super) async fn handle_report(
        &self,
        path: &str,
        req: &mut Request,
    ) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        // 读取请求体以判定报告类型
        let body = req.take_body();
        let xml_bytes = match body {
            ReqBody::Incoming(b) => b
                .collect()
                .await
                .map_err(|e| {
                    SilentError::business_error(
                        StatusCode::BAD_REQUEST,
                        format!("读取请求体失败: {}", e),
                    )
                })?
                .to_bytes()
                .to_vec(),
            ReqBody::Once(bytes) => bytes.to_vec(),
            ReqBody::Empty => Vec::new(),
        };
        let body_str_lower = String::from_utf8_lossy(&xml_bytes).to_lowercase();

        if body_str_lower.contains("sync-collection") {
            // WebDAV Sync (RFC 6578) 简化实现：返回全量条目 + 新的 sync-token
            // 支持 Depth: 1 与 infinity
            // 解析 limit 与 sync-token（若存在）
            let (limit, since_token_time) = Self::parse_sync_collection_request(&xml_bytes);
            let depth = req
                .headers()
                .get("Depth")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("1");
            let storage_path = self.storage.get_full_path(&path);
            let mut xml = String::new();
            xml.push_str(XML_HEADER);
            xml.push_str("<D:multistatus xmlns:D=\"DAV:\">");
            // 生成新的 sync-token（使用 scru128，符合ID规则；同时包含当前时间）
            let token = format!(
                "urn:sync:{}:{}",
                scru128::new_string(),
                chrono::Local::now().naive_local()
            );
            xml.push_str(&format!("<D:sync-token>{}</D:sync-token>", token));

            let meta = fs::metadata(&storage_path)
                .await
                .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "路径不存在"))?;
            if meta.is_dir() {
                // 列出自身（仅在全量请求时包含根目录）
                let href = self.build_full_href(&path);
                if since_token_time.is_none() {
                    Self::add_prop_response(&mut xml, &href, &storage_path, true).await;
                }
                if depth.eq_ignore_ascii_case("infinity") {
                    // 递归列出（仅包含变化项）
                    let mut count_left = limit.unwrap_or(usize::MAX);
                    self.walk_propfind_recursive_filtered(
                        &storage_path,
                        &path,
                        &mut xml,
                        since_token_time,
                        &mut count_left,
                    )
                    .await?;
                } else {
                    // 单层（仅包含变化项）
                    let mut entries = fs::read_dir(&storage_path).await.map_err(|e| {
                        SilentError::business_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("读取目录失败: {}", e),
                        )
                    })?;
                    let mut count = 0usize;
                    while let Some(entry) = entries.next_entry().await.map_err(|e| {
                        SilentError::business_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("读取目录项失败: {}", e),
                        )
                    })? {
                        if let Some(maxn) = limit { if count >= maxn { break; } }
                        let entry_path = entry.path();
                        if Self::modified_after(&entry_path, since_token_time) {
                            let relative_path = if path.is_empty() || path == "/" {
                                format!("/{}", entry.file_name().to_string_lossy())
                            } else {
                                format!("{}/{}", path, entry.file_name().to_string_lossy())
                            };
                            let href = self.build_full_href(&relative_path);
                            let is_dir = entry_path.is_dir();
                            Self::add_prop_response(&mut xml, &href, &entry_path, is_dir).await;
                            count += 1;
                        }
                    }
                }
            } else {
                // 单文件：若自 token 以来有变化则返回
                if Self::modified_after(&storage_path, since_token_time) || since_token_time.is_none() {
                    let href = self.build_full_href(&path);
                    Self::add_prop_response(&mut xml, &href, &storage_path, false).await;
                }
            }

            xml.push_str(XML_MULTISTATUS_END);
            let mut resp = Response::text(&xml);
            resp.set_status(StatusCode::MULTI_STATUS);
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static(CONTENT_TYPE_XML),
            );
            return Ok(resp);
        }

        // 默认：返回 DeltaV 版本列表（保持原有能力）
        let files = self.storage.list_files().await.map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("列出文件失败: {}", e),
            )
        })?;
        let file_id = files
            .iter()
            .find(|m| m.path == path)
            .map(|m| m.id.clone())
            .unwrap_or_else(|| path.trim_start_matches('/').to_string());
        let versions = self
            .version_manager
            .list_versions(&file_id)
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("获取版本失败: {}", e),
                )
            })?;

        let mut xml = String::new();
        xml.push_str(XML_HEADER);
        xml.push_str("<D:multistatus xmlns:D=\"DAV:\">");
        for v in versions {
            xml.push_str(&format!(
                "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:version-name>{}</D:version-name><D:version-created>{}</D:version-created></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
                self.build_full_href(&path), v.version_id, v.created_at
            ));
        }
        xml.push_str(XML_MULTISTATUS_END);

        let mut resp = Response::text(&xml);
        resp.set_status(StatusCode::MULTI_STATUS);
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static(CONTENT_TYPE_XML),
        );
        Ok(resp)
    }
}

impl WebDavHandler {
    fn parse_sync_collection_request(xml: &[u8]) -> (Option<usize>, Option<chrono::NaiveDateTime>) {
        let mut reader = Reader::from_reader(xml);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        let mut limit: Option<usize> = None;
        let mut in_nresults = false;
        let mut since: Option<chrono::NaiveDateTime> = None;
        let mut in_sync_token = false;
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_ascii_lowercase();
                    if name.ends_with("nresults") { in_nresults = true; }
                    if name.ends_with("sync-token") { in_sync_token = true; }
                }
                Ok(Event::End(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_ascii_lowercase();
                    if name.ends_with("nresults") { in_nresults = false; }
                    if name.ends_with("sync-token") { in_sync_token = false; }
                }
                Ok(Event::Text(t)) => {
                    let s = String::from_utf8_lossy(&t.into_inner()).to_string();
                    if in_nresults {
                        if let Ok(n) = s.trim().parse::<usize>() { limit = Some(n); }
                    }
                    if in_sync_token {
                        if let Some(ts) = Self::parse_sync_token_time(&s) { since = Some(ts); }
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
        (limit, since)
    }

    fn parse_sync_token_time(token: &str) -> Option<chrono::NaiveDateTime> {
        if let Some(pos) = token.rfind(':') {
            let ts = &token[pos + 1..];
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S%.f") { return Some(dt); }
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S") { return Some(dt); }
        }
        None
    }

    fn modified_after(path: &std::path::Path, since: Option<chrono::NaiveDateTime>) -> bool {
        let Some(since) = since else { return true };
        if let Ok(meta) = std::fs::metadata(path) {
            if let Ok(m) = meta.modified() {
                if let Ok(dur) = m.duration_since(std::time::UNIX_EPOCH) {
                    if let Some(dt) = chrono::NaiveDateTime::from_timestamp_opt(dur.as_secs() as i64, 0) {
                        return dt > since;
                    }
                }
            }
        }
        false
    }

    async fn walk_propfind_recursive_filtered(
        &self,
        dir_path: &std::path::Path,
        relative: &str,
        xml: &mut String,
        since: Option<chrono::NaiveDateTime>,
        count_left: &mut usize,
    ) -> silent::Result<()> {
        if *count_left == 0 { return Ok(()); }
        let mut stack: Vec<(std::path::PathBuf, String)> = vec![(dir_path.to_path_buf(), relative.to_string())];
        while let Some((cur_dir, cur_rel)) = stack.pop() {
            if *count_left == 0 { break; }
            let mut rd = fs::read_dir(&cur_dir).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("读取目录失败: {}", e),
                )
            })?;
            // 收集为向量，便于后续遍历与压栈
            let mut items: Vec<(std::path::PathBuf, String, bool)> = Vec::new();
            while let Some(entry) = rd.next_entry().await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("读取目录项失败: {}", e),
                )
            })? {
                let p = entry.path();
                let rel = if cur_rel.is_empty() || cur_rel == "/" {
                    format!("/{}", entry.file_name().to_string_lossy())
                } else {
                    format!("{}/{}", cur_rel, entry.file_name().to_string_lossy())
                };
                let is_dir = p.is_dir();
                items.push((p, rel, is_dir));
            }
            for (p, rel, is_dir) in items.into_iter() {
                if *count_left == 0 { break; }
                if Self::modified_after(&p, since) {
                    let href = self.build_full_href(&rel);
                    Self::add_prop_response(xml, &href, &p, is_dir).await;
                    *count_left = count_left.saturating_sub(1);
                }
                if is_dir {
                    stack.push((p, rel));
                }
            }
        }
        Ok(())
    }
}
