// 跨节点文件同步模块
#![allow(dead_code)]

use crate::error::{NasError, Result};
use crate::sync::crdt::SyncManager;
use chrono::{Local, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    /// 同步重试次数
    pub max_retries: u32,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            auto_sync: true,
            sync_interval: 60,
            max_files_per_sync: 100,
            max_retries: 3,
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

/// 跨节点同步协调器
pub struct NodeSyncCoordinator {
    /// 配置
    config: SyncConfig,
    /// 节点管理器
    node_manager: Arc<NodeManager>,
    /// 同步管理器
    sync_manager: Arc<SyncManager>,
    /// 同步统计
    stats: Arc<RwLock<SyncStats>>,
}

impl NodeSyncCoordinator {
    pub fn new(
        config: SyncConfig,
        node_manager: Arc<NodeManager>,
        sync_manager: Arc<SyncManager>,
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            node_manager,
            sync_manager,
            stats: Arc::new(RwLock::new(SyncStats::default())),
        })
    }

    /// 同步文件到指定节点
    pub async fn sync_to_node(&self, node_id: &str, file_ids: Vec<String>) -> Result<usize> {
        info!("开始同步 {} 个文件到节点: {}", file_ids.len(), node_id);

        let mut synced = 0;

        for file_id in file_ids.iter().take(self.config.max_files_per_sync) {
            // 获取文件的同步状态
            if let Some(_file_sync) = self.sync_manager.get_sync_state(file_id).await {
                // TODO: 通过 gRPC 发送到目标节点
                // 调用 NodeSyncService::SyncFileState

                synced += 1;
                debug!("文件同步成功: {}", file_id);
            }
        }

        // 更新统计
        let mut stats = self.stats.write().await;
        stats.synced_files += synced;
        stats.last_sync_time = Some(Local::now().naive_local());

        Ok(synced)
    }

    /// 从节点请求文件
    pub async fn request_files_from_node(
        &self,
        node_id: &str,
        file_ids: Vec<String>,
    ) -> Result<usize> {
        info!("从节点 {} 请求 {} 个文件", node_id, file_ids.len());

        // TODO: 实现通过 gRPC 请求文件
        // 调用 NodeSyncService::RequestFileSync

        Ok(0)
    }

    /// 启动自动同步任务
    pub async fn start_auto_sync(self: Arc<Self>) {
        if !self.config.auto_sync {
            return;
        }

        let mut interval = interval(Duration::from_secs(self.config.sync_interval));

        tokio::spawn(async move {
            loop {
                interval.tick().await;

                info!("开始自动同步...");

                // 获取所有在线节点
                let nodes = self.node_manager.list_online_nodes().await;

                if nodes.is_empty() {
                    debug!("没有在线节点，跳过同步");
                    continue;
                }

                // 获取所有需要同步的文件
                let all_states = self.sync_manager.get_all_sync_states().await;
                let file_ids: Vec<String> = all_states.iter().map(|s| s.file_id.clone()).collect();

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
}
