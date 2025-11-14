// 增量同步HTTP API模块
// 提供增量同步相关的HTTP接口处理逻辑

use crate::error::Result;
use crate::sync::incremental::{self as incremental_sync, IncrementalSyncHandler};

/// 处理获取文件签名的请求
pub async fn handle_get_signature(
    handler: &IncrementalSyncHandler,
    file_id: &str,
) -> Result<incremental_sync::FileSignature> {
    handler.calculate_local_signature(file_id).await
}

/// 处理获取文件差异块的请求
pub async fn handle_get_delta(
    handler: &IncrementalSyncHandler,
    file_id: &str,
    target_signature: &incremental_sync::FileSignature,
) -> Result<Vec<incremental_sync::DeltaChunk>> {
    handler
        .generate_delta_chunks(file_id, target_signature)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use silent_nas_core::StorageManagerTrait;

    #[tokio::test]
    async fn test_handle_get_signature() {
        // 使用共享的测试存储
        let storage = crate::storage::init_test_storage_async().await;

        // 创建测试文件
        let file_id = "test_sig_api";
        let data = b"Test content for signature";
        storage.save_file(file_id, data).await.unwrap();

        let handler = IncrementalSyncHandler::new(64 * 1024);
        let signature = handle_get_signature(&handler, file_id).await.unwrap();

        assert_eq!(signature.file_id, file_id);
        assert_eq!(signature.file_size, data.len() as u64);
    }

    #[tokio::test]
    async fn test_handle_get_delta() {
        // 使用共享的测试存储
        let storage = crate::storage::init_test_storage_async().await;

        // 创建测试文件
        let file_id = "test_delta_api";
        let data = b"Modified content for delta test";
        storage.save_file(file_id, data).await.unwrap();

        let handler = IncrementalSyncHandler::new(64 * 1024);

        // 创建一个假的目标签名（空文件）
        let target_sig = incremental_sync::FileSignature {
            file_id: file_id.to_string(),
            file_size: 0,
            chunk_size: 64 * 1024,
            file_hash: "empty".to_string(),
            chunks: vec![],
        };

        let delta_chunks = handle_get_delta(&handler, file_id, &target_sig)
            .await
            .unwrap();

        // 应该返回差异块
        assert!(!delta_chunks.is_empty());
    }
}
