//! HTTP 服务器模块
//!
//! 提供 REST API 服务，使用中间件和萃取器模式

use crate::error::Result;
use crate::metrics;
use crate::models::{EventType, FileEvent};
use crate::notify::EventNotifier;
use crate::search::SearchEngine;
use crate::storage::StorageManager;
use crate::version::VersionManager;
use http::StatusCode;
use http_body_util::BodyExt;
use serde::Deserialize;
use silent::extractor::{Configs as CfgExtractor, Path, Query};
use silent::prelude::*;
use silent::{Server, SilentError};
use std::sync::Arc;
use tracing::info;

// 当作为 main.rs 的子模块时，使用 super 访问同级模块
#[cfg(not(test))]
use super::sync::crdt::SyncManager;
#[cfg(not(test))]
use super::sync::incremental::{FileSignature, IncrementalSyncHandler, api};

// 测试时的占位符
#[cfg(test)]
use crate::sync::crdt::SyncManager;
#[cfg(test)]
use crate::sync::incremental::{FileSignature, IncrementalSyncHandler, api};

/// 应用共享状态
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<StorageManager>,
    pub notifier: Option<Arc<EventNotifier>>,
    pub sync_manager: Arc<SyncManager>,
    pub version_manager: Arc<VersionManager>,
    pub search_engine: Arc<SearchEngine>,
    pub inc_sync_handler: Arc<IncrementalSyncHandler>,
    pub source_http_addr: Arc<String>,
}

/// 搜索查询参数
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    20
}

// ============ HTTP 处理函数 ============

/// 健康检查
async fn health(_req: Request) -> silent::Result<&'static str> {
    Ok("OK")
}

/// Prometheus metrics 端点
async fn get_metrics(_req: Request) -> silent::Result<Response> {
    match metrics::export_metrics() {
        Ok(metrics_text) => {
            let mut resp = Response::empty();
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("text/plain; version=0.0.4"),
            );
            resp.set_body(full(metrics_text.into_bytes()));
            Ok(resp)
        }
        Err(e) => Err(SilentError::business_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("导出metrics失败: {}", e),
        )),
    }
}

/// 上传文件
async fn upload_file(
    mut req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let file_id = scru128::new_string();

    let body = req.take_body();
    let bytes = match body {
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

    let metadata = state
        .storage
        .save_file(&file_id, &bytes)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("保存文件失败: {}", e),
            )
        })?;

    // 索引文件到搜索引擎
    if let Err(e) = state.search_engine.index_file(&metadata).await {
        tracing::warn!("索引文件失败: {} - {}", file_id, e);
    }

    let mut event = FileEvent::new(EventType::Created, file_id.clone(), Some(metadata.clone()));
    event.source_http_addr = Some((*state.source_http_addr).clone());
    if let Some(ref n) = state.notifier {
        let _ = n.notify_created(event).await;
    }

    Ok(serde_json::json!({
        "file_id": file_id,
        "size": metadata.size,
        "hash": metadata.hash,
    }))
}

/// 下载文件
async fn download_file(
    (Path(id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<Response> {
    let data = state.storage.read_file(&id).await.map_err(|e| {
        SilentError::business_error(StatusCode::NOT_FOUND, format!("文件不存在: {}", e))
    })?;

    let mut resp = Response::empty();
    resp.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/octet-stream"),
    );
    resp.set_body(full(data));
    Ok(resp)
}

/// 删除文件
async fn delete_file(
    (Path(id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    state.storage.delete_file(&id).await.map_err(|e| {
        SilentError::business_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("删除文件失败: {}", e),
        )
    })?;

    // 从搜索引擎删除索引
    if let Err(e) = state.search_engine.delete_file(&id).await {
        tracing::warn!("删除索引失败: {} - {}", id, e);
    }

    let event = FileEvent::new(EventType::Deleted, id, None);
    if let Some(ref n) = state.notifier {
        let _ = n.notify_deleted(event).await;
    }

    Ok(serde_json::json!({"success": true}))
}

/// 列出文件
async fn list_files(
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<Vec<crate::models::FileMetadata>> {
    state.storage.list_files().await.map_err(|e| {
        SilentError::business_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("列出文件失败: {}", e),
        )
    })
}

// ============ 同步相关 API ============

/// 获取同步状态
async fn get_sync_state(
    (Path(id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    match state.sync_manager.get_sync_state(&id).await {
        Some(sync_state) => Ok(serde_json::to_value(sync_state).unwrap()),
        None => Err(SilentError::business_error(
            StatusCode::NOT_FOUND,
            "同步状态不存在",
        )),
    }
}

/// 列出所有同步状态
async fn list_sync_states(
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let states = state.sync_manager.get_all_sync_states().await;
    Ok(serde_json::to_value(states).unwrap())
}

/// 获取冲突列表
async fn get_conflicts(
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let conflicts = state.sync_manager.check_conflicts().await;
    Ok(serde_json::to_value(conflicts).unwrap())
}

// ============ 版本管理相关 API ============

/// 列出文件版本
async fn list_versions(
    (Path(id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    let versions = state
        .version_manager
        .list_versions(&id)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("获取版本列表失败: {}", e),
            )
        })?;
    Ok(serde_json::to_value(versions).unwrap())
}

/// 获取特定版本
async fn get_version(
    (Path(version_id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<Response> {
    let data = state
        .version_manager
        .read_version(&version_id)
        .await
        .map_err(|e| {
            SilentError::business_error(StatusCode::NOT_FOUND, format!("版本不存在: {}", e))
        })?;

    let mut resp = Response::empty();
    resp.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/octet-stream"),
    );
    resp.set_body(full(data));
    Ok(resp)
}

/// 恢复版本
async fn restore_version(
    (Path(file_id), Path(version_id), CfgExtractor(state)): (
        Path<String>,
        Path<String>,
        CfgExtractor<AppState>,
    ),
) -> silent::Result<serde_json::Value> {
    let version = state
        .version_manager
        .restore_version(&file_id, &version_id)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("恢复版本失败: {}", e),
            )
        })?;

    // 发送修改事件
    if let Ok(metadata) = state.storage.get_metadata(&file_id).await {
        let event = FileEvent::new(EventType::Modified, file_id.clone(), Some(metadata));
        if let Some(ref n) = state.notifier {
            let _ = n.notify_modified(event).await;
        }
    }

    Ok(serde_json::to_value(version).unwrap())
}

/// 删除版本
async fn delete_version(
    (Path(version_id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    state
        .version_manager
        .delete_version(&version_id)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("删除版本失败: {}", e),
            )
        })?;
    Ok(serde_json::json!({"success": true}))
}

/// 获取版本统计
async fn get_version_stats(
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let stats = state.version_manager.get_stats().await;
    Ok(serde_json::to_value(stats).unwrap())
}

// ============ 增量同步 API ============

/// 获取文件签名
async fn get_file_signature(
    (Path(id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    let signature = api::handle_get_signature(&state.inc_sync_handler, &id)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("计算文件签名失败: {}", e),
            )
        })?;
    Ok(serde_json::to_value(signature).unwrap())
}

/// 获取文件差异
async fn get_file_delta(
    mut req: Request,
    (Path(id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    // 从请求体中读取目标签名
    let body = req.take_body();
    let body_bytes = match body {
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

    let request: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|e| {
        SilentError::business_error(StatusCode::BAD_REQUEST, format!("解析请求失败: {}", e))
    })?;

    let target_sig: FileSignature = serde_json::from_value(request["target_signature"].clone())
        .map_err(|e| {
            SilentError::business_error(StatusCode::BAD_REQUEST, format!("解析目标签名失败: {}", e))
        })?;

    let delta_chunks = api::handle_get_delta(&state.inc_sync_handler, &id, &target_sig)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("生成差异块失败: {}", e),
            )
        })?;

    Ok(serde_json::to_value(delta_chunks).unwrap())
}

// ============ 搜索 API ============

/// 搜索文件
async fn search_files(
    (Query(query), CfgExtractor(state)): (Query<SearchQuery>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    if query.q.is_empty() {
        return Err(SilentError::business_error(
            StatusCode::BAD_REQUEST,
            "搜索查询不能为空",
        ));
    }

    let results = state
        .search_engine
        .search(&query.q, query.limit, query.offset)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("搜索失败: {}", e),
            )
        })?;

    Ok(serde_json::to_value(results).unwrap())
}

/// 获取搜索统计
async fn get_search_stats(
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let stats = state.search_engine.get_stats();
    Ok(serde_json::to_value(stats).unwrap())
}

// ============ HTTP 服务器启动 ============

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
            .append(Route::new("files").post(upload_file).get(list_files))
            .append(
                Route::new("files/<id>")
                    .get(download_file)
                    .delete(delete_file),
            )
            .append(Route::new("files/<id>/versions").get(list_versions))
            .append(
                Route::new("files/<id>/versions/<version_id>")
                    .get(get_version)
                    .delete(delete_version),
            )
            .append(Route::new("files/<id>/versions/<version_id>/restore").post(restore_version))
            .append(Route::new("versions/stats").get(get_version_stats))
            .append(Route::new("sync/states").get(list_sync_states))
            .append(Route::new("sync/states/<id>").get(get_sync_state))
            .append(Route::new("sync/conflicts").get(get_conflicts))
            .append(Route::new("sync/signature/<id>").get(get_file_signature))
            .append(Route::new("sync/delta/<id>").post(get_file_delta))
            .append(Route::new("search").get(search_files))
            .append(Route::new("search/stats").get(get_search_stats))
            .append(Route::new("health").get(health))
            .append(Route::new("metrics").get(get_metrics)),
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
    use tempfile::TempDir;

    async fn create_test_storage() -> (StorageManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = StorageManager::new(
            temp_dir.path().to_path_buf(),
            64 * 1024, // 64KB chunk size for tests
        );
        storage.init().await.unwrap();
        (storage, temp_dir)
    }

    async fn create_test_app_state() -> (AppState, TempDir) {
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
        let query = SearchQuery {
            q: String::new(),
            limit: default_limit(),
            offset: 0,
        };

        assert_eq!(query.q, "");
        assert_eq!(query.limit, 20);
        assert_eq!(query.offset, 0);
    }

    #[test]
    fn test_search_query_custom() {
        let query = SearchQuery {
            q: "test query".to_string(),
            limit: 50,
            offset: 10,
        };

        assert_eq!(query.q, "test query");
        assert_eq!(query.limit, 50);
        assert_eq!(query.offset, 10);
    }

    #[test]
    fn test_default_limit() {
        assert_eq!(default_limit(), 20);
    }

    #[tokio::test]
    async fn test_health_check() {
        let req = Request::empty();
        let result = health(req).await;

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
    async fn test_upload_file_empty_body() {
        let (app_state, _temp_dir) = create_test_app_state().await;
        let req = Request::empty();

        let result = upload_file(req, CfgExtractor(app_state)).await;

        // 空请求体应该返回错误
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_files_empty() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let result = list_files(CfgExtractor(app_state)).await;

        assert!(result.is_ok());
        let files = result.unwrap();
        // 空存储应该返回空列表
        assert!(files.is_empty());
    }

    #[tokio::test]
    async fn test_download_nonexistent_file() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let result =
            download_file((Path("nonexistent-id".to_string()), CfgExtractor(app_state))).await;

        // 不存在的文件应该返回错误
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_file() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let result =
            delete_file((Path("nonexistent-id".to_string()), CfgExtractor(app_state))).await;

        // 删除不存在的文件应该返回错误
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_version_stats() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let result = get_version_stats(CfgExtractor(app_state)).await;

        assert!(result.is_ok());
        let stats = result.unwrap();
        assert!(stats.is_object());
    }

    #[tokio::test]
    async fn test_get_search_stats() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let result = get_search_stats(CfgExtractor(app_state)).await;

        assert!(result.is_ok());
        let stats = result.unwrap();
        assert!(stats.is_object());
    }

    #[tokio::test]
    async fn test_search_files_empty_query() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let query = SearchQuery {
            q: String::new(),
            limit: 20,
            offset: 0,
        };

        let result = search_files((Query(query), CfgExtractor(app_state))).await;

        // 空查询应该返回错误
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_search_files_valid_query() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let query = SearchQuery {
            q: "test".to_string(),
            limit: 20,
            offset: 0,
        };

        let result = search_files((Query(query), CfgExtractor(app_state))).await;

        assert!(result.is_ok());
        let results = result.unwrap();
        // 搜索结果应该是有效的JSON
        assert!(results.is_object() || results.is_array());
    }

    #[tokio::test]
    async fn test_list_sync_states() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let result = list_sync_states(CfgExtractor(app_state)).await;

        assert!(result.is_ok());
        let states = result.unwrap();
        // 返回值应该是有效的JSON
        assert!(states.is_object() || states.is_array());
    }

    #[tokio::test]
    async fn test_get_conflicts() {
        let (app_state, _temp_dir) = create_test_app_state().await;

        let result = get_conflicts(CfgExtractor(app_state)).await;

        assert!(result.is_ok());
        let conflicts = result.unwrap();
        assert!(conflicts.is_array());
    }
}
