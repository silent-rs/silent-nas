// NodeSyncService gRPC 服务端实现
#![allow(dead_code)]

use crate::storage::{StorageManager, StorageManagerTrait};
use crate::sync::crdt::SyncManager;
use crate::sync::node::{NodeManager, NodeSyncCoordinator};
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::{debug, info, warn};

// 引入生成的 protobuf 代码
use crate::rpc::file_service::node_sync_service_server::{NodeSyncService, NodeSyncServiceServer};
use crate::rpc::file_service::*;

/// NodeSyncService 实现
pub struct NodeSyncServiceImpl {
    node_manager: Arc<NodeManager>,
    sync_coordinator: Arc<NodeSyncCoordinator>,
    sync_manager: Arc<SyncManager>,
    storage: Arc<StorageManager>,
}

impl NodeSyncServiceImpl {
    pub fn new(
        node_manager: Arc<NodeManager>,
        sync_coordinator: Arc<NodeSyncCoordinator>,
        sync_manager: Arc<SyncManager>,
        storage: Arc<StorageManager>,
    ) -> Self {
        Self {
            node_manager,
            sync_coordinator,
            sync_manager,
            storage,
        }
    }

    pub fn into_server(self) -> NodeSyncServiceServer<Self> {
        NodeSyncServiceServer::new(self)
    }

    /// 应用远程状态到本地（辅助方法）
    async fn apply_remote_state(
        &self,
        file_id: &str,
        state: &FileSyncState,
        vector_clock: &silent_crdt::crdt::VectorClock,
    ) -> Result<(), Status> {
        use chrono::NaiveDateTime;
        use silent_crdt::crdt::LWWRegister;

        // 解析远程元数据
        let metadata = state
            .metadata
            .as_ref()
            .map(|m| crate::models::FileMetadata {
                id: m.id.clone(),
                name: m.name.clone(),
                path: m.path.clone(),
                size: m.size,
                hash: m.hash.clone(),
                created_at: NaiveDateTime::parse_from_str(&m.created_at, "%Y-%m-%d %H:%M:%S%.f")
                    .unwrap_or_else(|_| chrono::Local::now().naive_local()),
                modified_at: NaiveDateTime::parse_from_str(&m.modified_at, "%Y-%m-%d %H:%M:%S%.f")
                    .unwrap_or_else(|_| chrono::Local::now().naive_local()),
            });

        // 构造远程 FileSync 对象
        let mut deleted_reg = LWWRegister::new();
        deleted_reg.set(state.deleted, state.timestamp, "remote");

        let remote_sync = crate::sync::crdt::FileSync {
            file_id: file_id.to_string(),
            metadata: LWWRegister {
                value: metadata.clone(),
                timestamp: state.timestamp,
                node_id: "remote".to_string(),
            },
            deleted: deleted_reg,
            vector_clock: vector_clock.clone(),
        };

        // 使用 handle_remote_sync 处理远程状态
        if let Err(e) = self.sync_manager.handle_remote_sync(remote_sync).await {
            warn!("更新文件状态失败: {}, 错误: {}", file_id, e);
            return Err(Status::internal(format!("更新状态失败: {}", e)));
        }

        debug!("成功应用远程状态: {}", file_id);
        Ok(())
    }
}

#[tonic::async_trait]
impl NodeSyncService for NodeSyncServiceImpl {
    /// 注册节点
    async fn register_node(
        &self,
        request: Request<RegisterNodeRequest>,
    ) -> Result<Response<RegisterNodeResponse>, Status> {
        let req = request.into_inner();

        let node_info = req
            .node
            .ok_or_else(|| Status::invalid_argument("节点信息不能为空"))?;

        info!("收到节点注册请求: {}", node_info.node_id);

        // 转换 protobuf NodeInfo 到内部 NodeInfo
        let node = convert_from_proto_node(&node_info)
            .map_err(|e| Status::internal(format!("转换节点信息失败: {}", e)))?;

        // 注册节点
        self.node_manager
            .register_node(node)
            .await
            .map_err(|e| Status::internal(format!("注册节点失败: {}", e)))?;

        // 获取所有已知节点
        let known_nodes = self.node_manager.list_nodes().await;
        let proto_nodes: Vec<crate::rpc::file_service::NodeInfo> =
            known_nodes.iter().map(convert_to_proto_node).collect();

        Ok(Response::new(RegisterNodeResponse {
            success: true,
            known_nodes: proto_nodes,
        }))
    }

    /// 心跳检测
    async fn heartbeat(
        &self,
        request: Request<HeartbeatRequest>,
    ) -> Result<Response<HeartbeatResponse>, Status> {
        let req = request.into_inner();

        debug!("收到心跳: 节点 {}", req.node_id);

        // 更新节点心跳
        self.node_manager
            .update_heartbeat(&req.node_id)
            .await
            .map_err(|e| Status::not_found(format!("节点不存在: {}", e)))?;

        Ok(Response::new(HeartbeatResponse {
            alive: true,
            server_timestamp: chrono::Local::now().timestamp_millis(),
        }))
    }

    /// 列出所有节点
    async fn list_nodes(
        &self,
        _request: Request<ListNodesRequest>,
    ) -> Result<Response<ListNodesResponse>, Status> {
        let nodes = self.node_manager.list_online_nodes().await;

        let proto_nodes: Vec<crate::rpc::file_service::NodeInfo> =
            nodes.iter().map(convert_to_proto_node).collect();

        Ok(Response::new(ListNodesResponse { nodes: proto_nodes }))
    }

    /// 同步文件状态
    async fn sync_file_state(
        &self,
        request: Request<SyncFileStateRequest>,
    ) -> Result<Response<SyncFileStateResponse>, Status> {
        let req = request.into_inner();

        info!(
            "收到文件状态同步请求: 来自节点 {}, {} 个文件",
            req.source_node_id,
            req.states.len()
        );

        let mut conflicts = Vec::new();

        // 处理每个文件状态
        for state in req.states {
            let file_id = state.file_id.clone();

            // 解析远程向量时钟
            let remote_vc: silent_crdt::crdt::VectorClock =
                serde_json::from_str(&state.vector_clock)
                    .map_err(|e| Status::invalid_argument(format!("解析向量时钟失败: {}", e)))?;

            // 获取本地文件状态
            match self.sync_manager.get_sync_state(&file_id).await {
                Some(local_state) => {
                    let local_vc = &local_state.vector_clock;

                    // 完整的冲突检测逻辑
                    let is_concurrent = remote_vc.is_concurrent(local_vc);

                    if is_concurrent {
                        // 检测到并发更新，标记为冲突
                        conflicts.push(file_id.clone());
                        warn!(
                            "检测到文件冲突: {}, 本地向量: {:?}, 远程向量: {:?}",
                            file_id, local_vc, remote_vc
                        );

                        // 使用 LWW 策略自动解决冲突
                        if let Some(ref remote_metadata) = state.metadata {
                            // 比较时间戳，选择较新的版本
                            let remote_timestamp = chrono::NaiveDateTime::parse_from_str(
                                &remote_metadata.modified_at,
                                "%Y-%m-%d %H:%M:%S%.f",
                            )
                            .unwrap_or_else(|_| chrono::Local::now().naive_local());

                            let local_timestamp = local_state
                                .metadata
                                .value
                                .as_ref()
                                .map(|m| m.modified_at)
                                .unwrap_or_else(|| chrono::Local::now().naive_local());

                            if remote_timestamp > local_timestamp {
                                // 远程更新，应用远程状态
                                info!("应用远程状态 (较新): {}", file_id);
                                self.apply_remote_state(&file_id, &state, &remote_vc)
                                    .await?;
                            } else {
                                info!("保留本地状态 (较新): {}", file_id);
                            }
                        }
                    } else if local_vc.happens_before(&remote_vc) {
                        // 本地状态在远程之前，远程状态更新，直接应用
                        info!("应用远程状态 (happens-before): {}", file_id);
                        self.apply_remote_state(&file_id, &state, &remote_vc)
                            .await?;
                    } else {
                        // 本地状态已是最新或在远程之后，无需操作
                        debug!("本地状态已是最新: {}", file_id);
                    }
                }
                None => {
                    // 本地没有该文件，直接应用远程状态
                    info!("创建新文件状态: {}", file_id);
                    self.apply_remote_state(&file_id, &state, &remote_vc)
                        .await?;
                }
            }
        }

        Ok(Response::new(SyncFileStateResponse {
            success: true,
            conflicts,
        }))
    }

    /// 请求文件同步
    async fn request_file_sync(
        &self,
        request: Request<RequestFileSyncRequest>,
    ) -> Result<Response<RequestFileSyncResponse>, Status> {
        let req = request.into_inner();

        info!(
            "收到文件同步请求: 节点 {}, {} 个文件",
            req.node_id,
            req.file_ids.len()
        );

        // 同步文件到请求的节点
        let synced = self
            .sync_coordinator
            .sync_to_node(&req.node_id, req.file_ids)
            .await
            .map_err(|e| Status::internal(format!("同步失败: {}", e)))?;

        Ok(Response::new(RequestFileSyncResponse {
            success: true,
            synced_count: synced as i32,
        }))
    }

    /// 获取同步状态
    async fn get_sync_status(
        &self,
        _request: Request<GetSyncStatusRequest>,
    ) -> Result<Response<GetSyncStatusResponse>, Status> {
        let stats = self.sync_coordinator.get_stats().await;

        Ok(Response::new(GetSyncStatusResponse {
            total_files: stats.total_files as i32,
            synced_files: stats.synced_files as i32,
            pending_files: stats.pending_files as i32,
            last_sync_time: stats
                .last_sync_time
                .map(|t| t.and_utc().timestamp_millis())
                .unwrap_or(0),
        }))
    }

    /// 传输文件（用于小文件）
    async fn transfer_file(
        &self,
        request: Request<TransferFileRequest>,
    ) -> Result<Response<TransferFileResponse>, Status> {
        let req = request.into_inner();

        info!(
            "收到文件传输请求: 文件 {}, 来自节点 {}",
            req.file_id, req.source_node_id
        );

        // 读取文件内容
        match self.storage.read_file(&req.file_id).await {
            Ok(content) => {
                info!(
                    "文件传输成功: {}, 大小: {} 字节",
                    req.file_id,
                    content.len()
                );

                Ok(Response::new(TransferFileResponse {
                    success: true,
                    content,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                warn!("文件传输失败: {}, 错误: {}", req.file_id, e);

                Ok(Response::new(TransferFileResponse {
                    success: false,
                    content: Vec::new(),
                    error_message: format!("读取文件失败: {}", e),
                }))
            }
        }
    }

    /// 流式传输文件内容（用于大文件）
    async fn stream_file_content(
        &self,
        request: Request<tonic::Streaming<FileChunk>>,
    ) -> Result<Response<StreamFileResponse>, Status> {
        let mut stream = request.into_inner();
        let mut file_id = String::new();
        let mut total_bytes = 0u64;
        let mut temp_data = Vec::new();
        let mut chunk_index: u64 = 0;

        info!("开始接收流式文件传输");

        // 接收所有块
        while let Some(chunk) = stream
            .message()
            .await
            .map_err(|e| Status::internal(format!("接收块失败: {}", e)))?
        {
            if file_id.is_empty() {
                file_id = chunk.file_id.clone();
                info!("接收文件: {}", file_id);
            }

            // 校验分块校验和（MD5），提升端到端一致性
            let calc = format!("{:x}", md5::compute(&chunk.data));
            if !chunk.checksum.is_empty() && calc != chunk.checksum {
                let msg = format!(
                    "分块校验失败: file_id={}, index={}, offset={}, expect={}, got={}",
                    file_id, chunk_index, chunk.offset, chunk.checksum, calc
                );
                tracing::warn!("{}", msg);
                return Ok(Response::new(StreamFileResponse {
                    success: false,
                    bytes_received: total_bytes,
                    error_message: msg,
                }));
            }

            total_bytes += chunk.data.len() as u64;
            temp_data.extend_from_slice(&chunk.data);
            chunk_index += 1;

            debug!(
                "接收块: 偏移 {}, 大小 {} 字节, 是否最后: {}",
                chunk.offset,
                chunk.data.len(),
                chunk.is_last
            );

            if chunk.is_last {
                break;
            }
        }

        if file_id.is_empty() {
            return Err(Status::invalid_argument("未接收到有效的文件块"));
        }

        // 使用 save_file 保存文件内容
        match self.storage.save_file(&file_id, &temp_data).await {
            Ok(_metadata) => {
                info!(
                    "流式文件传输完成: {}, 总大小: {} 字节",
                    file_id, total_bytes
                );

                Ok(Response::new(StreamFileResponse {
                    success: true,
                    bytes_received: total_bytes,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                warn!("保存文件失败: {}, 错误: {}", file_id, e);

                Err(Status::internal(format!("保存文件失败: {}", e)))
            }
        }
    }
}

// ========== 辅助函数 ==========

/// 将内部 NodeInfo 转换为 protobuf NodeInfo
fn convert_to_proto_node(node: &crate::sync::node::NodeInfo) -> crate::rpc::file_service::NodeInfo {
    crate::rpc::file_service::NodeInfo {
        node_id: node.node_id.clone(),
        address: node.address.clone(),
        last_seen: node.last_seen.and_utc().timestamp_millis(),
        version: node.version.clone(),
        metadata: node.metadata.clone(),
    }
}

/// 将 protobuf NodeInfo 转换为内部 NodeInfo
fn convert_from_proto_node(
    proto: &crate::rpc::file_service::NodeInfo,
) -> Result<crate::sync::node::NodeInfo, String> {
    use crate::sync::node::manager::NodeStatus;

    // 使用新的 DateTime API 进行转换
    let datetime = DateTime::<Utc>::from_timestamp_millis(proto.last_seen)
        .ok_or_else(|| "无效的时间戳".to_string())?;
    let last_seen = datetime.with_timezone(&chrono::Local).naive_local();

    Ok(crate::sync::node::NodeInfo {
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
    use std::sync::Arc;

    #[test]
    fn test_convert_node_info() {
        let internal_node = crate::sync::node::NodeInfo::new(
            "node-1".to_string(),
            "127.0.0.1:9000".to_string(),
            "1.0.0".to_string(),
        );

        let proto_node = convert_to_proto_node(&internal_node);
        assert_eq!(proto_node.node_id, "node-1");
        assert_eq!(proto_node.address, "127.0.0.1:9000");
        assert_eq!(proto_node.version, "1.0.0");

        let converted_back = convert_from_proto_node(&proto_node).unwrap();
        assert_eq!(converted_back.node_id, internal_node.node_id);
        assert_eq!(converted_back.address, internal_node.address);
    }

    async fn build_service() -> NodeSyncServiceImpl {
        // 构建最小依赖：Storage、SyncManager、NodeManager、Coordinator
        let dir = tempfile::tempdir().unwrap();
        let storage = crate::storage::StorageManager::new(
            dir.path().to_path_buf(),
            4 * 1024 * 1024,
            crate::storage::IncrementalConfig::default(),
        );
        storage.init().await.unwrap();

        // 初始化全局存储
        let _ = crate::storage::init_global_storage(storage.clone());

        let storage = Arc::new(storage);

        let sync_manager = crate::sync::crdt::SyncManager::new("node-local".to_string(), None);

        let node_manager = crate::sync::node::manager::NodeManager::new(
            crate::sync::node::manager::NodeDiscoveryConfig::default(),
            sync_manager.clone(),
        );

        let coordinator = crate::sync::node::manager::NodeSyncCoordinator::new(
            crate::sync::node::manager::SyncConfig::default(),
            node_manager.clone(),
            sync_manager.clone(),
            storage.clone(),
        );

        NodeSyncServiceImpl::new(node_manager, coordinator, sync_manager, storage)
    }

    #[tokio::test]
    async fn test_sync_file_state_apply_new() {
        let service = build_service().await;

        // 构造一个远程状态（本地不存在该文件）
        let file_id = "file-new".to_string();
        let now = chrono::Local::now().naive_local();
        let state = FileSyncState {
            file_id: file_id.clone(),
            metadata: Some(FileMetadata {
                id: file_id.clone(),
                name: "a.txt".into(),
                path: "/dir/a.txt".into(),
                size: 5,
                hash: "hash".into(),
                created_at: now.to_string(),
                modified_at: now.to_string(),
            }),
            deleted: false,
            // 空向量时钟（结构需包含 clocks 字段）
            vector_clock: serde_json::json!({"clocks":{}}).to_string(),
            timestamp: chrono::Local::now().timestamp_millis(),
        };

        let req = SyncFileStateRequest {
            source_node_id: "remote-node".into(),
            states: vec![state],
        };
        let resp = service
            .sync_file_state(tonic::Request::new(req))
            .await
            .unwrap()
            .into_inner();
        assert!(resp.success);
        assert!(resp.conflicts.is_empty());
    }

    #[tokio::test]
    async fn test_sync_file_state_conflict_detected() {
        let service = build_service().await;
        let file_id = "file-conflict".to_string();

        // 先写入本地状态，产生本地向量时钟（local 节点）
        let meta_local = crate::models::FileMetadata {
            id: file_id.clone(),
            name: "b.txt".into(),
            path: "/b.txt".into(),
            size: 1,
            hash: "h1".into(),
            created_at: chrono::Local::now().naive_local(),
            modified_at: chrono::Local::now().naive_local(),
        };
        service
            .sync_manager
            .handle_local_change(
                crate::models::EventType::Created,
                file_id.clone(),
                Some(meta_local),
            )
            .await
            .unwrap();

        // 构造一个仅包含不同节点键的向量时钟，形成并发（concurrent）
        let remote_vc = serde_json::json!({ "clocks": { "remote": 1 } }).to_string();
        let newer =
            (chrono::Local::now().naive_local() + chrono::TimeDelta::seconds(1)).to_string();
        let state = FileSyncState {
            file_id: file_id.clone(),
            metadata: Some(FileMetadata {
                id: file_id.clone(),
                name: "b.txt".into(),
                path: "/b.txt".into(),
                size: 2,
                hash: "h2".into(),
                created_at: newer.clone(),
                modified_at: newer,
            }),
            deleted: false,
            vector_clock: remote_vc,
            timestamp: chrono::Local::now().timestamp_millis(),
        };

        let req = SyncFileStateRequest {
            source_node_id: "remote-node".into(),
            states: vec![state],
        };
        let resp = service
            .sync_file_state(tonic::Request::new(req))
            .await
            .unwrap()
            .into_inner();
        assert!(resp.success);
        assert_eq!(resp.conflicts.len(), 1);
        assert_eq!(resp.conflicts[0], file_id);
    }

    #[tokio::test]
    async fn test_request_file_sync_node_not_found() {
        let service = build_service().await;
        let req = RequestFileSyncRequest {
            node_id: "non-exist".into(),
            file_ids: vec!["x".into()],
        };
        let err = service
            .request_file_sync(tonic::Request::new(req))
            .await
            .err()
            .unwrap();
        assert_eq!(err.code(), tonic::Code::Internal);
    }
}
