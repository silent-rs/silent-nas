use super::{WebDavHandler, constants::*};
use http_body_util::BodyExt;
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
        let body_str = String::from_utf8_lossy(&xml_bytes).to_lowercase();

        if body_str.contains("sync-collection") {
            // WebDAV Sync (RFC 6578) 简化实现：返回全量条目 + 新的 sync-token
            // 支持 Depth: 1 与 infinity
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
                // 列出自身
                let href = self.build_full_href(&path);
                Self::add_prop_response(&mut xml, &href, &storage_path, true).await;
                if depth.eq_ignore_ascii_case("infinity") {
                    // 递归列出
                    self.walk_propfind_recursive(&storage_path, &path, &mut xml)
                        .await?;
                } else {
                    // 单层
                    let mut entries = fs::read_dir(&storage_path).await.map_err(|e| {
                        SilentError::business_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("读取目录失败: {}", e),
                        )
                    })?;
                    while let Some(entry) = entries.next_entry().await.map_err(|e| {
                        SilentError::business_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("读取目录项失败: {}", e),
                        )
                    })? {
                        let entry_path = entry.path();
                        let relative_path = if path.is_empty() || path == "/" {
                            format!("/{}", entry.file_name().to_string_lossy())
                        } else {
                            format!("{}/{}", path, entry.file_name().to_string_lossy())
                        };
                        let href = self.build_full_href(&relative_path);
                        let is_dir = entry_path.is_dir();
                        Self::add_prop_response(&mut xml, &href, &entry_path, is_dir).await;
                    }
                }
            } else {
                let href = self.build_full_href(&path);
                Self::add_prop_response(&mut xml, &href, &storage_path, false).await;
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
