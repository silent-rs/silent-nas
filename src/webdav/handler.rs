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
    pub(super) locks: Arc<tokio::sync::RwLock<std::collections::HashMap<String, DavLock>>>,
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
                serde_json::from_slice::<std::collections::HashMap<String, DavLock>>(&bytes)
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

    pub(super) fn extract_if_lock_token(req: &Request) -> Option<String> {
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

    pub(super) fn decode_path(path: &str) -> silent::Result<String> {
        urlencoding::decode(path)
            .map(|s| s.to_string())
            .map_err(|e| {
                SilentError::business_error(StatusCode::BAD_REQUEST, format!("路径解码失败: {}", e))
            })
    }

    pub(super) fn build_full_href(&self, relative_path: &str) -> String {
        format!("{}{}", &self.base_path, relative_path)
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
