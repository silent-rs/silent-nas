use super::{WebDavHandler, constants::*};
use http_body_util::BodyExt;
use quick_xml::events::Event;
use quick_xml::Reader;
use silent::prelude::*;
use tokio::fs;

impl WebDavHandler {
    fn parse_prop_filter(xml: &[u8]) -> Option<std::collections::HashSet<String>> {
        let mut reader = Reader::from_reader(xml);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        let mut in_prop = false;
        let mut set = std::collections::HashSet::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    let lname = name.split(':').last().unwrap_or(&name).to_lowercase();
                    if lname == "prop" { in_prop = true; }
                    else if in_prop { set.insert(lname); }
                }
                Ok(Event::End(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    let lname = name.split(':').last().unwrap_or(&name).to_lowercase();
                    if lname == "prop" { in_prop = false; }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
        if set.is_empty() { None } else { Some(set) }
    }
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

        // 版本树（version-tree）：返回目标文件的版本列表
        if body_str_lower.contains("version-tree") {
            return self.report_versions(&path).await;
        }

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
            let props_filter = Self::parse_prop_filter(&xml_bytes);
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
            let mut count_used = 0usize;
            if meta.is_dir() {
                // 列出自身（仅在全量请求时包含根目录）
                let href = self.build_full_href(&path);
                if since_token_time.is_none() {
                    self.add_prop_response(&mut xml, &href, &storage_path, true).await;
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
                        props_filter.as_ref(),
                    )
                    .await?;
                    count_used = limit.unwrap_or(usize::MAX) - count_left;
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
                            if let Some(ref filt) = props_filter {
                                self.add_prop_response_with_filter(&mut xml, &href, &entry_path, is_dir, Some(filt)).await;
                            } else {
                                self.add_prop_response(&mut xml, &href, &entry_path, is_dir).await;
                            }
                            count += 1;
                        }
                    }
                    count_used = count;
                }
            } else {
                // 单文件：若自 token 以来有变化则返回
                if Self::modified_after(&storage_path, since_token_time) || since_token_time.is_none() {
                    let href = self.build_full_href(&path);
                    if let Some(ref filt) = props_filter {
                        self.add_prop_response_with_filter(&mut xml, &href, &storage_path, false, Some(filt)).await;
                    } else {
                        self.add_prop_response(&mut xml, &href, &storage_path, false).await;
                    }
                    count_used = 1;
                }
            }

            // 追加删除差异（404）
            if let Some(since) = since_token_time {
                let remain = limit.map(|l| l.saturating_sub(count_used)).unwrap_or(usize::MAX);
                if remain > 0 {
                    let deleted = self.list_deleted_since(&path, since, remain);
                    for p in deleted {
                        let href = self.build_full_href(&p);
                        xml.push_str("<D:response>");
                        xml.push_str(&format!("<D:href>{}</D:href>", href));
                        xml.push_str("<D:status>HTTP/1.1 404 Not Found</D:status>");
                        xml.push_str("</D:response>");
                    }
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

        // 自定义过滤（silent:filter）：按 mime/时间范围/limit 过滤（Depth: 1）
        if body_str_lower.contains("silent:filter") || body_str_lower.contains("silent-filter") {
            let (mime_prefix, after, before, limit, tags) = Self::parse_filter_request(&xml_bytes);
            let storage_path = self.storage.get_full_path(&path);
            let meta = fs::metadata(&storage_path)
                .await
                .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "路径不存在"))?;
            if !meta.is_dir() {
                return Err(SilentError::business_error(StatusCode::BAD_REQUEST, "仅支持目录"));
            }
            let mut xml = String::new();
            xml.push_str(XML_HEADER);
            xml.push_str("<D:multistatus xmlns:D=\"DAV:\">");
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
                let m = fs::metadata(&entry_path).await.map_err(|e| {
                    SilentError::business_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("读取元数据失败: {}", e),
                    )
                })?;
                // 时间过滤
                if let Some(a) = after {
                    if !Self::modified_after(&entry_path, Some(a)) { continue; }
                }
                if let Some(b) = before {
                    if Self::modified_after(&entry_path, Some(b)) { continue; }
                }
                // mime 过滤（仅文件）
                if m.is_file() {
                    if let Some(pref) = &mime_prefix {
                        if let Some(ext) = entry_path.extension() {
                            let mime = mime_guess::from_ext(&ext.to_string_lossy()).first_or_octet_stream();
                            if !mime.essence_str().starts_with(pref) { continue; }
                        }
                    }
                }
                let relative_path = if path.is_empty() || path == "/" {
                    format!("/{}", entry.file_name().to_string_lossy())
                } else {
                    format!("{}/{}", path, entry.file_name().to_string_lossy())
                };
                let href = self.build_full_href(&relative_path);
                let is_dir = m.is_dir();
                // 标签过滤（结构化键）
                if !tags.is_empty() {
                    let mut pass = true;
                    let key_for_props = if is_dir { href.trim_end_matches('/').to_string() } else { href.clone() };
                    let props_map = self.props.read().await;
                    let entry_props = props_map.get(&key_for_props);
                    for (tk, tv) in &tags {
                        if let Some(em) = entry_props {
                            if let Some(val) = em.get(tk) {
                                if let Some(expect) = tv {
                                    if val != expect { pass = false; break; }
                                }
                            } else { pass = false; break; }
                        } else { pass = false; break; }
                    }
                    drop(props_map);
                    if !pass { continue; }
                }
                if let Some(filt) = Self::parse_prop_filter(&xml_bytes) {
                    self.add_prop_response_with_filter(&mut xml, &href, &entry_path, is_dir, Some(&filt)).await;
                } else {
                    self.add_prop_response(&mut xml, &href, &entry_path, is_dir).await;
                }
                count += 1;
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

        // 默认：返回版本列表（兼容旧行为）
        self.report_versions(&path).await
    }
}

impl WebDavHandler {
    async fn report_versions(&self, path: &str) -> silent::Result<Response> {
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
                self.build_full_href(path), v.version_id, v.created_at
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

    fn parse_filter_request(xml: &[u8]) -> (
        Option<String>,
        Option<chrono::NaiveDateTime>,
        Option<chrono::NaiveDateTime>,
        Option<usize>,
        Vec<(String, Option<String>)>,
    ) {
        let mut reader = Reader::from_reader(xml);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        let mut in_mime = false;
        let mut in_after = false;
        let mut in_before = false;
        let mut in_limit = false;
        let mut in_tag = false;
        let mut mime: Option<String> = None;
        let mut after: Option<chrono::NaiveDateTime> = None;
        let mut before: Option<chrono::NaiveDateTime> = None;
        let mut limit: Option<usize> = None;
        let mut tags: Vec<(String, Option<String>)> = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_ascii_lowercase();
                    in_mime = name.ends_with("mime");
                    in_after = name.ends_with("modified-after");
                    in_before = name.ends_with("modified-before");
                    in_limit = name.ends_with("limit");
                    in_tag = name.ends_with("tag");
                }
                Ok(Event::End(_)) => {
                    in_mime = false; in_after = false; in_before = false; in_limit = false; in_tag = false;
                }
                Ok(Event::Text(t)) => {
                    let s = String::from_utf8_lossy(&t.into_inner()).trim().to_string();
                    if in_mime && !s.is_empty() { mime = Some(s.clone()); }
                    if in_limit && !s.is_empty() { if let Ok(n) = s.parse() { limit = Some(n); } }
                    if in_after && !s.is_empty() {
                        if let Some(dt) = Self::parse_datetime(&s) { after = Some(dt); }
                    }
                    if in_before && !s.is_empty() {
                        if let Some(dt) = Self::parse_datetime(&s) { before = Some(dt); }
                    }
                    if in_tag && !s.is_empty() {
                        // 支持格式："ns:{URI}#{local}=value" 或 "ns:{URI}#{local}"
                        if let Some((k, v)) = s.split_once('=') {
                            tags.push((k.trim().to_string(), Some(v.trim().to_string())));
                        } else {
                            tags.push((s.clone(), None));
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
        (mime, after, before, limit, tags)
    }

    fn parse_datetime(s: &str) -> Option<chrono::NaiveDateTime> {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) { return Some(dt.naive_local()); }
        if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(s) { return Some(dt.naive_local()); }
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()
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
        props_filter: Option<&std::collections::HashSet<String>>,
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
                    if let Some(filt) = props_filter {
                        self.add_prop_response_with_filter(xml, &href, &p, is_dir, Some(filt)).await;
                    } else {
                        self.add_prop_response(xml, &href, &p, is_dir).await;
                    }
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
