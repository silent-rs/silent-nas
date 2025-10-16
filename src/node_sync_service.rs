// NodeSyncService gRPC 服务端实现
#![allow(dead_code)]

use crate::node_sync::{NodeManager, NodeSyncCoordinator};
use crate::sync::SyncManager;
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
}

impl NodeSyncServiceImpl {
    pub fn new(
        node_manager: Arc<NodeManager>,
        sync_coordinator: Arc<NodeSyncCoordinator>,
        sync_manager: Arc<SyncManager>,
    ) -> Self {
        Self {
            node_manager,
            sync_coordinator,
            sync_manager,
        }
    }

    pub fn into_server(self) -> NodeSyncServiceServer<Self> {
        NodeSyncServiceServer::new(self)
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

            // 获取本地文件状态
            if let Some(local_state) = self.sync_manager.get_sync_state(&file_id).await {
                // 比较向量时钟，检测冲突
                // TODO: 实现完整的冲突检测逻辑

                // 解析远程向量时钟
                if let Ok(remote_vc) = serde_json::from_str(&state.vector_clock) {
                    let local_vc = &local_state.vector_clock;

                    // 如果存在并发更新，标记为冲突
                    if !local_vc.happens_before(&remote_vc) && !remote_vc.happens_before(local_vc) {
                        conflicts.push(file_id.clone());
                        warn!("检测到文件冲突: {}", file_id);
                    }
                }
            }

            // TODO: 应用远程状态到本地
            // 这里需要实现完整的 CRDT merge 逻辑
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
}

// ========== 辅助函数 ==========

/// 将内部 NodeInfo 转换为 protobuf NodeInfo
fn convert_to_proto_node(node: &crate::node_sync::NodeInfo) -> crate::rpc::file_service::NodeInfo {
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
) -> Result<crate::node_sync::NodeInfo, String> {
    // 使用新的 DateTime API 进行转换
    let datetime = DateTime::<Utc>::from_timestamp_millis(proto.last_seen)
        .ok_or_else(|| "无效的时间戳".to_string())?;
    let last_seen = datetime.naive_utc();

    Ok(crate::node_sync::NodeInfo {
        node_id: proto.node_id.clone(),
        address: proto.address.clone(),
        last_seen,
        version: proto.version.clone(),
        metadata: proto.metadata.clone(),
        status: crate::node_sync::NodeStatus::Online,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_node_info() {
        let internal_node = crate::node_sync::NodeInfo::new(
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
}
