// NodeSyncService gRPC 客户端实现
#![allow(dead_code)]

use crate::error::{NasError, Result};
use crate::rpc::file_service::node_sync_service_client::NodeSyncServiceClient;
use crate::rpc::file_service::*;
use crate::sync::node::{NodeInfo, manager::NodeStatus};
use chrono::{DateTime, Utc};
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::sync::RwLock;
use tonic::transport::{Channel, Endpoint};
use tonic::{Code, Status};
use tracing::{debug, info};

/// gRPC 客户端连接配置
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// 连接超时时间（秒）
    pub connect_timeout: u64,
    /// 请求超时时间（秒）
    pub request_timeout: u64,
    /// 重试次数
    pub max_retries: u32,
    /// 重试间隔（秒）
    pub retry_interval: u64,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: 10,
            request_timeout: 30,
            max_retries: 3,
            retry_interval: 5,
        }
    }
}

/// NodeSync gRPC 客户端
pub struct NodeSyncClient {
    /// 目标节点地址
    address: String,
    /// gRPC 客户端
    client: Arc<RwLock<Option<NodeSyncServiceClient<Channel>>>>,
    /// 客户端配置
    config: ClientConfig,
}

impl NodeSyncClient {
    /// 创建新的客户端
    pub fn new(address: String, config: ClientConfig) -> Self {
        Self {
            address,
            client: Arc::new(RwLock::new(None)),
            config,
        }
    }

    fn should_retry(&self, status: &Status) -> bool {
        matches!(
            status.code(),
            Code::Unavailable
                | Code::DeadlineExceeded
                | Code::ResourceExhausted
                | Code::Aborted
                | Code::Unknown
                | Code::Internal
        )
    }

    /// 在目标节点校验文件哈希（端到端一致性）
    pub async fn verify_remote_hash(&self, file_id: &str, expected_hash: &str) -> Result<bool> {
        use crate::rpc::file_service::file_service_client::FileServiceClient;

        let endpoint = format!("http://{}", self.address);
        let ep = Endpoint::from_shared(endpoint)
            .map_err(|e| NasError::Other(format!("无效的地址: {}", e)))?
            .connect_timeout(StdDuration::from_secs(self.config.connect_timeout))
            .timeout(StdDuration::from_secs(self.config.request_timeout))
            .tcp_nodelay(true);

        let channel = ep
            .connect()
            .await
            .map_err(|e| NasError::Other(format!("连接失败: {}", e)))?;

        let mut client = FileServiceClient::new(channel);
        let req = tonic::Request::new(GetMetadataRequest {
            file_id: file_id.to_string(),
        });

        match client.get_metadata(req).await {
            Ok(resp) => {
                let meta = resp.into_inner().metadata;
                if let Some(m) = meta {
                    Ok(m.hash == expected_hash)
                } else {
                    Ok(false)
                }
            }
            Err(e) => Err(NasError::Other(format!(
                "获取目标元数据失败以校验哈希: {}",
                e
            ))),
        }
    }

    fn backoff_delay(&self, attempt: u32) -> tokio::time::Duration {
        let base = self.config.retry_interval;
        let factor = 1u64 << attempt.min(5); // 上限 2^5 = 32
        let secs = (base.saturating_mul(factor)).min(60);
        tokio::time::Duration::from_secs(secs)
    }

    /// 连接到远程节点
    pub async fn connect(&self) -> Result<()> {
        info!("连接到节点: {}", self.address);

        let endpoint = format!("http://{}", self.address);

        let ep = Endpoint::from_shared(endpoint)
            .map_err(|e| NasError::Other(format!("无效的地址: {}", e)))?
            .connect_timeout(StdDuration::from_secs(self.config.connect_timeout))
            .timeout(StdDuration::from_secs(self.config.request_timeout))
            .tcp_nodelay(true);

        let channel = ep
            .connect()
            .await
            .map_err(|e| NasError::Other(format!("连接失败: {}", e)))?;

        let client = NodeSyncServiceClient::new(channel);

        let mut client_lock = self.client.write().await;
        *client_lock = Some(client);

        info!("成功连接到节点: {}", self.address);
        Ok(())
    }

    /// 确保客户端已连接
    async fn ensure_connected(&self) -> Result<NodeSyncServiceClient<Channel>> {
        let client_lock = self.client.read().await;

        if let Some(client) = client_lock.as_ref() {
            Ok(client.clone())
        } else {
            drop(client_lock);
            self.connect().await?;

            let client_lock = self.client.read().await;
            client_lock
                .as_ref()
                .cloned()
                .ok_or_else(|| NasError::Other("连接失败".to_string()))
        }
    }

    /// 注册节点到远程节点
    pub async fn register_node(&self, node: &NodeInfo) -> Result<Vec<NodeInfo>> {
        debug!("向 {} 注册节点: {}", self.address, node.node_id);

        let mut client = self.ensure_connected().await?;

        let proto_node = crate::rpc::file_service::NodeInfo {
            node_id: node.node_id.clone(),
            address: node.address.clone(),
            last_seen: node.last_seen.and_utc().timestamp_millis(),
            version: node.version.clone(),
            metadata: node.metadata.clone(),
        };

        let payload = RegisterNodeRequest {
            node: Some(proto_node),
        };
        // 重试调用
        let mut last_err = None;
        for attempt in 0..=self.config.max_retries {
            let request = tonic::Request::new(payload.clone());
            match client.register_node(request).await {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    // 转换返回的节点列表
                    let nodes = resp
                        .known_nodes
                        .into_iter()
                        .filter_map(|proto_node| convert_from_proto_node(&proto_node).ok())
                        .collect();
                    return Ok(nodes);
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < self.config.max_retries {
                        if let Some(ref st) = last_err
                            && !self.should_retry(st)
                        {
                            break;
                        }
                        tokio::time::sleep(self.backoff_delay(attempt)).await;
                        continue;
                    }
                }
            }
        }
        Err(NasError::Other(format!(
            "注册节点失败: {}",
            last_err.unwrap()
        )))
    }

    /// 发送心跳
    pub async fn send_heartbeat(&self, node_id: &str) -> Result<i64> {
        debug!("向 {} 发送心跳", self.address);

        let mut client = self.ensure_connected().await?;

        let mut last_err = None;
        for attempt in 0..=self.config.max_retries {
            let request = tonic::Request::new(HeartbeatRequest {
                node_id: node_id.to_string(),
                timestamp: chrono::Local::now().timestamp_millis(),
            });
            match client.heartbeat(request).await {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    return Ok(resp.server_timestamp);
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < self.config.max_retries {
                        if let Some(ref st) = last_err
                            && !self.should_retry(st)
                        {
                            break;
                        }
                        tokio::time::sleep(self.backoff_delay(attempt)).await;
                        continue;
                    }
                }
            }
        }
        Err(NasError::Other(format!("心跳失败: {}", last_err.unwrap())))
    }

    /// 获取节点列表
    pub async fn list_nodes(&self) -> Result<Vec<NodeInfo>> {
        debug!("从 {} 获取节点列表", self.address);

        let mut client = self.ensure_connected().await?;

        let mut last_err = None;
        for attempt in 0..=self.config.max_retries {
            let request = tonic::Request::new(ListNodesRequest {});
            match client.list_nodes(request).await {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    let nodes = resp
                        .nodes
                        .into_iter()
                        .filter_map(|proto_node| convert_from_proto_node(&proto_node).ok())
                        .collect();
                    return Ok(nodes);
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < self.config.max_retries {
                        if let Some(ref st) = last_err
                            && !self.should_retry(st)
                        {
                            break;
                        }
                        tokio::time::sleep(self.backoff_delay(attempt)).await;
                        continue;
                    }
                }
            }
        }
        Err(NasError::Other(format!(
            "获取节点列表失败: {}",
            last_err.unwrap()
        )))
    }

    /// 同步文件状态到远程节点
    pub async fn sync_file_states(
        &self,
        source_node_id: &str,
        states: Vec<FileSyncState>,
    ) -> Result<Vec<String>> {
        info!("同步 {} 个文件状态到 {}", states.len(), self.address);

        let mut client = self.ensure_connected().await?;

        let payload = SyncFileStateRequest {
            source_node_id: source_node_id.to_string(),
            states,
        };
        let mut last_err = None;
        for attempt in 0..=self.config.max_retries {
            let request = tonic::Request::new(payload.clone());
            match client.sync_file_state(request).await {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    return Ok(resp.conflicts);
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < self.config.max_retries {
                        if let Some(ref st) = last_err
                            && !self.should_retry(st)
                        {
                            break;
                        }
                        tokio::time::sleep(self.backoff_delay(attempt)).await;
                        continue;
                    }
                }
            }
        }
        Err(NasError::Other(format!(
            "同步文件状态失败: {}",
            last_err.unwrap()
        )))
    }

    /// 请求从远程节点同步文件
    pub async fn request_file_sync(&self, node_id: &str, file_ids: Vec<String>) -> Result<i32> {
        info!("向 {} 请求同步 {} 个文件", self.address, file_ids.len());

        let mut client = self.ensure_connected().await?;

        let payload = RequestFileSyncRequest {
            node_id: node_id.to_string(),
            file_ids,
        };

        let mut last_err = None;
        for attempt in 0..=self.config.max_retries {
            let request = tonic::Request::new(payload.clone());
            match client.request_file_sync(request).await {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    return Ok(resp.synced_count);
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < self.config.max_retries {
                        if let Some(ref st) = last_err
                            && !self.should_retry(st)
                        {
                            break;
                        }
                        tokio::time::sleep(self.backoff_delay(attempt)).await;
                        continue;
                    }
                }
            }
        }
        Err(NasError::Other(format!(
            "请求文件同步失败: {}",
            last_err.unwrap()
        )))
    }

    /// 获取远程节点的同步状态
    pub async fn get_sync_status(&self, node_id: &str) -> Result<SyncStatusInfo> {
        debug!("获取节点 {} 的同步状态", self.address);

        let mut client = self.ensure_connected().await?;

        let request = tonic::Request::new(GetSyncStatusRequest {
            node_id: node_id.to_string(),
        });

        let response = client
            .get_sync_status(request)
            .await
            .map_err(|e| NasError::Other(format!("获取同步状态失败: {}", e)))?;

        let resp = response.into_inner();

        Ok(SyncStatusInfo {
            total_files: resp.total_files as usize,
            synced_files: resp.synced_files as usize,
            pending_files: resp.pending_files as usize,
            last_sync_time: if resp.last_sync_time > 0 {
                DateTime::<Utc>::from_timestamp_millis(resp.last_sync_time).map(|dt| dt.naive_utc())
            } else {
                None
            },
        })
    }

    /// 断开连接
    pub async fn disconnect(&self) {
        let mut client_lock = self.client.write().await;
        *client_lock = None;
        info!("断开与节点 {} 的连接", self.address);
    }

    /// 传输文件到远程节点
    pub async fn transfer_file(
        &self,
        file_id: &str,
        content: Vec<u8>,
        metadata: Option<crate::models::FileMetadata>,
    ) -> Result<bool> {
        info!(
            "传输文件 {} 到 {}, 大小: {} 字节",
            file_id,
            self.address,
            content.len()
        );

        let mut client = self.ensure_connected().await?;

        // 转换元数据
        let proto_metadata = metadata.map(|m| crate::rpc::file_service::FileMetadata {
            id: m.id,
            name: m.name,
            path: m.path,
            size: m.size,
            hash: m.hash,
            created_at: m.created_at.to_string(),
            modified_at: m.modified_at.to_string(),
        });

        let payload = crate::rpc::file_service::TransferFileRequest {
            file_id: file_id.to_string(),
            source_node_id: String::new(), // 将由服务端填充
            metadata: proto_metadata,
        };

        let mut last_err = None;
        for attempt in 0..=self.config.max_retries {
            let request = tonic::Request::new(payload.clone());
            match client.transfer_file(request).await {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    if !resp.success {
                        return Err(NasError::Other(format!(
                            "文件传输失败: {}",
                            resp.error_message
                        )));
                    }
                    return Ok(true);
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < self.config.max_retries {
                        if let Some(ref st) = last_err
                            && !self.should_retry(st)
                        {
                            break;
                        }
                        tokio::time::sleep(self.backoff_delay(attempt)).await;
                        continue;
                    }
                }
            }
        }
        Err(NasError::Other(format!(
            "文件传输失败: {}",
            last_err.unwrap()
        )))
    }

    /// 流式传输大文件到远程节点
    pub async fn stream_file_content(
        &self,
        file_id: &str,
        content: Vec<u8>,
        chunk_size: usize,
    ) -> Result<u64> {
        info!(
            "流式传输文件 {} 到 {}, 总大小: {} 字节, 块大小: {} 字节",
            file_id,
            self.address,
            content.len(),
            chunk_size
        );

        let mut client = self.ensure_connected().await?;

        // 创建块流
        let chunks: Vec<crate::rpc::file_service::FileChunk> = content
            .chunks(chunk_size)
            .enumerate()
            .map(|(i, chunk_data)| {
                let offset = (i * chunk_size) as u64;
                let is_last = (i + 1) * chunk_size >= content.len();

                crate::rpc::file_service::FileChunk {
                    file_id: file_id.to_string(),
                    offset,
                    data: chunk_data.to_vec(),
                    is_last,
                    checksum: format!("{:x}", md5::compute(chunk_data)),
                }
            })
            .collect();

        let mut last_err = None;
        for attempt in 0..=self.config.max_retries {
            // 转换为 Stream（每次重试都需重建流）
            let stream = tokio_stream::iter(chunks.clone());
            let request = tonic::Request::new(stream);
            match client.stream_file_content(request).await {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    if !resp.success {
                        return Err(NasError::Other(format!(
                            "流式传输失败: {}",
                            resp.error_message
                        )));
                    }
                    return Ok(resp.bytes_received);
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < self.config.max_retries {
                        if let Some(ref st) = last_err
                            && !self.should_retry(st)
                        {
                            break;
                        }
                        tokio::time::sleep(self.backoff_delay(attempt)).await;
                        continue;
                    }
                }
            }
        }
        Err(NasError::Other(format!(
            "流式传输失败: {}",
            last_err.unwrap()
        )))
    }
}

/// 同步状态信息
#[derive(Debug, Clone)]
pub struct SyncStatusInfo {
    pub total_files: usize,
    pub synced_files: usize,
    pub pending_files: usize,
    pub last_sync_time: Option<chrono::NaiveDateTime>,
}

// ========== 辅助函数 ==========

/// 将 protobuf NodeInfo 转换为内部 NodeInfo
fn convert_from_proto_node(proto: &crate::rpc::file_service::NodeInfo) -> Result<NodeInfo> {
    let datetime = DateTime::<Utc>::from_timestamp_millis(proto.last_seen)
        .ok_or_else(|| NasError::Other("无效的时间戳".to_string()))?;
    let last_seen = datetime.with_timezone(&chrono::Local).naive_local();

    Ok(NodeInfo {
        node_id: proto.node_id.clone(),
        address: proto.address.clone(),
        last_seen,
        version: proto.version.clone(),
        metadata: proto.metadata.clone(),
        status: NodeStatus::Online,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_config_default() {
        let config = ClientConfig::default();

        assert_eq!(config.connect_timeout, 10);
        assert_eq!(config.request_timeout, 30);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.retry_interval, 5);
    }

    #[test]
    fn test_client_creation() {
        let config = ClientConfig::default();
        let client = NodeSyncClient::new("127.0.0.1:9000".to_string(), config);

        assert_eq!(client.address, "127.0.0.1:9000");
    }

    #[test]
    fn test_should_retry_and_backoff() {
        let client = NodeSyncClient::new(
            "127.0.0.1:50051".into(),
            ClientConfig {
                connect_timeout: 1,
                request_timeout: 1,
                max_retries: 3,
                retry_interval: 5,
            },
        );
        // 可重试状态
        assert!(client.should_retry(&Status::unavailable("")));
        assert!(client.should_retry(&Status::deadline_exceeded("")));
        // 不可重试
        assert!(!client.should_retry(&Status::invalid_argument("")));

        // 退避：5, 10, 20, 40, 60(封顶)
        assert_eq!(client.backoff_delay(0).as_secs(), 5);
        assert_eq!(client.backoff_delay(1).as_secs(), 10);
        assert_eq!(client.backoff_delay(2).as_secs(), 20);
        assert_eq!(client.backoff_delay(3).as_secs(), 40);
        assert_eq!(client.backoff_delay(5).as_secs(), 60);
        assert_eq!(client.backoff_delay(6).as_secs(), 60);
    }

    #[test]
    fn test_sync_status_info() {
        let status = SyncStatusInfo {
            total_files: 100,
            synced_files: 80,
            pending_files: 20,
            last_sync_time: None,
        };

        assert_eq!(status.total_files, 100);
        assert_eq!(status.synced_files, 80);
        assert_eq!(status.pending_files, 20);
        assert!(status.last_sync_time.is_none());
    }

    #[test]
    fn test_convert_proto_node() {
        let proto_node = crate::rpc::file_service::NodeInfo {
            node_id: "test-node".to_string(),
            address: "192.168.1.10:9000".to_string(),
            last_seen: chrono::Utc::now().timestamp_millis(),
            version: "1.0.0".to_string(),
            metadata: std::collections::HashMap::new(),
        };

        let node = convert_from_proto_node(&proto_node).unwrap();

        assert_eq!(node.node_id, "test-node");
        assert_eq!(node.address, "192.168.1.10:9000");
        assert_eq!(node.version, "1.0.0");
        assert_eq!(node.status, NodeStatus::Online);
    }
}
