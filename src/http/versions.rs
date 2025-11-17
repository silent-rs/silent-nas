//! 版本管理 API 端点

use super::state::AppState;
use crate::models::{EventType, FileEvent};
use http::StatusCode;
use silent::SilentError;
use silent::extractor::{Configs as CfgExtractor, Path};
use silent::prelude::*;
use silent_nas_core::StorageManagerTrait;

/// 列出文件版本
pub async fn list_versions(
    (Path(id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    let storage = &state.storage;

    let versions = storage.list_file_versions(&id).await.map_err(|e| {
        SilentError::business_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("获取版本列表失败: {}", e),
        )
    })?;

    Ok(serde_json::to_value(versions).unwrap())
}

/// 获取特定版本
pub async fn get_version(
    (Path(version_id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<Response> {
    let storage = &state.storage;

    let data = storage.read_version_data(&version_id).await.map_err(|e| {
        SilentError::business_error(StatusCode::NOT_FOUND, format!("版本不存在: {}", e))
    })?;

    let mut resp = Response::empty();
    resp.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/octet-stream"),
    );
    resp.set_body(full(data));
    Ok(resp)
}

/// 恢复版本
pub async fn restore_version(
    (Path(file_id), Path(version_id), CfgExtractor(state)): (
        Path<String>,
        Path<String>,
        CfgExtractor<AppState>,
    ),
) -> silent::Result<serde_json::Value> {
    let storage = &state.storage;

    storage
        .restore_file_version(&file_id, &version_id)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("恢复版本失败: {}", e),
            )
        })?;

    // 发送修改事件
    if let Ok(metadata) = storage.get_metadata(&file_id).await {
        let event = FileEvent::new(EventType::Modified, file_id.clone(), Some(metadata));
        if let Some(ref n) = state.notifier {
            let _ = n.notify_modified(event).await;
        }
    }

    Ok(serde_json::json!({"success": true, "file_id": file_id, "version_id": version_id}))
}

/// 删除版本
pub async fn delete_version(
    (Path(version_id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    let storage = &state.storage;

    storage
        .delete_file_version(&version_id)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("删除版本失败: {}", e),
            )
        })?;

    Ok(serde_json::json!({"success": true}))
}

/// 获取版本统计
pub async fn get_version_stats(
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let storage = &state.storage;

    let stats = storage.get_storage_stats().await.map_err(|e| {
        SilentError::business_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("获取统计信息失败: {}", e),
        )
    })?;

    Ok(serde_json::to_value(stats).unwrap())
}
