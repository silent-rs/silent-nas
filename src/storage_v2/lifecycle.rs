//! 数据生命周期管理模块
//!
//! 实现TTL、版本保留和自动清理功能

use crate::error::{NasError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;
use tracing::{info, warn};

/// 生命周期策略
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LifecyclePolicy {
    /// 永久保存
    Permanent,
    /// 基于TTL的自动过期
    Ttl {
        /// 存活时间（秒）
        ttl_seconds: u64,
    },
    /// 基于访问的自动过期
    LastAccess {
        /// 最后访问后多少天过期
        days_after_last_access: u32,
    },
    /// 基于修改的自动过期
    LastModified {
        /// 最后修改后多少天过期
        days_after_modification: u32,
    },
    /// 版本保留策略
    VersionRetention {
        /// 最大版本数
        max_versions: u32,
        /// 版本保留天数
        retain_days: u64,
    },
}

impl Default for LifecyclePolicy {
    fn default() -> Self {
        LifecyclePolicy::Permanent
    }
}

/// 生命周期配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleConfig {
    /// 默认策略
    pub default_policy: LifecyclePolicy,
    /// 检查间隔（秒）
    pub check_interval_secs: u64,
    /// 清理批大小
    pub cleanup_batch_size: usize,
    /// 启用自动清理
    pub enable_auto_cleanup: bool,
    /// 清理前通知
    pub notify_before_cleanup: bool,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            default_policy: LifecyclePolicy::Permanent,
            check_interval_secs: 3600, // 1小时
            cleanup_batch_size: 100,
            enable_auto_cleanup: true,
            notify_before_cleanup: false,
        }
    }
}

/// 生命周期状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleState {
    /// 活跃
    Active,
    /// 即将过期
    ExpiringSoon,
    /// 已过期
    Expired,
    /// 已清理
    Cleaned,
}

/// 生命周期条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleEntry {
    /// 文件ID
    pub file_id: String,
    /// 策略
    pub policy: LifecyclePolicy,
    /// 创建时间
    pub created_at: chrono::NaiveDateTime,
    /// 最后修改时间
    pub last_modified: chrono::NaiveDateTime,
    /// 最后访问时间
    pub last_accessed: chrono::NaiveDateTime,
    /// 当前状态
    pub state: LifecycleState,
    /// 版本信息
    pub version_id: Option<String>,
    /// 存储路径
    pub storage_path: PathBuf,
    /// 清理计划时间
    pub scheduled_cleanup_at: Option<chrono::NaiveDateTime>,
}

/// 生命周期管理器
pub struct LifecycleManager {
    config: LifecycleConfig,
    /// 生命周期条目
    entries: HashMap<String, LifecycleEntry>,
    /// 统计信息
    stats: LifecycleStats,
}

impl LifecycleManager {
    pub fn new(config: LifecycleConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
            stats: LifecycleStats::new(),
        }
    }

    /// 初始化生命周期管理器
    pub fn init(&mut self) -> Result<()> {
        info!("生命周期管理器初始化完成");
        Ok(())
    }

    /// 添加生命周期条目
    pub fn add_entry(&mut self, entry: LifecycleEntry) -> Result<()> {
        let file_id = entry.file_id.clone();
        self.entries.insert(file_id, entry);
        info!("添加生命周期条目: {}", file_id);
        Ok(())
    }

    /// 获取生命周期条目
    pub fn get_entry(&self, file_id: &str) -> Option<&LifecycleEntry> {
        self.entries.get(file_id)
    }

    /// 更新访问时间
    pub fn update_access_time(&mut self, file_id: &str) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(file_id) {
            entry.last_accessed = chrono::Local::now().naive_local();
            // 重新计算清理时间
            entry.scheduled_cleanup_at = self.calculate_cleanup_time(entry);
            info!("更新访问时间: {}", file_id);
        }
        Ok(())
    }

    /// 更新修改时间
    pub fn update_modification_time(&mut self, file_id: &str) -> Result<()> {
        if let Some(entry) = self.entries.get_mut(file_id) {
            entry.last_modified = chrono::Local::now().naive_local();
            // 重新计算清理时间
            entry.scheduled_cleanup_at = self.calculate_cleanup_time(entry);
            info!("更新修改时间: {}", file_id);
        }
        Ok(())
    }

    /// 执行生命周期检查
    pub fn check_lifecycle(&mut self) -> Result<LifecycleCheckResult> {
        let mut result = LifecycleCheckResult::default();
        let now = chrono::Local::now().naive_local();

        // 扫描所有条目
        for (file_id, entry) in self.entries.iter_mut() {
            let new_state = self.calculate_state(entry, now);

            if new_state != entry.state {
                result.state_changes.push(StateChange {
                    file_id: file_id.clone(),
                    old_state: entry.state.clone(),
                    new_state: new_state.clone(),
                });
                entry.state = new_state;
            }

            // 收集过期条目
            if entry.state == LifecycleState::Expired {
                result.expired_files.push(file_id.clone());
            }
        }

        info!("生命周期检查完成: {} 项状态变更, {} 个过期文件",
              result.state_changes.len(), result.expired_files.len());

        // 更新统计
        self.update_stats();

        Ok(result)
    }

    /// 执行自动清理
    pub async fn perform_cleanup(&mut self) -> Result<CleanupResult> {
        if !self.config.enable_auto_cleanup {
            return Ok(CleanupResult::default());
        }

        let mut result = CleanupResult::default();
        let now = chrono::Local::now().naive_local();

        // 收集所有已过期的文件
        let mut to_cleanup: Vec<String> = Vec::new();
        for (file_id, entry) in &self.entries {
            if entry.state == LifecycleState::Expired {
                to_cleanup.push(file_id.clone());
            }
        }

        // 限制清理批大小
        let batch = to_cleanup.into_iter()
            .take(self.config.cleanup_batch_size)
            .collect::<Vec<_>>();

        // 执行清理
        for file_id in batch {
            let cleanup_result = self.cleanup_file(&file_id).await?;
            result.total_files += 1;
            result.total_size += cleanup_result.size;

            if cleanup_result.success {
                result.success_count += 1;
                if let Some(entry) = self.entries.get_mut(&file_id) {
                    entry.state = LifecycleState::Cleaned;
                }
            } else {
                result.failed_count += 1;
            }
        }

        info!("自动清理完成: {} 成功, {} 失败, 总大小 {} 字节",
              result.success_count, result.failed_count, result.total_size);

        // 更新统计
        self.update_stats();

        Ok(result)
    }

    /// 清理单个文件
    async fn cleanup_file(&self, file_id: &str) -> Result<CleanupItemResult> {
        if let Some(entry) = self.entries.get(file_id) {
            // 检查是否需要通知
            if self.config.notify_before_cleanup {
                warn!("准备清理文件: {} (路径: {:?})", file_id, entry.storage_path);
            }

            // 实际删除文件
            match fs::remove_file(&entry.storage_path).await {
                Ok(_) => {
                    info!("已清理文件: {}", file_id);
                    Ok(CleanupItemResult {
                        file_id: file_id.to_string(),
                        success: true,
                        size: 0, // 需要从文件系统获取
                    })
                }
                Err(e) => {
                    warn!("清理文件失败: {}, 错误: {}", file_id, e);
                    Ok(CleanupItemResult {
                        file_id: file_id.to_string(),
                        success: false,
                        size: 0,
                    })
                }
            }
        } else {
            Ok(CleanupItemResult {
                file_id: file_id.to_string(),
                success: false,
                size: 0,
            })
        }
    }

    /// 计算清理时间
    fn calculate_cleanup_time(&self, entry: &LifecycleEntry) -> Option<chrono::NaiveDateTime> {
        let now = chrono::Local::now().naive_local();

        match &entry.policy {
            LifecyclePolicy::Permanent => None,
            LifecyclePolicy::Ttl { ttl_seconds } => {
                Some(entry.created_at + chrono::Duration::seconds(*ttl_seconds as i64))
            }
            LifecyclePolicy::LastAccess { days_after_last_access } => {
                Some(entry.last_accessed + chrono::Duration::days(*days_after_last_access as i64))
            }
            LifecyclePolicy::LastModified { days_after_modification } => {
                Some(entry.last_modified + chrono::Duration::days(*days_after_modification as i64))
            }
            LifecyclePolicy::VersionRetention { .. } => {
                // 版本保留策略由版本管理器处理
                None
            }
        }
    }

    /// 计算生命周期状态
    fn calculate_state(&self, entry: &LifecycleEntry, now: chrono::NaiveDateTime) -> LifecycleState {
        let cleanup_time = self.calculate_cleanup_time(entry);

        match cleanup_time {
            None => LifecycleState::Active, // 永久保存
            Some(cleanup_at) => {
                if now < cleanup_at {
                    // 检查是否即将过期（24小时内）
                    let time_to_expire = cleanup_at - now;
                    if time_to_expire < chrono::Duration::hours(24) {
                        LifecycleState::ExpiringSoon
                    } else {
                        LifecycleState::Active
                    }
                } else {
                    LifecycleState::Expired
                }
            }
        }
    }

    /// 获取统计信息
    pub fn get_stats(&self) -> &LifecycleStats {
        &self.stats
    }

    /// 更新统计信息
    fn update_stats(&mut self) {
        self.stats.total_files = self.entries.len() as u64;
        self.stats.active_files = self.entries.values()
            .filter(|e| e.state == LifecycleState::Active)
            .count() as u64;
        self.stats.expiring_soon_files = self.entries.values()
            .filter(|e| e.state == LifecycleState::ExpiringSoon)
            .count() as u64;
        self.stats.expired_files = self.entries.values()
            .filter(|e| e.state == LifecycleState::Expired)
            .count() as u64;
        self.stats.cleaned_files = self.entries.values()
            .filter(|e| e.state == LifecycleState::Cleaned)
            .count() as u64;

        // 统计各策略的文件数
        self.stats.policy_stats.clear();
        for entry in self.entries.values() {
            let policy_name = match &entry.policy {
                LifecyclePolicy::Permanent => "permanent",
                LifecyclePolicy::Ttl { .. } => "ttl",
                LifecyclePolicy::LastAccess { .. } => "last_access",
                LifecyclePolicy::LastModified { .. } => "last_modified",
                LifecyclePolicy::VersionRetention { .. } => "version_retention",
            };
            *self.stats.policy_stats.entry(policy_name.to_string()).or_insert(0) += 1;
        }
    }

    /// 列出所有条目
    pub fn list_entries(&self) -> Vec<&LifecycleEntry> {
        self.entries.values().collect()
    }

    /// 删除条目
    pub fn remove_entry(&mut self, file_id: &str) -> Result<()> {
        self.entries.remove(file_id);
        info!("删除生命周期条目: {}", file_id);
        Ok(())
    }
}

/// 生命周期检查结果
#[derive(Debug, Default)]
pub struct LifecycleCheckResult {
    pub state_changes: Vec<StateChange>,
    pub expired_files: Vec<String>,
}

/// 状态变更
#[derive(Debug, Clone)]
pub struct StateChange {
    pub file_id: String,
    pub old_state: LifecycleState,
    pub new_state: LifecycleState,
}

/// 清理结果
#[derive(Debug, Default)]
pub struct CleanupResult {
    pub total_files: u32,
    pub success_count: u32,
    pub failed_count: u32,
    pub total_size: u64,
}

/// 清理项结果
#[derive(Debug, Clone)]
pub struct CleanupItemResult {
    pub file_id: String,
    pub success: bool,
    pub size: u64,
}

/// 生命周期统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleStats {
    pub total_files: u64,
    pub active_files: u64,
    pub expiring_soon_files: u64,
    pub expired_files: u64,
    pub cleaned_files: u64,
    pub policy_stats: HashMap<String, u64>,
}

impl LifecycleStats {
    pub fn new() -> Self {
        Self {
            total_files: 0,
            active_files: 0,
            expiring_soon_files: 0,
            expired_files: 0,
            cleaned_files: 0,
            policy_stats: HashMap::new(),
        }
    }

    /// 获取过期率
    pub fn get_expired_rate(&self) -> f32 {
        if self.total_files > 0 {
            self.expired_files as f32 / self.total_files as f32
        } else {
            0.0
        }
    }

    /// 获取即将过期率
    pub fn get_expiring_soon_rate(&self) -> f32 {
        if self.total_files > 0 {
            self.expiring_soon_files as f32 / self.total_files as f32
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_lifecycle_policy_default() {
        let policy = LifecyclePolicy::default();
        assert!(matches!(policy, LifecyclePolicy::Permanent));
    }

    #[test]
    fn test_lifecycle_state_equality() {
        assert_eq!(LifecycleState::Active, LifecycleState::Active);
        assert_ne!(LifecycleState::Active, LifecycleState::Expired);
    }

    #[tokio::test]
    async fn test_lifecycle_manager_add_entry() {
        let mut manager = LifecycleManager::new(LifecycleConfig::default());
        manager.init().unwrap();

        let entry = LifecycleEntry {
            file_id: "test_file".to_string(),
            policy: LifecyclePolicy::Permanent,
            created_at: chrono::Local::now().naive_local(),
            last_modified: chrono::Local::now().naive_local(),
            last_accessed: chrono::Local::now().naive_local(),
            state: LifecycleState::Active,
            version_id: None,
            storage_path: PathBuf::new(),
            scheduled_cleanup_at: None,
        };

        manager.add_entry(entry).unwrap();
        assert_eq!(manager.entries.len(), 1);
    }

    #[tokio::test]
    async fn test_lifecycle_manager_check_lifecycle() {
        let mut manager = LifecycleManager::new(LifecycleConfig::default());
        manager.init().unwrap();

        // 添加TTL策略条目
        let entry = LifecycleEntry {
            file_id: "test_ttl".to_string(),
            policy: LifecyclePolicy::Ttl { ttl_seconds: 1 },
            created_at: chrono::Local::now().naive_local() - chrono::Duration::seconds(2),
            last_modified: chrono::Local::now().naive_local(),
            last_accessed: chrono::Local::now().naive_local(),
            state: LifecycleState::Active,
            version_id: None,
            storage_path: PathBuf::new(),
            scheduled_cleanup_at: None,
        };

        manager.add_entry(entry).unwrap();
        let result = manager.check_lifecycle().unwrap();

        assert_eq!(result.expired_files.len(), 1);
        assert_eq!(result.expired_files[0], "test_ttl");
    }

    #[tokio::test]
    async fn test_lifecycle_manager_update_access_time() {
        let mut manager = LifecycleManager::new(LifecycleConfig::default());
        manager.init().unwrap();

        let entry = LifecycleEntry {
            file_id: "test_file".to_string(),
            policy: LifecyclePolicy::Permanent,
            created_at: chrono::Local::now().naive_local(),
            last_modified: chrono::Local::now().naive_local(),
            last_accessed: chrono::Local::now().naive_local(),
            state: LifecycleState::Active,
            version_id: None,
            storage_path: PathBuf::new(),
            scheduled_cleanup_at: None,
        };

        manager.add_entry(entry).unwrap();
        manager.update_access_time("test_file").unwrap();

        let updated = manager.get_entry("test_file").unwrap();
        assert!(updated.last_accessed > chrono::Local::now().naive_local() - chrono::Duration::seconds(1));
    }

    #[tokio::test]
    async fn test_cleanup_file() {
        let mut manager = LifecycleManager::new(LifecycleConfig::default());
        manager.init().unwrap();

        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test_file.txt");

        // 创建测试文件
        fs::write(&file_path, "test data").await.unwrap();

        let entry = LifecycleEntry {
            file_id: "test_file".to_string(),
            policy: LifecyclePolicy::Permanent,
            created_at: chrono::Local::now().naive_local(),
            last_modified: chrono::Local::now().naive_local(),
            last_accessed: chrono::Local::now().naive_local(),
            state: LifecycleState::Expired,
            version_id: None,
            storage_path: file_path,
            scheduled_cleanup_at: Some(chrono::Local::now().naive_local()),
        };

        manager.add_entry(entry).unwrap();
        let result = manager.cleanup_file("test_file").await.unwrap();

        assert!(result.success);
    }
}
