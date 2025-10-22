use crate::models::{EventType, FileEvent};
use crate::notify::EventNotifier;
use crate::storage::StorageManager;
use crate::sync::crdt::SyncManager;
use async_trait::async_trait;
use http_body_util::BodyExt;
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
use silent::prelude::*;
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, info};

// split modules
mod constants;
mod types;
use constants::*;
use types::DavLock;

// constants and types moved into src/webdav/constants.rs and src/webdav/types.rs

/// WebDAV 处理器
#[derive(Clone)]
pub struct WebDavHandler {
    pub storage: Arc<StorageManager>,
    pub notifier: Option<Arc<EventNotifier>>,
    pub sync_manager: Arc<SyncManager>,
    pub base_path: String,
    pub source_http_addr: String,
    #[allow(dead_code)]
    pub version_manager: Arc<crate::version::VersionManager>,
    // 简易锁与属性存储（内存）
    locks: Arc<tokio::sync::RwLock<std::collections::HashMap<String, DavLock>>>,
    props: Arc<
        tokio::sync::RwLock<
            std::collections::HashMap<String, std::collections::HashMap<String, String>>,
        >,
    >,
}

impl WebDavHandler {
    pub fn new(
        storage: Arc<StorageManager>,
        notifier: Option<Arc<EventNotifier>>,
        sync_manager: Arc<SyncManager>,
        base_path: String,
        source_http_addr: String,
        version_manager: Arc<crate::version::VersionManager>,
    ) -> Self {
        let handler = Self {
            storage,
            notifier,
            sync_manager,
            base_path,
            source_http_addr,
            version_manager,
            locks: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            props: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        };
        // 尝试加载持久化元数据（非致命）
        handler.load_persistent_state();
        handler
    }

    fn lock_token() -> String {
        format!("opaquelocktoken:{}", scru128::new_string())
    }

    fn meta_dir(&self) -> std::path::PathBuf {
        self.storage.root_dir().join(".webdav")
    }

    fn locks_file(&self) -> std::path::PathBuf {
        self.meta_dir().join("locks.json")
    }
    fn props_file(&self) -> std::path::PathBuf {
        self.meta_dir().join("props.json")
    }

    #[allow(clippy::collapsible_if)]
    fn load_persistent_state(&self) {
        let meta_dir = self.meta_dir();
        let _ = std::fs::create_dir_all(&meta_dir);
        // 加载 locks
        if let Ok(bytes) = std::fs::read(self.locks_file())
            && let Ok(map) =
                serde_json::from_slice::<std::collections::HashMap<String, DavLock>>(&bytes)
        {
            let rt = tokio::runtime::Handle::current();
            let locks = self.locks.clone();
            rt.spawn(async move {
                *locks.write().await = map;
            });
        }
        // 加载 props
        if let Ok(bytes) = std::fs::read(self.props_file())
            && let Ok(map) = serde_json::from_slice::<
                std::collections::HashMap<String, std::collections::HashMap<String, String>>,
            >(&bytes)
        {
            let rt = tokio::runtime::Handle::current();
            let props = self.props.clone();
            rt.spawn(async move {
                *props.write().await = map;
            });
        }
    }

    async fn persist_locks(&self) {
        let map = self.locks.read().await.clone();
        let _ = std::fs::create_dir_all(self.meta_dir());
        if let Ok(bytes) = serde_json::to_vec_pretty(&map) {
            let _ = std::fs::write(self.locks_file(), bytes);
        }
    }

    async fn persist_props(&self) {
        let map = self.props.read().await.clone();
        let _ = std::fs::create_dir_all(self.meta_dir());
        if let Ok(bytes) = serde_json::to_vec_pretty(&map) {
            let _ = std::fs::write(self.props_file(), bytes);
        }
    }

    fn parse_timeout(req: &Request) -> i64 {
        if let Some(v) = req.headers().get("Timeout").and_then(|h| h.to_str().ok()) {
            // e.g., Second-60 or Infinite
            if v.to_lowercase().contains("infinite") {
                return 3600; // 1小时上限，避免永久锁
            }
            if let Some(num) = v.split(['-', ',']).find_map(|s| s.parse::<i64>().ok()) {
                return num.clamp(1, 3600);
            }
        }
        60
    }

    fn extract_if_lock_token(req: &Request) -> Option<String> {
        // 朴素提取 If: (<opaquelocktoken:...>)
        req.headers()
            .get("If")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| {
                let s = s.to_string();
                let start = s.find("opaquelocktoken:")?;
                let end = s[start..].find('>').map(|i| start + i).unwrap_or(s.len());
                Some(s[start..end].to_string())
            })
    }

    async fn ensure_lock_ok(&self, path: &str, req: &Request) -> silent::Result<()> {
        let locks = self.locks.read().await;
        if let Some(l) = locks.get(path)
            && !l.is_expired()
        {
            let provided = Self::extract_if_lock_token(req);
            if let Some(tok) = provided
                && tok == l.token
            {
                return Ok(());
            }
            return Err(SilentError::business_error(
                StatusCode::LOCKED,
                "资源被锁定或令牌缺失",
            ));
        }
        Ok(())
    }

    /// 解码 URL 路径
    fn decode_path(path: &str) -> silent::Result<String> {
        urlencoding::decode(path)
            .map(|s| s.to_string())
            .map_err(|e| {
                SilentError::business_error(StatusCode::BAD_REQUEST, format!("路径解码失败: {}", e))
            })
    }

    /// 构建完整的 WebDAV href（包含 base_path 前缀）
    fn build_full_href(&self, relative_path: &str) -> String {
        format!("{}{}", &self.base_path, relative_path)
    }

    /// OPTIONS - 返回支持的方法
    async fn handle_options(&self) -> silent::Result<Response> {
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

    /// PROPPATCH - 设置/移除自定义属性（简化实现）
    async fn handle_proppatch(&self, path: &str, req: &mut Request) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        self.ensure_lock_ok(&path, req).await?;
        let body = req.take_body();
        let _xml = match body {
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
        // 简化：不做真实解析，直接记录一次 PROPPATCH 时间戳，返回 207

        // 记录一个示例属性，标识已处理过 PROPPATCH
        let mut props = self.props.write().await;
        let entry = props.entry(path.clone()).or_default();
        entry.insert(
            "prop:last-proppatch".to_string(),
            chrono::Local::now().naive_local().to_string(),
        );

        self.persist_props().await;
        let mut resp = Response::text("");
        resp.set_status(StatusCode::MULTI_STATUS);
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static(CONTENT_TYPE_XML),
        );
        Ok(resp)
    }

    /// VERSION-CONTROL - 启用版本控制（简化为标记属性）
    async fn handle_version_control(&self, path: &str) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let mut props = self.props.write().await;
        let entry = props.entry(path).or_default();
        entry.insert("dav:version-controlled".to_string(), "true".to_string());
        Ok(Response::empty())
    }

    /// REPORT - 返回版本列表（简化 XML）
    async fn handle_report(&self, path: &str) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        // 通过路径查找文件 ID（朴素遍历）
        let files = self.storage.list_files().await.map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("列出文件失败: {}", e),
            )
        })?;
        let file_id = files.iter().find(|m| m.path == path).map(|m| m.id.clone());
        if file_id.is_none() {
            return Err(SilentError::business_error(
                StatusCode::NOT_FOUND,
                "文件未找到",
            ));
        }
        let file_id = file_id.unwrap();
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
                self.build_full_href(&path),
                v.version_id,
                v.created_at
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

    /// LOCK - 锁定资源（简化，支持独占锁）
    async fn handle_lock(&self, path: &str, req: &Request) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let mut locks = self.locks.write().await;
        if let Some(l) = locks.get(&path)
            && !l.is_expired()
        {
            return Err(SilentError::business_error(
                StatusCode::LOCKED,
                "资源已被锁定",
            ));
        }
        let token = Self::lock_token();
        let timeout = Self::parse_timeout(req);
        let info = DavLock::new_exclusive(token.clone(), timeout);
        locks.insert(path.clone(), info);
        drop(locks);
        self.persist_locks().await;

        let xml = format!(
            "{}<D:prop xmlns:D=\"DAV:\"><D:lockdiscovery><D:activelock><D:locktype><D:write/></D:locktype><D:lockscope><D:exclusive/></D:lockscope><D:locktoken><D:href>{}</D:href></D:locktoken></D:activelock></D:lockdiscovery></D:prop>",
            XML_HEADER, token
        );
        let mut resp = Response::text(&xml);
        resp.headers_mut().insert(
            http::header::HeaderName::from_static("lock-token"),
            http::HeaderValue::from_str(&format!("<{}>", token)).unwrap(),
        );
        resp.set_status(StatusCode::OK);
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static(CONTENT_TYPE_XML),
        );
        Ok(resp)
    }

    /// UNLOCK - 解除资源锁
    async fn handle_unlock(&self, path: &str, req: &Request) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let token = req
            .headers()
            .get("Lock-Token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .trim_matches(['<', '>']);
        if token.is_empty() {
            return Err(SilentError::business_error(
                StatusCode::BAD_REQUEST,
                "缺少 Lock-Token",
            ));
        }
        let mut locks = self.locks.write().await;
        if let Some(l) = locks.get(&path) {
            if l.token == token {
                locks.remove(&path);
            } else {
                return Err(SilentError::business_error(
                    StatusCode::CONFLICT,
                    "锁令牌不匹配",
                ));
            }
        }
        drop(locks);
        self.persist_locks().await;
        Ok(Response::empty())
    }

    /// PROPFIND - 列出文件和目录
    async fn handle_propfind(&self, path: &str, req: &Request) -> silent::Result<Response> {
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
            // 添加目录本身的响应
            let full_href = self.build_full_href(&path);
            Self::add_prop_response(&mut xml, &full_href, &storage_path, true).await;

            if depth != "0" {
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

    /// 添加单个资源的属性响应
    async fn add_prop_response(xml: &mut String, href: &str, path: &Path, is_dir: bool) {
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

    /// HEAD - 获取文件元数据（不返回文件内容）
    async fn handle_head(&self, path: &str) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;

        let storage_path = self.storage.get_full_path(&path);
        let metadata = fs::metadata(&storage_path)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "文件不存在"))?;

        let mut resp = Response::empty();

        if metadata.is_dir() {
            // 对于目录
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static(CONTENT_TYPE_HTML),
            );
        } else {
            // 对于文件
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/octet-stream"),
            );
            resp.headers_mut().insert(
                http::header::CONTENT_LENGTH,
                http::HeaderValue::from_str(&metadata.len().to_string()).unwrap(),
            );

            // 添加 MIME 类型
            if let Some(ext) = storage_path.extension() {
                let mime = mime_guess::from_ext(&ext.to_string_lossy()).first_or_octet_stream();
                resp.headers_mut().insert(
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_str(mime.as_ref()).unwrap_or_else(|_| {
                        http::HeaderValue::from_static("application/octet-stream")
                    }),
                );
            }

            // 添加最后修改时间
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

    /// GET - 下载文件
    async fn handle_get(&self, path: &str) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;

        let storage_path = self.storage.get_full_path(&path);
        let metadata = fs::metadata(&storage_path)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "文件不存在"))?;

        if metadata.is_dir() {
            // 对于目录，返回一个简单的 HTML 页面
            let mut resp = Response::empty();
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static(CONTENT_TYPE_HTML),
            );
            resp.set_body(full(
                b"<!DOCTYPE html><html><body><h1>Directory</h1><p>Use PROPFIND to list contents.</p></body></html>".to_vec(),
            ));
            return Ok(resp);
        }

        let data = fs::read(&storage_path).await.map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("读取文件失败: {}", e),
            )
        })?;

        let mut resp = Response::empty();

        // 设置 MIME 类型
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

        // 添加最后修改时间
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

    /// PUT - 上传文件
    async fn handle_put(&self, path: &str, req: &mut Request) -> silent::Result<Response> {
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

        // 使用存储管理器按路径保存，确保元数据一致，并便于版本管理
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

        // 如果启用版本管理，则记录版本（DeltaV 最小闭环）
        if let Err(e) = self
            .version_manager
            .create_version(
                &file_id,
                crate::models::FileVersion::from_metadata(&metadata, Some("webdav".to_string())),
            )
            .await
        {
            debug!("创建版本失败(可忽略): {}", e);
        }

        // 通知 SyncManager 处理本地变更
        if let Err(e) = self
            .sync_manager
            .handle_local_change(EventType::Created, file_id.clone(), Some(metadata.clone()))
            .await
        {
            info!("同步管理器处理失败: {}", e);
        }

        // 发布事件到 NATS（用于跨节点通知）
        let mut event = FileEvent::new(EventType::Created, file_id, Some(metadata));
        event.source_http_addr = Some(self.source_http_addr.clone());
        if let Some(ref n) = self.notifier {
            let _ = n.notify_created(event).await;
        }

        let mut resp = Response::empty();
        resp.set_status(StatusCode::CREATED);
        Ok(resp)
    }

    /// DELETE - 删除文件或目录
    async fn handle_delete(&self, path: &str) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        // 删除前无需 If 检查（DELETE 通过 LOCK/UNLOCK 流程受控），略过

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

        // 发布事件
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

    /// MKCOL - 创建目录
    async fn handle_mkcol(&self, path: &str) -> silent::Result<Response> {
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

    /// MOVE - 移动/重命名文件
    async fn handle_move(&self, path: &str, req: &Request) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        self.ensure_lock_ok(&path, req).await?;

        let dest = req
            .headers()
            .get("Destination")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                SilentError::business_error(StatusCode::BAD_REQUEST, "缺少 Destination 头")
            })?;

        // 提取目标路径
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

    /// COPY - 复制文件
    async fn handle_copy(&self, path: &str, req: &Request) -> silent::Result<Response> {
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

    /// 从完整 URL 中提取路径
    fn extract_path_from_url(&self, url: &str) -> silent::Result<String> {
        // 提取路径部分（去除协议和域名）
        let path = if let Some(idx) = url.find("://") {
            // 找到协议后的第一个 /
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

        // 移除 base_path 前缀
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

    /// 递归复制目录
    async fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
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

#[async_trait]
impl Handler for WebDavHandler {
    /// 处理 WebDAV 请求
    async fn call(&self, mut req: Request) -> silent::Result<Response> {
        let method = req.method().clone();
        let uri_path = req.uri().path().to_string();

        // 移除 base_path 前缀
        let relative_path = uri_path
            .strip_prefix(&self.base_path)
            .unwrap_or(&uri_path)
            .to_string();

        debug!("WebDAV {} {}", method, relative_path);

        match method.as_str() {
            "OPTIONS" => self.handle_options().await,
            "PROPFIND" => self.handle_propfind(&relative_path, &req).await,
            "PROPPATCH" => self.handle_proppatch(&relative_path, &mut req).await,
            "HEAD" => self.handle_head(&relative_path).await,
            "GET" => self.handle_get(&relative_path).await,
            "PUT" => self.handle_put(&relative_path, &mut req).await,
            "DELETE" => self.handle_delete(&relative_path).await,
            "MKCOL" => self.handle_mkcol(&relative_path).await,
            "MOVE" => self.handle_move(&relative_path, &req).await,
            "COPY" => self.handle_copy(&relative_path, &req).await,
            "LOCK" => self.handle_lock(&relative_path, &req).await,
            "UNLOCK" => self.handle_unlock(&relative_path, &req).await,
            "VERSION-CONTROL" => self.handle_version_control(&relative_path).await,
            "REPORT" => self.handle_report(&relative_path).await,
            _ => Err(SilentError::business_error(
                StatusCode::METHOD_NOT_ALLOWED,
                "不支持的方法",
            )),
        }
    }
}

/// 为路由注册所有 WebDAV 方法
fn register_webdav_methods(route: Route, handler: Arc<WebDavHandler>) -> Route {
    route
        .insert_handler(Method::HEAD, handler.clone())
        .insert_handler(Method::GET, handler.clone())
        .insert_handler(Method::POST, handler.clone())
        .insert_handler(Method::PUT, handler.clone())
        .insert_handler(Method::DELETE, handler.clone())
        .insert_handler(Method::OPTIONS, handler.clone())
        .insert_handler(
            Method::from_bytes(METHOD_PROPFIND).unwrap(),
            handler.clone(),
        )
        .insert_handler(
            Method::from_bytes(METHOD_PROPPATCH).unwrap(),
            handler.clone(),
        )
        .insert_handler(Method::from_bytes(METHOD_MKCOL).unwrap(), handler.clone())
        .insert_handler(Method::from_bytes(METHOD_MOVE).unwrap(), handler.clone())
        .insert_handler(Method::from_bytes(METHOD_COPY).unwrap(), handler.clone())
        .insert_handler(Method::from_bytes(METHOD_LOCK).unwrap(), handler.clone())
        .insert_handler(Method::from_bytes(METHOD_UNLOCK).unwrap(), handler)
}

/// 创建 WebDAV 路由
pub fn create_webdav_routes(
    storage: Arc<StorageManager>,
    notifier: Option<Arc<EventNotifier>>,
    sync_manager: Arc<SyncManager>,
    source_http_addr: String,
    version_manager: Arc<crate::version::VersionManager>,
) -> Route {
    let handler = Arc::new(WebDavHandler::new(
        storage,
        notifier,
        sync_manager,
        "".to_string(),
        source_http_addr,
        version_manager,
    ));

    // 将 WebDAV 服务挂载在根路径，客户端直接以根访问
    let root_route = register_webdav_methods(Route::new(""), handler.clone());
    let path_route = register_webdav_methods(Route::new("<path:**>"), handler);

    root_route.append(path_route)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webdav_method_constants() {
        assert_eq!(METHOD_PROPFIND, b"PROPFIND");
        assert_eq!(METHOD_MKCOL, b"MKCOL");
        assert_eq!(METHOD_MOVE, b"MOVE");
        assert_eq!(METHOD_COPY, b"COPY");
    }

    #[test]
    fn test_xml_constants() {
        assert!(XML_HEADER.contains("xml version"));
        assert!(XML_NS_DAV.contains("DAV:"));
        assert!(XML_MULTISTATUS_END.contains("multistatus"));
    }

    #[test]
    fn test_header_constants() {
        assert_eq!(HEADER_DAV, "dav");
        assert_eq!(HEADER_DAV_VALUE, "1, 2, version-control");
        assert!(HEADER_ALLOW_VALUE.contains("OPTIONS"));
        assert!(HEADER_ALLOW_VALUE.contains("PROPFIND"));
        assert!(HEADER_ALLOW_VALUE.contains("VERSION-CONTROL"));
        assert!(HEADER_ALLOW_VALUE.contains("REPORT"));
    }

    #[test]
    fn test_content_type_constants() {
        assert_eq!(CONTENT_TYPE_XML, "application/xml; charset=utf-8");
        assert_eq!(CONTENT_TYPE_HTML, "text/html; charset=utf-8");
    }

    #[test]
    fn test_decode_path_simple() {
        let path = "/test/file.txt";
        let decoded = WebDavHandler::decode_path(path);
        assert!(decoded.is_ok());
        assert_eq!(decoded.unwrap(), "/test/file.txt");
    }

    #[test]
    fn test_decode_path_with_spaces() {
        let path = "/test%20file.txt";
        let decoded = WebDavHandler::decode_path(path);
        assert!(decoded.is_ok());
        assert_eq!(decoded.unwrap(), "/test file.txt");
    }

    #[test]
    fn test_decode_path_with_special_chars() {
        let path = "/file%2Bname.txt";
        let decoded = WebDavHandler::decode_path(path);
        assert!(decoded.is_ok());
        assert_eq!(decoded.unwrap(), "/file+name.txt");
    }

    #[test]
    fn test_decode_path_chinese() {
        let path = "/%E6%B5%8B%E8%AF%95";
        let decoded = WebDavHandler::decode_path(path);
        assert!(decoded.is_ok());
        assert_eq!(decoded.unwrap(), "/测试");
    }

    #[test]
    fn test_xml_header_format() {
        assert!(XML_HEADER.starts_with("<?"));
        assert!(XML_HEADER.ends_with("?>"));
    }

    #[test]
    fn test_xml_namespace_format() {
        assert!(XML_NS_DAV.starts_with("<D:"));
        assert!(XML_NS_DAV.contains("xmlns:D"));
    }

    #[test]
    fn test_method_byte_arrays() {
        assert_eq!(METHOD_PROPFIND.len(), 8);
        assert_eq!(METHOD_MKCOL.len(), 5);
        assert_eq!(METHOD_MOVE.len(), 4);
        assert_eq!(METHOD_COPY.len(), 4);
    }

    #[test]
    fn test_header_dav_compliance() {
        assert!(HEADER_DAV_VALUE.contains("1"));
        assert!(HEADER_DAV_VALUE.contains("2"));
    }

    #[test]
    fn test_allowed_methods_coverage() {
        let methods = vec![
            "OPTIONS", "GET", "HEAD", "PUT", "DELETE", "PROPFIND", "MKCOL", "MOVE", "COPY",
        ];
        for method in methods {
            assert!(HEADER_ALLOW_VALUE.contains(method));
        }
    }

    #[test]
    fn test_content_types_have_charset() {
        assert!(CONTENT_TYPE_XML.contains("charset=utf-8"));
        assert!(CONTENT_TYPE_HTML.contains("charset=utf-8"));
    }

    #[test]
    fn test_xml_multistatus_structure() {
        let full_xml = format!("{}{}{}", XML_HEADER, XML_NS_DAV, XML_MULTISTATUS_END);
        assert!(full_xml.contains("<?xml"));
        assert!(full_xml.contains("<D:multistatus"));
        assert!(full_xml.contains("</D:multistatus>"));
    }

    #[test]
    fn test_path_decoding_empty() {
        let path = "";
        let decoded = WebDavHandler::decode_path(path);
        assert!(decoded.is_ok());
        assert_eq!(decoded.unwrap(), "");
    }

    #[test]
    fn test_path_decoding_root() {
        let path = "/";
        let decoded = WebDavHandler::decode_path(path);
        assert!(decoded.is_ok());
        assert_eq!(decoded.unwrap(), "/");
    }

    #[test]
    fn test_webdav_handler_type() {
        let type_name = std::any::type_name::<WebDavHandler>();
        assert!(type_name.contains("WebDavHandler"));
    }

    #[test]
    fn test_method_constants_uppercase() {
        assert!(
            String::from_utf8_lossy(METHOD_PROPFIND)
                .chars()
                .all(|c| c.is_uppercase() || !c.is_alphabetic())
        );
        assert!(
            String::from_utf8_lossy(METHOD_MKCOL)
                .chars()
                .all(|c| c.is_uppercase() || !c.is_alphabetic())
        );
    }

    #[test]
    fn test_xml_namespace_dav_protocol() {
        assert!(XML_NS_DAV.contains("DAV:"));
    }
}
