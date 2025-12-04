//! HTTP 服务器模块
//!
//! 提供 REST API 服务，使用中间件和萃取器模式

mod admin;
mod admin_handlers;
mod audit_api;
mod auth_handlers;
mod auth_middleware;
mod files;
mod health;
mod incremental_sync;
mod metrics_api;
mod search;
mod state;
mod static_files;
mod storage_v2_metrics;
mod sync;
mod upload_sessions;
mod versions;

pub use auth_middleware::{AuthHook, OptionalAuthHook};
pub use state::AppState;
pub use storage_v2_metrics::StorageV2MetricsState;

use crate::error::Result;
use crate::notify::EventNotifier;
use crate::search::SearchEngine;
use crate::storage::StorageManager;
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
    notifier: Option<EventNotifier>,
    sync_manager: Arc<SyncManager>,
    storage: Arc<StorageManager>,
    search_engine: Arc<SearchEngine>,
    config: crate::config::Config,
) -> Result<()> {
    // 创建增量同步处理器
    let inc_sync_handler = Arc::new(IncrementalSyncHandler::new(64 * 1024));

    // 创建审计日志管理器（可选，通过环境变量启用）
    let audit_logger = if std::env::var("ENABLE_AUDIT").is_ok() {
        Some(Arc::new(crate::audit::AuditLogger::new(1000)))
    } else {
        None
    };

    // 创建认证管理器（使用配置）
    let auth_manager = if config.auth.enable {
        match crate::auth::AuthManager::new(&config.auth.db_path) {
            Ok(manager) => {
                // 设置JWT配置
                manager.set_jwt_config(crate::auth::JwtConfig {
                    secret: config.auth.jwt_secret.clone(),
                    access_token_exp: config.auth.access_token_exp,
                    refresh_token_exp: config.auth.refresh_token_exp,
                });

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

    // 创建 Storage V2 指标状态
    let storage_v2_metrics = Arc::new(StorageV2MetricsState::new());

    // 创建上传会话管理器
    let upload_sessions = {
        use crate::webdav::upload_session::UploadSessionManager;

        // 使用临时目录存储上传会话
        let temp_dir = std::env::temp_dir().join("silent-nas-uploads");
        #[allow(clippy::collapsible_if)]
        if !temp_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&temp_dir) {
                tracing::warn!("创建上传临时目录失败: {} - {}", temp_dir.display(), e);
            }
        }

        Some(Arc::new(UploadSessionManager::new(
            temp_dir, 24, // 24小时过期
            10, // 最大10个并发上传
        )))
    };

    // 创建应用状态
    let app_state = AppState {
        storage,
        notifier: notifier.map(Arc::new),
        sync_manager,
        search_engine: search_engine.clone(),
        inc_sync_handler,
        source_http_addr,
        audit_logger,
        auth_manager,
        storage_v2_metrics: storage_v2_metrics.clone(),
        upload_sessions,
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

    // 定期清理过期上传会话
    if let Some(sessions_mgr) = app_state.upload_sessions.clone() {
        tokio::spawn(async move {
            use tokio::time::{Duration, interval};
            let mut timer = interval(Duration::from_secs(3600)); // 每小时清理一次
            loop {
                timer.tick().await;
                let cleaned = sessions_mgr.cleanup_expired_sessions().await;
                if cleaned > 0 {
                    tracing::info!("清理了 {} 个过期上传会话", cleaned);
                }
            }
        });
    }

    // 构建路由
    let mut api_route = Route::new("api")
        .append(
            Route::new("auth")
                .append(Route::new("register").post(auth_handlers::register_handler))
                .append(Route::new("login").post(auth_handlers::login_handler))
                .append(Route::new("refresh").post(auth_handlers::refresh_handler))
                .append(Route::new("logout").post(auth_handlers::logout_handler))
                .append(Route::new("me").get(auth_handlers::me_handler))
                .append(Route::new("password").put(auth_handlers::change_password_handler)),
        )
        .append(Route::new("health").get(health::health))
        .append(Route::new("health/readiness").get(health::readiness))
        .append(Route::new("health/status").get(health::health_status));

    // 如果启用认证，为需要保护的API添加认证Hook
    if let Some(ref auth_mgr) = app_state.auth_manager {
        let auth_hook = AuthHook::new(auth_mgr.clone());
        let admin_hook = AuthHook::admin_only(auth_mgr.clone());
        let optional_auth_hook = OptionalAuthHook::new(auth_mgr.clone());

        // 管理员API - 需要管理员权限
        api_route = api_route
            // 仪表盘 API
            .append(
                Route::new("admin/dashboard/overview")
                    .hook(admin_hook.clone())
                    .get(admin::get_overview),
            )
            .append(
                Route::new("admin/dashboard/metrics")
                    .hook(admin_hook.clone())
                    .get(admin::get_metrics),
            )
            .append(
                Route::new("admin/dashboard/activities")
                    .hook(admin_hook.clone())
                    .get(admin::get_activities),
            )
            // 用户管理 API
            .append(
                Route::new("admin/users")
                    .hook(admin_hook.clone())
                    .get(admin_handlers::list_users)
                    .post(admin_handlers::create_user),
            )
            .append(
                Route::new("admin/users/<id>")
                    .hook(admin_hook.clone())
                    .get(admin_handlers::get_user)
                    .put(admin_handlers::update_user)
                    .delete(admin_handlers::delete_user),
            )
            .append(
                Route::new("admin/users/<id>/password")
                    .hook(admin_hook.clone())
                    .put(admin_handlers::reset_password),
            )
            .append(
                Route::new("admin/users/<id>/status")
                    .hook(admin_hook.clone())
                    .put(admin_handlers::update_user_status),
            )
            // S3 密钥管理 API
            .append(
                Route::new("admin/s3-keys")
                    .hook(auth_hook.clone())
                    .get(admin_handlers::list_s3_keys)
                    .post(admin_handlers::create_s3_key),
            )
            .append(
                Route::new("admin/s3-keys/all")
                    .hook(admin_hook.clone())
                    .get(admin_handlers::list_all_s3_keys),
            )
            .append(
                Route::new("admin/s3-keys/<id>")
                    .hook(auth_hook.clone())
                    .get(admin_handlers::get_s3_key)
                    .put(admin_handlers::update_s3_key)
                    .delete(admin_handlers::delete_s3_key),
            );

        // 文件操作 - 需要认证
        api_route = api_route
            .append(
                Route::new("files")
                    .hook(auth_hook.clone())
                    .post(files::upload_file)
                    .get(files::list_files),
            )
            .append(
                Route::new("files/<id>")
                    .hook(auth_hook.clone())
                    .get(files::download_file)
                    .delete(files::delete_file),
            )
            // 版本管理 - 需要认证
            .append(
                Route::new("files/<id>/versions")
                    .hook(auth_hook.clone())
                    .get(versions::list_versions),
            )
            // 同步管理 - 需要管理员权限
            .append(
                Route::new("admin/sync/push")
                    .hook(admin_hook.clone())
                    .post(admin_handlers::trigger_push_sync),
            )
            .append(
                Route::new("admin/sync/request")
                    .hook(admin_hook.clone())
                    .post(admin_handlers::trigger_request_sync),
            )
            // GC管理 - 需要管理员权限
            .append(
                Route::new("admin/gc/trigger")
                    .hook(admin_hook.clone())
                    .post(admin_handlers::trigger_gc),
            )
            .append(
                Route::new("admin/gc/status")
                    .hook(admin_hook.clone())
                    .get(admin_handlers::get_gc_status),
            )
            .append(
                Route::new("files/<id>/versions/<version_id>")
                    .hook(auth_hook.clone())
                    .get(versions::get_version)
                    .delete(versions::delete_version),
            )
            .append(
                Route::new("files/<id>/versions/<version_id>/restore")
                    .hook(auth_hook.clone())
                    .post(versions::restore_version),
            )
            .append(
                Route::new("versions/stats")
                    .hook(auth_hook.clone())
                    .get(versions::get_version_stats),
            )
            // 同步功能 - 可选认证
            .append(
                Route::new("sync/states")
                    .hook(optional_auth_hook.clone())
                    .get(sync::list_sync_states),
            )
            .append(
                Route::new("sync/states/<id>")
                    .hook(optional_auth_hook.clone())
                    .get(sync::get_sync_state),
            )
            .append(
                Route::new("sync/conflicts")
                    .hook(optional_auth_hook.clone())
                    .get(sync::get_conflicts),
            )
            .append(
                Route::new("sync/signature/<id>")
                    .hook(optional_auth_hook.clone())
                    .get(incremental_sync::get_file_signature),
            )
            .append(
                Route::new("sync/delta/<id>")
                    .hook(optional_auth_hook.clone())
                    .post(incremental_sync::get_file_delta),
            )
            // 搜索 - 需要认证
            .append(
                Route::new("search")
                    .hook(auth_hook.clone())
                    .get(search::search_files),
            )
            .append(
                Route::new("search/stats")
                    .hook(auth_hook.clone())
                    .get(search::get_search_stats),
            )
            // 指标 - 需要认证
            .append(
                Route::new("metrics")
                    .hook(auth_hook.clone())
                    .get(metrics_api::get_metrics),
            )
            // Storage V2 指标 - 需要认证
            .append(
                Route::new("metrics/storage-v2")
                    .hook(auth_hook.clone())
                    .get(storage_v2_metrics::get_storage_v2_metrics),
            )
            .append(
                Route::new("metrics/storage-v2/health")
                    .hook(auth_hook.clone())
                    .get(storage_v2_metrics::get_storage_v2_health),
            )
            .append(
                Route::new("metrics/storage-v2/json")
                    .hook(auth_hook.clone())
                    .get(storage_v2_metrics::get_storage_v2_metrics_json),
            )
            // 审计日志 - 需要认证
            .append(
                Route::new("audit/logs")
                    .hook(auth_hook.clone())
                    .get(audit_api::get_audit_logs),
            )
            .append(
                Route::new("audit/stats")
                    .hook(auth_hook.clone())
                    .get(audit_api::get_audit_stats),
            )
            // 上传会话管理 - 需要认证
            .append(
                Route::new("upload/sessions")
                    .hook(auth_hook.clone())
                    .get(upload_sessions::list_sessions),
            )
            .append(
                Route::new("upload/sessions/<session_id>")
                    .hook(auth_hook.clone())
                    .get(upload_sessions::get_session)
                    .delete(upload_sessions::cancel_session),
            )
            .append(
                Route::new("upload/sessions/<session_id>/pause")
                    .hook(auth_hook.clone())
                    .post(upload_sessions::pause_session),
            );

        info!("🔒 认证功能已启用 - API端点已受保护");
    } else {
        // 未启用认证，使用原始路由（无保护）
        api_route = api_route
            // 仪表盘 API
            .append(Route::new("admin/dashboard/overview").get(admin::get_overview))
            .append(Route::new("admin/dashboard/metrics").get(admin::get_metrics))
            .append(Route::new("admin/dashboard/activities").get(admin::get_activities))
            // 文件操作
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
            .append(Route::new("admin/sync/push").post(admin_handlers::trigger_push_sync))
            .append(Route::new("admin/sync/request").post(admin_handlers::trigger_request_sync))
            .append(Route::new("admin/gc/trigger").post(admin_handlers::trigger_gc))
            .append(Route::new("admin/gc/status").get(admin_handlers::get_gc_status))
            .append(Route::new("sync/states").get(sync::list_sync_states))
            .append(Route::new("sync/states/<id>").get(sync::get_sync_state))
            .append(Route::new("sync/conflicts").get(sync::get_conflicts))
            .append(Route::new("sync/signature/<id>").get(incremental_sync::get_file_signature))
            .append(Route::new("sync/delta/<id>").post(incremental_sync::get_file_delta))
            .append(Route::new("search").get(search::search_files))
            .append(Route::new("search/stats").get(search::get_search_stats))
            .append(Route::new("metrics").get(metrics_api::get_metrics))
            .append(
                Route::new("metrics/storage-v2").get(storage_v2_metrics::get_storage_v2_metrics),
            )
            .append(
                Route::new("metrics/storage-v2/health")
                    .get(storage_v2_metrics::get_storage_v2_health),
            )
            .append(
                Route::new("metrics/storage-v2/json")
                    .get(storage_v2_metrics::get_storage_v2_metrics_json),
            )
            .append(Route::new("audit/logs").get(audit_api::get_audit_logs))
            .append(Route::new("audit/stats").get(audit_api::get_audit_stats))
            .append(Route::new("upload/sessions").get(upload_sessions::list_sessions))
            .append(
                Route::new("upload/sessions/<session_id>")
                    .get(upload_sessions::get_session)
                    .delete(upload_sessions::cancel_session),
            )
            .append(
                Route::new("upload/sessions/<session_id>/pause")
                    .post(upload_sessions::pause_session),
            );

        info!("⚠️  认证功能未启用 - API端点无保护");
    }

    let route = Route::new_root()
        .hook(state_injector(app_state))
        .append(api_route)
        // 暴露根路径 /metrics（便于 Prometheus 默认抓取路径），与 /api/metrics 并存
        .append(Route::new("metrics").get(metrics_api::get_metrics))
        // 管理端前端静态文件服务
        .append(Route::new("admin").get(static_files::serve_static_file))
        .append(Route::new("admin/<**>").get(static_files::serve_static_file));

    info!("HTTP 服务器启动: {}", addr);
    info!("  - REST API: http://{}/api", addr);
    info!("  - 管理控制台: http://{}/admin", addr);

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
    use crate::sync::crdt::SyncManager;
    use silent::extractor::Configs as CfgExtractor;
    use tempfile::TempDir;

    pub(crate) async fn create_test_app_state() -> (AppState, TempDir) {
        // 使用共享的测试存储（并发安全）
        let storage = crate::storage::init_test_storage_async().await;
        let storage_arc = Arc::new(storage.clone());

        // 为 SearchEngine 创建独立的临时目录
        let temp_dir = TempDir::new().unwrap();

        let sync_manager = SyncManager::new("test-node".to_string(), None);
        let search_engine = Arc::new(
            SearchEngine::new(
                temp_dir.path().join("search_index"),
                temp_dir.path().to_path_buf(),
            )
            .unwrap(),
        );
        let inc_sync_handler = Arc::new(IncrementalSyncHandler::new(64 * 1024));
        let source_http_addr = Arc::new("http://localhost:8080".to_string());
        let storage_v2_metrics = Arc::new(StorageV2MetricsState::new());

        let app_state = AppState {
            storage: storage_arc,
            notifier: None,
            sync_manager,
            search_engine,
            inc_sync_handler,
            source_http_addr,
            audit_logger: None,
            auth_manager: None,
            storage_v2_metrics,
            upload_sessions: None,
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
            Arc::as_ptr(&app_state.sync_manager),
            Arc::as_ptr(&cloned.sync_manager)
        );
        assert_eq!(
            Arc::as_ptr(&app_state.search_engine),
            Arc::as_ptr(&cloned.search_engine)
        );
    }

    #[test]
    fn test_search_query_default() {
        use state::SearchQuery;

        let query = SearchQuery {
            q: String::new(),
            limit: 20,
            offset: 0,
            ..Default::default()
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
            ..Default::default()
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
            Arc::as_ptr(&injector.state.sync_manager),
            Arc::as_ptr(&app_state.sync_manager)
        );
    }

    #[tokio::test]
    async fn test_state_injector_middleware() {
        let (app_state, _temp_dir) = create_test_app_state().await;
        let injector = StateInjector::new(app_state.clone());

        // 验证状态注入器的基本属性
        assert_eq!(
            Arc::as_ptr(&injector.state.sync_manager),
            Arc::as_ptr(&app_state.sync_manager)
        );

        // 验证状态注入器实现了正确的trait
        let _: &dyn MiddleWareHandler = &injector;
    }

    #[tokio::test]
    async fn test_state_injector_function() {
        let (app_state, _temp_dir) = create_test_app_state().await;
        let injector = state_injector(app_state.clone());

        assert_eq!(
            Arc::as_ptr(&injector.state.sync_manager),
            Arc::as_ptr(&app_state.sync_manager)
        );
    }

    #[tokio::test]
    async fn test_list_files_empty() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let result = files::list_files(CfgExtractor(app_state)).await;

        assert!(result.is_ok());
        let _files = result.unwrap();
        // 由于测试使用共享存储，可能包含其他测试的文件
        // 只验证 list_files 能正常工作
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
