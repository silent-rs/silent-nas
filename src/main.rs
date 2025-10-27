mod audit;
mod auth;
mod cache;
mod config;
mod error;
mod event_listener;
mod http;
mod metrics;
mod models;
mod notify;
mod rpc;
mod s3;
mod search;
mod storage;
mod sync;
mod transfer;
mod version;
mod webdav;

use config::Config;
use error::Result;
use event_listener::EventListener;
use notify::EventNotifier;
use rpc::FileServiceImpl;
use sha2::Digest;
use silent::prelude::*;
use std::net::SocketAddr;
use std::sync::Arc;
use storage::StorageManager;
use sync::crdt::SyncManager;
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

    // 尝试连接 NATS（可选，单节点模式下可不连接）
    let notifier =
        EventNotifier::try_connect(&config.nats.url, config.nats.topic_prefix.clone()).await;
    if notifier.is_some() {
        info!("✅ NATS 已连接 - 多节点模式启用");
    } else {
        info!("ℹ️  未连接 NATS - 单节点模式运行");
    }

    // 初始化同步管理器
    let node_id = scru128::new_string();
    let sync_manager = SyncManager::new(
        node_id.clone(),
        Arc::new(storage.clone()),
        notifier.clone().map(Arc::new),
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

    // 初始化搜索引擎
    let index_path = std::path::PathBuf::from(&config.storage.root_path).join("index");
    let search_engine = Arc::new(crate::search::SearchEngine::new(index_path)?);
    info!("搜索引擎已初始化");

    // 计算对外 HTTP 基址（优先 ADVERTISE_HOST，否则容器 HOSTNAME），用于事件携带源地址
    let advertise_host = std::env::var("ADVERTISE_HOST")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| config.server.host.clone());
    let source_http_addr = format!("http://{}:{}", advertise_host, config.server.http_port);

    // 创建退出信号通道
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // 收集所有服务器的任务句柄，用于退出时中止
    let mut server_handles = Vec::new();

    // 启动事件监听器（仅在 NATS 连接成功时）
    if let Some(ref nats_notifier) = notifier {
        let event_listener = EventListener::new(
            sync_manager.clone(),
            nats_notifier.get_client(),
            config.nats.topic_prefix.clone(),
            storage.clone(),
            config.storage.chunk_size,
            config.sync.http_connect_timeout,
            config.sync.http_request_timeout,
            config.sync.fetch_max_retries,
            config.sync.fetch_base_backoff,
            config.sync.fetch_max_backoff,
        );
        let mut shutdown_rx_clone = shutdown_rx.clone();
        tokio::spawn(async move {
            tokio::select! {
                result = event_listener.start() => {
                    if let Err(e) = result {
                        error!("事件监听器错误: {}", e);
                    }
                }
                _ = shutdown_rx_clone.changed() => {
                    info!("事件监听器收到退出信号");
                }
            }
        });
        info!("事件监听器已启动");
    } else {
        info!("跳过事件监听器（单节点模式）");
    }

    // 启动 HTTP 服务器（使用 Silent 框架）
    let http_addr = format!("{}:{}", config.server.host, config.server.http_port);
    let http_addr_clone = http_addr.clone();
    let storage_clone = storage.clone();
    let notifier_clone = notifier.clone();
    let sync_clone = sync_manager.clone();
    let version_clone = version_manager.clone();
    let search_clone = search_engine.clone();
    let config_clone = config.clone();
    // source_http_addr 已用于 HTTP/WebDAV/S3 三处，不再单独复制

    let http_handle = tokio::spawn(async move {
        if let Err(e) = http::start_http_server(
            &http_addr_clone,
            storage_clone,
            notifier_clone,
            sync_clone,
            version_clone,
            search_clone,
            config_clone,
        )
        .await
        {
            error!("HTTP 服务器错误: {}", e);
        }
    });
    server_handles.push(http_handle);

    // 启动定期巡检补拉任务（仅在多节点/NATS开启时需要）
    if notifier.is_some() {
        let storage_reconcile = storage.clone();
        let sync_reconcile = sync_manager.clone();
        let sync_cfg_reconcile = config.sync.clone();
        let mut shutdown_rx_reconcile = shutdown_rx.clone();
        tokio::spawn(async move {
            use tokio::time::{Duration, sleep};
            loop {
                tokio::select! {
                    _ = sleep(Duration::from_secs(30)) => {
                        let states = sync_reconcile.get_all_sync_states().await;
                        for st in states {
                            if st.is_deleted() { continue; }
                            if let Some(meta) = st.get_metadata().cloned() {
                                let need_fetch = match storage_reconcile.get_metadata(&st.file_id).await {
                                    Ok(local) => local.hash != meta.hash || local.size != meta.size,
                                    Err(_) => true,
                                };
                                if need_fetch && let Some(src) = sync_reconcile.get_last_source(&st.file_id).await {
                                    let client = reqwest::Client::builder()
                                        .connect_timeout(Duration::from_secs(sync_cfg_reconcile.http_connect_timeout))
                                        .timeout(Duration::from_secs(sync_cfg_reconcile.http_request_timeout))
                                        .build()
                                        .unwrap_or_else(|_| reqwest::Client::new());
                                    let url = format!("{}/api/files/{}", src.trim_end_matches('/'), st.file_id);
                                    let mut last_err: Option<String> = None;
                                    let mut ok = false;
                                    for attempt in 0..=sync_cfg_reconcile.fetch_max_retries {
                                        match client.get(&url).send().await {
                                            Ok(resp) if resp.status().is_success() => {
                                                if let Ok(bytes) = resp.bytes().await {
                                                    let actual = format!("{:x}", sha2::Sha256::digest(&bytes));
                                                    if actual != meta.hash {
                                                        last_err = Some(format!("哈希不一致 expected={} actual={}", meta.hash, actual));
                                                    } else if let Err(e) = storage_reconcile.save_file(&st.file_id, &bytes).await {
                                                        last_err = Some(format!("保存失败: {}", e));
                                                    } else {
                                                        info!("📥 补拉已完成: {}", st.file_id);
                                                        ok = true;
                                                        break;
                                                    }
                                                }
                                            }
                                            Ok(resp) => { last_err = Some(format!("HTTP {}", resp.status())); }
                                            Err(e) => { last_err = Some(format!("请求失败: {}", e)); }
                                        }
                                        if attempt < sync_cfg_reconcile.fetch_max_retries {
                                            let factor = 1u64 << (attempt.min(6));
                                            let mut secs = sync_cfg_reconcile.fetch_base_backoff.saturating_mul(factor);
                                            if secs > sync_cfg_reconcile.fetch_max_backoff { secs = sync_cfg_reconcile.fetch_max_backoff; }
                                            let jitter = rand::random::<f64>() * 0.4 + 0.8;
                                            let dur = Duration::from_secs(((secs as f64) * jitter).round() as u64);
                                            sleep(dur).await;
                                        }
                                    }
                                    if !ok {
                                        warn!("补拉失败: {} - {}", st.file_id, last_err.unwrap_or_else(||"unknown".into()));
                                    }
                                }
                            }
                        }
                    }
                    _ = shutdown_rx_reconcile.changed() => {
                        info!("巡检补拉任务收到退出信号");
                        break;
                    }
                }
            }
        });
    } else {
        debug!("跳过巡检补拉任务（单节点或 NATS 未启用）");
    }

    // 启动 gRPC 服务器
    let grpc_addr: SocketAddr = format!("{}:{}", config.server.host, config.server.grpc_port)
        .parse()
        .expect("无效的 gRPC 地址");

    let storage_clone = storage.clone();
    let notifier_clone = notifier.clone();
    let source_http_addr_clone = source_http_addr.clone();

    let sync_for_grpc = sync_manager.clone();
    let node_cfg = config.node.clone();
    let sync_cfg = config.sync.clone();
    let grpc_handle = tokio::spawn(async move {
        if let Err(e) = start_grpc_server(
            grpc_addr,
            storage_clone,
            notifier_clone,
            source_http_addr_clone,
            sync_for_grpc,
            node_cfg,
            sync_cfg,
        )
        .await
        {
            error!("gRPC 服务器错误: {}", e);
        }
    });
    server_handles.push(grpc_handle);

    // 启动 WebDAV 服务器
    let webdav_addr = format!("{}:{}", config.server.host, config.server.webdav_port);
    let webdav_addr_clone = webdav_addr.clone();
    let storage_webdav = storage.clone();
    let notifier_webdav = notifier.clone();
    let sync_webdav = sync_manager.clone();
    let version_webdav = version_manager.clone();

    let webdav_handle = tokio::spawn(async move {
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
            version_webdav,
        )
        .await
        {
            error!("WebDAV 服务器错误: {}", e);
        }
    });
    server_handles.push(webdav_handle);

    // 初始化 S3 版本控制管理器
    let s3_versioning_manager = Arc::new(s3::VersioningManager::new());
    info!("S3 版本控制管理器已初始化");

    // 启动 S3 服务器
    let s3_addr = format!("{}:{}", config.server.host, config.server.s3_port);
    let s3_addr_clone = s3_addr.clone();
    let storage_s3 = storage.clone();
    let notifier_s3 = notifier.clone();
    let s3_config = config.s3.clone();
    let source_http_addr_for_s3 = source_http_addr.clone();
    let s3_versioning_clone = s3_versioning_manager.clone();
    let version_s3 = version_manager.clone();

    let s3_handle = tokio::spawn(async move {
        if let Err(e) = start_s3_server(
            &s3_addr_clone,
            storage_s3,
            notifier_s3,
            s3_config,
            source_http_addr_for_s3,
            s3_versioning_clone,
            version_s3,
        )
        .await
        {
            error!("S3 服务器错误: {}", e);
        }
    });
    server_handles.push(s3_handle);

    // 启动 QUIC 服务器
    let quic_addr: SocketAddr = format!("{}:{}", config.server.host, config.server.quic_port)
        .parse()
        .expect("无效的 QUIC 地址");

    let storage_quic = storage.clone();
    let notifier_quic = notifier.clone();
    let quic_handle = tokio::spawn(async move {
        let mut quic_server = transfer::QuicTransferServer::new(storage_quic, notifier_quic);
        if let Err(e) = quic_server.start(quic_addr).await {
            error!("QUIC 服务器错误: {}", e);
        }
    });
    server_handles.push(quic_handle);

    info!("所有服务已启动");
    info!("  HTTP:    http://{}", http_addr);
    info!("  gRPC:    {}", grpc_addr);
    info!("  WebDAV:  http://{}", webdav_addr);
    info!("  S3:      http://{}", s3_addr);
    info!("  QUIC:    {}", quic_addr);

    // 保持运行，优雅处理 SIGINT/SIGTERM（同时监听两种信号）
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate()).expect("注册 SIGTERM 失败");
        let mut sigint = signal(SignalKind::interrupt()).expect("注册 SIGINT 失败");

        tokio::select! {
            _ = sigterm.recv() => {
                info!("收到 SIGTERM 信号，正在退出...");
            }
            _ = sigint.recv() => {
                info!("收到 SIGINT 信号 (Ctrl+C)，正在退出...");
            }
        }
    }

    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.expect("监听 Ctrl+C 失败");
        info!("收到关闭信号，正在退出...");
    }

    // 发送退出信号给所有后台任务
    let _ = shutdown_tx.send(true);
    info!("已通知所有后台任务退出");

    // 中止所有服务器任务
    for handle in server_handles {
        handle.abort();
    }
    info!("已中止所有服务器任务");

    // 等待一小段时间让任务清理
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    info!("应用已退出");

    Ok(())
}

/// 启动 gRPC 服务器
async fn start_grpc_server(
    addr: SocketAddr,
    storage: StorageManager,
    notifier: Option<EventNotifier>,
    source_http_addr: String,
    sync_manager: Arc<SyncManager>,
    node_cfg: config::NodeConfig,
    sync_cfg: config::SyncBehaviorConfig,
) -> Result<()> {
    use crate::sync::node::manager::{
        NodeDiscoveryConfig, NodeManager, NodeSyncCoordinator, SyncConfig,
    };
    use crate::sync::node::service::NodeSyncServiceImpl;

    let file_service = FileServiceImpl::new(
        storage.clone(),
        notifier.clone(),
        Some(source_http_addr.clone()),
    );

    // 初始化节点同步服务（NodeSyncService）
    let listen_addr = addr.to_string();
    let node_discovery = NodeDiscoveryConfig {
        node_id: sync_manager.node_id().to_string(),
        listen_addr: listen_addr.clone(),
        seed_nodes: if node_cfg.enable {
            node_cfg.seed_nodes.clone()
        } else {
            Vec::new()
        },
        heartbeat_interval: node_cfg.heartbeat_interval,
        node_timeout: node_cfg.node_timeout,
    };

    let node_manager = NodeManager::new(node_discovery, sync_manager.clone());
    let node_sync = NodeSyncCoordinator::new(
        SyncConfig {
            auto_sync: sync_cfg.auto_sync,
            sync_interval: sync_cfg.sync_interval,
            max_files_per_sync: sync_cfg.max_files_per_sync,
            max_retries: sync_cfg.max_retries,
            fail_queue_max: sync_cfg.fail_queue_max,
            fail_task_ttl_secs: sync_cfg.fail_task_ttl_secs,
        },
        node_manager.clone(),
        sync_manager.clone(),
        Arc::new(storage.clone()),
    );

    // 启动节点心跳与自动同步任务
    if node_cfg.enable {
        let nm_for_heartbeat = node_manager.clone();
        tokio::spawn(async move { nm_for_heartbeat.start_heartbeat_check().await });
        // 启动向外发送心跳任务，降低节点离线误判概率
        let nm_for_outbound = node_manager.clone();
        tokio::spawn(async move { nm_for_outbound.start_outbound_heartbeat().await });
    }

    if node_cfg.enable && sync_cfg.auto_sync {
        let nsc_for_auto = node_sync.clone();
        tokio::spawn(async move { nsc_for_auto.start_auto_sync().await });
    }

    // 可选：连接到种子节点（默认空列表）
    if node_cfg.enable
        && !node_cfg.seed_nodes.is_empty()
        && let Err(e) = node_manager.connect_to_seeds().await
    {
        tracing::warn!("连接种子节点失败: {}", e);
    }

    let node_service = NodeSyncServiceImpl::new(
        node_manager,
        node_sync,
        sync_manager,
        Arc::new(storage.clone()),
    );

    info!("gRPC 服务器启动: {}", addr);

    TonicServer::builder()
        .add_service(file_service.into_server())
        .add_service(node_service.into_server())
        .serve(addr)
        .await
        .map_err(|e| error::NasError::Storage(format!("gRPC 服务器错误: {}", e)))?;

    Ok(())
}

/// 启动 WebDAV 服务器
async fn start_webdav_server(
    addr: &str,
    storage: StorageManager,
    notifier: Option<EventNotifier>,
    sync_manager: Arc<SyncManager>,
    source_http_addr: String,
    version_manager: Arc<VersionManager>,
) -> Result<()> {
    let storage = Arc::new(storage);
    let notifier = notifier.map(Arc::new);

    let route = webdav::create_webdav_routes(
        storage,
        notifier,
        sync_manager,
        source_http_addr,
        version_manager,
    );

    info!("WebDAV 服务器启动: {}", addr);
    // 实际挂载在根路径，避免误导为 /webdav
    info!("  - WebDAV: http://{}/", addr);

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
    notifier: Option<EventNotifier>,
    s3_config: config::S3Config,
    source_http_addr: String,
    versioning_manager: Arc<s3::VersioningManager>,
    version_manager: Arc<VersionManager>,
) -> Result<()> {
    let storage = Arc::new(storage);
    let notifier = notifier.map(Arc::new);

    // 配置S3认证
    let auth = if s3_config.enable_auth {
        Some(s3::S3Auth::new(s3_config.access_key, s3_config.secret_key))
    } else {
        None
    };

    let route = s3::create_s3_routes(
        storage,
        notifier,
        auth,
        source_http_addr.clone(),
        versioning_manager,
        version_manager,
    );

    info!("S3 服务器启动: {}", addr);
    info!("  - S3 API: http://{}/", addr);

    Server::new()
        .bind(addr.parse().expect("无效的 S3 地址"))
        .serve(route)
        .await;

    Ok(())
}
