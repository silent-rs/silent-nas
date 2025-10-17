//! 增量同步 API 端点

use super::state::AppState;
use http::StatusCode;
use http_body_util::BodyExt;
use silent::SilentError;
use silent::extractor::{Configs as CfgExtractor, Path};
use silent::prelude::*;

#[cfg(not(test))]
use crate::sync::incremental::{FileSignature, api};

#[cfg(test)]
use crate::sync::incremental::{FileSignature, api};

/// 获取文件签名
pub async fn get_file_signature(
    (Path(id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    let signature = api::handle_get_signature(&state.inc_sync_handler, &id)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("计算文件签名失败: {}", e),
            )
        })?;
    Ok(serde_json::to_value(signature).unwrap())
}

/// 获取文件差异
pub async fn get_file_delta(
    mut req: Request,
    (Path(id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    // 从请求体中读取目标签名
    let body = req.take_body();
    let body_bytes = match body {
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

    let request: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|e| {
        SilentError::business_error(StatusCode::BAD_REQUEST, format!("解析请求失败: {}", e))
    })?;

    let target_sig: FileSignature = serde_json::from_value(request["target_signature"].clone())
        .map_err(|e| {
            SilentError::business_error(StatusCode::BAD_REQUEST, format!("解析目标签名失败: {}", e))
        })?;

    let delta_chunks = api::handle_get_delta(&state.inc_sync_handler, &id, &target_sig)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("生成差异块失败: {}", e),
            )
        })?;

    Ok(serde_json::to_value(delta_chunks).unwrap())
}
