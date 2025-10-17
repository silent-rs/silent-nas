//! 审计日志 API 端点

use super::state::AppState;
use crate::audit::AuditAction;
use http::StatusCode;
use serde::Deserialize;
use silent::SilentError;
use silent::extractor::{Configs as CfgExtractor, Query};

/// 审计查询参数
#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    /// 限制返回数量
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// 按操作类型筛选
    pub action: Option<String>,
    /// 按资源ID筛选
    pub resource_id: Option<String>,
}

fn default_limit() -> usize {
    50
}

/// 获取审计日志
pub async fn get_audit_logs(
    (Query(query), CfgExtractor(state)): (Query<AuditQuery>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    if let Some(ref audit_logger) = state.audit_logger {
        let events = if let Some(ref action_str) = query.action {
            // 按操作类型筛选
            let action = parse_audit_action(action_str)?;
            audit_logger.filter_by_action(action, query.limit).await
        } else if let Some(ref resource_id) = query.resource_id {
            // 按资源ID筛选
            audit_logger
                .filter_by_resource(resource_id, query.limit)
                .await
        } else {
            // 获取最近的事件
            audit_logger.get_recent_events(query.limit).await
        };

        Ok(serde_json::json!({
            "events": events,
            "count": events.len()
        }))
    } else {
        Err(SilentError::business_error(
            StatusCode::NOT_IMPLEMENTED,
            "审计日志功能未启用",
        ))
    }
}

/// 获取审计统计
pub async fn get_audit_stats(
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    if let Some(ref audit_logger) = state.audit_logger {
        let stats = audit_logger.get_stats().await;
        Ok(serde_json::to_value(stats).unwrap())
    } else {
        Err(SilentError::business_error(
            StatusCode::NOT_IMPLEMENTED,
            "审计日志功能未启用",
        ))
    }
}

/// 解析操作类型字符串
fn parse_audit_action(s: &str) -> silent::Result<AuditAction> {
    match s.to_lowercase().as_str() {
        "fileupload" | "file_upload" => Ok(AuditAction::FileUpload),
        "filedownload" | "file_download" => Ok(AuditAction::FileDownload),
        "filedelete" | "file_delete" => Ok(AuditAction::FileDelete),
        "versioncreate" | "version_create" => Ok(AuditAction::VersionCreate),
        "versionrestore" | "version_restore" => Ok(AuditAction::VersionRestore),
        "versiondelete" | "version_delete" => Ok(AuditAction::VersionDelete),
        "searchquery" | "search_query" => Ok(AuditAction::SearchQuery),
        "syncoperation" | "sync_operation" => Ok(AuditAction::SyncOperation),
        "configchange" | "config_change" => Ok(AuditAction::ConfigChange),
        "authattempt" | "auth_attempt" => Ok(AuditAction::AuthAttempt),
        _ => Err(SilentError::business_error(
            StatusCode::BAD_REQUEST,
            format!("无效的操作类型: {}", s),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_audit_action() {
        assert!(matches!(
            parse_audit_action("fileupload").unwrap(),
            AuditAction::FileUpload
        ));
        assert!(matches!(
            parse_audit_action("file_upload").unwrap(),
            AuditAction::FileUpload
        ));
        assert!(matches!(
            parse_audit_action("FileUpload").unwrap(),
            AuditAction::FileUpload
        ));

        assert!(parse_audit_action("invalid").is_err());
    }

    #[test]
    fn test_default_limit() {
        assert_eq!(default_limit(), 50);
    }
}
