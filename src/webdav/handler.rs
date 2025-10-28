use crate::notify::EventNotifier;
use crate::storage::StorageManager;
use crate::sync::crdt::SyncManager;
use async_trait::async_trait;
use silent::prelude::*;
use std::sync::Arc;

#[allow(unused_imports)]
use super::{constants::*, types::DavLock};

#[derive(Clone)]
pub struct WebDavHandler {
    pub storage: Arc<StorageManager>,
    pub notifier: Option<Arc<EventNotifier>>,
    #[allow(dead_code)]
    pub sync_manager: Arc<SyncManager>,
    pub base_path: String,
    pub source_http_addr: String,
    #[allow(dead_code)]
    pub version_manager: Arc<crate::version::VersionManager>,
    pub(super) locks: Arc<tokio::sync::RwLock<std::collections::HashMap<String, Vec<DavLock>>>>,
    pub(super) props: Arc<
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
        let _ = std::fs::create_dir_all(self.meta_dir());
        if let Ok(bytes) = std::fs::read(self.locks_file())
            && let Ok(map) =
                serde_json::from_slice::<std::collections::HashMap<String, Vec<DavLock>>>(&bytes)
        {
            let rt = tokio::runtime::Handle::current();
            let locks = self.locks.clone();
            rt.spawn(async move {
                *locks.write().await = map;
            });
        }
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
            if v.to_lowercase().contains("infinite") {
                return 3600;
            }
            if let Some(num) = v.split(['-', ',']).find_map(|s| s.parse::<i64>().ok()) {
                return num.clamp(1, 3600);
            }
        }
        60
    }

    pub(super) fn extract_if_lock_tokens(req: &Request) -> Vec<String> {
        let mut tokens = Vec::new();
        if let Some(val) = req.headers().get("If").and_then(|h| h.to_str().ok()) {
            let s = val.as_bytes();
            let needle = b"opaquelocktoken:";
            let mut i = 0;
            while i + needle.len() <= s.len() {
                if &s[i..i + needle.len()] == needle {
                    let start = i;
                    // 向后找到 > 作为结束
                    let mut j = i;
                    while j < s.len() && s[j] != b'>' as u8 { j += 1; }
                    let end = j.min(s.len());
                    if end > start {
                        if let Ok(tok) = std::str::from_utf8(&s[start..end]) {
                            tokens.push(tok.to_string());
                        }
                    }
                    i = end;
                } else {
                    i += 1;
                }
            }
        }
        tokens
    }

    pub(super) async fn ensure_lock_ok(&self, path: &str, req: &Request) -> silent::Result<()> {
        let locks = self.locks.read().await;
        if let Some(list) = locks.get(path) {
            let active: Vec<&crate::webdav::types::DavLock> = list.iter().filter(|l| !l.is_expired()).collect();
            if !active.is_empty() {
                let provided = Self::extract_if_lock_tokens(req);
                if !provided.is_empty() {
                    // 任一提供 token 匹配即可
                    if active.iter().any(|l| provided.iter().any(|t| t == &l.token)) {
                        return Ok(());
                    }
                }
                return Err(SilentError::business_error(
                    StatusCode::LOCKED,
                    "资源被锁定或令牌缺失",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn decode_path(path: &str) -> silent::Result<String> {
        urlencoding::decode(path)
            .map(|s| s.to_string())
            .map_err(|e| {
                SilentError::business_error(StatusCode::BAD_REQUEST, format!("路径解码失败: {}", e))
            })
    }

    pub(super) fn build_full_href(&self, relative_path: &str) -> String {
        // Finder 期望 href 为相对路径（不含 schema/host），目录以尾斜杠结尾
        // base_path 作为相对前缀（通常为空字符串）
        let mut path = format!("{}{}", &self.base_path, relative_path);
        if !path.starts_with('/') {
            path = format!("/{}", path);
        }
        path
    }
}

#[async_trait]
impl Handler for WebDavHandler {
    async fn call(&self, mut req: Request) -> silent::Result<Response> {
        let method = req.method().clone();
        let uri_path = req.uri().path().to_string();
        let relative_path = uri_path
            .strip_prefix(&self.base_path)
            .unwrap_or(&uri_path)
            .to_string();
        tracing::debug!("WebDAV {} {}", method, relative_path);
        match method.as_str() {
            "OPTIONS" => self.handle_options().await,
            "PROPFIND" => self.handle_propfind(&relative_path, &req).await,
            "PROPPATCH" => self.handle_proppatch(&relative_path, &mut req).await,
            "HEAD" => self.handle_head(&relative_path, &req).await,
            "GET" => self.handle_get(&relative_path, &req).await,
            "PUT" => self.handle_put(&relative_path, &mut req).await,
            "DELETE" => self.handle_delete(&relative_path).await,
            "MKCOL" => self.handle_mkcol(&relative_path).await,
            "MOVE" => self.handle_move(&relative_path, &req).await,
            "COPY" => self.handle_copy(&relative_path, &req).await,
            "LOCK" => self.handle_lock(&relative_path, &mut req).await,
            "UNLOCK" => self.handle_unlock(&relative_path, &req).await,
            "VERSION-CONTROL" => self.handle_version_control(&relative_path).await,
            "REPORT" => self.handle_report(&relative_path, &mut req).await,
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
    fn test_lock_token_format() {
        let token = WebDavHandler::lock_token();
        assert!(token.starts_with("opaquelocktoken:"));
        // scru128 由 [0-9a-z] 和分隔符组成，一般长度固定
        assert!(token.len() > 20);
    }

    #[test]
    fn test_decode_path_ok() {
        let s = WebDavHandler::decode_path("/dir/%E4%B8%AD%E6%96%87.txt").unwrap();
        assert_eq!(s, "/dir/中文.txt");
    }

    #[tokio::test]
    async fn test_build_full_href_rules() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(StorageManager::new(
            dir.path().to_path_buf(),
            4 * 1024 * 1024,
        ));
        storage.init().await.unwrap();
        let syncm = SyncManager::new("node-test".into(), storage.clone(), None);
        let ver = crate::version::VersionManager::new(
            storage.clone(),
            Default::default(),
            dir.path().to_str().unwrap(),
        );
        let handler = WebDavHandler::new(
            storage,
            None,
            syncm,
            "".into(),
            "http://127.0.0.1:8080".into(),
            ver,
        );
        assert_eq!(handler.build_full_href("/"), "/");
        assert_eq!(handler.build_full_href("/a/b"), "/a/b");
        assert_eq!(handler.build_full_href("a/b"), "/a/b");
    }

    #[test]
    fn test_parse_timeout() {
        let mut req = Request::empty();
        req.headers_mut()
            .insert("Timeout", http::HeaderValue::from_static("Second-120"));
        assert_eq!(WebDavHandler::parse_timeout(&req), 120);

        let mut req2 = Request::empty();
        req2.headers_mut()
            .insert("Timeout", http::HeaderValue::from_static("Infinite"));
        assert_eq!(WebDavHandler::parse_timeout(&req2), 3600);
    }
}
