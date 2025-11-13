//! 文件操作 API 端点

use super::state::AppState;
use crate::models::{EventType, FileEvent};
use http::StatusCode;
use http_body_util::BodyExt;
use silent::SilentError;
use silent::extractor::{Configs as CfgExtractor, Path};
use silent::prelude::*;
use silent_nas_core::StorageManager as StorageManagerTrait;

/// 上传文件
pub async fn upload_file(
    mut req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let file_id = scru128::new_string();

    let body = req.take_body();
    let bytes = match body {
        ReqBody::Incoming(body) => body
            .collect()
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::BAD_REQUEST,
                    format!("读取请求体失败: {}", e),
                )
            })?
            .to_bytes()
            .to_vec(),
        ReqBody::Once(bytes) => bytes.to_vec(),
        ReqBody::Empty => {
            return Err(SilentError::business_error(
                StatusCode::BAD_REQUEST,
                "请求体为空",
            ));
        }
    };

    let metadata = crate::storage::storage()
        .save_file(&file_id, &bytes)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("保存文件失败: {}", e),
            )
        })?;

    // 索引文件到搜索引擎
    if let Err(e) = state.search_engine.index_file(&metadata).await {
        tracing::warn!("索引文件失败: {} - {}", file_id, e);
    }

    let mut event = FileEvent::new(EventType::Created, file_id.clone(), Some(metadata.clone()));
    event.source_http_addr = Some((*state.source_http_addr).clone());
    if let Some(ref n) = state.notifier {
        let _ = n.notify_created(event).await;
    }

    Ok(serde_json::json!({
        "file_id": file_id,
        "size": metadata.size,
        "hash": metadata.hash,
    }))
}

/// 下载文件
pub async fn download_file(
    (Path(id), CfgExtractor(_state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<Response> {
    let data = crate::storage::storage()
        .read_file(&id)
        .await
        .map_err(|e| {
            SilentError::business_error(StatusCode::NOT_FOUND, format!("文件不存在: {}", e))
        })?;

    let mut resp = Response::empty();
    resp.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/octet-stream"),
    );
    resp.set_body(full(data));
    Ok(resp)
}

/// 删除文件
pub async fn delete_file(
    (Path(id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    crate::storage::storage()
        .delete_file(&id)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("删除文件失败: {}", e),
            )
        })?;

    // 从搜索引擎删除索引
    if let Err(e) = state.search_engine.delete_file(&id).await {
        tracing::warn!("删除索引失败: {} - {}", id, e);
    }

    let event = FileEvent::new(EventType::Deleted, id, None);
    if let Some(ref n) = state.notifier {
        let _ = n.notify_deleted(event).await;
    }

    Ok(serde_json::json!({"success": true}))
}

/// 列出文件
pub async fn list_files(
    CfgExtractor(_state): CfgExtractor<AppState>,
) -> silent::Result<Vec<crate::models::FileMetadata>> {
    crate::storage::storage().list_files().await.map_err(|e| {
        SilentError::business_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("列出文件失败: {}", e),
        )
    })
}
