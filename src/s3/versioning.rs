// S3 Bucket 版本控制管理
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 版本控制状态
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VersioningStatus {
    /// 未启用
    Disabled,
    /// 启用中
    Enabled,
    /// 已暂停
    Suspended,
}

impl Default for VersioningStatus {
    fn default() -> Self {
        Self::Disabled
    }
}

impl VersioningStatus {
    pub fn to_string(&self) -> &'static str {
        match self {
            Self::Disabled => "",
            Self::Enabled => "Enabled",
            Self::Suspended => "Suspended",
        }
    }

    #[allow(dead_code)]
    pub fn parse(s: &str) -> Self {
        match s {
            "Enabled" => Self::Enabled,
            "Suspended" => Self::Suspended,
            _ => Self::Disabled,
        }
    }
}

/// Bucket 版本控制配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketVersioning {
    pub status: VersioningStatus,
    /// 是否启用 MFA 删除（暂不实现）
    #[allow(dead_code)]
    pub mfa_delete: Option<bool>,
}

impl Default for BucketVersioning {
    fn default() -> Self {
        Self {
            status: VersioningStatus::Disabled,
            mfa_delete: None,
        }
    }
}

/// 版本控制管理器
pub struct VersioningManager {
    /// bucket -> 版本控制配置
    configs: Arc<RwLock<HashMap<String, BucketVersioning>>>,
}

impl Default for VersioningManager {
    fn default() -> Self {
        Self {
            configs: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl VersioningManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// 获取 bucket 的版本控制配置
    pub async fn get_versioning(&self, bucket: &str) -> BucketVersioning {
        let configs = self.configs.read().await;
        configs.get(bucket).cloned().unwrap_or_default()
    }

    /// 设置 bucket 的版本控制状态
    pub async fn set_versioning(&self, bucket: &str, status: VersioningStatus) {
        let mut configs = self.configs.write().await;
        let config = configs.entry(bucket.to_string()).or_default();
        config.status = status;
    }

    /// 检查 bucket 是否启用了版本控制
    pub async fn is_versioning_enabled(&self, bucket: &str) -> bool {
        let config = self.get_versioning(bucket).await;
        config.status == VersioningStatus::Enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_versioning_status_default() {
        let status = VersioningStatus::default();
        assert_eq!(status, VersioningStatus::Disabled);
    }

    #[test]
    fn test_versioning_status_to_string() {
        assert_eq!(VersioningStatus::Disabled.to_string(), "");
        assert_eq!(VersioningStatus::Enabled.to_string(), "Enabled");
        assert_eq!(VersioningStatus::Suspended.to_string(), "Suspended");
    }

    #[test]
    fn test_versioning_status_parse() {
        assert_eq!(
            VersioningStatus::parse("Enabled"),
            VersioningStatus::Enabled
        );
        assert_eq!(
            VersioningStatus::parse("Suspended"),
            VersioningStatus::Suspended
        );
        assert_eq!(VersioningStatus::parse(""), VersioningStatus::Disabled);
        assert_eq!(
            VersioningStatus::parse("Unknown"),
            VersioningStatus::Disabled
        );
    }

    #[test]
    fn test_bucket_versioning_default() {
        let config = BucketVersioning::default();
        assert_eq!(config.status, VersioningStatus::Disabled);
        assert!(config.mfa_delete.is_none());
    }

    #[tokio::test]
    async fn test_versioning_manager_default() {
        let manager = VersioningManager::new();
        let config = manager.get_versioning("test-bucket").await;
        assert_eq!(config.status, VersioningStatus::Disabled);
    }

    #[tokio::test]
    async fn test_versioning_manager_set_and_get() {
        let manager = VersioningManager::new();

        // 设置为启用
        manager
            .set_versioning("test-bucket", VersioningStatus::Enabled)
            .await;

        let config = manager.get_versioning("test-bucket").await;
        assert_eq!(config.status, VersioningStatus::Enabled);

        // 设置为暂停
        manager
            .set_versioning("test-bucket", VersioningStatus::Suspended)
            .await;

        let config = manager.get_versioning("test-bucket").await;
        assert_eq!(config.status, VersioningStatus::Suspended);
    }

    #[tokio::test]
    async fn test_versioning_manager_is_enabled() {
        let manager = VersioningManager::new();

        // 默认未启用
        assert!(!manager.is_versioning_enabled("test-bucket").await);

        // 启用版本控制
        manager
            .set_versioning("test-bucket", VersioningStatus::Enabled)
            .await;
        assert!(manager.is_versioning_enabled("test-bucket").await);

        // 暂停版本控制
        manager
            .set_versioning("test-bucket", VersioningStatus::Suspended)
            .await;
        assert!(!manager.is_versioning_enabled("test-bucket").await);
    }

    #[tokio::test]
    async fn test_versioning_manager_multiple_buckets() {
        let manager = VersioningManager::new();

        manager
            .set_versioning("bucket1", VersioningStatus::Enabled)
            .await;
        manager
            .set_versioning("bucket2", VersioningStatus::Suspended)
            .await;

        assert_eq!(
            manager.get_versioning("bucket1").await.status,
            VersioningStatus::Enabled
        );
        assert_eq!(
            manager.get_versioning("bucket2").await.status,
            VersioningStatus::Suspended
        );
        assert_eq!(
            manager.get_versioning("bucket3").await.status,
            VersioningStatus::Disabled
        );
    }
}
