//! HTTP 服务器模块
//!
//! 提供 REST API 服务，包括文件操作、同步管理、版本控制等功能

use crate::error::Result;
use crate::models::{EventType, FileEvent};
use crate::notify::EventNotifier;
use crate::search::SearchEngine;
use crate::storage::StorageManager;
use crate::version::VersionManager;
use http::StatusCode;
use http_body_util::BodyExt;
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

/// 解析查询参数
fn parse_query_param(uri: &http::Uri, key: &str) -> Option<String> {
    uri.query()?.split('&').find_map(|pair| {
        let mut parts = pair.splitn(2, '=');
        if parts.next()? == key {
            let value = parts.next()?;
            Some(urlencoding::decode(value).ok()?.to_string())
        } else {
            None
        }
    })
}

/// 启动 HTTP 服务器（使用 Silent 框架）
pub async fn start_http_server(
    addr: &str,
    storage: StorageManager,
    notifier: Option<EventNotifier>,
    sync_manager: Arc<SyncManager>,
    version_manager: Arc<VersionManager>,
    search_engine: Arc<SearchEngine>,
) -> Result<()> {
    let storage = Arc::new(storage);
    let notifier = notifier.map(Arc::new);

    // 创建增量同步处理器
    let inc_sync_handler = Arc::new(IncrementalSyncHandler::new(
        storage.clone(),
        64 * 1024, // 使用64KB块大小
    ));

    // 健康检查
    async fn health(_req: Request) -> silent::Result<&'static str> {
        Ok("OK")
    }

    // 上传文件
    let storage_upload = storage.clone();
    let notifier_upload = notifier.clone();
    let search_upload = search_engine.clone();
    let advertise_host = std::env::var("ADVERTISE_HOST")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "localhost".to_string());
    let http_port: u16 = addr
        .rsplit(':')
        .next()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let source_http_addr = std::sync::Arc::new(format!("http://{}:{}", advertise_host, http_port));
    let upload = move |mut req: Request| {
        let storage = storage_upload.clone();
        let notifier = notifier_upload.clone();
        let search = search_upload.clone();
        let src_http = source_http_addr.clone();
        async move {
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

            let metadata = storage.save_file(&file_id, &bytes).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("保存文件失败: {}", e),
                )
            })?;

            // 索引文件到搜索引擎
            if let Err(e) = search.index_file(&metadata).await {
                tracing::warn!("索引文件失败: {} - {}", file_id, e);
            }

            let mut event =
                FileEvent::new(EventType::Created, file_id.clone(), Some(metadata.clone()));
            event.source_http_addr = Some((*src_http).clone());
            if let Some(ref n) = notifier {
                let _ = n.notify_created(event).await;
            }

            Ok(serde_json::json!({
                "file_id": file_id,
                "size": metadata.size,
                "hash": metadata.hash,
            }))
        }
    };

    // 下载文件
    let storage_download = storage.clone();
    let download = move |req: Request| {
        let storage = storage_download.clone();
        async move {
            let file_id: String = req.get_path_params("id")?;

            let data = storage.read_file(&file_id).await.map_err(|e| {
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
    };

    // 删除文件
    let storage_delete = storage.clone();
    let notifier_delete = notifier.clone();
    let search_delete = search_engine.clone();
    let delete = move |req: Request| {
        let storage = storage_delete.clone();
        let notifier = notifier_delete.clone();
        let search = search_delete.clone();
        async move {
            let file_id: String = req.get_path_params("id")?;

            storage.delete_file(&file_id).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("删除文件失败: {}", e),
                )
            })?;

            // 从搜索引擎删除索引
            if let Err(e) = search.delete_file(&file_id).await {
                tracing::warn!("删除索引失败: {} - {}", file_id, e);
            }

            let event = FileEvent::new(EventType::Deleted, file_id, None);
            if let Some(ref n) = notifier {
                let _ = n.notify_deleted(event).await;
            }

            Ok(serde_json::json!({"success": true}))
        }
    };

    // 列出文件
    let storage_list = storage.clone();
    let list = move |_req: Request| {
        let storage = storage_list.clone();
        async move {
            let files = storage.list_files().await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("列出文件失败: {}", e),
                )
            })?;
            Ok(files)
        }
    };

    // 同步相关 API
    let sync_get_state = sync_manager.clone();
    let get_sync_state = move |req: Request| {
        let sync = sync_get_state.clone();
        async move {
            let file_id: String = req.get_path_params("id")?;
            match sync.get_sync_state(&file_id).await {
                Some(state) => Ok(serde_json::to_value(state).unwrap()),
                None => Err(SilentError::business_error(
                    StatusCode::NOT_FOUND,
                    "同步状态不存在",
                )),
            }
        }
    };

    let sync_list_states = sync_manager.clone();
    let list_sync_states = move |_req: Request| {
        let sync = sync_list_states.clone();
        async move {
            let states = sync.get_all_sync_states().await;
            Ok(serde_json::to_value(states).unwrap())
        }
    };

    let sync_conflicts = sync_manager.clone();
    let get_conflicts = move |_req: Request| {
        let sync = sync_conflicts.clone();
        async move {
            let conflicts = sync.check_conflicts().await;
            Ok(serde_json::to_value(conflicts).unwrap())
        }
    };

    // 版本管理相关 API
    let version_list = version_manager.clone();
    let list_versions = move |req: Request| {
        let vm = version_list.clone();
        async move {
            let file_id: String = req.get_path_params("id")?;
            let versions = vm.list_versions(&file_id).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("获取版本列表失败: {}", e),
                )
            })?;
            Ok(serde_json::to_value(versions).unwrap())
        }
    };

    let version_get = version_manager.clone();
    let get_version = move |req: Request| {
        let vm = version_get.clone();
        async move {
            let version_id: String = req.get_path_params("version_id")?;
            let data = vm.read_version(&version_id).await.map_err(|e| {
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
    };

    let version_restore = version_manager.clone();
    let storage_restore = storage.clone();
    let notifier_restore = notifier.clone();
    let restore_version = move |req: Request| {
        let vm = version_restore.clone();
        let storage = storage_restore.clone();
        let notifier = notifier_restore.clone();
        async move {
            let file_id: String = req.get_path_params("id")?;
            let version_id: String = req.get_path_params("version_id")?;

            let version = vm
                .restore_version(&file_id, &version_id)
                .await
                .map_err(|e| {
                    SilentError::business_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("恢复版本失败: {}", e),
                    )
                })?;

            // 发送修改事件
            if let Ok(metadata) = storage.get_metadata(&file_id).await {
                let event = FileEvent::new(EventType::Modified, file_id.clone(), Some(metadata));
                if let Some(ref n) = notifier {
                    let _ = n.notify_modified(event).await;
                }
            }

            Ok(serde_json::to_value(version).unwrap())
        }
    };

    let version_delete = version_manager.clone();
    let delete_version = move |req: Request| {
        let vm = version_delete.clone();
        async move {
            let version_id: String = req.get_path_params("version_id")?;
            vm.delete_version(&version_id).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("删除版本失败: {}", e),
                )
            })?;
            Ok(serde_json::json!({"success": true}))
        }
    };

    let version_stats = version_manager.clone();
    let get_version_stats = move |_req: Request| {
        let vm = version_stats.clone();
        async move {
            let stats = vm.get_stats().await;
            Ok(serde_json::to_value(stats).unwrap())
        }
    };

    // 增量同步 API（使用独立模块）
    let inc_sync_signature = inc_sync_handler.clone();
    let get_file_signature = move |req: Request| {
        let handler = inc_sync_signature.clone();
        async move {
            let file_id: String = req.get_path_params("id")?;
            let signature = api::handle_get_signature(&handler, &file_id)
                .await
                .map_err(|e| {
                    SilentError::business_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("计算文件签名失败: {}", e),
                    )
                })?;
            Ok(serde_json::to_value(signature).unwrap())
        }
    };

    let inc_sync_delta = inc_sync_handler.clone();
    let get_file_delta = move |mut req: Request| {
        let handler = inc_sync_delta.clone();
        async move {
            let file_id: String = req.get_path_params("id")?;

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

            let target_sig: FileSignature =
                serde_json::from_value(request["target_signature"].clone()).map_err(|e| {
                    SilentError::business_error(
                        StatusCode::BAD_REQUEST,
                        format!("解析目标签名失败: {}", e),
                    )
                })?;

            let delta_chunks = api::handle_get_delta(&handler, &file_id, &target_sig)
                .await
                .map_err(|e| {
                    SilentError::business_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("生成差异块失败: {}", e),
                    )
                })?;

            Ok(serde_json::to_value(delta_chunks).unwrap())
        }
    };

    // 搜索 API
    let search_query = search_engine.clone();
    let search_files = move |req: Request| {
        let search = search_query.clone();
        async move {
            // 获取查询参数
            let query = parse_query_param(req.uri(), "q").unwrap_or_default();

            let limit: usize = parse_query_param(req.uri(), "limit")
                .and_then(|v| v.parse().ok())
                .unwrap_or(20);

            let offset: usize = parse_query_param(req.uri(), "offset")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);

            if query.is_empty() {
                return Err(SilentError::business_error(
                    StatusCode::BAD_REQUEST,
                    "搜索查询不能为空",
                ));
            }

            let results = search.search(&query, limit, offset).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("搜索失败: {}", e),
                )
            })?;

            Ok(serde_json::to_value(results).unwrap())
        }
    };

    let search_stats = search_engine.clone();
    let get_search_stats = move |_req: Request| {
        let search = search_stats.clone();
        async move {
            let stats = search.get_stats();
            Ok(serde_json::to_value(stats).unwrap())
        }
    };

    // 定期提交索引
    let search_commit = search_engine.clone();
    tokio::spawn(async move {
        use tokio::time::{Duration, interval};
        let mut timer = interval(Duration::from_secs(30));
        loop {
            timer.tick().await;
            if let Err(e) = search_commit.commit().await {
                tracing::warn!("定期提交索引失败: {}", e);
            }
        }
    });

    let route = Route::new_root().append(
        Route::new("api")
            .append(Route::new("files").post(upload).get(list))
            .append(Route::new("files/<id>").get(download).delete(delete))
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
            .append(Route::new("health").get(health)),
    );

    info!("HTTP 服务器启动: {}", addr);
    info!("  - REST API: http://{}/api", addr);

    Server::new()
        .bind(addr.parse().expect("无效的 HTTP 地址"))
        .serve(route)
        .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_http_module_exists() {
        // 确保模块可以编译通过
    }
}
