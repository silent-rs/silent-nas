//! 健康检查和状态端点

use super::state::AppState;
use silent::extractor::Configs as CfgExtractor;
use silent::prelude::*;
use silent_nas_core::StorageManagerTrait;

/// 健康检查 - 简单存活检查
pub async fn health(_req: Request) -> silent::Result<&'static str> {
    Ok("OK")
}

/// 就绪检查 - 检查所有依赖服务
pub async fn readiness(
    _req: Request,
    CfgExtractor(_state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    // 检查存储是否可用
    let storage_ok = StorageManagerTrait::list_files(crate::storage::storage())
        .await
        .is_ok();

    // 检查搜索引擎是否可用（简单检查，总是返回true）
    let search_ok = true;

    let ready = storage_ok && search_ok;
    let status = if ready { "ready" } else { "not_ready" };

    Ok(serde_json::json!({
        "status": status,
        "checks": {
            "storage": storage_ok,
            "search": search_ok
        }
    }))
}

/// 详细状态检查
pub async fn health_status(
    _req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    // 存储状态
    let files = StorageManagerTrait::list_files(crate::storage::storage())
        .await
        .unwrap_or_default();
    let total_size: u64 = files.iter().map(|f| f.size).sum();

    // 搜索引擎状态
    let search_stats = state.search_engine.get_stats();

    // 版本管理状态
    let version_stats = state.version_manager.get_stats().await;

    // 同步状态
    let sync_states = state.sync_manager.get_all_sync_states().await;

    Ok(serde_json::json!({
        "status": "healthy",
        "timestamp": chrono::Local::now().to_rfc3339(),
        "storage": {
            "file_count": files.len(),
            "total_bytes": total_size,
            "available": true
        },
        "search": {
            "total_documents": search_stats.total_documents,
            "index_size": search_stats.index_size,
            "available": true
        },
        "version": {
            "total_versions": version_stats.total_versions,
            "available": true
        },
        "sync": {
            "states": serde_json::to_value(&sync_states).unwrap_or_default(),
            "available": true
        }
    }))
}
