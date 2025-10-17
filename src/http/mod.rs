//! HTTP 服务器模块
//!
//! 提供 REST API 服务，使用中间件和萃取器模式

mod audit_api;
mod auth_handlers;
mod files;
mod health;
mod incremental_sync;
mod metrics_api;
mod search;
mod state;
mod sync;
mod versions;

pub use state::AppState;

use crate::error::Result;
use crate::notify::EventNotifier;
use crate::search::SearchEngine;
use crate::storage::StorageManager;
use crate::version::VersionManager;
use silent::Server;
use silent::prelude::*;
use std::sync::Arc;
use tracing::info;

// 当作为 main.rs 的子模块时，使用 super 访问同级模块
#[cfg(not(test))]
use super::sync::crdt::SyncManager;
#[cfg(not(test))]
use super::sync::incremental::IncrementalSyncHandler;

// 测试时的占位符
#[cfg(test)]
use crate::sync::crdt::SyncManager;
#[cfg(test)]
use crate::sync::incremental::IncrementalSyncHandler;

/// 启动 HTTP 服务器
pub async fn start_http_server(
    addr: &str,
    storage: StorageManager,
    notifier: Option<EventNotifier>,
    sync_manager: Arc<SyncManager>,
    version_manager: Arc<VersionManager>,
    search_engine: Arc<SearchEngine>,
) -> Result<()> {
    let storage = Arc::new(storage);

    // 创建增量同步处理器
    let inc_sync_handler = Arc::new(IncrementalSyncHandler::new(storage.clone(), 64 * 1024));

    // 创建审计日志管理器（可选，通过环境变量启用）
    let audit_logger = if std::env::var("ENABLE_AUDIT").is_ok() {
        Some(Arc::new(crate::audit::AuditLogger::new(1000)))
    } else {
        None
    };

    // 创建认证管理器（可选，通过环境变量启用）
    let auth_manager = if std::env::var("ENABLE_AUTH").is_ok() {
        let db_path = std::env::var("AUTH_DB_PATH").unwrap_or_else(|_| "./data/auth.db".to_string());
        match crate::auth::AuthManager::new(&db_path) {
            Ok(manager) => {
                // 初始化默认管理员
                if let Err(e) = manager.init_default_admin() {
                    tracing::warn!("初始化默认管理员失败: {}", e);
                }
                Some(Arc::new(manager))
            }
            Err(e) => {
                tracing::error!("创建认证管理器失败: {}", e);
                None
            }
        }
    } else {
        None
    };

    // 计算源 HTTP 地址
    let advertise_host = std::env::var("ADVERTISE_HOST")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "localhost".to_string());
    let http_port: u16 = addr
        .rsplit(':')
        .next()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let source_http_addr = Arc::new(format!("http://{}:{}", advertise_host, http_port));

    // 创建应用状态
    let app_state = AppState {
        storage,
        notifier: notifier.map(Arc::new),
        sync_manager,
        version_manager,
        search_engine: search_engine.clone(),
        inc_sync_handler,
        source_http_addr,
        audit_logger,
        auth_manager,
    };

    // 定期提交索引
    tokio::spawn(async move {
        use tokio::time::{Duration, interval};
        let mut timer = interval(Duration::from_secs(30));
        loop {
            timer.tick().await;
            if let Err(e) = search_engine.commit().await {
                tracing::warn!("定期提交索引失败: {}", e);
            }
        }
    });

    // 构建路由
    let route = Route::new_root().hook(state_injector(app_state)).append(
        Route::new("api")
            .append(
                Route::new("auth")
                    .append(Route::new("register").post(auth_handlers::register_handler))
                    .append(Route::new("login").post(auth_handlers::login_handler))
                    .append(Route::new("refresh").post(auth_handlers::refresh_handler))
                    .append(Route::new("me").get(auth_handlers::me_handler))
                    .append(Route::new("password").put(auth_handlers::change_password_handler)),
            )
            .append(
                Route::new("files")
                    .post(files::upload_file)
                    .get(files::list_files),
            )
            .append(
                Route::new("files/<id>")
                    .get(files::download_file)
                    .delete(files::delete_file),
            )
            .append(Route::new("files/<id>/versions").get(versions::list_versions))
            .append(
                Route::new("files/<id>/versions/<version_id>")
                    .get(versions::get_version)
                    .delete(versions::delete_version),
            )
            .append(
                Route::new("files/<id>/versions/<version_id>/restore")
                    .post(versions::restore_version),
            )
            .append(Route::new("versions/stats").get(versions::get_version_stats))
            .append(Route::new("sync/states").get(sync::list_sync_states))
            .append(Route::new("sync/states/<id>").get(sync::get_sync_state))
            .append(Route::new("sync/conflicts").get(sync::get_conflicts))
            .append(Route::new("sync/signature/<id>").get(incremental_sync::get_file_signature))
            .append(Route::new("sync/delta/<id>").post(incremental_sync::get_file_delta))
            .append(Route::new("search").get(search::search_files))
            .append(Route::new("search/stats").get(search::get_search_stats))
            .append(Route::new("health").get(health::health))
            .append(Route::new("health/readiness").get(health::readiness))
            .append(Route::new("health/status").get(health::health_status))
            .append(Route::new("metrics").get(metrics_api::get_metrics))
            .append(Route::new("audit/logs").get(audit_api::get_audit_logs))
            .append(Route::new("audit/stats").get(audit_api::get_audit_stats)),
    );

    info!("HTTP 服务器启动: {}", addr);
    info!("  - REST API: http://{}/api", addr);

    Server::new()
        .bind(addr.parse().expect("无效的 HTTP 地址"))
        .serve(route)
        .await;

    Ok(())
}

/// 中间件：注入应用状态到 Request configs
struct StateInjector {
    state: AppState,
}

impl StateInjector {
    fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[async_trait::async_trait]
impl MiddleWareHandler for StateInjector {
    async fn handle(&self, mut req: Request, next: &Next) -> silent::Result<Response> {
        req.configs_mut().insert(self.state.clone());
        next.call(req).await
    }
}

fn state_injector(state: AppState) -> StateInjector {
    StateInjector::new(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageManager;
    use crate::sync::crdt::SyncManager;
    use crate::version::VersionManager;
    use silent::extractor::Configs as CfgExtractor;
    use tempfile::TempDir;

    pub(crate) async fn create_test_storage() -> (StorageManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = StorageManager::new(
            temp_dir.path().to_path_buf(),
            64 * 1024, // 64KB chunk size for tests
        );
        storage.init().await.unwrap();
        (storage, temp_dir)
    }

    pub(crate) async fn create_test_app_state() -> (AppState, TempDir) {
        let (storage, temp_dir) = create_test_storage().await;
        let storage = Arc::new(storage);

        let sync_manager = SyncManager::new("test-node".to_string(), storage.clone(), None);
        let version_config = crate::version::VersionConfig::default();
        let version_manager = VersionManager::new(
            storage.clone(),
            version_config,
            temp_dir.path().to_str().unwrap(),
        );
        let search_engine =
            Arc::new(SearchEngine::new(temp_dir.path().join("search_index")).unwrap());
        let inc_sync_handler = Arc::new(IncrementalSyncHandler::new(storage.clone(), 64 * 1024));
        let source_http_addr = Arc::new("http://localhost:8080".to_string());

        let app_state = AppState {
            storage,
            notifier: None,
            sync_manager,
            version_manager,
            search_engine,
            inc_sync_handler,
            source_http_addr,
            audit_logger: None,
            auth_manager: None,
        };

        (app_state, temp_dir)
    }

    #[tokio::test]
    async fn test_app_state_creation() {
        let (_app_state, _temp_dir) = create_test_app_state().await;
        // 验证 AppState 可以成功创建
    }

    #[tokio::test]
    async fn test_app_state_clone() {
        let (app_state, _temp_dir) = create_test_app_state().await;
        let cloned = app_state.clone();

        // 验证克隆后的状态指向相同的资源
        assert_eq!(
            Arc::as_ptr(&app_state.storage),
            Arc::as_ptr(&cloned.storage)
        );
        assert_eq!(
            Arc::as_ptr(&app_state.sync_manager),
            Arc::as_ptr(&cloned.sync_manager)
        );
    }

    #[test]
    fn test_search_query_default() {
        use state::SearchQuery;

        let query = SearchQuery {
            q: String::new(),
            limit: 20,
            offset: 0,
        };

        assert_eq!(query.q, "");
        assert_eq!(query.limit, 20);
        assert_eq!(query.offset, 0);
    }

    #[test]
    fn test_search_query_custom() {
        use state::SearchQuery;

        let query = SearchQuery {
            q: "test query".to_string(),
            limit: 50,
            offset: 10,
        };

        assert_eq!(query.q, "test query");
        assert_eq!(query.limit, 50);
        assert_eq!(query.offset, 10);
    }

    #[tokio::test]
    async fn test_health_check() {
        let req = Request::empty();
        let result = health::health(req).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "OK");
    }

    #[tokio::test]
    async fn test_state_injector_creation() {
        let (app_state, _temp_dir) = create_test_app_state().await;
        let injector = StateInjector::new(app_state.clone());

        // 验证状态注入器可以正确创建
        assert_eq!(
            Arc::as_ptr(&injector.state.storage),
            Arc::as_ptr(&app_state.storage)
        );
    }

    #[tokio::test]
    async fn test_state_injector_middleware() {
        let (app_state, _temp_dir) = create_test_app_state().await;
        let injector = StateInjector::new(app_state.clone());

        // 验证状态注入器的基本属性
        assert_eq!(
            Arc::as_ptr(&injector.state.storage),
            Arc::as_ptr(&app_state.storage)
        );

        // 验证状态注入器实现了正确的trait
        let _: &dyn MiddleWareHandler = &injector;
    }

    #[tokio::test]
    async fn test_state_injector_function() {
        let (app_state, _temp_dir) = create_test_app_state().await;
        let injector = state_injector(app_state.clone());

        assert_eq!(
            Arc::as_ptr(&injector.state.storage),
            Arc::as_ptr(&app_state.storage)
        );
    }

    #[tokio::test]
    async fn test_list_files_empty() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let result = files::list_files(CfgExtractor(app_state)).await;

        assert!(result.is_ok());
        let files = result.unwrap();
        // 空存储应该返回空列表
        assert!(files.is_empty());
    }

    #[tokio::test]
    async fn test_get_version_stats() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let result = versions::get_version_stats(CfgExtractor(app_state)).await;

        assert!(result.is_ok());
        let stats = result.unwrap();
        assert!(stats.is_object());
    }

    #[tokio::test]
    async fn test_get_search_stats() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let result = search::get_search_stats(CfgExtractor(app_state)).await;

        assert!(result.is_ok());
        let stats = result.unwrap();
        assert!(stats.is_object());
    }

    #[tokio::test]
    async fn test_list_sync_states() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let result = sync::list_sync_states(CfgExtractor(app_state)).await;

        assert!(result.is_ok());
        let states = result.unwrap();
        // 返回值应该是有效的JSON
        assert!(states.is_object() || states.is_array());
    }

    #[tokio::test]
    async fn test_get_conflicts() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let result = sync::get_conflicts(CfgExtractor(app_state)).await;

        assert!(result.is_ok());
        let conflicts = result.unwrap();
        assert!(conflicts.is_array());
    }
}
