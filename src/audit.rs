//! 审计日志模块
//!
//! 记录关键操作的审计日志，用于安全审查和合规性

#![allow(dead_code)] // 这些方法将在后续集成时使用

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 审计事件类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuditAction {
    /// 文件上传
    FileUpload,
    /// 文件下载
    FileDownload,
    /// 文件删除
    FileDelete,
    /// 版本创建
    VersionCreate,
    /// 版本恢复
    VersionRestore,
    /// 版本删除
    VersionDelete,
    /// 搜索查询
    SearchQuery,
    /// 同步操作
    SyncOperation,
    /// 配置更改
    ConfigChange,
    /// 认证尝试
    AuthAttempt,
}

/// 审计事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// 事件ID
    pub id: String,
    /// 事件时间
    pub timestamp: DateTime<Local>,
    /// 操作类型
    pub action: AuditAction,
    /// 资源ID（文件ID、版本ID等）
    pub resource_id: Option<String>,
    /// 用户ID（当前为可选，未来认证系统完善后必填）
    pub user_id: Option<String>,
    /// 客户端IP
    pub client_ip: Option<String>,
    /// 操作结果
    pub success: bool,
    /// 错误信息（失败时）
    pub error_message: Option<String>,
    /// 附加元数据
    pub metadata: serde_json::Value,
}

impl AuditEvent {
    /// 创建新的审计事件
    pub fn new(action: AuditAction, resource_id: Option<String>) -> Self {
        Self {
            id: scru128::new_string(),
            timestamp: Local::now(),
            action,
            resource_id,
            user_id: None,
            client_ip: None,
            success: true,
            error_message: None,
            metadata: serde_json::json!({}),
        }
    }

    /// 设置用户ID
    pub fn with_user(mut self, user_id: String) -> Self {
        self.user_id = Some(user_id);
        self
    }

    /// 设置客户端IP
    pub fn with_client_ip(mut self, client_ip: String) -> Self {
        self.client_ip = Some(client_ip);
        self
    }

    /// 设置失败状态
    pub fn with_error(mut self, error: String) -> Self {
        self.success = false;
        self.error_message = Some(error);
        self
    }

    /// 设置元数据
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// 记录到日志
    pub fn log(&self) {
        let json = serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string());
        if self.success {
            tracing::info!(target: "audit", "{}", json);
        } else {
            tracing::warn!(target: "audit", "{}", json);
        }
    }
}

/// 审计日志管理器
pub struct AuditLogger {
    /// 内存缓存的审计事件（可选，用于查询最近事件）
    events: Arc<RwLock<Vec<AuditEvent>>>,
    /// 最大缓存事件数
    max_events: usize,
}

impl AuditLogger {
    /// 创建审计日志管理器
    pub fn new(max_events: usize) -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::with_capacity(max_events))),
            max_events,
        }
    }

    /// 记录审计事件
    pub async fn log(&self, event: AuditEvent) {
        // 写入日志
        event.log();

        // 缓存到内存
        let mut events = self.events.write().await;
        events.push(event);

        // 保持缓存大小限制
        if events.len() > self.max_events {
            let drain_count = events.len() - self.max_events;
            events.drain(0..drain_count);
        }
    }

    /// 获取最近的审计事件
    pub async fn get_recent_events(&self, limit: usize) -> Vec<AuditEvent> {
        let events = self.events.read().await;
        let start = events.len().saturating_sub(limit);
        events[start..].to_vec()
    }

    /// 按操作类型筛选事件
    pub async fn filter_by_action(&self, action: AuditAction, limit: usize) -> Vec<AuditEvent> {
        let events = self.events.read().await;
        events
            .iter()
            .filter(|e| e.action == action)
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    /// 按资源ID筛选事件
    pub async fn filter_by_resource(&self, resource_id: &str, limit: usize) -> Vec<AuditEvent> {
        let events = self.events.read().await;
        events
            .iter()
            .filter(|e| e.resource_id.as_deref() == Some(resource_id))
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    /// 获取统计信息
    pub async fn get_stats(&self) -> AuditStats {
        let events = self.events.read().await;

        let total = events.len();
        let successful = events.iter().filter(|e| e.success).count();
        let failed = total - successful;

        // 按操作类型统计
        let mut action_counts = std::collections::HashMap::new();
        for event in events.iter() {
            *action_counts
                .entry(format!("{:?}", event.action))
                .or_insert(0) += 1;
        }

        AuditStats {
            total_events: total,
            successful_events: successful,
            failed_events: failed,
            action_counts,
        }
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        Self::new(1000)
    }
}

/// 审计统计信息
#[derive(Debug, Clone, Serialize)]
pub struct AuditStats {
    pub total_events: usize,
    pub successful_events: usize,
    pub failed_events: usize,
    pub action_counts: std::collections::HashMap<String, usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_event_creation() {
        let event = AuditEvent::new(AuditAction::FileUpload, Some("file-123".to_string()));

        assert_eq!(event.action, AuditAction::FileUpload);
        assert_eq!(event.resource_id, Some("file-123".to_string()));
        assert!(event.success);
        assert!(event.error_message.is_none());
    }

    #[test]
    fn test_audit_event_with_error() {
        let event = AuditEvent::new(AuditAction::FileDownload, Some("file-456".to_string()))
            .with_error("File not found".to_string());

        assert!(!event.success);
        assert_eq!(event.error_message, Some("File not found".to_string()));
    }

    #[test]
    fn test_audit_event_with_user() {
        let event = AuditEvent::new(AuditAction::FileDelete, Some("file-789".to_string()))
            .with_user("user-001".to_string())
            .with_client_ip("192.168.1.100".to_string());

        assert_eq!(event.user_id, Some("user-001".to_string()));
        assert_eq!(event.client_ip, Some("192.168.1.100".to_string()));
    }

    #[test]
    fn test_audit_event_serialization() {
        let event = AuditEvent::new(AuditAction::SearchQuery, None)
            .with_metadata(serde_json::json!({"query": "test"}));

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("SearchQuery"));
        assert!(json.contains("test"));
    }

    #[tokio::test]
    async fn test_audit_logger_basic() {
        let logger = AuditLogger::new(10);

        let event = AuditEvent::new(AuditAction::FileUpload, Some("file-1".to_string()));
        logger.log(event).await;

        let recent = logger.get_recent_events(10).await;
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].action, AuditAction::FileUpload);
    }

    #[tokio::test]
    async fn test_audit_logger_max_events() {
        let logger = AuditLogger::new(5);

        // 添加10个事件
        for i in 0..10 {
            let event = AuditEvent::new(AuditAction::FileUpload, Some(format!("file-{}", i)));
            logger.log(event).await;
        }

        let recent = logger.get_recent_events(100).await;
        // 应该只保留最后5个
        assert_eq!(recent.len(), 5);
        assert_eq!(recent[0].resource_id, Some("file-5".to_string()));
        assert_eq!(recent[4].resource_id, Some("file-9".to_string()));
    }

    #[tokio::test]
    async fn test_audit_logger_filter_by_action() {
        let logger = AuditLogger::new(100);

        // 添加不同类型的事件
        logger
            .log(AuditEvent::new(
                AuditAction::FileUpload,
                Some("file-1".to_string()),
            ))
            .await;
        logger
            .log(AuditEvent::new(
                AuditAction::FileDownload,
                Some("file-2".to_string()),
            ))
            .await;
        logger
            .log(AuditEvent::new(
                AuditAction::FileUpload,
                Some("file-3".to_string()),
            ))
            .await;

        let uploads = logger.filter_by_action(AuditAction::FileUpload, 10).await;
        assert_eq!(uploads.len(), 2);

        let downloads = logger.filter_by_action(AuditAction::FileDownload, 10).await;
        assert_eq!(downloads.len(), 1);
    }

    #[tokio::test]
    async fn test_audit_logger_filter_by_resource() {
        let logger = AuditLogger::new(100);

        // 同一文件的多个操作
        logger
            .log(AuditEvent::new(
                AuditAction::FileUpload,
                Some("file-123".to_string()),
            ))
            .await;
        logger
            .log(AuditEvent::new(
                AuditAction::FileDownload,
                Some("file-123".to_string()),
            ))
            .await;
        logger
            .log(AuditEvent::new(
                AuditAction::FileUpload,
                Some("file-456".to_string()),
            ))
            .await;

        let events = logger.filter_by_resource("file-123", 10).await;
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn test_audit_logger_stats() {
        let logger = AuditLogger::new(100);

        // 添加成功和失败的事件
        logger
            .log(AuditEvent::new(
                AuditAction::FileUpload,
                Some("file-1".to_string()),
            ))
            .await;
        logger
            .log(
                AuditEvent::new(AuditAction::FileDownload, Some("file-2".to_string()))
                    .with_error("Not found".to_string()),
            )
            .await;
        logger
            .log(AuditEvent::new(
                AuditAction::FileUpload,
                Some("file-3".to_string()),
            ))
            .await;

        let stats = logger.get_stats().await;
        assert_eq!(stats.total_events, 3);
        assert_eq!(stats.successful_events, 2);
        assert_eq!(stats.failed_events, 1);
        assert!(stats.action_counts.contains_key("FileUpload"));
    }
}
