//! 上传会话管理 API
//!
//! 提供 HTTP REST API 用于管理大文件上传会话

use crate::http::state::AppState;
use crate::webdav::upload_session::{UploadSession, UploadStatus};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use silent::extractor::{Configs as CfgExtractor, Path};
use silent::prelude::*;

/// 会话响应（简化版，用于 API 返回）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResponse {
    pub session_id: String,
    pub file_path: String,
    pub total_size: u64,
    pub uploaded_size: u64,
    pub file_hash: Option<String>,
    pub status: String,
    pub progress_percent: f64,
    pub created_at: String,
    pub updated_at: String,
    pub expires_at: String,
    pub can_resume: bool,
    pub memory_usage: u64,
}

impl From<UploadSession> for SessionResponse {
    fn from(session: UploadSession) -> Self {
        let progress_percent = session.progress_percent();
        let can_resume = session.can_resume();

        Self {
            session_id: session.session_id,
            file_path: session.file_path,
            total_size: session.total_size,
            uploaded_size: session.uploaded_size,
            file_hash: session.file_hash,
            status: format!("{:?}", session.status),
            progress_percent,
            created_at: session.created_at.to_string(),
            updated_at: session.updated_at.to_string(),
            expires_at: session.expires_at.to_string(),
            can_resume,
            memory_usage: session.memory_usage,
        }
    }
}

/// 会话列表响应
#[derive(Debug, Serialize)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionResponse>,
    pub total: usize,
    pub active: usize,
}

/// 会话取消响应
#[derive(Debug, Serialize)]
pub struct SessionCancelResponse {
    pub session_id: String,
    pub message: String,
}

/// 会话恢复请求
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ResumeUploadRequest {
    /// 起始字节位置（可选，默认从已上传位置继续）
    pub start_byte: Option<u64>,
}

/// GET /api/upload/sessions/{session_id} - 查询会话状态
///
/// 返回指定会话的详细信息
pub async fn get_session(
    (Path(session_id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    // 获取会话管理器
    let sessions_manager = match state.upload_sessions {
        Some(ref mgr) => mgr,
        None => {
            return Err(SilentError::business_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "上传会话功能未启用",
            ));
        }
    };

    // 查询会话
    match sessions_manager.get_session(&session_id).await {
        Some(session) => {
            let response = SessionResponse::from(session);
            Ok(serde_json::to_value(&response).unwrap())
        }
        None => Err(SilentError::business_error(
            StatusCode::NOT_FOUND,
            format!("会话不存在: {}", session_id),
        )),
    }
}

/// GET /api/upload/sessions - 列出所有活跃会话
///
/// 返回所有活跃上传会话的列表
pub async fn list_sessions(
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    // 获取会话管理器
    let sessions_manager = match state.upload_sessions {
        Some(ref mgr) => mgr,
        None => {
            return Err(SilentError::business_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "上传会话功能未启用",
            ));
        }
    };

    // 获取所有活跃会话
    let sessions = sessions_manager.get_active_sessions().await;
    let total = sessions.len();
    let active = sessions
        .iter()
        .filter(|s| s.status == UploadStatus::Uploading)
        .count();

    let session_responses: Vec<SessionResponse> =
        sessions.into_iter().map(SessionResponse::from).collect();

    let response = SessionListResponse {
        sessions: session_responses,
        total,
        active,
    };

    Ok(serde_json::to_value(&response).unwrap())
}

/// DELETE /api/upload/sessions/{session_id} - 取消上传
///
/// 取消指定的上传会话并清理临时文件
pub async fn cancel_session(
    (Path(session_id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    // 获取会话管理器
    let sessions_manager = match state.upload_sessions {
        Some(ref mgr) => mgr,
        None => {
            return Err(SilentError::business_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "上传会话功能未启用",
            ));
        }
    };

    // 获取会话
    let session = match sessions_manager.get_session(&session_id).await {
        Some(s) => s,
        None => {
            return Err(SilentError::business_error(
                StatusCode::NOT_FOUND,
                format!("会话不存在: {}", session_id),
            ));
        }
    };

    // 标记会话为已取消
    let mut updated_session = session.clone();
    updated_session.status = UploadStatus::Cancelled;
    updated_session.updated_at = chrono::Local::now().naive_local();

    // 更新会话
    if let Err(e) = sessions_manager.update_session(updated_session).await {
        tracing::error!("更新会话状态失败: {}", e);
    }

    // 清理临时文件
    #[allow(clippy::collapsible_if)]
    if let Some(temp_path) = &session.temp_path {
        if temp_path.exists() {
            if let Err(e) = tokio::fs::remove_file(temp_path).await {
                tracing::warn!("删除临时文件失败: {} - {}", temp_path.display(), e);
            }
        }
    }

    // 删除会话
    sessions_manager.remove_session(&session_id).await;

    tracing::info!("上传会话已取消: session_id={}", session_id);

    let response = SessionCancelResponse {
        session_id: session_id.clone(),
        message: format!("会话 {} 已取消", session_id),
    };

    Ok(serde_json::to_value(&response).unwrap())
}

/// POST /api/upload/sessions/{session_id}/pause - 暂停上传
///
/// 暂停正在进行的上传会话
pub async fn pause_session(
    (Path(session_id), CfgExtractor(state)): (Path<String>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    // 获取会话管理器
    let sessions_manager = match state.upload_sessions {
        Some(ref mgr) => mgr,
        None => {
            return Err(SilentError::business_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "上传会话功能未启用",
            ));
        }
    };

    // 获取会话
    let mut session = match sessions_manager.get_session(&session_id).await {
        Some(s) => s,
        None => {
            return Err(SilentError::business_error(
                StatusCode::NOT_FOUND,
                format!("会话不存在: {}", session_id),
            ));
        }
    };

    // 检查会话状态
    if session.status != UploadStatus::Uploading {
        return Err(SilentError::business_error(
            StatusCode::BAD_REQUEST,
            format!("会话状态不允许暂停: {:?}", session.status),
        ));
    }

    // 标记为暂停
    session.status = UploadStatus::Paused;
    session.updated_at = chrono::Local::now().naive_local();

    // 更新会话
    sessions_manager
        .update_session(session.clone())
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("暂停会话失败: {}", e),
            )
        })?;

    tracing::info!("上传会话已暂停: session_id={}", session_id);

    let response = SessionResponse::from(session);
    Ok(serde_json::to_value(&response).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webdav::upload_session::UploadSession;

    #[test]
    fn test_session_response_from_upload_session() {
        let session = UploadSession::new("/test/file.txt".to_string(), 1000, 24);
        let response = SessionResponse::from(session.clone());

        assert_eq!(response.session_id, session.session_id);
        assert_eq!(response.file_path, "/test/file.txt");
        assert_eq!(response.total_size, 1000);
        assert_eq!(response.uploaded_size, 0);
        assert_eq!(response.progress_percent, 0.0);
        assert!(!response.can_resume);
    }

    #[test]
    fn test_session_response_with_progress() {
        let mut session = UploadSession::new("/test/file.txt".to_string(), 1000, 24);
        session.uploaded_size = 500;
        session.status = UploadStatus::Uploading;

        let response = SessionResponse::from(session);

        assert_eq!(response.uploaded_size, 500);
        assert_eq!(response.progress_percent, 50.0);
        assert_eq!(response.status, "Uploading");
    }

    #[test]
    fn test_session_list_response_serialization() {
        let sessions = vec![SessionResponse {
            session_id: "session1".to_string(),
            file_path: "/test/file1.txt".to_string(),
            total_size: 1000,
            uploaded_size: 500,
            file_hash: None,
            status: "Uploading".to_string(),
            progress_percent: 50.0,
            created_at: "2024-01-01 00:00:00".to_string(),
            updated_at: "2024-01-01 00:00:00".to_string(),
            expires_at: "2024-01-02 00:00:00".to_string(),
            can_resume: false,
            memory_usage: 8388608, // 8MB
        }];

        let response = SessionListResponse {
            sessions,
            total: 1,
            active: 1,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("session1"));
        assert!(json.contains("\"total\":1"));
        assert!(json.contains("\"active\":1"));
    }

    #[test]
    fn test_session_cancel_response_serialization() {
        let response = SessionCancelResponse {
            session_id: "session1".to_string(),
            message: "会话已取消".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("session1"));
        assert!(json.contains("会话已取消"));
    }

    #[test]
    fn test_resume_upload_request_deserialization() {
        let json = r#"{"start_byte": 1024}"#;
        let req: ResumeUploadRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.start_byte, Some(1024));

        let json = r#"{}"#;
        let req: ResumeUploadRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.start_byte, None);
    }
}
