//! 增量索引更新管理器
//!
//! 提供增量索引更新功能，包括：
//! - 监控文件变化
//! - 自动更新索引
//! - 批量更新支持
//! - 更新统计与性能指标

use crate::error::Result;
use crate::models::FileMetadata;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// 增量索引配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncrementalIndexerConfig {
    /// 批量更新大小
    pub batch_size: usize,
    /// 更新检查间隔（毫秒）
    pub check_interval_ms: u64,
    /// 最大缓存文件数
    pub max_cached_files: usize,
    /// 启用自动更新
    pub enable_auto_update: bool,
    /// 更新缓冲时间（秒）
    pub update_buffer_secs: u64,
}

/// 增量索引配置默认值
impl Default for IncrementalIndexerConfig {
    fn default() -> Self {
        Self {
            batch_size: 100,
            check_interval_ms: 5000, // 5秒
            max_cached_files: 10000,
            enable_auto_update: true,
            update_buffer_secs: 60, // 1分钟
        }
    }
}

/// 文件变化类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeType {
    /// 文件添加
    Added,
    /// 文件修改
    Modified,
    /// 文件删除
    Deleted,
}

/// 文件变化事件
#[derive(Debug, Clone)]
pub struct FileChangeEvent {
    /// 文件路径
    pub path: PathBuf,
    /// 变化类型
    pub change_type: FileChangeType,
    /// 文件元数据
    pub metadata: Option<FileMetadata>,
    /// 检测时间
    #[allow(dead_code)]
    pub detected_at: SystemTime,
}

/// 更新统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStats {
    /// 总更新次数
    pub total_updates: u64,
    /// 成功更新次数
    pub successful_updates: u64,
    /// 失败更新次数
    pub failed_updates: u64,
    /// 最后更新时间
    pub last_update: Option<SystemTime>,
    /// 平均更新耗时（毫秒）
    pub avg_update_time_ms: f64,
    /// 缓存命中率
    pub cache_hit_rate: f64,
}

/// 增量索引管理器
pub struct IncrementalIndexer {
    /// 配置
    config: IncrementalIndexerConfig,
    /// 文件路径到文件元数据的映射（缓存）
    file_cache: Arc<RwLock<HashMap<PathBuf, FileMetadata>>>,
    /// 待更新的文件队列
    #[allow(dead_code)]
    pending_updates: Arc<RwLock<HashSet<PathBuf>>>,
    /// 更新统计
    stats: Arc<RwLock<UpdateStats>>,
}

impl IncrementalIndexer {
    /// 创建新的增量索引管理器
    pub fn new(config: IncrementalIndexerConfig) -> Self {
        Self {
            config,
            file_cache: Arc::new(RwLock::new(HashMap::new())),
            pending_updates: Arc::new(RwLock::new(HashSet::new())),
            stats: Arc::new(RwLock::new(UpdateStats {
                total_updates: 0,
                successful_updates: 0,
                failed_updates: 0,
                last_update: None,
                avg_update_time_ms: 0.0,
                cache_hit_rate: 0.0,
            })),
        }
    }

    /// 初始化增量索引管理器
    #[allow(dead_code)]
    pub async fn init(&self) -> Result<()> {
        info!("增量索引管理器已初始化");
        Ok(())
    }

    /// 扫描目录变化
    pub async fn scan_changes(&self, root_path: &Path) -> Result<Vec<FileChangeEvent>> {
        debug!("开始扫描目录变化: {:?}", root_path);
        let mut changes = Vec::new();

        // 读取当前目录状态
        let current_files = self.read_directory_recursive(root_path).await?;

        // 与缓存比较
        let cached_files = self.file_cache.read().await;

        // 收集当前文件的路径，用于后续检查删除文件
        let current_file_paths: HashSet<PathBuf> = current_files.keys().cloned().collect();

        // 检查新增和修改的文件
        for (path, metadata) in current_files {
            if let Some(cached_meta) = cached_files.get(&path) {
                // 检查文件是否被修改
                if metadata.modified_at > cached_meta.modified_at {
                    changes.push(FileChangeEvent {
                        path: path.clone(),
                        change_type: FileChangeType::Modified,
                        metadata: Some(metadata),
                        detected_at: SystemTime::now(),
                    });
                }
            } else {
                // 新文件
                changes.push(FileChangeEvent {
                    path: path.clone(),
                    change_type: FileChangeType::Added,
                    metadata: Some(metadata),
                    detected_at: SystemTime::now(),
                });
            }
        }

        // 检查删除的文件
        for (path, _) in cached_files.iter() {
            if !current_file_paths.contains(path) {
                changes.push(FileChangeEvent {
                    path: path.clone(),
                    change_type: FileChangeType::Deleted,
                    metadata: None,
                    detected_at: SystemTime::now(),
                });
            }
        }

        debug!("扫描完成，发现 {} 个变化", changes.len());
        Ok(changes)
    }

    /// 递归读取目录
    async fn read_directory_recursive(
        &self,
        root_path: &Path,
    ) -> Result<HashMap<PathBuf, FileMetadata>> {
        let mut files = HashMap::new();
        let mut stack = vec![root_path.to_path_buf()];

        while let Some(current_path) = stack.pop() {
            if let Ok(entries) = fs::read_dir(&current_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.is_file() {
                        // 读取文件元数据
                        if let Ok(metadata) = entry.metadata() {
                            let created_secs = metadata
                                .created()
                                .unwrap_or(SystemTime::now())
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();

                            let modified_secs = metadata
                                .modified()
                                .unwrap_or(SystemTime::now())
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();

                            let file_meta = FileMetadata {
                                id: scru128::new().to_string(),
                                name: path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("")
                                    .to_string(),
                                path: path.to_string_lossy().to_string(),
                                size: metadata.len(),
                                hash: "".to_string(), // 将在需要时计算
                                created_at: chrono::DateTime::from_timestamp(
                                    created_secs as i64,
                                    0,
                                )
                                .unwrap_or_default()
                                .naive_local(),
                                modified_at: chrono::DateTime::from_timestamp(
                                    modified_secs as i64,
                                    0,
                                )
                                .unwrap_or_default()
                                .naive_local(),
                            };
                            files.insert(path, file_meta);
                        }
                    }
                }
            }
        }

        Ok(files)
    }

    /// 提交文件变化
    pub async fn commit_changes(&self, changes: Vec<FileChangeEvent>) -> Result<usize> {
        let mut success_count = 0;
        let changes_len = changes.len();

        for change in changes.into_iter() {
            match change.change_type {
                FileChangeType::Added | FileChangeType::Modified => {
                    if let Some(metadata) = change.metadata {
                        self.update_file_cache(&change.path, &metadata).await;
                        success_count += 1;
                    } else {
                        warn!("文件变化事件缺少元数据: {:?}", change.path);
                    }
                }
                FileChangeType::Deleted => {
                    self.remove_file_cache(&change.path).await;
                    success_count += 1;
                }
            }
        }

        // 更新统计
        {
            let mut stats = self.stats.write().await;
            stats.total_updates += changes_len as u64;
            stats.successful_updates += success_count as u64;
            stats.last_update = Some(SystemTime::now());
        }

        info!("提交了 {} 个文件变化", success_count);
        Ok(success_count)
    }

    /// 更新文件缓存
    async fn update_file_cache(&self, path: &Path, metadata: &FileMetadata) {
        let mut cache = self.file_cache.write().await;
        cache.insert(path.to_path_buf(), metadata.clone());

        // 限制缓存大小
        if cache.len() > self.config.max_cached_files {
            let oldest_key = cache.keys().next().cloned();
            if let Some(key) = oldest_key {
                cache.remove(&key);
            }
        }
    }

    /// 从缓存中移除文件
    async fn remove_file_cache(&self, path: &Path) {
        let mut cache = self.file_cache.write().await;
        cache.remove(path);
    }

    /// 添加到待更新队列
    #[allow(dead_code)]
    pub async fn queue_update(&self, path: &Path) -> Result<()> {
        let mut pending = self.pending_updates.write().await;
        pending.insert(path.to_path_buf());
        debug!("文件已加入更新队列: {:?}", path);
        Ok(())
    }

    /// 获取待更新的文件列表
    #[allow(dead_code)]
    pub async fn get_pending_updates(&self) -> Vec<PathBuf> {
        let pending = self.pending_updates.read().await;
        pending.iter().cloned().collect()
    }

    /// 清空待更新队列
    #[allow(dead_code)]
    pub async fn clear_pending_updates(&self) -> usize {
        let mut pending = self.pending_updates.write().await;
        let count = pending.len();
        pending.clear();
        count
    }

    /// 获取更新统计
    pub async fn get_stats(&self) -> UpdateStats {
        let stats = self.stats.read().await;
        stats.clone()
    }

    /// 重置统计
    #[allow(dead_code)]
    pub async fn reset_stats(&self) {
        let mut stats = self.stats.write().await;
        *stats = UpdateStats {
            total_updates: 0,
            successful_updates: 0,
            failed_updates: 0,
            last_update: None,
            avg_update_time_ms: 0.0,
            cache_hit_rate: 0.0,
        };
        info!("增量索引统计已重置");
    }

    /// 检查是否需要更新
    #[allow(dead_code)]
    pub fn should_update(&self, last_update: &SystemTime) -> bool {
        let now = SystemTime::now();
        let elapsed = now.duration_since(*last_update).unwrap_or_default();
        elapsed.as_secs() >= self.config.update_buffer_secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::time::Duration;
    use tempfile::TempDir;

    fn create_test_metadata(name: &str, path: &str) -> FileMetadata {
        FileMetadata {
            id: scru128::new().to_string(),
            name: name.to_string(),
            path: path.to_string(),
            size: 1024,
            hash: "test_hash".to_string(),
            created_at: Utc::now().naive_local(),
            modified_at: Utc::now().naive_local(),
        }
    }

    #[tokio::test]
    async fn test_incremental_indexer_creation() {
        let config = IncrementalIndexerConfig::default();
        let indexer = IncrementalIndexer::new(config);

        indexer.init().await.unwrap();
    }

    #[tokio::test]
    async fn test_update_file_cache() {
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalIndexerConfig::default();
        let indexer = IncrementalIndexer::new(config);

        let file_path = temp_dir.path().join("test.txt");
        let metadata = create_test_metadata("test.txt", file_path.to_str().unwrap());

        indexer.update_file_cache(&file_path, &metadata).await;

        let cache = indexer.file_cache.read().await;
        assert!(cache.contains_key(&file_path));
    }

    #[tokio::test]
    async fn test_remove_file_cache() {
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalIndexerConfig::default();
        let indexer = IncrementalIndexer::new(config);

        let file_path = temp_dir.path().join("test.txt");
        let metadata = create_test_metadata("test.txt", file_path.to_str().unwrap());

        indexer.update_file_cache(&file_path, &metadata).await;
        indexer.remove_file_cache(&file_path).await;

        let cache = indexer.file_cache.read().await;
        assert!(!cache.contains_key(&file_path));
    }

    #[tokio::test]
    async fn test_queue_update() {
        let config = IncrementalIndexerConfig::default();
        let indexer = IncrementalIndexer::new(config);

        let file_path = Path::new("test.txt");
        indexer.queue_update(file_path).await.unwrap();

        let pending = indexer.get_pending_updates().await;
        assert_eq!(pending.len(), 1);
    }

    #[tokio::test]
    async fn test_clear_pending_updates() {
        let config = IncrementalIndexerConfig::default();
        let indexer = IncrementalIndexer::new(config);

        let file_path = Path::new("test.txt");
        indexer.queue_update(file_path).await.unwrap();

        let count = indexer.clear_pending_updates().await;
        assert_eq!(count, 1);

        let pending = indexer.get_pending_updates().await;
        assert_eq!(pending.len(), 0);
    }

    #[tokio::test]
    async fn test_commit_changes() {
        let config = IncrementalIndexerConfig::default();
        let indexer = IncrementalIndexer::new(config);

        let file_path = Path::new("test.txt");
        let metadata = create_test_metadata("test.txt", "test.txt");

        let changes = vec![FileChangeEvent {
            path: file_path.to_path_buf(),
            change_type: FileChangeType::Added,
            metadata: Some(metadata),
            detected_at: SystemTime::now(),
        }];

        let count = indexer.commit_changes(changes).await.unwrap();
        assert_eq!(count, 1);

        let cache = indexer.file_cache.read().await;
        assert!(cache.contains_key(file_path));
    }

    #[tokio::test]
    async fn test_should_update() {
        let config = IncrementalIndexerConfig::default();
        let indexer = IncrementalIndexer::new(config);

        // 刚刚更新过
        let recent_time = SystemTime::now();
        assert!(!indexer.should_update(&recent_time));

        // 超过缓冲时间
        let old_time = SystemTime::now() - Duration::from_secs(120);
        assert!(indexer.should_update(&old_time));
    }
}
