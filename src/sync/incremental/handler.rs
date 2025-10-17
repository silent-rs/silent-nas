// 增量同步处理模块
// 负责协调增量同步流程

use crate::error::{NasError, Result};
use crate::storage::StorageManager;
use crate::sync::incremental::{DeltaChunk, FileSignature, IncrementalSyncManager, SyncDelta};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// 增量同步请求
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncrementalSyncRequest {
    /// 文件ID
    pub file_id: String,
    /// 目标节点的文件签名（如果有）
    pub target_signature: Option<FileSignature>,
}

/// 增量同步响应
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncrementalSyncResponse {
    /// 文件ID
    pub file_id: String,
    /// 源文件签名
    pub source_signature: FileSignature,
    /// 同步差异（如果需要）
    pub delta: Option<SyncDelta>,
    /// 差异块数据
    pub delta_chunks: Vec<DeltaChunk>,
}

/// 增量同步处理器
pub struct IncrementalSyncHandler {
    /// 存储管理器
    storage: Arc<StorageManager>,
    /// 增量同步管理器
    sync_manager: Arc<IncrementalSyncManager>,
    /// HTTP客户端
    http_client: Client,
}

impl IncrementalSyncHandler {
    /// 创建新的处理器
    pub fn new(storage: Arc<StorageManager>, chunk_size: usize) -> Self {
        Self {
            storage,
            sync_manager: Arc::new(IncrementalSyncManager::new(chunk_size)),
            http_client: Client::new(),
        }
    }

    /// 从远程节点拉取文件（增量方式）
    pub async fn pull_incremental(&self, file_id: &str, source_http_addr: &str) -> Result<Vec<u8>> {
        info!(
            "开始增量拉取文件: file_id={}, source={}",
            file_id, source_http_addr
        );

        // 1. 尝试读取本地文件和计算签名
        let local_signature = match self.storage.read_file(file_id).await {
            Ok(local_data) => {
                debug!("本地文件存在，计算签名: file_id={}", file_id);
                Some(
                    self.sync_manager
                        .calculate_signature(file_id, &local_data)?,
                )
            }
            Err(_) => {
                debug!("本地文件不存在，将进行全量拉取: file_id={}", file_id);
                None
            }
        };

        // 2. 请求远程节点的文件签名
        let signature_url = format!(
            "{}/api/sync/signature/{}",
            source_http_addr.trim_end_matches('/'),
            file_id
        );

        let remote_signature: FileSignature =
            match self.http_client.get(&signature_url).send().await {
                Ok(resp) if resp.status().is_success() => resp
                    .json()
                    .await
                    .map_err(|e| NasError::Other(format!("解析远程签名失败: {}", e)))?,
                Ok(resp) => {
                    warn!("获取远程签名失败: HTTP {}", resp.status());
                    // Fallback到全量下载
                    return self.pull_full(file_id, source_http_addr).await;
                }
                Err(e) => {
                    warn!("请求远程签名失败: {}", e);
                    return self.pull_full(file_id, source_http_addr).await;
                }
            };

        // 3. 如果本地没有文件或者哈希完全不同，进行全量下载
        if local_signature.is_none() {
            info!("本地无文件，进行全量下载: file_id={}", file_id);
            return self.pull_full(file_id, source_http_addr).await;
        }

        let local_sig = local_signature.unwrap();

        // 4. 如果哈希相同，无需同步
        if local_sig.file_hash == remote_signature.file_hash {
            info!("文件哈希相同，无需同步: file_id={}", file_id);
            return self.storage.read_file(file_id).await;
        }

        // 5. 计算差异
        let delta = match self
            .sync_manager
            .calculate_delta(&remote_signature, &local_sig)?
        {
            Some(d) => d,
            None => {
                info!("无差异，无需同步: file_id={}", file_id);
                return self.storage.read_file(file_id).await;
            }
        };

        // 6. 计算并输出节省信息
        let (transferred, saved, percent) = self
            .sync_manager
            .calculate_savings(remote_signature.file_size, &delta);
        info!(
            "增量同步统计: file_id={}, 传输={} bytes, 节省={} bytes ({:.1}%)",
            file_id, transferred, saved, percent
        );

        // 7. 请求差异块
        let delta_url = format!(
            "{}/api/sync/delta/{}",
            source_http_addr.trim_end_matches('/'),
            file_id
        );

        let delta_request = serde_json::json!({
            "file_id": file_id,
            "target_signature": local_sig,
        });

        let delta_chunks: Vec<DeltaChunk> = match self
            .http_client
            .post(&delta_url)
            .json(&delta_request)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => resp
                .json()
                .await
                .map_err(|e| NasError::Other(format!("解析差异块失败: {}", e)))?,
            Ok(resp) => {
                warn!("获取差异块失败: HTTP {}, 回退到全量下载", resp.status());
                return self.pull_full(file_id, source_http_addr).await;
            }
            Err(e) => {
                warn!("请求差异块失败: {}, 回退到全量下载", e);
                return self.pull_full(file_id, source_http_addr).await;
            }
        };

        // 8. 应用差异块
        let local_data = self.storage.read_file(file_id).await?;
        let updated_data = self.sync_manager.apply_delta(&local_data, &delta_chunks)?;

        // 9. 验证哈希
        if !self
            .sync_manager
            .verify_hash(&updated_data, &remote_signature.file_hash)
        {
            error!(
                "增量同步后哈希验证失败，回退到全量下载: file_id={}",
                file_id
            );
            return self.pull_full(file_id, source_http_addr).await;
        }

        info!("✅ 增量同步完成: file_id={}", file_id);
        Ok(updated_data)
    }

    /// 全量拉取文件（回退方案）
    async fn pull_full(&self, file_id: &str, source_http_addr: &str) -> Result<Vec<u8>> {
        info!("开始全量拉取文件: file_id={}", file_id);

        let url = format!(
            "{}/api/files/{}",
            source_http_addr.trim_end_matches('/'),
            file_id
        );

        let resp = self
            .http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| NasError::Other(format!("请求文件失败: {}", e)))?;

        if !resp.status().is_success() {
            return Err(NasError::Other(format!(
                "下载文件失败: HTTP {}",
                resp.status()
            )));
        }

        let data = resp
            .bytes()
            .await
            .map_err(|e| NasError::Other(format!("读取响应体失败: {}", e)))?;

        info!(
            "✅ 全量拉取完成: file_id={}, size={} bytes",
            file_id,
            data.len()
        );
        Ok(data.to_vec())
    }

    /// 计算本地文件签名
    pub async fn calculate_local_signature(&self, file_id: &str) -> Result<FileSignature> {
        let data = self.storage.read_file(file_id).await?;
        self.sync_manager.calculate_signature(file_id, &data)
    }

    /// 生成差异块
    pub async fn generate_delta_chunks(
        &self,
        file_id: &str,
        target_signature: &FileSignature,
    ) -> Result<Vec<DeltaChunk>> {
        // 读取源文件
        let data = self.storage.read_file(file_id).await?;

        // 计算源签名
        let source_sig = self.sync_manager.calculate_signature(file_id, &data)?;

        // 计算差异
        let delta = match self
            .sync_manager
            .calculate_delta(&source_sig, target_signature)?
        {
            Some(d) => d,
            None => return Ok(Vec::new()),
        };

        // 提取差异块
        self.sync_manager
            .extract_delta_chunks(&data, &delta, &source_sig)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_handler_creation() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(StorageManager::new(
            PathBuf::from(temp_dir.path()),
            64 * 1024,
        ));
        storage.init().await.unwrap();

        let handler = IncrementalSyncHandler::new(storage, 64 * 1024);
        assert!(Arc::strong_count(&handler.sync_manager) >= 1);
    }

    #[tokio::test]
    async fn test_calculate_local_signature() {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(StorageManager::new(
            PathBuf::from(temp_dir.path()),
            64 * 1024,
        ));
        storage.init().await.unwrap();

        // 创建测试文件
        let file_id = "test_file";
        let data = b"Test content for signature calculation";
        storage.save_file(file_id, data).await.unwrap();

        let handler = IncrementalSyncHandler::new(storage, 64 * 1024);
        let signature = handler.calculate_local_signature(file_id).await.unwrap();

        assert_eq!(signature.file_id, file_id);
        assert_eq!(signature.file_size, data.len() as u64);
        assert!(!signature.chunks.is_empty());
    }
}
