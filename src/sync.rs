// 允许未使用的代码警告 - 这些 API 将在后续阶段使用
#![allow(dead_code)]

use crate::error::Result;
use crate::models::{EventType, FileEvent, FileMetadata};
use crate::notify::EventNotifier;
use crate::storage::StorageManager;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use silent_crdt::crdt::{LWWRegister, VectorClock};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// 文件同步状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSync {
    /// 文件 ID
    pub file_id: String,
    /// 文件元数据（使用 LWW-Register 存储）
    pub metadata: LWWRegister<FileMetadata>,
    /// 文件是否被删除
    pub deleted: LWWRegister<bool>,
    /// 向量时钟（追踪因果关系）
    pub vector_clock: VectorClock,
}

impl FileSync {
    pub fn new(file_id: String, metadata: FileMetadata, node_id: &str) -> Self {
        let mut vc = VectorClock::new();
        vc.increment(node_id);

        let timestamp = metadata.modified_at.and_utc().timestamp_millis();

        let mut metadata_reg = LWWRegister::new();
        metadata_reg.set(metadata, timestamp, node_id);

        let mut deleted_reg = LWWRegister::new();
        deleted_reg.set(false, 0, node_id);

        Self {
            file_id,
            metadata: metadata_reg,
            deleted: deleted_reg,
            vector_clock: vc,
        }
    }

    /// 更新文件元数据
    pub fn update_metadata(&mut self, metadata: FileMetadata, node_id: &str) {
        let timestamp = metadata.modified_at.and_utc().timestamp_millis();
        self.metadata.set(metadata, timestamp, node_id);
        self.vector_clock.increment(node_id);
    }

    /// 标记文件为已删除
    pub fn mark_deleted(&mut self, timestamp: i64, node_id: &str) {
        self.deleted.set(true, timestamp, node_id);
        self.vector_clock.increment(node_id);
    }

    /// 合并另一个节点的状态
    pub fn merge(&mut self, other: &FileSync) {
        self.metadata.merge(&other.metadata);
        self.deleted.merge(&other.deleted);
        self.vector_clock.merge(&other.vector_clock);
    }

    /// 获取当前文件元数据
    pub fn get_metadata(&self) -> Option<&FileMetadata> {
        if self.deleted.value.unwrap_or(false) {
            None
        } else {
            self.metadata.value.as_ref()
        }
    }

    /// 文件是否被删除
    pub fn is_deleted(&self) -> bool {
        self.deleted.value.unwrap_or(false)
    }

    /// 检测是否有冲突
    pub fn has_conflict(&self, other: &FileSync) -> bool {
        // 如果两个状态的向量时钟并发，则存在冲突
        self.vector_clock.is_concurrent(&other.vector_clock)
    }
}

/// 文件同步管理器
pub struct SyncManager {
    /// 节点 ID
    node_id: String,
    /// 存储管理器
    storage: Arc<StorageManager>,
    /// 事件通知器
    notifier: Arc<EventNotifier>,
    /// 文件同步状态缓存
    sync_states: Arc<RwLock<HashMap<String, FileSync>>>,
}

impl SyncManager {
    pub fn new(
        node_id: String,
        storage: Arc<StorageManager>,
        notifier: Arc<EventNotifier>,
    ) -> Arc<Self> {
        Arc::new(Self {
            node_id,
            storage,
            notifier,
            sync_states: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// 获取节点 ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// 处理本地文件变更事件
    pub async fn handle_local_change(
        &self,
        event_type: EventType,
        file_id: String,
        metadata: Option<FileMetadata>,
    ) -> Result<()> {
        let mut states = self.sync_states.write().await;

        match event_type {
            EventType::Created | EventType::Modified => {
                if let Some(meta) = metadata {
                    if let Some(file_sync) = states.get_mut(&file_id) {
                        // 更新现有文件
                        file_sync.update_metadata(meta.clone(), &self.node_id);
                        info!("更新文件同步状态: {}", file_id);
                    } else {
                        // 创建新文件同步状态
                        let file_sync = FileSync::new(file_id.clone(), meta, &self.node_id);
                        states.insert(file_id.clone(), file_sync);
                        info!("创建文件同步状态: {}", file_id);
                    }
                }
            }
            EventType::Deleted => {
                if let Some(file_sync) = states.get_mut(&file_id) {
                    let timestamp = chrono::Utc::now().timestamp_millis();
                    file_sync.mark_deleted(timestamp, &self.node_id);
                    info!("标记文件已删除: {}", file_id);
                }
            }
        }

        Ok(())
    }

    /// 处理远程同步请求
    pub async fn handle_remote_sync(&self, remote_state: FileSync) -> Result<Option<FileSync>> {
        let mut states = self.sync_states.write().await;
        let file_id = remote_state.file_id.clone();

        match states.get_mut(&file_id) {
            Some(local_state) => {
                // 检测冲突
                if local_state.has_conflict(&remote_state) {
                    warn!("检测到文件冲突: {}, 使用 LWW 策略自动合并", file_id);
                    self.handle_conflict(local_state, &remote_state).await?;
                }

                // 合并状态
                local_state.merge(&remote_state);
                info!("合并远程文件状态: {}", file_id);

                // 应用合并后的状态到存储
                self.apply_merged_state(local_state).await?;

                Ok(Some(local_state.clone()))
            }
            None => {
                // 新文件，直接添加
                states.insert(file_id.clone(), remote_state.clone());
                info!("添加远程文件状态: {}", file_id);

                // 应用到存储
                self.apply_merged_state(&remote_state).await?;

                Ok(Some(remote_state))
            }
        }
    }

    /// 处理冲突
    async fn handle_conflict(&self, local_state: &FileSync, remote_state: &FileSync) -> Result<()> {
        debug!(
            "冲突详情 - 本地时间: {:?}, 远程时间: {:?}",
            local_state.metadata.timestamp, remote_state.metadata.timestamp
        );

        // LWW 策略会自动选择时间戳更大的版本
        // 这里可以记录冲突事件或创建冲突副本
        let conflict_info = ConflictInfo {
            file_id: local_state.file_id.clone(),
            local_timestamp: local_state.metadata.timestamp,
            remote_timestamp: remote_state.metadata.timestamp,
            resolved_by: "LWW".to_string(),
            timestamp: chrono::Utc::now().naive_utc(),
        };

        debug!("冲突已解决: {:?}", conflict_info);

        Ok(())
    }

    /// 应用合并后的状态到存储
    async fn apply_merged_state(&self, state: &FileSync) -> Result<()> {
        if state.is_deleted() {
            // 文件已被删除
            if self.storage.get_metadata(&state.file_id).await.is_ok() {
                self.storage.delete_file(&state.file_id).await?;
                info!("应用删除: {}", state.file_id);
            }
        } else if let Some(metadata) = state.get_metadata() {
            // 更新文件元数据
            // 注意：这里不更新文件内容，只更新元数据
            // 文件内容应该通过其他机制（如 QUIC 传输）同步
            debug!("应用元数据更新: {} -> {:?}", state.file_id, metadata.name);
        }

        Ok(())
    }

    /// 获取文件同步状态
    pub async fn get_sync_state(&self, file_id: &str) -> Option<FileSync> {
        let states = self.sync_states.read().await;
        states.get(file_id).cloned()
    }

    /// 获取所有同步状态
    pub async fn get_all_sync_states(&self) -> Vec<FileSync> {
        let states = self.sync_states.read().await;
        states.values().cloned().collect()
    }

    /// 检查文件是否有冲突
    pub async fn check_conflicts(&self) -> Vec<ConflictInfo> {
        // 这里可以实现冲突检测逻辑
        // 比如比较本地状态和远程状态
        vec![]
    }

    /// 广播文件变更到其他节点
    pub async fn broadcast_change(&self, file_sync: &FileSync) -> Result<()> {
        // 通过 NATS 发送同步事件
        let event = FileEvent::new(
            EventType::Modified,
            file_sync.file_id.clone(),
            file_sync.get_metadata().cloned(),
        );

        self.notifier.publish_event(&event).await?;
        debug!("广播文件变更: {}", file_sync.file_id);

        Ok(())
    }
}

/// 冲突信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictInfo {
    pub file_id: String,
    pub local_timestamp: i64,
    pub remote_timestamp: i64,
    pub resolved_by: String,
    pub timestamp: NaiveDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    #[test]
    fn test_file_sync_creation() {
        let metadata = FileMetadata {
            id: "test-file-1".to_string(),
            name: "test.txt".to_string(),
            path: "/test.txt".to_string(),
            size: 1024,
            hash: "abc123".to_string(),
            created_at: Local::now().naive_local(),
            modified_at: Local::now().naive_local(),
        };

        let file_sync = FileSync::new("test-file-1".to_string(), metadata.clone(), "node1");

        assert_eq!(file_sync.file_id, "test-file-1");
        assert!(!file_sync.is_deleted());
        assert!(file_sync.get_metadata().is_some());
    }

    #[test]
    fn test_file_sync_merge() {
        let metadata1 = FileMetadata {
            id: "test-file-1".to_string(),
            name: "test.txt".to_string(),
            path: "/test.txt".to_string(),
            size: 1024,
            hash: "abc123".to_string(),
            created_at: Local::now().naive_local(),
            modified_at: Local::now().naive_local(),
        };

        let metadata2 = FileMetadata {
            id: "test-file-1".to_string(),
            name: "test_updated.txt".to_string(),
            path: "/test_updated.txt".to_string(),
            size: 2048,
            hash: "def456".to_string(),
            created_at: Local::now().naive_local(),
            modified_at: Local::now().naive_local() + chrono::Duration::seconds(10),
        };

        let mut sync1 = FileSync::new("test-file-1".to_string(), metadata1, "node1");
        let sync2 = FileSync::new("test-file-1".to_string(), metadata2.clone(), "node2");

        // 合并
        sync1.merge(&sync2);

        // LWW 应该选择时间戳更大的版本（metadata2）
        assert_eq!(sync1.get_metadata().unwrap().name, metadata2.name);
        assert_eq!(sync1.get_metadata().unwrap().size, 2048);
    }

    #[test]
    fn test_conflict_detection() {
        let metadata1 = FileMetadata {
            id: "test-file-1".to_string(),
            name: "test.txt".to_string(),
            path: "/test.txt".to_string(),
            size: 1024,
            hash: "abc123".to_string(),
            created_at: Local::now().naive_local(),
            modified_at: Local::now().naive_local(),
        };

        let metadata2 = FileMetadata {
            id: "test-file-1".to_string(),
            name: "test_v2.txt".to_string(),
            path: "/test_v2.txt".to_string(),
            size: 2048,
            hash: "def456".to_string(),
            created_at: Local::now().naive_local(),
            modified_at: Local::now().naive_local(),
        };

        let sync1 = FileSync::new("test-file-1".to_string(), metadata1, "node1");
        let sync2 = FileSync::new("test-file-1".to_string(), metadata2, "node2");

        // 两个独立的节点修改同一个文件应该检测到冲突
        assert!(sync1.has_conflict(&sync2));
    }
}
