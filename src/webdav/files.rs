use super::{WebDavHandler, constants::*};
use crate::models::{EventType, FileEvent};
use http_body_util::BodyExt;
use silent::prelude::*;
use std::path::Path;
use tokio::fs;

impl WebDavHandler {
    pub(super) async fn handle_options(&self) -> silent::Result<Response> {
        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::HeaderName::from_static(HEADER_DAV),
            http::HeaderValue::from_static(HEADER_DAV_VALUE),
        );
        resp.headers_mut().insert(
            http::header::ALLOW,
            http::HeaderValue::from_static(HEADER_ALLOW_VALUE),
        );
        Ok(resp)
    }

    pub(super) async fn handle_propfind(
        &self,
        path: &str,
        req: &Request,
    ) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let depth = req
            .headers()
            .get("Depth")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("0");
        let storage_path = self.storage.get_full_path(&path);
        let metadata = fs::metadata(&storage_path)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "路径不存在"))?;

        let mut xml = String::new();
        xml.push_str(XML_HEADER);
        xml.push_str(XML_NS_DAV);
        if metadata.is_dir() {
            let full_href = self.build_full_href(&path);
            Self::add_prop_response(&mut xml, &full_href, &storage_path, true).await;
            if depth != "0" {
                if depth.eq_ignore_ascii_case("infinity") {
                    self.walk_propfind_recursive(&storage_path, &path, &mut xml)
                        .await?;
                } else {
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
                        let full_href = self.build_full_href(&relative_path);
                        let is_dir = entry_path.is_dir();
                        Self::add_prop_response(&mut xml, &full_href, &entry_path, is_dir).await;
                    }
                }
            }
        } else {
            let full_href = self.build_full_href(&path);
            Self::add_prop_response(&mut xml, &full_href, &storage_path, false).await;
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

    pub(super) async fn add_prop_response(xml: &mut String, href: &str, path: &Path, is_dir: bool) {
        let metadata = match fs::metadata(path).await {
            Ok(m) => m,
            Err(_) => return,
        };
        let href_encoded = urlencoding::encode(href);
        let href_with_slash = if is_dir && !href.ends_with('/') {
            format!("{}/", href_encoded)
        } else {
            href_encoded.to_string()
        };
        xml.push_str("<D:response>");
        xml.push_str(&format!("<D:href>{}</D:href>", href_with_slash));
        xml.push_str("<D:propstat>");
        xml.push_str("<D:prop>");
        if is_dir {
            xml.push_str("<D:resourcetype><D:collection/></D:resourcetype>");
        } else {
            xml.push_str("<D:resourcetype/>");
            xml.push_str(&format!(
                "<D:getcontentlength>{}</D:getcontentlength>",
                metadata.len()
            ));
            if let Some(ext) = path.extension() {
                let mime = mime_guess::from_ext(&ext.to_string_lossy()).first_or_octet_stream();
                xml.push_str(&format!("<D:getcontenttype>{}</D:getcontenttype>", mime));
            }
            if let Some(etag) = Self::calc_etag_from_meta(&metadata) {
                xml.push_str(&format!("<D:getetag>{}</D:getetag>", etag));
            }
        }
        if let Ok(modified) = metadata.modified()
            && let Ok(datetime) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            let timestamp = chrono::DateTime::from_timestamp(datetime.as_secs() as i64, 0);
            if let Some(dt) = timestamp {
                xml.push_str(&format!(
                    "<D:getlastmodified>{}</D:getlastmodified>",
                    dt.format("%a, %d %b %Y %H:%M:%S GMT")
                ));
            }
        }
        xml.push_str("</D:prop>");
        xml.push_str("<D:status>HTTP/1.1 200 OK</D:status>");
        xml.push_str("</D:propstat>");
        xml.push_str("</D:response>");
    }

    fn calc_etag_from_meta(metadata: &std::fs::Metadata) -> Option<String> {
        let len = metadata.len();
        let ts = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some(format!("\"{}-{}\"", len, ts))
    }

    pub(super) async fn walk_propfind_recursive(
        &self,
        storage_dir: &Path,
        relative_dir: &str,
        xml: &mut String,
    ) -> silent::Result<()> {
        let mut stack: Vec<(std::path::PathBuf, String)> =
            vec![(storage_dir.to_path_buf(), relative_dir.to_string())];
        while let Some((dir_path, rel_path)) = stack.pop() {
            let mut entries = fs::read_dir(&dir_path).await.map_err(|e| {
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
                let relative_path = if rel_path.is_empty() || rel_path == "/" {
                    format!("/{}", entry.file_name().to_string_lossy())
                } else {
                    format!("{}/{}", rel_path, entry.file_name().to_string_lossy())
                };
                let full_href = self.build_full_href(&relative_path);
                let is_dir = entry_path.is_dir();
                Self::add_prop_response(xml, &full_href, &entry_path, is_dir).await;
                if is_dir {
                    stack.push((entry_path, relative_path));
                }
            }
        }
        Ok(())
    }

    pub(super) async fn handle_head(&self, path: &str, req: &Request) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let storage_path = self.storage.get_full_path(&path);
        let metadata = fs::metadata(&storage_path)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "文件不存在"))?;
        let mut resp = Response::empty();
        if metadata.is_dir() {
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static(CONTENT_TYPE_HTML),
            );
        } else {
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/octet-stream"),
            );
            resp.headers_mut().insert(
                http::header::CONTENT_LENGTH,
                http::HeaderValue::from_str(&metadata.len().to_string()).unwrap(),
            );
            if let Some(ext) = storage_path.extension() {
                let mime = mime_guess::from_ext(&ext.to_string_lossy()).first_or_octet_stream();
                resp.headers_mut().insert(
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_str(mime.as_ref()).unwrap_or_else(|_| {
                        http::HeaderValue::from_static("application/octet-stream")
                    }),
                );
            }
            if let Some(etag) = Self::calc_etag_from_meta(&metadata) {
                if let Ok(val) = http::HeaderValue::from_str(&etag) {
                    resp.headers_mut().insert(http::header::ETAG, val);
                }
                if let Some(if_none_match) = req
                    .headers()
                    .get("If-None-Match")
                    .and_then(|h| h.to_str().ok())
                {
                    let matches = if_none_match == "*"
                        || if_none_match
                            .split(',')
                            .map(|s| s.trim())
                            .any(|t| t == etag);
                    if matches {
                        resp.set_status(StatusCode::NOT_MODIFIED);
                        return Ok(resp);
                    }
                }
            }
            if let Ok(modified) = metadata.modified()
                && let Ok(datetime) = modified.duration_since(std::time::UNIX_EPOCH)
                && let Some(dt) = chrono::DateTime::from_timestamp(datetime.as_secs() as i64, 0)
                && let Ok(last_modified) =
                    http::HeaderValue::from_str(&dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
            {
                resp.headers_mut()
                    .insert(http::header::LAST_MODIFIED, last_modified);
            }
        }
        Ok(resp)
    }

    pub(super) async fn handle_get(&self, path: &str, req: &Request) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let storage_path = self.storage.get_full_path(&path);
        let metadata = fs::metadata(&storage_path)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "文件不存在"))?;
        if metadata.is_dir() {
            let mut resp = Response::empty();
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static(CONTENT_TYPE_HTML),
            );
            resp.set_body(full(b"<!DOCTYPE html><html><body><h1>Directory</h1><p>Use PROPFIND to list contents.</p></body></html>".to_vec()));
            return Ok(resp);
        }
        if let Some(etag) = Self::calc_etag_from_meta(&metadata)
            && let Some(if_none_match) = req
                .headers()
                .get("If-None-Match")
                .and_then(|h| h.to_str().ok())
        {
            let matches = if_none_match == "*"
                || if_none_match
                    .split(',')
                    .map(|s| s.trim())
                    .any(|t| t == etag);
            if matches {
                let mut resp = Response::empty();
                if let Ok(val) = http::HeaderValue::from_str(&etag) {
                    resp.headers_mut().insert(http::header::ETAG, val);
                }
                if let Ok(modified) = metadata.modified()
                    && let Ok(datetime) = modified.duration_since(std::time::UNIX_EPOCH)
                    && let Some(dt) = chrono::DateTime::from_timestamp(datetime.as_secs() as i64, 0)
                    && let Ok(last_modified) = http::HeaderValue::from_str(
                        &dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string(),
                    )
                {
                    resp.headers_mut()
                        .insert(http::header::LAST_MODIFIED, last_modified);
                }
                resp.set_status(StatusCode::NOT_MODIFIED);
                return Ok(resp);
            }
        }
        let data = fs::read(&storage_path).await.map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("读取文件失败: {}", e),
            )
        })?;
        let mut resp = Response::empty();
        if let Some(ext) = storage_path.extension() {
            let mime = mime_guess::from_ext(&ext.to_string_lossy()).first_or_octet_stream();
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_str(mime.as_ref())
                    .unwrap_or_else(|_| http::HeaderValue::from_static("application/octet-stream")),
            );
        } else {
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/octet-stream"),
            );
        }
        resp.headers_mut().insert(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_str(&data.len().to_string()).unwrap(),
        );
        if let Some(etag) = Self::calc_etag_from_meta(&metadata)
            && let Ok(val) = http::HeaderValue::from_str(&etag)
        {
            resp.headers_mut().insert(http::header::ETAG, val);
        }
        if let Ok(modified) = metadata.modified()
            && let Ok(datetime) = modified.duration_since(std::time::UNIX_EPOCH)
            && let Some(dt) = chrono::DateTime::from_timestamp(datetime.as_secs() as i64, 0)
            && let Ok(last_modified) =
                http::HeaderValue::from_str(&dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
        {
            resp.headers_mut()
                .insert(http::header::LAST_MODIFIED, last_modified);
        }
        resp.set_body(full(data));
        Ok(resp)
    }

    pub(super) async fn handle_put(
        &self,
        path: &str,
        req: &mut Request,
    ) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        self.ensure_lock_ok(&path, req).await?;
        let body = req.take_body();
        let body_data = match body {
            ReqBody::Incoming(body) => body
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
            ReqBody::Empty => {
                return Err(SilentError::business_error(
                    StatusCode::BAD_REQUEST,
                    "请求体为空",
                ));
            }
        };
        let storage_path = self.storage.get_full_path(&path);
        if let Some(parent) = storage_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("创建目录失败: {}", e),
                )
            })?;
        }
        let metadata = self
            .storage
            .save_at_path(&path, &body_data)
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("写入文件失败: {}", e),
                )
            })?;
        let file_id = metadata.id.clone();
        if let Err(e) = self
            .version_manager
            .create_version(
                &file_id,
                crate::models::FileVersion::from_metadata(&metadata, Some("webdav".to_string())),
            )
            .await
        {
            tracing::debug!("创建版本失败(可忽略): {}", e);
        }
        // 发布事件
        let mut event = FileEvent::new(EventType::Created, file_id, Some(metadata));
        event.source_http_addr = Some(self.source_http_addr.clone());
        if let Some(ref n) = self.notifier {
            let _ = n.notify_created(event).await;
        }
        let mut resp = Response::empty();
        resp.set_status(StatusCode::CREATED);
        Ok(resp)
    }

    pub(super) async fn handle_delete(&self, path: &str) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let storage_path = self.storage.get_full_path(&path);
        let metadata = fs::metadata(&storage_path)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "路径不存在"))?;
        if metadata.is_dir() {
            fs::remove_dir_all(&storage_path).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("删除目录失败: {}", e),
                )
            })?;
        } else {
            fs::remove_file(&storage_path).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("删除文件失败: {}", e),
                )
            })?;
        }
        let file_id = scru128::new_string();
        let mut event = FileEvent::new(EventType::Deleted, file_id, None);
        if let Ok(host) = std::env::var("ADVERTISE_HOST").or_else(|_| std::env::var("HOSTNAME")) {
            event.source_http_addr = Some(format!(
                "http://{}:{}",
                host,
                std::env::var("HTTP_PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(8081 - 1)
            ));
        }
        if let Some(ref n) = self.notifier {
            let _ = n.notify_deleted(event).await;
        }
        let mut resp = Response::empty();
        resp.set_status(StatusCode::NO_CONTENT);
        Ok(resp)
    }

    pub(super) async fn handle_mkcol(&self, path: &str) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let storage_path = self.storage.get_full_path(&path);
        if storage_path.exists() {
            return Err(SilentError::business_error(
                StatusCode::METHOD_NOT_ALLOWED,
                "路径已存在",
            ));
        }
        fs::create_dir_all(&storage_path).await.map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("创建目录失败: {}", e),
            )
        })?;
        let mut resp = Response::empty();
        resp.set_status(StatusCode::CREATED);
        Ok(resp)
    }

    pub(super) async fn handle_move(&self, path: &str, req: &Request) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        self.ensure_lock_ok(&path, req).await?;
        let dest = req
            .headers()
            .get("Destination")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                SilentError::business_error(StatusCode::BAD_REQUEST, "缺少 Destination 头")
            })?;
        let dest_path = self.extract_path_from_url(dest)?;
        let storage_path = self.storage.get_full_path(&path);
        let dest_storage_path = self.storage.get_full_path(&dest_path);
        if let Some(parent) = dest_storage_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("创建目标目录失败: {}", e),
                )
            })?;
        }
        fs::rename(&storage_path, &dest_storage_path)
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("移动失败: {}", e),
                )
            })?;
        // 发布事件
        let file_id = scru128::new_string();
        let mut event = FileEvent::new(EventType::Modified, file_id, None);
        if let Ok(host) = std::env::var("ADVERTISE_HOST").or_else(|_| std::env::var("HOSTNAME")) {
            event.source_http_addr = Some(format!(
                "http://{}:{}",
                host,
                std::env::var("HTTP_PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(8081 - 1)
            ));
        }
        if let Some(ref n) = self.notifier {
            let _ = n.notify_created(event).await;
        }
        let mut resp = Response::empty();
        resp.set_status(StatusCode::CREATED);
        Ok(resp)
    }

    pub(super) async fn handle_copy(&self, path: &str, req: &Request) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        self.ensure_lock_ok(&path, req).await?;
        let dest = req
            .headers()
            .get("Destination")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                SilentError::business_error(StatusCode::BAD_REQUEST, "缺少 Destination 头")
            })?;
        let dest_path = self.extract_path_from_url(dest)?;
        let src_storage_path = self.storage.get_full_path(&path);
        let dest_storage_path = self.storage.get_full_path(&dest_path);
        let metadata = fs::metadata(&src_storage_path)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "源路径不存在"))?;
        if let Some(parent) = dest_storage_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("创建目标目录失败: {}", e),
                )
            })?;
        }
        if metadata.is_dir() {
            Self::copy_dir_all(&src_storage_path, &dest_storage_path)
                .await
                .map_err(|e| {
                    SilentError::business_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("复制目录失败: {}", e),
                    )
                })?;
        } else {
            fs::copy(&src_storage_path, &dest_storage_path)
                .await
                .map_err(|e| {
                    SilentError::business_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("复制文件失败: {}", e),
                    )
                })?;
        }
        let mut resp = Response::empty();
        resp.set_status(StatusCode::CREATED);
        Ok(resp)
    }

    pub(super) fn extract_path_from_url(&self, url: &str) -> silent::Result<String> {
        let path = if let Some(idx) = url.find("://") {
            if let Some(path_start) = url[idx + 3..].find('/') {
                &url[idx + 3 + path_start..]
            } else {
                "/"
            }
        } else if url.starts_with('/') {
            url
        } else {
            return Err(SilentError::business_error(
                StatusCode::BAD_REQUEST,
                "无效的目标 URL",
            ));
        };
        let relative_path = path.strip_prefix(&self.base_path).unwrap_or(path);
        urlencoding::decode(relative_path)
            .map(|s| s.to_string())
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::BAD_REQUEST,
                    format!("目标路径解码失败: {}", e),
                )
            })
    }

    pub(super) async fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dst).await?;
        let mut entries = fs::read_dir(src).await?;
        while let Some(entry) = entries.next_entry().await? {
            let ty = entry.file_type().await?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if ty.is_dir() {
                Box::pin(Self::copy_dir_all(&src_path, &dst_path)).await?;
            } else {
                fs::copy(&src_path, &dst_path).await?;
            }
        }
        Ok(())
    }
}
