mod config;
mod error;
mod models;
mod notify;
mod rpc;
mod storage;
mod transfer;

use config::Config;
use error::Result;
use http_body_util::BodyExt;
use models::{EventType, FileEvent};
use notify::EventNotifier;
use rpc::FileServiceImpl;
use silent::prelude::*;
use std::net::SocketAddr;
use std::sync::Arc;
use storage::StorageManager;
use tonic::transport::Server as TonicServer;
use tracing::{error, info};

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

    // 启动 HTTP 服务器（使用 Silent 框架）
    let http_addr = format!("{}:{}", config.server.host, config.server.http_port);
    let http_addr_clone = http_addr.clone();
    let storage_clone = storage.clone();
    let notifier_clone = notifier.clone();

    tokio::spawn(async move {
        if let Err(e) = start_http_server(&http_addr_clone, storage_clone, notifier_clone).await {
            error!("HTTP 服务器错误: {}", e);
        }
    });

    // 启动 gRPC 服务器
    let grpc_addr: SocketAddr = format!("{}:{}", config.server.host, config.server.grpc_port)
        .parse()
        .expect("无效的 gRPC 地址");

    let storage_clone = storage.clone();
    let notifier_clone = notifier.clone();

    tokio::spawn(async move {
        if let Err(e) = start_grpc_server(grpc_addr, storage_clone, notifier_clone).await {
            error!("gRPC 服务器错误: {}", e);
        }
    });

    // 启动 QUIC 服务器
    let quic_addr: SocketAddr = format!("{}:{}", config.server.host, config.server.quic_port)
        .parse()
        .expect("无效的 QUIC 地址");

    let mut quic_server = transfer::QuicTransferServer::new(storage.clone(), notifier.clone());
    quic_server.start(quic_addr).await?;

    info!("所有服务已启动");
    info!("  HTTP:  http://{}", http_addr);
    info!("  gRPC:  {}", grpc_addr);
    info!("  QUIC:  {}", quic_addr);

    // 保持运行
    tokio::signal::ctrl_c().await.expect("监听 Ctrl+C 失败");
    info!("收到关闭信号，正在退出...");

    Ok(())
}

/// 启动 HTTP 服务器（使用 Silent 框架）
async fn start_http_server(
    addr: &str,
    storage: StorageManager,
    notifier: EventNotifier,
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
    let upload = move |mut req: Request| {
        let storage = storage_upload.clone();
        let notifier = notifier_upload.clone();
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

            let event = FileEvent::new(EventType::Created, file_id.clone(), Some(metadata.clone()));
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

    let route = Route::new("api")
        .append(Route::new("files").post(upload).get(list))
        .append(Route::new("files/<id>").get(download).delete(delete))
        .append(Route::new("health").get(health));

    info!("HTTP 服务器启动: {}", addr);
    Server::new().run(route);

    Ok(())
}

/// 启动 gRPC 服务器
async fn start_grpc_server(
    addr: SocketAddr,
    storage: StorageManager,
    notifier: EventNotifier,
) -> Result<()> {
    let file_service = FileServiceImpl::new(storage, notifier);

    info!("gRPC 服务器启动: {}", addr);

    TonicServer::builder()
        .add_service(file_service.into_server())
        .serve(addr)
        .await
        .map_err(|e| error::NasError::Storage(format!("gRPC 服务器错误: {}", e)))?;

    Ok(())
}
