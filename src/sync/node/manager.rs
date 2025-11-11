// 跨节点文件同步模块
#![allow(dead_code)]

use crate::error::{NasError, Result};
use crate::sync::crdt::SyncManager;
use chrono::{Local, NaiveDateTime};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, interval};
use tracing::{debug, error, info, warn};

/// 节点信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    /// 节点 ID
    pub node_id: String,
    /// 节点地址 (host:port)
    pub address: String,
    /// 最后心跳时间
    pub last_seen: NaiveDateTime,
    /// 节点版本
    pub version: String,
    /// 节点元数据
    pub metadata: HashMap<String, String>,
    /// 节点状态
    pub status: NodeStatus,
}

/// 节点状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeStatus {
    /// 在线
    Online,
    /// 离线
    Offline,
    /// 故障
    Faulty,
}

impl NodeInfo {
    pub fn new(node_id: String, address: String, version: String) -> Self {
        Self {
            node_id,
            address,
            last_seen: Local::now().naive_local(),
            version,
            metadata: HashMap::new(),
            status: NodeStatus::Online,
        }
    }

    /// 更新心跳时间
    pub fn update_heartbeat(&mut self) {
        self.last_seen = Local::now().naive_local();
        self.status = NodeStatus::Online;
    }

    /// 检查节点是否在线
    pub fn is_alive(&self, timeout_secs: i64) -> bool {
        let now = Local::now().naive_local();
        let elapsed = (now - self.last_seen).num_seconds();
        elapsed < timeout_secs
    }
}

/// 节点发现配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDiscoveryConfig {
    /// 当前节点 ID
    pub node_id: String,
    /// 监听地址
    pub listen_addr: String,
    /// 已知节点列表（启动时连接）
    pub seed_nodes: Vec<String>,
    /// 心跳间隔（秒）
    pub heartbeat_interval: u64,
    /// 节点超时时间（秒）
    pub node_timeout: i64,
}

impl Default for NodeDiscoveryConfig {
    fn default() -> Self {
        Self {
            node_id: format!("node-{}", scru128::new_string()),
            listen_addr: "127.0.0.1:9000".to_string(),
            seed_nodes: Vec::new(),
            heartbeat_interval: 10,
            node_timeout: 30,
        }
    }
}

/// 节点管理器
pub struct NodeManager {
    /// 配置
    config: NodeDiscoveryConfig,
    /// 已知节点列表
    nodes: Arc<RwLock<HashMap<String, NodeInfo>>>,
    /// 同步管理器
    sync_manager: Arc<SyncManager>,
}

impl NodeManager {
    pub fn new(config: NodeDiscoveryConfig, sync_manager: Arc<SyncManager>) -> Arc<Self> {
        Arc::new(Self {
            config,
            nodes: Arc::new(RwLock::new(HashMap::new())),
            sync_manager,
        })
    }

    /// 注册一个新节点
    pub async fn register_node(&self, node: NodeInfo) -> Result<()> {
        let mut nodes = self.nodes.write().await;

        info!("注册新节点: {} @ {}", node.node_id, node.address);

        nodes.insert(node.node_id.clone(), node);
        Ok(())
    }

    /// 移除节点
    pub async fn remove_node(&self, node_id: &str) -> Result<()> {
        let mut nodes = self.nodes.write().await;

        if nodes.remove(node_id).is_some() {
            info!("移除节点: {}", node_id);
            Ok(())
        } else {
            Err(NasError::Other(format!("节点不存在: {}", node_id)))
        }
    }

    /// 更新节点心跳
    pub async fn update_heartbeat(&self, node_id: &str) -> Result<()> {
        let mut nodes = self.nodes.write().await;

        if let Some(node) = nodes.get_mut(node_id) {
            node.update_heartbeat();
            debug!("更新节点心跳: {}", node_id);
            Ok(())
        } else {
            Err(NasError::Other(format!("节点不存在: {}", node_id)))
        }
    }

    /// 获取所有节点
    pub async fn list_nodes(&self) -> Vec<NodeInfo> {
        let nodes = self.nodes.read().await;
        nodes.values().cloned().collect()
    }

    /// 获取在线节点
    pub async fn list_online_nodes(&self) -> Vec<NodeInfo> {
        let nodes = self.nodes.read().await;
        nodes
            .values()
            .filter(|n| n.is_alive(self.config.node_timeout))
            .cloned()
            .collect()
    }

    /// 启动对外心跳发送任务（周期性向已知节点发送心跳）
    pub async fn start_outbound_heartbeat(self: Arc<Self>) {
        let mut interval = interval(Duration::from_secs(self.config.heartbeat_interval));

        tokio::spawn(async move {
            loop {
                interval.tick().await;

                let nodes_snapshot: Vec<(String, String)> = {
                    let nodes = self.nodes.read().await;
                    nodes
                        .values()
                        .map(|n| (n.node_id.clone(), n.address.clone()))
                        .collect()
                };

                for (node_id, address) in nodes_snapshot {
                    if let Err(e) = self.send_heartbeat_to_node(&node_id, &address).await {
                        warn!("向节点发送心跳失败: {} @ {}, 错误: {}", node_id, address, e);
                    } else {
                        debug!("已发送心跳: {} @ {}", node_id, address);
                    }
                }
            }
        });
    }

    /// 启动心跳检查任务
    pub async fn start_heartbeat_check(self: Arc<Self>) {
        let mut interval = interval(Duration::from_secs(self.config.heartbeat_interval));

        tokio::spawn(async move {
            loop {
                interval.tick().await;

                let mut nodes_to_remove = Vec::new();
                {
                    let nodes = self.nodes.read().await;
                    for (node_id, node) in nodes.iter() {
                        if !node.is_alive(self.config.node_timeout) {
                            warn!("节点超时: {} @ {}", node_id, node.address);
                            nodes_to_remove.push(node_id.clone());
                        }
                    }
                }

                // 移除超时节点
                for node_id in nodes_to_remove {
                    if let Err(e) = self.remove_node(&node_id).await {
                        error!("移除超时节点失败: {}", e);
                    }
                }

                debug!(
                    "心跳检查完成，在线节点数: {}",
                    self.list_online_nodes().await.len()
                );
            }
        });
    }

    /// 连接到种子节点
    pub async fn connect_to_seeds(&self) -> Result<()> {
        use crate::sync::node::client::{ClientConfig, NodeSyncClient};

        for seed_addr in &self.config.seed_nodes {
            info!("连接到种子节点: {}", seed_addr);

            // 创建客户端并连接
            let client = NodeSyncClient::new(seed_addr.clone(), ClientConfig::default());

            match client.connect().await {
                Ok(_) => {
                    // 注册当前节点
                    let current_node = NodeInfo::new(
                        self.config.node_id.clone(),
                        self.config.listen_addr.clone(),
                        env!("CARGO_PKG_VERSION").to_string(),
                    );

                    match client.register_node(&current_node).await {
                        Ok(known_nodes) => {
                            info!(
                                "成功注册到种子节点 {}, 发现 {} 个节点",
                                seed_addr,
                                known_nodes.len()
                            );

                            // 注册所有已知节点
                            for node in known_nodes {
                                if node.node_id != self.config.node_id {
                                    let _ = self.register_node(node).await;
                                }
                            }
                        }
                        Err(e) => {
                            warn!("注册到种子节点 {} 失败: {}", seed_addr, e);
                        }
                    }
                }
                Err(e) => {
                    warn!("连接到种子节点 {} 失败: {}", seed_addr, e);
                }
            }
        }

        Ok(())
    }

    /// 向指定节点发送心跳
    pub async fn send_heartbeat_to_node(&self, _node_id: &str, address: &str) -> Result<()> {
        use crate::sync::node::{client::ClientConfig, client::NodeSyncClient};

        let client = NodeSyncClient::new(address.to_string(), ClientConfig::default());
        client.connect().await?;
        client.send_heartbeat(&self.config.node_id).await?;

        Ok(())
    }
}

/// 同步配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// 是否启用自动同步
    pub auto_sync: bool,
    /// 同步间隔（秒）
    pub sync_interval: u64,
    /// 每次同步的最大文件数
    pub max_files_per_sync: usize,
    /// 同步并发文件数
    pub max_concurrency: usize,
    /// 同步重试次数
    pub max_retries: u32,
    /// 失败补偿队列容量上限
    pub fail_queue_max: usize,
    /// 失败任务TTL（秒）
    pub fail_task_ttl_secs: u64,
    /// gRPC 连接超时（秒）
    pub grpc_connect_timeout: u64,
    /// gRPC 请求超时（秒）
    pub grpc_request_timeout: u64,
    /// 故障注入：传输失败概率（0.0-1.0）
    pub fault_transfer_error_rate: f64,
    /// 故障注入：校验失败概率（0.0-1.0）
    pub fault_verify_error_rate: f64,
    /// 故障注入：额外延迟（毫秒）
    pub fault_delay_ms: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            auto_sync: true,
            sync_interval: 60,
            max_files_per_sync: 100,
            max_concurrency: 8,
            max_retries: 3,
            fail_queue_max: 1000,
            fail_task_ttl_secs: 24 * 3600,
            grpc_connect_timeout: 10,
            grpc_request_timeout: 30,
            fault_transfer_error_rate: 0.0,
            fault_verify_error_rate: 0.0,
            fault_delay_ms: 0,
        }
    }
}

/// 同步统计信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncStats {
    /// 总文件数
    pub total_files: usize,
    /// 已同步文件数
    pub synced_files: usize,
    /// 待同步文件数
    pub pending_files: usize,
    /// 最后同步时间
    pub last_sync_time: Option<NaiveDateTime>,
    /// 同步错误计数
    pub error_count: u32,
}

/// 失败补偿任务
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompTask {
    /// 任务 ID（scru128）
    id: String,
    /// 目标节点 ID
    target_node_id: String,
    /// 文件 ID
    file_id: String,
    /// 已尝试次数
    attempt: u32,
    /// 下次执行时间
    next_at: NaiveDateTime,
    /// 创建时间
    #[serde(default = "CompTask::default_created_at")]
    created_at: NaiveDateTime,
    /// 最近一次错误信息（可选，用于诊断）
    #[serde(default)]
    last_error: Option<String>,
}

/// 跨节点同步协调器
pub struct NodeSyncCoordinator {
    /// 配置
    config: Arc<RwLock<SyncConfig>>,
    /// 节点管理器
    node_manager: Arc<NodeManager>,
    /// 同步管理器
    sync_manager: Arc<SyncManager>,
    /// 存储管理器
    storage: Arc<crate::storage::StorageManager>,
    /// 同步统计
    stats: Arc<RwLock<SyncStats>>,
    /// 失败补偿队列
    fail_queue: Arc<RwLock<VecDeque<CompTask>>>,
    /// 失败补偿队列持久化路径
    fail_queue_path: std::path::PathBuf,
}

impl NodeSyncCoordinator {
    /// 更新运行时同步配置（热更新）
    pub async fn update_config(&self, new_cfg: SyncConfig) {
        let mut cfg = self.config.write().await;
        *cfg = new_cfg;
        info!("NodeSync 配置已更新");
    }
    fn prune_expired_and_trim(&self, q: &mut VecDeque<CompTask>, ttl_secs: i64, max_len: usize) {
        if ttl_secs > 0 {
            let now = Local::now().naive_local();
            q.retain(|t| (now - t.created_at).num_seconds() <= ttl_secs);
        }
        if max_len > 0 {
            while q.len() > max_len {
                q.pop_front();
            }
        }
    }
    pub fn new(
        config: SyncConfig,
        node_manager: Arc<NodeManager>,
        sync_manager: Arc<SyncManager>,
        storage: Arc<crate::storage::StorageManager>,
    ) -> Arc<Self> {
        // 确定补偿队列持久化路径：<root>/.sync/fail_queue.json
        let persist_dir = storage.root_dir().join(".sync");
        let persist_path = persist_dir.join("fail_queue.json");

        let this = Arc::new(Self {
            config: Arc::new(RwLock::new(config)),
            node_manager,
            sync_manager,
            storage,
            stats: Arc::new(RwLock::new(SyncStats::default())),
            fail_queue: Arc::new(RwLock::new(VecDeque::new())),
            fail_queue_path: persist_path,
        });

        // 尝试加载持久化队列
        let loader = this.clone();
        tokio::spawn(async move { loader.load_fail_queue().await });

        // 启动失败补偿后台任务
        let comp_clone = this.clone();
        tokio::spawn(async move { comp_clone.start_compensation_worker().await });

        // 订阅本地变更事件，触发快速 push
        let this_clone = this.clone();
        let mut rx = this_clone.sync_manager.subscribe();
        tokio::spawn(async move {
            while let Ok(file_id) = rx.recv().await {
                let nodes = this_clone.node_manager.list_online_nodes().await;
                if nodes.is_empty() {
                    debug!("快速同步跳过：无在线节点");
                    continue;
                }
                info!(
                    "快速同步触发: file_id={}, 在线节点数={}",
                    file_id,
                    nodes.len()
                );
                for n in nodes {
                    if let Err(e) = this_clone
                        .sync_to_node(&n.node_id, vec![file_id.clone()])
                        .await
                    {
                        warn!("快速同步失败: {} -> {}: {}", file_id, n.node_id, e);
                    }
                }
            }
        });

        this
    }

    /// 入队失败补偿任务
    async fn enqueue_compensation(
        &self,
        target_node_id: &str,
        file_id: &str,
        attempt: u32,
        last_error: Option<String>,
    ) {
        let next_secs = Self::backoff_secs(attempt);
        let when = Local::now().naive_local() + chrono::TimeDelta::seconds(next_secs as i64);
        let err_dbg = last_error.clone();
        let task = CompTask {
            id: scru128::new_string(),
            target_node_id: target_node_id.to_string(),
            file_id: file_id.to_string(),
            attempt,
            next_at: when,
            created_at: Local::now().naive_local(),
            last_error,
        };
        {
            let mut q = self.fail_queue.write().await;
            q.push_back(task);
            let cfg = self.config.read().await.clone();
            self.prune_expired_and_trim(&mut q, cfg.fail_task_ttl_secs as i64, cfg.fail_queue_max);
        }
        if let Err(e) = self.persist_fail_queue().await {
            warn!("补偿队列持久化失败: {}", e);
        }
        // 指标：更新队列长度
        let q_len = self.fail_queue.read().await.len() as i64;
        crate::metrics::set_sync_fail_queue_length(q_len);
        warn!(
            "补偿入队: file_id={}, node={}, attempt={}, next_in={}s, err={:?}",
            file_id, target_node_id, attempt, next_secs, err_dbg
        );
    }

    fn backoff_secs(attempt: u32) -> u64 {
        // 指数退避上限 60s，基础 2s，带抖动（0.8-1.2）
        let base = 2u64.saturating_mul(1u64 << attempt.min(5));
        let capped = base.min(60);
        let mut rng = rand::thread_rng();
        let jitter: f64 = rng.gen_range(0.8..=1.2);
        ((capped as f64 * jitter).round() as u64).max(1)
    }

    /// 后台失败补偿 worker
    async fn start_compensation_worker(self: Arc<Self>) {
        let mut tick = interval(Duration::from_secs(1));
        loop {
            tick.tick().await;

            let now = Local::now().naive_local();
            let maybe_task = {
                let mut q = self.fail_queue.write().await;
                if let Some(front) = q.front() {
                    if front.next_at <= now {
                        q.pop_front()
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            let Some(task) = maybe_task else { continue };

            // 执行单文件补偿同步
            match self
                .sync_to_node(&task.target_node_id, vec![task.file_id.clone()])
                .await
            {
                Ok(n) if n > 0 => {
                    info!(
                        "补偿成功: file_id={}, node={}, attempt={}",
                        task.file_id, task.target_node_id, task.attempt
                    );
                    if let Err(e) = self.persist_fail_queue().await {
                        warn!("补偿后持久化失败: {}", e);
                    }
                }
                Ok(_) => {
                    let next_attempt = task.attempt.saturating_add(1);
                    let max_retry = { self.config.read().await.max_retries };
                    if next_attempt <= (max_retry * 3).max(3) {
                        self.enqueue_compensation(
                            &task.target_node_id,
                            &task.file_id,
                            next_attempt,
                            Some("no_files_synced".to_string()),
                        )
                        .await;
                        crate::metrics::record_sync_retry("transfer");
                    } else {
                        error!(
                            "补偿放弃: file_id={}, node={}, final_attempt={}",
                            task.file_id, task.target_node_id, task.attempt
                        );
                        if let Err(e) = self.persist_fail_queue().await {
                            warn!("放弃后持久化失败: {}", e);
                        }
                    }
                }
                Err(e) => {
                    let next_attempt = task.attempt.saturating_add(1);
                    let max_retry = { self.config.read().await.max_retries };
                    if next_attempt <= (max_retry * 3).max(3) {
                        self.enqueue_compensation(
                            &task.target_node_id,
                            &task.file_id,
                            next_attempt,
                            Some(e.to_string()),
                        )
                        .await;
                        crate::metrics::record_sync_retry("transfer");
                    } else {
                        error!(
                            "补偿放弃: file_id={}, node={}, final_attempt={}",
                            task.file_id, task.target_node_id, task.attempt
                        );
                        if let Err(e) = self.persist_fail_queue().await {
                            warn!("放弃后持久化失败: {}", e);
                        }
                    }
                }
            }
        }
    }

    /// 将失败补偿队列持久化到磁盘
    async fn persist_fail_queue(&self) -> Result<()> {
        use tokio::fs;
        if let Some(parent) = self.fail_queue_path.parent() {
            let _ = fs::create_dir_all(parent).await;
        }
        let q = self.fail_queue.read().await;
        let data = serde_json::to_vec_pretty(&*q)
            .map_err(|e| NasError::Other(format!("序列化补偿队列失败: {}", e)))?;
        fs::write(&self.fail_queue_path, data)
            .await
            .map_err(|e| NasError::Other(format!("写入补偿队列失败: {}", e)))?;
        crate::metrics::set_sync_fail_queue_length(q.len() as i64);
        Ok(())
    }

    /// 启动时尝试加载失败补偿队列
    async fn load_fail_queue(&self) {
        use tokio::fs;
        match fs::read(&self.fail_queue_path).await {
            Ok(bytes) => match serde_json::from_slice::<VecDeque<CompTask>>(&bytes) {
                Ok(mut items) => {
                    let mut q = self.fail_queue.write().await;
                    while let Some(it) = items.pop_front() {
                        q.push_back(it);
                    }
                    let cfg = self.config.read().await.clone();
                    self.prune_expired_and_trim(
                        &mut q,
                        cfg.fail_task_ttl_secs as i64,
                        cfg.fail_queue_max,
                    );
                    crate::metrics::set_sync_fail_queue_length(q.len() as i64);
                    info!(
                        "已加载补偿队列: {} 项 -> {:?}",
                        q.len(),
                        self.fail_queue_path
                    );
                }
                Err(e) => warn!("补偿队列解析失败: {}", e),
            },
            Err(_) => {
                // 文件不存在不视为错误
            }
        }
    }

    /// 同步文件到指定节点
    pub async fn sync_to_node(&self, node_id: &str, file_ids: Vec<String>) -> Result<usize> {
        use crate::rpc::file_service::{
            FileMetadata as ProtoFileMetadata, FileSyncState as ProtoFileSyncState,
        };
        use crate::sync::node::client::{ClientConfig, NodeSyncClient};
        use tokio::fs;

        info!("开始同步 {} 个文件到节点: {}", file_ids.len(), node_id);

        // 获取目标节点信息
        let nodes = self.node_manager.nodes.read().await;
        let target_node = nodes
            .get(node_id)
            .ok_or_else(|| NasError::Other(format!("节点不存在: {}", node_id)))?;

        let node_address = target_node.address.clone();
        drop(nodes);

        // 创建 gRPC 客户端
        let cfg_now = self.config.read().await.clone();
        let client_cfg = ClientConfig {
            max_retries: cfg_now.max_retries,
            connect_timeout: cfg_now.grpc_connect_timeout,
            request_timeout: cfg_now.grpc_request_timeout,
            max_backoff_secs: 60,
            retry_budget_secs: 120,
            ..Default::default()
        };
        let client = NodeSyncClient::new(node_address.clone(), client_cfg);
        client.connect().await?;
        debug!(
            "gRPC 客户端已连接: {} -> {}",
            self.node_manager.config.node_id, node_address
        );

        use futures_util::stream::{FuturesUnordered, StreamExt};
        use tokio::sync::Semaphore;

        let mut synced = 0usize;
        let sem = Arc::new(Semaphore::new(cfg_now.max_concurrency.max(1)));
        let client = Arc::new(client);
        let mut futs = FuturesUnordered::new();

        for file_id in file_ids.iter().take(cfg_now.max_files_per_sync) {
            // 克隆必要的上下文
            let sem = sem.clone();
            let client = client.clone();
            let storage = self.storage.clone();
            let sync_manager = self.sync_manager.clone();
            let node_address = node_address.clone();
            let node_id = node_id.to_string();
            let cfg_now = cfg_now.clone();
            let file_id = file_id.clone();

            futs.push(tokio::spawn(async move {
                let _permit = sem.acquire_owned().await.unwrap();
                // 获取文件的同步状态
                if let Some(file_sync) = sync_manager.get_sync_state(&file_id).await {
                    // 先同步状态（VectorClock/LWW），以便对端处理冲突
                    let proto_meta = file_sync.metadata.value.clone().map(|m| ProtoFileMetadata {
                        id: m.id,
                        name: m.name,
                        path: m.path,
                        size: m.size,
                        hash: m.hash,
                        created_at: m.created_at.to_string(),
                        modified_at: m.modified_at.to_string(),
                    });

                    let vc_json = serde_json::to_string(&file_sync.vector_clock)
                        .unwrap_or_else(|_| "{}".to_string());

                    let state = ProtoFileSyncState {
                        file_id: file_id.clone(),
                        metadata: proto_meta,
                        deleted: file_sync.deleted.value.unwrap_or(false),
                        vector_clock: vc_json,
                        timestamp: chrono::Local::now().timestamp_millis(),
                    };

                    // 状态同步阶段埋点
                    let span_state = tracing::info_span!(
                        "state_sync",
                        file_id = %file_id,
                        target = %node_address
                    );
                    let _enter_s = span_state.enter();
                    // 忽略返回的冲突列表，由服务端记录日志与审计
                    let _ = client.sync_file_states(&node_id, vec![state]).await;

                    // 读取文件内容：优先按路径（WebDAV/S3场景），否则按ID
                    let content_res = if let Some(meta) = file_sync.metadata.value.as_ref() {
                        let full_path = storage.get_full_path(&meta.path);
                        debug!(
                            "读取文件内容(按路径): file_id={}, path={}, addr={}",
                            file_id, meta.path, node_address
                        );
                        fs::read(full_path)
                            .await
                            .map_err(|e| NasError::Other(e.to_string()))
                    } else {
                        debug!(
                            "读取文件内容(按ID): file_id={}, addr={}",
                            file_id, node_address
                        );
                        storage.read_file(&file_id).await.map_err(Into::into)
                    };

                    match content_res {
                        Ok(content) => {
                            let file_size = content.len();

                            // 统一采用流式传输，避免与服务端 TransferFile（拉取语义）不一致
                            const CHUNK_SIZE: usize = 1024 * 1024; // 1MB
                            let span = tracing::info_span!(
                                "sync_transfer",
                                file_id = %file_id,
                                target = %node_address
                            );
                            let _enter = span.enter();
                            // 故障注入：可选的延迟
                            if cfg_now.fault_delay_ms > 0 {
                                tokio::time::sleep(Duration::from_millis(cfg_now.fault_delay_ms))
                                    .await;
                            }
                            let t_transfer = std::time::Instant::now();
                            // 故障注入：按概率制造传输失败
                            let inject_transfer =
                                rand::random::<f64>() < cfg_now.fault_transfer_error_rate;
                            let transfer_result = if inject_transfer {
                                Err(NasError::Other("fault_injected_transfer".into()))
                            } else {
                                client
                                    .stream_file_content(&file_id, content, CHUNK_SIZE)
                                    .await
                                    .map(|_| true)
                            };

                            match transfer_result {
                                Ok(_) => {
                                    crate::metrics::record_sync_stage(
                                        "transfer",
                                        "success",
                                        t_transfer.elapsed().as_secs_f64(),
                                    );
                                    // 端到端一致性校验（SHA-256）
                                    let span_v = tracing::info_span!(
                                        "sync_verify",
                                        file_id = %file_id,
                                        target = %node_address
                                    );
                                    let _enter_v = span_v.enter();
                                    let t_verify = std::time::Instant::now();
                                    let mut verified = true;
                                    if let Some(meta) = file_sync.metadata.value.as_ref()
                                        && !meta.hash.is_empty()
                                    {
                                        match client.verify_remote_hash(&file_id, &meta.hash).await
                                        {
                                            Ok(ok) => {
                                                verified = ok;
                                                if !ok {
                                                    error!(
                                                        "端到端校验失败: {} -> {}，期望哈希不一致",
                                                        file_id, node_address
                                                    );
                                                    crate::metrics::record_sync_operation(
                                                        "full", "error", 0,
                                                    );
                                                    crate::metrics::record_sync_stage(
                                                        "verify",
                                                        "error",
                                                        t_verify.elapsed().as_secs_f64(),
                                                    );
                                                }
                                            }
                                            Err(e) => {
                                                verified = false;
                                                error!(
                                                    "端到端校验错误: {} -> {}, 错误: {}",
                                                    file_id, node_address, e
                                                );
                                                crate::metrics::record_sync_operation(
                                                    "full", "error", 0,
                                                );
                                                crate::metrics::record_sync_stage(
                                                    "verify",
                                                    "error",
                                                    t_verify.elapsed().as_secs_f64(),
                                                );
                                                crate::metrics::record_sync_retry("verify");
                                            }
                                        }
                                    }
                                    // 故障注入：按概率制造校验失败
                                    if verified
                                        && (rand::random::<f64>() < cfg_now.fault_verify_error_rate)
                                    {
                                        verified = false;
                                        warn!("故障注入：校验失败 file_id={}", file_id);
                                        crate::metrics::record_sync_retry("verify");
                                    }

                                    if verified {
                                        // 成功
                                        info!(
                                            "文件同步成功: {}, 大小: {} 字节 -> {}",
                                            file_id, file_size, node_address
                                        );
                                        crate::metrics::record_sync_operation(
                                            "full",
                                            "success",
                                            file_size as u64,
                                        );
                                        crate::metrics::record_sync_stage(
                                            "verify",
                                            "success",
                                            t_verify.elapsed().as_secs_f64(),
                                        );
                                        return Ok::<(String, bool, Option<String>), ()>((
                                            file_id, true, None,
                                        ));
                                    } else {
                                        // 校验不通过，入队补偿重试
                                        // 无法直接访问 self，这里仅上报重试，由调用方负责补偿入队
                                        crate::metrics::record_sync_retry("verify");
                                        // 维持原有短暂等待与重试退出逻辑，避免阻塞批量
                                        tokio::time::sleep(Duration::from_secs(2)).await;
                                    }
                                }
                                Err(e) => {
                                    error!(
                                        "文件同步失败: {} -> {}, 错误: {}",
                                        file_id, node_address, e
                                    );
                                    crate::metrics::record_sync_retry("transfer");

                                    // 等待后重试
                                    tokio::time::sleep(Duration::from_secs(2)).await;
                                    crate::metrics::record_sync_operation("full", "error", 0);
                                    crate::metrics::record_sync_stage(
                                        "transfer",
                                        "error",
                                        t_transfer.elapsed().as_secs_f64(),
                                    );
                                    return Ok::<(String, bool, Option<String>), ()>((
                                        file_id,
                                        false,
                                        Some(e.to_string()),
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            warn!("读取文件失败: {}, 错误: {}", file_id, e);
                            return Ok::<(String, bool, Option<String>), ()>((
                                file_id,
                                false,
                                Some(e.to_string()),
                            ));
                        }
                    }
                    // 失败或未校验通过（校验失败）
                    Ok::<(String, bool, Option<String>), ()>((
                        file_id,
                        false,
                        Some("verify_failed".into()),
                    ))
                } else {
                    Ok::<(String, bool, Option<String>), ()>((
                        file_id,
                        false,
                        Some("no_state".into()),
                    ))
                }
            }));
        }

        // 收集结果并处理补偿入队
        while let Some(res) = futs.next().await {
            if let Ok(Ok((_fid, true, _))) = res {
                synced += 1;
            } else if let Ok(Ok((fid, false, err))) = res {
                // 失败则入队补偿
                self.enqueue_compensation(node_id, &fid, 1, err).await;
            }
        }

        // 更新统计
        let mut stats = self.stats.write().await;
        stats.synced_files += synced;
        stats.last_sync_time = Some(Local::now().naive_local());

        // 断开连接
        client.disconnect().await;

        info!(
            "同步任务完成: 目标={}, 文件数={}, 成功数={}",
            node_address,
            file_ids.len().min(cfg_now.max_files_per_sync),
            synced
        );
        Ok(synced)
    }

    /// 从节点请求文件
    pub async fn request_files_from_node(
        &self,
        node_id: &str,
        file_ids: Vec<String>,
    ) -> Result<usize> {
        use crate::sync::node::client::{ClientConfig, NodeSyncClient};

        info!("从节点 {} 请求 {} 个文件", node_id, file_ids.len());

        // 获取目标节点信息
        let nodes = self.node_manager.nodes.read().await;
        let target_node = nodes
            .get(node_id)
            .ok_or_else(|| NasError::Other(format!("节点不存在: {}", node_id)))?;

        let node_address = target_node.address.clone();
        drop(nodes);

        // 创建 gRPC 客户端
        let cfg_now = self.config.read().await.clone();
        let client_cfg = ClientConfig {
            max_retries: cfg_now.max_retries,
            connect_timeout: cfg_now.grpc_connect_timeout,
            request_timeout: cfg_now.grpc_request_timeout,
            max_backoff_secs: 60,
            retry_budget_secs: 120,
            ..Default::default()
        };
        let client = NodeSyncClient::new(node_address.clone(), client_cfg);
        client.connect().await?;

        // 通过 gRPC 请求文件同步
        let synced_count = client.request_file_sync(node_id, file_ids).await?;

        // 断开连接
        client.disconnect().await;

        info!("成功从节点 {} 请求 {} 个文件", node_id, synced_count);

        Ok(synced_count as usize)
    }

    /// 启动自动同步任务
    pub async fn start_auto_sync(self: Arc<Self>) {
        if !self.config.read().await.auto_sync {
            return;
        }

        let mut interval = interval(Duration::from_secs(self.config.read().await.sync_interval));

        tokio::spawn(async move {
            loop {
                interval.tick().await;

                info!("开始自动同步...");

                // 获取所有在线节点
                let nodes = self.node_manager.list_online_nodes().await;
                let total_nodes = nodes.len();

                if nodes.is_empty() {
                    debug!("没有在线节点，跳过同步");
                    continue;
                }

                // 获取所有需要同步的文件
                let all_states = self.sync_manager.get_all_sync_states().await;
                let file_ids: Vec<String> = all_states
                    .iter()
                    .filter(|s| !s.is_deleted())
                    .map(|s| s.file_id.clone())
                    .collect();
                info!(
                    "自动同步准备: 在线节点={}, 待同步文件数={}",
                    total_nodes,
                    file_ids.len()
                );

                // 同步到每个节点
                for node in nodes {
                    if let Err(e) = self.sync_to_node(&node.node_id, file_ids.clone()).await {
                        error!("同步到节点 {} 失败: {}", node.node_id, e);

                        let mut stats = self.stats.write().await;
                        stats.error_count += 1;
                    }
                }

                info!("自动同步完成");
            }
        });
    }

    /// 获取同步统计
    pub async fn get_stats(&self) -> SyncStats {
        self.stats.read().await.clone()
    }
}

impl CompTask {
    fn default_created_at() -> NaiveDateTime {
        Local::now().naive_local()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_info_creation() {
        let node = NodeInfo::new(
            "node-1".to_string(),
            "127.0.0.1:9000".to_string(),
            "1.0.0".to_string(),
        );

        assert_eq!(node.node_id, "node-1");
        assert_eq!(node.address, "127.0.0.1:9000");
        assert_eq!(node.status, NodeStatus::Online);
    }

    #[test]
    fn test_node_heartbeat() {
        let mut node = NodeInfo::new(
            "node-1".to_string(),
            "127.0.0.1:9000".to_string(),
            "1.0.0".to_string(),
        );

        assert!(node.is_alive(30));
        node.update_heartbeat();
        assert_eq!(node.status, NodeStatus::Online);
    }

    #[test]
    fn test_node_discovery_config_default() {
        let config = NodeDiscoveryConfig::default();

        assert!(config.node_id.starts_with("node-"));
        assert_eq!(config.heartbeat_interval, 10);
        assert_eq!(config.node_timeout, 30);
    }

    #[test]
    fn test_sync_config_default() {
        let config = SyncConfig::default();

        assert!(config.auto_sync);
        assert_eq!(config.sync_interval, 60);
        assert_eq!(config.max_files_per_sync, 100);
    }

    #[test]
    fn test_sync_stats_default() {
        let stats = SyncStats::default();

        assert_eq!(stats.total_files, 0);
        assert_eq!(stats.synced_files, 0);
        assert_eq!(stats.pending_files, 0);
        assert!(stats.last_sync_time.is_none());
    }

    #[test]
    fn test_node_status_types() {
        let online = NodeStatus::Online;
        let offline = NodeStatus::Offline;
        let faulty = NodeStatus::Faulty;

        assert_eq!(online, NodeStatus::Online);
        assert_ne!(online, offline);
        assert_ne!(online, faulty);
    }

    #[test]
    fn test_node_info_clone() {
        let node = NodeInfo {
            node_id: "test-node".to_string(),
            address: "127.0.0.1:9000".to_string(),
            last_seen: Local::now().naive_local(),
            version: "1.0.0".to_string(),
            metadata: HashMap::new(),
            status: NodeStatus::Online,
        };

        let cloned = node.clone();
        assert_eq!(node.node_id, cloned.node_id);
        assert_eq!(node.address, cloned.address);
        assert_eq!(node.version, cloned.version);
        assert_eq!(node.status, cloned.status);
    }

    #[test]
    fn test_node_discovery_config_creation() {
        let config = NodeDiscoveryConfig {
            node_id: "test-node".to_string(),
            listen_addr: "0.0.0.0:9000".to_string(),
            seed_nodes: vec!["seed1:9000".to_string(), "seed2:9000".to_string()],
            heartbeat_interval: 30,
            node_timeout: 60,
        };

        assert_eq!(config.node_id, "test-node");
        assert_eq!(config.listen_addr, "0.0.0.0:9000");
        assert_eq!(config.seed_nodes.len(), 2);
        assert_eq!(config.heartbeat_interval, 30);
        assert_eq!(config.node_timeout, 60);
    }

    #[test]
    fn test_sync_config_custom() {
        let config = SyncConfig {
            auto_sync: false,
            sync_interval: 120,
            max_files_per_sync: 50,
            max_concurrency: 8,
            max_retries: 5,
            fail_queue_max: 1000,
            fail_task_ttl_secs: 24 * 3600,
            grpc_connect_timeout: 10,
            grpc_request_timeout: 30,
            fault_transfer_error_rate: 0.0,
            fault_verify_error_rate: 0.0,
            fault_delay_ms: 0,
        };

        assert!(!config.auto_sync);
        assert_eq!(config.sync_interval, 120);
        assert_eq!(config.max_files_per_sync, 50);
        assert_eq!(config.max_retries, 5);
    }

    #[test]
    fn test_sync_stats_creation() {
        let stats = SyncStats {
            total_files: 100,
            synced_files: 80,
            pending_files: 20,
            last_sync_time: Some(Local::now().naive_local()),
            error_count: 5,
        };

        assert_eq!(stats.total_files, 100);
        assert_eq!(stats.synced_files, 80);
        assert_eq!(stats.pending_files, 20);
        assert!(stats.last_sync_time.is_some());
        assert_eq!(stats.error_count, 5);
    }

    #[test]
    fn test_backoff_secs_bounds_and_cap() {
        // 所有值应在 [1, 72] 内（60*1.2 抖动上限），不保证严格单调
        for i in 0..10 {
            let v = NodeSyncCoordinator::backoff_secs(i);
            assert!((1..=72).contains(&v), "backoff {} out of range: {}", i, v);
        }
    }

    #[tokio::test]
    async fn test_enqueue_compensation() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(crate::storage::StorageManager::new(
            dir.path().to_path_buf(),
            4 * 1024 * 1024,
        ));
        storage.init().await.unwrap();
        let syncm = SyncManager::new("node-test".into(), storage.clone(), None);
        let nm = NodeManager::new(NodeDiscoveryConfig::default(), syncm.clone());
        let coord = NodeSyncCoordinator::new(SyncConfig::default(), nm, syncm, storage);
        coord
            .enqueue_compensation("node-x", "file-1", 0, Some("unit_test".into()))
            .await;
        let q = coord.fail_queue.read().await;
        assert_eq!(q.len(), 1);
        let t = q.front().unwrap();
        assert_eq!(t.target_node_id, "node-x");
        assert_eq!(t.file_id, "file-1");
        assert_eq!(t.last_error.as_deref(), Some("unit_test"));
    }
}
