//! 管理员API处理器

use super::state::AppState;
use crate::auth::{UserInfo, UserRole, UserStatus};
use crate::error::NasError;
use http::StatusCode;
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use silent::SilentError;
use silent::extractor::Configs as CfgExtractor;
use silent::prelude::*;
use tracing::{info, warn};
use validator::Validate;

/// 触发跨节点 push 同步请求
#[derive(Debug, Deserialize)]
pub struct PushSyncRequest {
    /// 目标节点 gRPC 地址：host:port
    pub target: String,
    /// 指定文件ID列表，若为空或缺省则默认推送所有未删除文件
    pub file_ids: Option<Vec<String>>,
}

/// POST /api/admin/sync/push
/// 触发本节点向指定 gRPC 地址的节点推送文件（先同步状态，再流式推送内容）
pub async fn trigger_push_sync(
    mut req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    // 解析请求体
    let body = req.take_body();
    let bytes = match body {
        ReqBody::Incoming(body) => body.collect().await?.to_bytes().to_vec(),
        ReqBody::Once(bytes) => bytes.to_vec(),
        ReqBody::Empty => {
            return Err(SilentError::business_error(
                StatusCode::BAD_REQUEST,
                "请求体为空",
            ));
        }
    };

    let payload: PushSyncRequest = serde_json::from_slice(&bytes).map_err(|e| {
        SilentError::business_error(StatusCode::BAD_REQUEST, format!("解析请求失败: {}", e))
    })?;

    info!("管理员触发push同步 -> {}", payload.target);

    // 组装待推送文件
    let file_ids: Vec<String> = if let Some(list) = payload.file_ids {
        list
    } else {
        state
            .sync_manager
            .get_all_sync_states()
            .await
            .into_iter()
            .filter(|s| !s.is_deleted())
            .map(|s| s.file_id)
            .collect()
    };

    // 客户端
    use crate::sync::node::client::{ClientConfig, NodeSyncClient};
    let client = NodeSyncClient::new(payload.target.clone(), ClientConfig::default());
    client
        .connect()
        .await
        .map_err(|e| SilentError::business_error(StatusCode::BAD_GATEWAY, e.to_string()))?;

    // 逐个文件：先同步状态，再流式推送内容
    use crate::rpc::file_service::{
        FileMetadata as ProtoFileMetadata, FileSyncState as ProtoFileSyncState,
    };
    use tokio::fs;

    let mut success = 0usize;
    for file_id in file_ids.iter() {
        if let Some(file_sync) = state.sync_manager.get_sync_state(file_id).await {
            // 发送状态
            let proto_meta = file_sync.metadata.value.clone().map(|m| ProtoFileMetadata {
                id: m.id,
                name: m.name,
                path: m.path,
                size: m.size,
                hash: m.hash,
                created_at: m.created_at.to_string(),
                modified_at: m.modified_at.to_string(),
            });
            let vc_json =
                serde_json::to_string(&file_sync.vector_clock).unwrap_or_else(|_| "{}".to_string());
            let state_msg = ProtoFileSyncState {
                file_id: file_id.clone(),
                metadata: proto_meta,
                deleted: file_sync.deleted.value.unwrap_or(false),
                vector_clock: vc_json,
                timestamp: chrono::Local::now().timestamp_millis(),
            };
            let _ = client
                .sync_file_states(state.sync_manager.node_id(), vec![state_msg])
                .await;

            // 读取文件内容（优先路径）
            let content_res = if let Some(meta) = file_sync.metadata.value.as_ref() {
                let full = state.storage.get_full_path(&meta.path);
                fs::read(full).await.map_err(|e| {
                    SilentError::business_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
                })
            } else {
                state.storage.read_file(file_id).await.map_err(|e| {
                    SilentError::business_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
                })
            };

            match content_res {
                Ok(content) => {
                    if let Err(e) = client
                        .stream_file_content(file_id, content, 1024 * 1024)
                        .await
                    {
                        warn!("流式推送失败: {} - {}", file_id, e);
                    } else {
                        success += 1;
                    }
                }
                Err(e) => {
                    warn!("读取文件失败: {} - {}", file_id, e);
                }
            }
        }
    }

    client.disconnect().await;

    Ok(serde_json::json!({
        "target": payload.target,
        "requested": file_ids.len(),
        "pushed": success,
    }))
}

/// 更新用户请求
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateUserRequest {
    /// 新角色（可选）
    pub role: Option<UserRole>,
    /// 新状态（可选）
    pub status: Option<UserStatus>,
}

/// 重置密码请求
#[derive(Debug, Deserialize, Validate)]
pub struct ResetPasswordRequest {
    /// 新密码（8-72个字符）
    #[validate(length(min = 8, max = 72, message = "密码长度必须在8-72个字符之间"))]
    pub new_password: String,
}

/// 用户列表响应
#[derive(Debug, Serialize)]
pub struct UserListResponse {
    pub users: Vec<UserInfo>,
    pub total: usize,
}

/// 成功响应
#[derive(Serialize)]
struct SuccessResponse {
    message: String,
}

/// 获取所有用户列表
///
/// GET /api/admin/users
/// 需要管理员权限
pub async fn list_users(
    _req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let auth_manager = state.auth_manager.as_ref().ok_or_else(|| {
        SilentError::business_error(StatusCode::SERVICE_UNAVAILABLE, "认证系统未初始化")
    })?;

    let users = auth_manager.list_users().await.map_err(|e| {
        SilentError::business_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("获取用户列表失败: {}", e),
        )
    })?;

    let total = users.len();
    let user_infos: Vec<UserInfo> = users.into_iter().map(UserInfo::from).collect();

    let response = UserListResponse {
        users: user_infos,
        total,
    };

    Ok(serde_json::to_value(&response).unwrap())
}

/// 获取指定用户信息
///
/// GET /api/admin/users/:id
/// 需要管理员权限
pub async fn get_user(
    mut req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let user_id = req
        .params()
        .get("id")
        .ok_or_else(|| SilentError::business_error(StatusCode::BAD_REQUEST, "缺少用户ID参数"))?
        .to_string();

    let auth_manager = state.auth_manager.as_ref().ok_or_else(|| {
        SilentError::business_error(StatusCode::SERVICE_UNAVAILABLE, "认证系统未初始化")
    })?;

    let user = auth_manager
        .get_user_by_id(&user_id)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("获取用户失败: {}", e),
            )
        })?
        .ok_or_else(|| SilentError::business_error(StatusCode::NOT_FOUND, "用户不存在"))?;

    Ok(serde_json::to_value(UserInfo::from(user)).unwrap())
}

/// 更新用户信息
///
/// PUT /api/admin/users/:id
/// 需要管理员权限
pub async fn update_user(
    mut req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let user_id = req
        .params()
        .get("id")
        .ok_or_else(|| SilentError::business_error(StatusCode::BAD_REQUEST, "缺少用户ID参数"))?
        .to_string();

    // 解析请求体
    let body = req.take_body();
    let bytes = match body {
        ReqBody::Incoming(body) => body.collect().await?.to_bytes().to_vec(),
        ReqBody::Once(bytes) => bytes.to_vec(),
        ReqBody::Empty => {
            return Err(SilentError::business_error(
                StatusCode::BAD_REQUEST,
                "请求体为空",
            ));
        }
    };

    let update_req: UpdateUserRequest = serde_json::from_slice(&bytes)
        .map_err(|e| SilentError::business_error(StatusCode::BAD_REQUEST, e.to_string()))?;

    // 验证请求
    update_req
        .validate()
        .map_err(|e| SilentError::business_error(StatusCode::BAD_REQUEST, e.to_string()))?;

    let auth_manager = state.auth_manager.as_ref().ok_or_else(|| {
        SilentError::business_error(StatusCode::SERVICE_UNAVAILABLE, "认证系统未初始化")
    })?;

    // 获取目标用户
    let mut user = auth_manager
        .get_user_by_id(&user_id)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("获取用户失败: {}", e),
            )
        })?
        .ok_or_else(|| SilentError::business_error(StatusCode::NOT_FOUND, "用户不存在"))?;

    let old_role = user.role;
    let old_status = user.status;

    // 应用更新
    if let Some(role) = update_req.role {
        user.role = role;
    }
    if let Some(status) = update_req.status {
        user.status = status;
    }

    // 更新数据库
    auth_manager.update_user(&user).await.map_err(|e| {
        SilentError::business_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("更新用户失败: {}", e),
        )
    })?;

    // 记录审计日志
    if let Some(audit_logger) = &state.audit_logger {
        use crate::audit::{AuditAction, AuditEvent};

        let mut details = Vec::new();
        if update_req.role.is_some() && old_role != user.role {
            details.push(format!("角色: {} -> {}", old_role, user.role));
        }
        if update_req.status.is_some() && old_status != user.status {
            details.push(format!("状态: {} -> {}", old_status, user.status));
        }

        if !details.is_empty() {
            let event = AuditEvent::new(AuditAction::ConfigChange, Some(user_id.clone()))
                .with_user("admin".to_string())
                .with_metadata(serde_json::json!({
                    "action": "update_user",
                    "details": details.join(", ")
                }));
            let _ = audit_logger.log(event).await;
        }
    }

    Ok(serde_json::to_value(UserInfo::from(user)).unwrap())
}

/// 重置用户密码
///
/// POST /api/admin/users/:id/reset-password
/// 需要管理员权限
pub async fn reset_password(
    mut req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let user_id = req
        .params()
        .get("id")
        .ok_or_else(|| SilentError::business_error(StatusCode::BAD_REQUEST, "缺少用户ID参数"))?
        .to_string();

    // 解析请求体
    let body = req.take_body();
    let bytes = match body {
        ReqBody::Incoming(body) => body.collect().await?.to_bytes().to_vec(),
        ReqBody::Once(bytes) => bytes.to_vec(),
        ReqBody::Empty => {
            return Err(SilentError::business_error(
                StatusCode::BAD_REQUEST,
                "请求体为空",
            ));
        }
    };

    let reset_req: ResetPasswordRequest = serde_json::from_slice(&bytes)
        .map_err(|e| SilentError::business_error(StatusCode::BAD_REQUEST, e.to_string()))?;

    // 验证请求
    reset_req
        .validate()
        .map_err(|e| SilentError::business_error(StatusCode::BAD_REQUEST, e.to_string()))?;

    let auth_manager = state.auth_manager.as_ref().ok_or_else(|| {
        SilentError::business_error(StatusCode::SERVICE_UNAVAILABLE, "认证系统未初始化")
    })?;

    // 重置密码
    auth_manager
        .reset_password(&user_id, &reset_req.new_password)
        .await
        .map_err(|e| match e {
            NasError::Auth(msg) => SilentError::business_error(StatusCode::BAD_REQUEST, msg),
            _ => SilentError::business_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        })?;

    // 记录审计日志
    if let Some(audit_logger) = &state.audit_logger {
        use crate::audit::{AuditAction, AuditEvent};

        let event = AuditEvent::new(AuditAction::ConfigChange, Some(user_id.clone()))
            .with_user("admin".to_string())
            .with_metadata(serde_json::json!({
                "action": "reset_password",
                "details": "管理员重置用户密码"
            }));
        let _ = audit_logger.log(event).await;
    }

    Ok(serde_json::to_value(&SuccessResponse {
        message: "密码重置成功".to_string(),
    })
    .unwrap())
}

/// 删除用户
///
/// DELETE /API/admin/users/:id
/// 需要管理员权限
pub async fn delete_user(
    mut req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let user_id = req
        .params()
        .get("id")
        .ok_or_else(|| SilentError::business_error(StatusCode::BAD_REQUEST, "缺少用户ID参数"))?
        .to_string();

    let auth_manager = state.auth_manager.as_ref().ok_or_else(|| {
        SilentError::business_error(StatusCode::SERVICE_UNAVAILABLE, "认证系统未初始化")
    })?;

    // 获取目标用户
    let user = auth_manager
        .get_user_by_id(&user_id)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("获取用户失败: {}", e),
            )
        })?
        .ok_or_else(|| SilentError::business_error(StatusCode::NOT_FOUND, "用户不存在"))?;

    // 删除用户（软删除）
    auth_manager.delete_user(&user_id).await.map_err(|e| {
        SilentError::business_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("删除用户失败: {}", e),
        )
    })?;

    // 记录审计日志
    if let Some(audit_logger) = &state.audit_logger {
        use crate::audit::{AuditAction, AuditEvent};

        let event = AuditEvent::new(AuditAction::ConfigChange, Some(user_id.clone()))
            .with_user("admin".to_string())
            .with_metadata(serde_json::json!({
                "action": "delete_user",
                "username": user.username,
                "details": format!("删除用户: {}", user.username)
            }));
        let _ = audit_logger.log(event).await;
    }

    Ok(serde_json::to_value(&SuccessResponse {
        message: "用户删除成功".to_string(),
    })
    .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_user_request_validation() {
        let valid = UpdateUserRequest {
            role: Some(UserRole::User),
            status: Some(UserStatus::Active),
        };
        assert!(valid.validate().is_ok());

        let empty = UpdateUserRequest {
            role: None,
            status: None,
        };
        assert!(empty.validate().is_ok());
    }

    #[test]
    fn test_reset_password_request_validation() {
        let valid = ResetPasswordRequest {
            new_password: "NewSecure123!".to_string(),
        };
        assert!(valid.validate().is_ok());

        let short = ResetPasswordRequest {
            new_password: "short".to_string(),
        };
        assert!(short.validate().is_err());

        let long = ResetPasswordRequest {
            new_password: "a".repeat(73),
        };
        assert!(long.validate().is_err());
    }
}
