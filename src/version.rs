// 文件版本管理模块
use crate::error::{NasError, Result};
use crate::models::FileVersion;
use crate::storage::StorageManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// 版本管理配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionConfig {
    /// 最大版本数量（0 表示无限制）
    pub max_versions: usize,
    /// 版本保留天数（0 表示永久保留）
    pub retention_days: u64,
    /// 是否启用版本管理
    pub enabled: bool,
}

impl Default for VersionConfig {
    fn default() -> Self {
        Self {
            max_versions: 10,
            retention_days: 30,
            enabled: true,
        }
    }
}

/// 文件版本管理器
pub struct VersionManager {
    /// 存储管理器
    storage: Arc<StorageManager>,
    /// 版本配置
    config: VersionConfig,
    /// 版本存储根目录
    version_root: PathBuf,
    /// 版本索引缓存 (file_id -> versions)
    version_index: Arc<RwLock<HashMap<String, Vec<FileVersion>>>>,
}

impl VersionManager {
    pub fn new(storage: Arc<StorageManager>, config: VersionConfig, root_path: &str) -> Arc<Self> {
        let version_root = Path::new(root_path).join("versions");

        Arc::new(Self {
            storage,
            config,
            version_root,
            version_index: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// 初始化版本管理器
    pub async fn init(&self) -> Result<()> {
        if !self.config.enabled {
            info!("版本管理未启用");
            return Ok(());
        }

        // 创建版本存储目录
        fs::create_dir_all(&self.version_root)
            .await
            .map_err(NasError::Io)?;

        info!("版本管理器已初始化: {:?}", self.version_root);
        Ok(())
    }

    /// 创建文件版本
    pub async fn create_version(&self, file_id: &str, version: FileVersion) -> Result<FileVersion> {
        if !self.config.enabled {
            return Err(NasError::Other("版本管理未启用".to_string()));
        }

        // 读取当前文件内容
        let file_data = self.storage.read_file(file_id).await?;

        // 保存版本文件
        let version_path = self.get_version_path(&version.version_id);
        fs::write(&version_path, &file_data)
            .await
            .map_err(NasError::Io)?;

        // 更新版本索引
        let mut index = self.version_index.write().await;
        let versions = index.entry(file_id.to_string()).or_insert_with(Vec::new);

        // 将旧版本标记为非当前版本
        for v in versions.iter_mut() {
            v.is_current = false;
        }

        versions.push(version.clone());

        // 应用版本数量限制
        if self.config.max_versions > 0 && versions.len() > self.config.max_versions {
            let to_remove = versions.len() - self.config.max_versions;
            for _ in 0..to_remove {
                if let Some(old_version) = versions.first()
                    && !old_version.is_current
                {
                    let old_id = old_version.version_id.clone();
                    versions.remove(0);
                    // 删除旧版本文件
                    let old_path = self.get_version_path(&old_id);
                    let _ = fs::remove_file(old_path).await;
                    debug!("删除旧版本: {}", old_id);
                }
            }
        }

        info!("创建文件版本: {} -> {}", file_id, version.version_id);
        Ok(version)
    }

    /// 获取文件的所有版本
    pub async fn list_versions(&self, file_id: &str) -> Result<Vec<FileVersion>> {
        let index = self.version_index.read().await;
        let versions = index.get(file_id).cloned().unwrap_or_default();

        // 按创建时间降序排序
        let mut sorted = versions;
        sorted.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(sorted)
    }

    /// 获取特定版本
    pub async fn get_version(&self, version_id: &str) -> Result<FileVersion> {
        let index = self.version_index.read().await;

        for versions in index.values() {
            if let Some(version) = versions.iter().find(|v| v.version_id == version_id) {
                return Ok(version.clone());
            }
        }

        Err(NasError::Other(format!("版本不存在: {}", version_id)))
    }

    /// 读取版本文件内容
    pub async fn read_version(&self, version_id: &str) -> Result<Vec<u8>> {
        let version_path = self.get_version_path(version_id);

        fs::read(&version_path).await.map_err(NasError::Io)
    }

    /// 恢复文件到指定版本
    pub async fn restore_version(&self, file_id: &str, version_id: &str) -> Result<FileVersion> {
        if !self.config.enabled {
            return Err(NasError::Other("版本管理未启用".to_string()));
        }

        // 获取版本信息
        let version = self.get_version(version_id).await?;

        if version.file_id != file_id {
            return Err(NasError::Other("版本与文件不匹配".to_string()));
        }

        // 读取版本内容
        let version_data = self.read_version(version_id).await?;

        // 先创建当前版本的备份
        if let Ok(current_metadata) = self.storage.get_metadata(file_id).await {
            let backup_version =
                FileVersion::from_metadata(&current_metadata, Some("system".to_string()));
            let _ = self.create_version(file_id, backup_version).await;
        }

        // 恢复版本内容到文件
        let _metadata = self.storage.save_file(file_id, &version_data).await?;

        // 更新版本索引，标记恢复的版本为当前版本
        let mut index = self.version_index.write().await;
        if let Some(versions) = index.get_mut(file_id) {
            for v in versions.iter_mut() {
                v.is_current = v.version_id == version_id;
            }
        }

        info!("恢复文件到版本: {} -> {}", file_id, version_id);
        Ok(version)
    }

    /// 删除指定版本
    pub async fn delete_version(&self, version_id: &str) -> Result<()> {
        let version = self.get_version(version_id).await?;

        if version.is_current {
            return Err(NasError::Other("无法删除当前版本".to_string()));
        }

        // 从索引中移除
        let mut index = self.version_index.write().await;
        if let Some(versions) = index.get_mut(&version.file_id) {
            versions.retain(|v| v.version_id != version_id);
        }

        // 删除版本文件
        let version_path = self.get_version_path(version_id);
        fs::remove_file(&version_path).await.map_err(NasError::Io)?;

        info!("删除版本: {}", version_id);
        Ok(())
    }

    /// 清理过期版本
    #[allow(dead_code)]
    pub async fn cleanup_expired_versions(&self) -> Result<usize> {
        if !self.config.enabled || self.config.retention_days == 0 {
            return Ok(0);
        }

        let now = chrono::Local::now().naive_local();
        let retention_duration = chrono::Duration::days(self.config.retention_days as i64);
        let cutoff_time = now - retention_duration;

        let mut deleted_count = 0;
        let index = self.version_index.read().await;

        for (_file_id, versions) in index.iter() {
            for version in versions {
                if !version.is_current
                    && version.created_at < cutoff_time
                    && self.delete_version(&version.version_id).await.is_ok()
                {
                    deleted_count += 1;
                }
            }
        }

        if deleted_count > 0 {
            info!("清理过期版本: {} 个", deleted_count);
        }

        Ok(deleted_count)
    }

    /// 获取版本文件路径
    fn get_version_path(&self, version_id: &str) -> PathBuf {
        self.version_root.join(version_id)
    }

    /// 获取版本统计信息
    pub async fn get_stats(&self) -> VersionStats {
        let index = self.version_index.read().await;

        let total_versions: usize = index.values().map(|v| v.len()).sum();
        let total_files = index.len();

        let total_size: u64 = index
            .values()
            .flat_map(|versions| versions.iter())
            .map(|v| v.size)
            .sum();

        VersionStats {
            total_files,
            total_versions,
            total_size,
        }
    }
}

/// 版本统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionStats {
    pub total_files: usize,
    pub total_versions: usize,
    pub total_size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::FileMetadata;

    fn create_test_storage() -> Arc<StorageManager> {
        Arc::new(StorageManager::new(
            "./test_storage".into(),
            4 * 1024 * 1024,
        ))
    }

    fn create_test_version_manager(config: VersionConfig) -> Arc<VersionManager> {
        let storage = create_test_storage();
        VersionManager::new(storage, config, "./test_storage_versions")
    }

    fn create_test_metadata(id: &str, name: &str, size: u64) -> FileMetadata {
        FileMetadata {
            id: id.to_string(),
            name: name.to_string(),
            path: format!("/test/{}", name),
            size,
            hash: format!("hash_{}", id),
            created_at: chrono::Local::now().naive_local(),
            modified_at: chrono::Local::now().naive_local(),
        }
    }

    #[test]
    fn test_version_config_default() {
        let config = VersionConfig::default();
        assert_eq!(config.max_versions, 10);
        assert_eq!(config.retention_days, 30);
        assert!(config.enabled);
    }

    #[test]
    fn test_version_config_custom() {
        let config = VersionConfig {
            max_versions: 5,
            retention_days: 7,
            enabled: false,
        };
        assert_eq!(config.max_versions, 5);
        assert_eq!(config.retention_days, 7);
        assert!(!config.enabled);
    }

    #[test]
    fn test_version_path() {
        let config = VersionConfig::default();
        let manager = create_test_version_manager(config);

        let version_id = "test-version-123";
        let path = manager.get_version_path(version_id);

        assert!(path.to_str().unwrap().contains("versions"));
        assert!(path.to_str().unwrap().contains(version_id));
    }

    #[test]
    fn test_file_version_creation() {
        let metadata = create_test_metadata("file123", "test.txt", 1024);
        let version = FileVersion::from_metadata(&metadata, Some("user1".to_string()));

        assert_eq!(version.file_id, "file123");
        assert_eq!(version.name, "test.txt");
        assert_eq!(version.size, 1024);
        assert_eq!(version.hash, "hash_file123");
        assert_eq!(version.author, Some("user1".to_string()));
        assert!(version.is_current);
        assert!(!version.version_id.is_empty());
    }

    #[test]
    fn test_file_version_new() {
        let version = FileVersion::new(
            "file123".to_string(),
            "test.txt".to_string(),
            2048,
            "hash_abc".to_string(),
            Some("admin".to_string()),
            Some("Initial version".to_string()),
        );

        assert_eq!(version.file_id, "file123");
        assert_eq!(version.name, "test.txt");
        assert_eq!(version.size, 2048);
        assert_eq!(version.hash, "hash_abc");
        assert_eq!(version.author, Some("admin".to_string()));
        assert_eq!(version.comment, Some("Initial version".to_string()));
        assert!(!version.is_current); // new() 默认不是当前版本
    }

    #[tokio::test]
    async fn test_version_manager_init() {
        let config = VersionConfig::default();
        let manager = create_test_version_manager(config);

        let result = manager.init().await;
        assert!(result.is_ok());

        // 清理测试目录
        let _ = tokio::fs::remove_dir_all("./test_storage_versions/versions").await;
    }

    #[tokio::test]
    async fn test_version_manager_init_disabled() {
        let config = VersionConfig {
            enabled: false,
            ..Default::default()
        };
        let manager = create_test_version_manager(config);

        let result = manager.init().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_versions_empty() {
        let config = VersionConfig::default();
        let manager = create_test_version_manager(config);

        let versions = manager.list_versions("file123").await;
        assert!(versions.is_ok());
        assert_eq!(versions.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_get_version_not_found() {
        let config = VersionConfig::default();
        let manager = create_test_version_manager(config);

        let result = manager.get_version("non_existent_version").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_version_stats_empty() {
        let config = VersionConfig::default();
        let manager = create_test_version_manager(config);

        let stats = manager.get_stats().await;
        assert_eq!(stats.total_files, 0);
        assert_eq!(stats.total_versions, 0);
        assert_eq!(stats.total_size, 0);
    }

    #[tokio::test]
    async fn test_restore_version_disabled() {
        let config = VersionConfig {
            enabled: false,
            ..Default::default()
        };
        let manager = create_test_version_manager(config);

        let result = manager.restore_version("file123", "version123").await;
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("版本管理未启用"));
        }
    }

    #[test]
    fn test_version_stats_display() {
        let stats = VersionStats {
            total_files: 10,
            total_versions: 50,
            total_size: 1024 * 1024 * 100, // 100MB
        };

        assert_eq!(stats.total_files, 10);
        assert_eq!(stats.total_versions, 50);
        assert_eq!(stats.total_size, 104_857_600);
    }

    #[test]
    fn test_version_config_serialization() {
        let config = VersionConfig {
            max_versions: 20,
            retention_days: 60,
            enabled: true,
        };

        // 测试序列化
        let json = serde_json::to_string(&config).unwrap();

        // 测试反序列化
        let config2: VersionConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config2.max_versions, 20);
        assert_eq!(config2.retention_days, 60);
        assert!(config2.enabled);
    }

    #[test]
    fn test_file_version_serialization() {
        let version = FileVersion::new(
            "file123".to_string(),
            "test.txt".to_string(),
            2048,
            "hash_abc".to_string(),
            Some("admin".to_string()),
            Some("Test version".to_string()),
        );

        // 测试序列化
        let json = serde_json::to_string(&version).unwrap();

        // 测试反序列化
        let version2: FileVersion = serde_json::from_str(&json).unwrap();
        assert_eq!(version2.file_id, "file123");
        assert_eq!(version2.name, "test.txt");
        assert_eq!(version2.size, 2048);
        assert_eq!(version2.hash, "hash_abc");
    }

    #[tokio::test]
    async fn test_create_version_disabled() {
        let config = VersionConfig {
            enabled: false,
            ..Default::default()
        };
        let manager = create_test_version_manager(config);
        manager.init().await.unwrap();

        let storage = manager.storage.clone();
        storage.init().await.unwrap();
        storage.save_file("test_file", b"test data").await.unwrap();

        let version = FileVersion::new(
            "test_file".to_string(),
            "test.txt".to_string(),
            9,
            "hash".to_string(),
            None,
            None,
        );

        let result = manager.create_version("test_file", version).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_version_config_clone() {
        let config = VersionConfig {
            max_versions: 5,
            retention_days: 7,
            enabled: true,
        };

        let cloned = config.clone();
        assert_eq!(cloned.max_versions, 5);
        assert_eq!(cloned.retention_days, 7);
        assert!(cloned.enabled);
    }

    #[tokio::test]
    async fn test_version_config_debug() {
        let config = VersionConfig::default();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("VersionConfig"));
    }

    #[tokio::test]
    async fn test_list_versions_sorting() {
        let config = VersionConfig::default();
        let manager = create_test_version_manager(config);

        // 版本列表应该按创建时间降序排序
        // 这里测试空列表情况
        let versions = manager.list_versions("test_file").await.unwrap();
        assert_eq!(versions.len(), 0);
    }

    #[test]
    fn test_version_path_generation() {
        let config = VersionConfig::default();
        let manager = create_test_version_manager(config);

        // 测试 get_version_path（通过其他方法间接测试）
        let version_id = "version_123";
        let path = manager.get_version_path(version_id);
        assert!(path.to_string_lossy().contains("version_123"));
        assert!(path.to_string_lossy().contains("versions"));
    }

    #[tokio::test]
    async fn test_version_with_special_characters() {
        let version = FileVersion::new(
            "文件123".to_string(),
            "测试文件.txt".to_string(),
            1024,
            "hash_中文".to_string(),
            Some("用户_🔥".to_string()),
            Some("版本说明 with emoji 🎉".to_string()),
        );

        assert_eq!(version.file_id, "文件123");
        assert_eq!(version.name, "测试文件.txt");
        assert!(version.comment.unwrap().contains("🎉"));
    }

    #[tokio::test]
    async fn test_version_large_size() {
        let large_size = 10_737_418_240u64; // 10GB
        let version = FileVersion::new(
            "large_file".to_string(),
            "large.bin".to_string(),
            large_size,
            "hash_large".to_string(),
            None,
            None,
        );

        assert_eq!(version.size, large_size);
    }

    #[test]
    fn test_version_config_disabled() {
        let config = VersionConfig {
            max_versions: 0,
            retention_days: 0,
            enabled: false,
        };

        assert!(!config.enabled);
        assert_eq!(config.max_versions, 0);
        assert_eq!(config.retention_days, 0);
    }

    #[test]
    fn test_version_config_unlimited() {
        let config = VersionConfig {
            max_versions: 0,   // 0 表示无限制
            retention_days: 0, // 0 表示永久保留
            enabled: true,
        };

        assert!(config.enabled);
        assert_eq!(config.max_versions, 0);
        assert_eq!(config.retention_days, 0);
    }

    #[test]
    fn test_version_stats_zero() {
        let stats = VersionStats {
            total_files: 0,
            total_versions: 0,
            total_size: 0,
        };

        assert_eq!(stats.total_files, 0);
        assert_eq!(stats.total_versions, 0);
        assert_eq!(stats.total_size, 0);
    }

    #[test]
    fn test_file_version_minimal() {
        let version = FileVersion::new(
            "file1".to_string(),
            "file1.txt".to_string(),
            100,
            "hash1".to_string(),
            Some("user".to_string()),
            None, // 没有注释
        );

        assert!(version.comment.is_none());
        assert_eq!(version.size, 100);
    }

    #[test]
    fn test_file_version_full() {
        let version = FileVersion::new(
            "file1".to_string(),
            "file1.txt".to_string(),
            100,
            "hash1".to_string(),
            Some("user".to_string()),
            Some("comment".to_string()),
        );

        assert!(version.comment.is_some());
        assert_eq!(version.comment.unwrap(), "comment");
    }
}
