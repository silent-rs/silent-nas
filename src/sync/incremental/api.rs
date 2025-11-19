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

// 测试已移至 handler.rs 中，避免重复
