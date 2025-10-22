use crate::models::{EventType, FileEvent};
use crate::notify::EventNotifier;
use crate::storage::StorageManager;
use crate::sync::crdt::SyncManager;
use async_trait::async_trait;
use http_body_util::BodyExt;
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
use silent::prelude::*;
// use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tracing::{debug, info};

// split modules
pub mod constants;
mod deltav;
mod files;
mod locks;
mod props;
mod routes;
pub mod types;

#[allow(unused_imports)]
use constants::*;
pub use routes::create_webdav_routes;
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

    pub(super) fn lock_token() -> String {
        format!("opaquelocktoken:{}", scru128::new_string())
    }

    pub(super) fn meta_dir(&self) -> std::path::PathBuf {
        self.storage.root_dir().join(".webdav")
    }

    pub(super) fn locks_file(&self) -> std::path::PathBuf {
        self.meta_dir().join("locks.json")
    }
    pub(super) fn props_file(&self) -> std::path::PathBuf {
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

    pub(super) async fn persist_locks(&self) {
        let map = self.locks.read().await.clone();
        let _ = std::fs::create_dir_all(self.meta_dir());
        if let Ok(bytes) = serde_json::to_vec_pretty(&map) {
            let _ = std::fs::write(self.locks_file(), bytes);
        }
    }

    pub(super) async fn persist_props(&self) {
        let map = self.props.read().await.clone();
        let _ = std::fs::create_dir_all(self.meta_dir());
        if let Ok(bytes) = serde_json::to_vec_pretty(&map) {
            let _ = std::fs::write(self.props_file(), bytes);
        }
    }

    pub(super) fn parse_timeout(req: &Request) -> i64 {
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

    pub(super) fn extract_if_lock_token(req: &Request) -> Option<String> {
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

    pub(super) async fn ensure_lock_ok(&self, path: &str, req: &Request) -> silent::Result<()> {
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
    pub(super) fn decode_path(path: &str) -> silent::Result<String> {
        urlencoding::decode(path)
            .map(|s| s.to_string())
            .map_err(|e| {
                SilentError::business_error(StatusCode::BAD_REQUEST, format!("路径解码失败: {}", e))
            })
    }

    /// 构建完整的 WebDAV href（包含 base_path 前缀）
    pub(super) fn build_full_href(&self, relative_path: &str) -> String {
        format!("{}{}", &self.base_path, relative_path)
    }
}

// Files operations moved to files module
// Locks/Props/DeltaV moved to respective modules

impl WebDavHandler {
    //
    /// PUT - 上传文件
    #[allow(dead_code)]
    async fn handle_put_old(&self, path: &str, req: &mut Request) -> silent::Result<Response> {
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
    #[allow(dead_code)]
    async fn handle_delete_old(&self, path: &str) -> silent::Result<Response> {
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
