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

    #[test]
    fn test_version_config_default() {
        let config = VersionConfig::default();
        assert_eq!(config.max_versions, 10);
        assert_eq!(config.retention_days, 30);
        assert!(config.enabled);
    }

    #[test]
    fn test_version_path() {
        let storage = Arc::new(StorageManager::new(
            "./test_storage".into(),
            4 * 1024 * 1024,
        ));
        let config = VersionConfig::default();
        let manager = VersionManager::new(storage, config, "./test_storage");

        let version_id = "test-version-123";
        let path = manager.get_version_path(version_id);

        assert!(path.to_str().unwrap().contains("versions"));
        assert!(path.to_str().unwrap().contains(version_id));
    }
}
