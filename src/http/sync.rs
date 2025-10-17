//! 同步相关 API 端点

use super::state::AppState;
use http::StatusCode;
use silent::SilentError;
use silent::extractor::{Configs as CfgExtractor, Path};

/// 获取同步状态
pub async fn get_sync_state(
    (Path(id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    match state.sync_manager.get_sync_state(&id).await {
        Some(sync_state) => Ok(serde_json::to_value(sync_state).unwrap()),
        None => Err(SilentError::business_error(
            StatusCode::NOT_FOUND,
            "同步状态不存在",
        )),
    }
}

/// 列出所有同步状态
pub async fn list_sync_states(
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let states = state.sync_manager.get_all_sync_states().await;
    Ok(serde_json::to_value(states).unwrap())
}

/// 获取冲突列表
pub async fn get_conflicts(
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let conflicts = state.sync_manager.check_conflicts().await;
    Ok(serde_json::to_value(conflicts).unwrap())
}
