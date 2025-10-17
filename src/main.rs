mod auth;
mod config;
mod error;
mod event_listener;
mod models;
mod node_sync;
mod node_sync_client;
mod node_sync_service;
mod notify;
mod rpc;
mod s3;
mod storage;
mod sync;
mod transfer;
mod version;
mod webdav;

use config::Config;
use error::Result;
use event_listener::EventListener;
use http_body_util::BodyExt;
use models::{EventType, FileEvent};
use notify::EventNotifier;
use rpc::FileServiceImpl;
use silent::prelude::*;
use std::net::SocketAddr;
use std::sync::Arc;
use storage::StorageManager;
use sync::SyncManager;
use tonic::transport::Server as TonicServer;
use tracing::{Level, error, info};
use tracing_subscriber as logger;
use version::{VersionConfig, VersionManager};

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    logger::fmt().with_max_level(Level::INFO).init();

    info!("Silent-NAS 服务器启动中...");

    // 加载配置
    let config = Config::load();
    info!("配置加载完成: {:?}", config);

    // 初始化存储管理器
    let storage = StorageManager::new(config.storage.root_path.clone(), config.storage.chunk_size);
    storage.init().await?;

    // 连接 NATS
    let notifier = EventNotifier::connect(&config.nats.url, config.nats.topic_prefix.clone())
        .await
        .map_err(|e| {
            error!("连接 NATS 失败: {}", e);
            e
        })?;

    // 初始化同步管理器
    let node_id = scru128::new_string();
    let sync_manager = SyncManager::new(
        node_id.clone(),
        Arc::new(storage.clone()),
        Arc::new(notifier.clone()),
    );
    info!("同步管理器已初始化: node_id={}", node_id);

    // 初始化版本管理器
    let version_config = VersionConfig::default();
    let version_manager = VersionManager::new(
        Arc::new(storage.clone()),
        version_config,
        &config.storage.root_path.to_string_lossy(),
    );
    version_manager.init().await?;
    info!("版本管理器已初始化");

    // 计算对外 HTTP 基址（优先 ADVERTISE_HOST，否则容器 HOSTNAME），用于事件携带源地址
    let advertise_host = std::env::var("ADVERTISE_HOST")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| config.server.host.clone());
    let source_http_addr = format!("http://{}:{}", advertise_host, config.server.http_port);

    // 启动事件监听器（监听其他节点的文件变更）
    let event_listener = EventListener::new(
        sync_manager.clone(),
        notifier.get_client(),
        config.nats.topic_prefix.clone(),
        storage.clone(),
    );
    tokio::spawn(async move {
        if let Err(e) = event_listener.start().await {
            error!("事件监听器错误: {}", e);
        }
    });

    // 启动 HTTP 服务器（使用 Silent 框架）
    let http_addr = format!("{}:{}", config.server.host, config.server.http_port);
    let http_addr_clone = http_addr.clone();
    let storage_clone = storage.clone();
    let notifier_clone = notifier.clone();
    let sync_clone = sync_manager.clone();
    let version_clone = version_manager.clone();
    // source_http_addr 已用于 HTTP/WebDAV/S3 三处，不再单独复制

    tokio::spawn(async move {
        if let Err(e) = start_http_server(
            &http_addr_clone,
            storage_clone,
            notifier_clone,
            sync_clone,
            version_clone,
        )
        .await
        {
            error!("HTTP 服务器错误: {}", e);
        }
    });

    // 启动定期巡检补拉任务
    let storage_reconcile = storage.clone();
    let sync_reconcile = sync_manager.clone();
    tokio::spawn(async move {
        use tokio::time::{Duration, sleep};
        loop {
            sleep(Duration::from_secs(30)).await;
            let states = sync_reconcile.get_all_sync_states().await;
            for st in states {
                if st.is_deleted() {
                    continue;
                }
                if let Some(meta) = st.get_metadata().cloned() {
                    let need_fetch = match storage_reconcile.get_metadata(&st.file_id).await {
                        Ok(local) => local.hash != meta.hash || local.size != meta.size,
                        Err(_) => true,
                    };
                    if need_fetch
                        && let Some(src) = sync_reconcile.get_last_source(&st.file_id).await
                    {
                        let url = format!("{}/api/files/{}", src.trim_end_matches('/'), st.file_id);
                        match reqwest::get(&url).await {
                            Ok(resp) if resp.status().is_success() => {
                                if let Ok(bytes) = resp.bytes().await {
                                    if let Err(e) =
                                        storage_reconcile.save_file(&st.file_id, &bytes).await
                                    {
                                        error!("补拉保存失败: {} - {}", st.file_id, e);
                                    } else {
                                        info!("📥 补拉已完成: {}", st.file_id);
                                    }
                                }
                            }
                            Ok(resp) => warn!("补拉HTTP失败: {} - {}", st.file_id, resp.status()),
                            Err(e) => warn!("补拉请求失败: {} - {}", st.file_id, e),
                        }
                    }
                }
            }
        }
    });

    // 启动 gRPC 服务器
    let grpc_addr: SocketAddr = format!("{}:{}", config.server.host, config.server.grpc_port)
        .parse()
        .expect("无效的 gRPC 地址");

    let storage_clone = storage.clone();
    let notifier_clone = notifier.clone();
    let source_http_addr_clone = source_http_addr.clone();

    tokio::spawn(async move {
        if let Err(e) = start_grpc_server(
            grpc_addr,
            storage_clone,
            notifier_clone,
            source_http_addr_clone,
        )
        .await
        {
            error!("gRPC 服务器错误: {}", e);
        }
    });

    // 启动 WebDAV 服务器
    let webdav_addr = format!("{}:{}", config.server.host, config.server.webdav_port);
    let webdav_addr_clone = webdav_addr.clone();
    let storage_webdav = storage.clone();
    let notifier_webdav = notifier.clone();
    let sync_webdav = sync_manager.clone();

    tokio::spawn(async move {
        let webdav_base = format!(
            "http://{}:{}",
            advertise_host,
            // 从监听地址中提取端口以确保一致
            webdav_addr_clone
                .rsplit(':')
                .next()
                .unwrap_or(&config.server.webdav_port.to_string())
        );
        if let Err(e) = start_webdav_server(
            &webdav_addr_clone,
            storage_webdav,
            notifier_webdav,
            sync_webdav,
            webdav_base,
        )
        .await
        {
            error!("WebDAV 服务器错误: {}", e);
        }
    });

    // 启动 S3 服务器
    let s3_addr = format!("{}:{}", config.server.host, config.server.s3_port);
    let s3_addr_clone = s3_addr.clone();
    let storage_s3 = storage.clone();
    let notifier_s3 = notifier.clone();
    let s3_config = config.s3.clone();
    let source_http_addr_for_s3 = source_http_addr.clone();

    tokio::spawn(async move {
        if let Err(e) = start_s3_server(
            &s3_addr_clone,
            storage_s3,
            notifier_s3,
            s3_config,
            source_http_addr_for_s3,
        )
        .await
        {
            error!("S3 服务器错误: {}", e);
        }
    });

    // 启动 QUIC 服务器
    let quic_addr: SocketAddr = format!("{}:{}", config.server.host, config.server.quic_port)
        .parse()
        .expect("无效的 QUIC 地址");

    let mut quic_server = transfer::QuicTransferServer::new(storage.clone(), notifier.clone());
    quic_server.start(quic_addr).await?;

    info!("所有服务已启动");
    info!("  HTTP:    http://{}", http_addr);
    info!("  gRPC:    {}", grpc_addr);
    info!("  WebDAV:  http://{}", webdav_addr);
    info!("  S3:      http://{}", s3_addr);
    info!("  QUIC:    {}", quic_addr);

    // 保持运行，优雅处理 SIGINT/SIGTERM（容器内常用 SIGTERM）
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        // 在容器内，优先监听 SIGTERM；避免某些环境下 ctrl_c() 立即返回导致进程退出
        let mut sigterm = signal(SignalKind::terminate()).expect("注册 SIGTERM 失败");
        sigterm.recv().await;
        info!("收到 SIGTERM 信号，正在退出...");
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.expect("监听 Ctrl+C 失败");
        info!("收到关闭信号，正在退出...");
    }

    Ok(())
}

/// 启动 HTTP 服务器（使用 Silent 框架）
async fn start_http_server(
    addr: &str,
    storage: StorageManager,
    notifier: EventNotifier,
    sync_manager: Arc<SyncManager>,
    version_manager: Arc<VersionManager>,
) -> Result<()> {
    let storage = Arc::new(storage);
    let notifier = Arc::new(notifier);

    // 健康检查
    async fn health(_req: Request) -> silent::Result<&'static str> {
        Ok("OK")
    }

    // 上传文件
    let storage_upload = storage.clone();
    let notifier_upload = notifier.clone();
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

            let mut event =
                FileEvent::new(EventType::Created, file_id.clone(), Some(metadata.clone()));
            event.source_http_addr = Some((*src_http).clone());
            let _ = notifier.notify_created(event).await;

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
    let delete = move |req: Request| {
        let storage = storage_delete.clone();
        let notifier = notifier_delete.clone();
        async move {
            let file_id: String = req.get_path_params("id")?;

            storage.delete_file(&file_id).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("删除文件失败: {}", e),
                )
            })?;

            let event = FileEvent::new(EventType::Deleted, file_id, None);
            let _ = notifier.notify_deleted(event).await;

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
                let _ = notifier.notify_modified(event).await;
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

/// 启动 gRPC 服务器
async fn start_grpc_server(
    addr: SocketAddr,
    storage: StorageManager,
    notifier: EventNotifier,
    source_http_addr: String,
) -> Result<()> {
    let file_service = FileServiceImpl::new(storage, notifier, Some(source_http_addr));

    info!("gRPC 服务器启动: {}", addr);

    TonicServer::builder()
        .add_service(file_service.into_server())
        .serve(addr)
        .await
        .map_err(|e| error::NasError::Storage(format!("gRPC 服务器错误: {}", e)))?;

    Ok(())
}

/// 启动 WebDAV 服务器
async fn start_webdav_server(
    addr: &str,
    storage: StorageManager,
    notifier: EventNotifier,
    sync_manager: Arc<SyncManager>,
    source_http_addr: String,
) -> Result<()> {
    let storage = Arc::new(storage);
    let notifier = Arc::new(notifier);

    let route = webdav::create_webdav_routes(storage, notifier, sync_manager, source_http_addr);

    info!("WebDAV 服务器启动: {}", addr);
    info!("  - WebDAV: http://{}/webdav", addr);

    Server::new()
        .bind(addr.parse().expect("无效的 WebDAV 地址"))
        .serve(route)
        .await;

    Ok(())
}

/// 启动 S3 服务器
async fn start_s3_server(
    addr: &str,
    storage: StorageManager,
    notifier: EventNotifier,
    s3_config: config::S3Config,
    source_http_addr: String,
) -> Result<()> {
    let storage = Arc::new(storage);
    let notifier = Arc::new(notifier);

    // 配置S3认证
    let auth = if s3_config.enable_auth {
        Some(s3::S3Auth::new(s3_config.access_key, s3_config.secret_key))
    } else {
        None
    };

    let route = s3::create_s3_routes(storage, notifier, auth, source_http_addr.clone());

    info!("S3 服务器启动: {}", addr);
    info!("  - S3 API: http://{}/", addr);

    Server::new()
        .bind(addr.parse().expect("无效的 S3 地址"))
        .serve(route)
        .await;

    Ok(())
}
