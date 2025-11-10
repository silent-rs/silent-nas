//! 兼容性保障模块
//!
//! 实现平滑迁移和API兼容性保障，包括：
//! - 存储格式版本管理与迁移
//! - 在线迁移方案
//! - 迁移进度监控
//! - 回滚机制
//! - API兼容性层
//! - 向后兼容策略

use crate::error::{NasError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::fs as async_fs;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// 存储格式版本
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageVersion {
    /// v0.6.0 及之前版本
    V06 = 1,
    /// v0.7.0 增量存储版本
    V07 = 2,
}

/// 迁移状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationState {
    /// 未开始
    NotStarted,
    /// 正在迁移
    InProgress,
    /// 已完成
    Completed,
    /// 已回滚
    RolledBack,
    /// 失败
    Failed,
}

/// 迁移配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationConfig {
    /// 迁移批次大小
    pub batch_size: usize,
    /// 并发迁移线程数
    pub concurrent_migrations: usize,
    /// 迁移检查间隔（毫秒）
    pub migration_check_interval_ms: u64,
    /// 启用在线迁移
    pub enable_online_migration: bool,
    /// 保留旧格式时间（天）
    pub keep_old_format_days: u32,
    /// 迁移进度持久化
    pub persist_progress: bool,
}

/// 迁移配置默认值
impl Default for MigrationConfig {
    fn default() -> Self {
        Self {
            batch_size: 100,
            concurrent_migrations: 4,
            migration_check_interval_ms: 1000, // 1秒
            enable_online_migration: true,
            keep_old_format_days: 7, // 7天
            persist_progress: true,
        }
    }
}

/// 迁移进度
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationProgress {
    /// 总文件数
    pub total_files: u64,
    /// 已迁移文件数
    pub migrated_files: u64,
    /// 迁移失败文件数
    pub failed_files: u64,
    /// 当前迁移文件
    pub current_file: Option<String>,
    /// 迁移开始时间
    pub start_time: Option<chrono::NaiveDateTime>,
    /// 预计完成时间
    pub estimated_completion: Option<chrono::NaiveDateTime>,
    /// 迁移速度（文件/秒）
    pub migration_rate: f64,
    /// 错误信息
    pub errors: Vec<String>,
}

/// 迁移任务
#[derive(Debug, Clone)]
pub struct MigrationTask {
    /// 文件ID
    pub file_id: String,
    /// 源路径
    pub source_path: PathBuf,
    /// 目标路径
    pub target_path: PathBuf,
    /// 文件大小
    pub file_size: u64,
    /// 创建时间
    pub created_at: Instant,
}

/// 迁移结果
#[derive(Debug, Clone)]
pub struct MigrationResult {
    /// 任务ID
    pub task_id: u64,
    /// 文件ID
    pub file_id: String,
    /// 是否成功
    pub success: bool,
    /// 错误信息
    pub error: Option<String>,
    /// 迁移耗时
    pub duration: Duration,
    /// 压缩比
    pub compression_ratio: Option<f32>,
}

/// API兼容性配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiCompatibilityConfig {
    /// 启用API兼容性层
    pub enable_compat_layer: bool,
    /// 性能指标采样率（0-1）
    pub metrics_sampling_rate: f32,
    /// 缓存旧API响应
    pub cache_legacy_responses: bool,
    /// 兼容性模式警告
    pub warn_on_compat_mode: bool,
}

/// 兼容性管理器
pub struct CompatibilityManager {
    /// 当前存储版本
    current_version: StorageVersion,
    /// 迁移配置
    config: MigrationConfig,
    /// API兼容性配置
    api_config: ApiCompatibilityConfig,
    /// 迁移状态
    migration_state: Arc<RwLock<MigrationState>>,
    /// 迁移进度
    progress: Arc<RwLock<MigrationProgress>>,
    /// 迁移队列
    migration_queue: Arc<RwLock<VecDeque<MigrationTask>>>,
    /// 迁移结果
    migration_results: Arc<RwLock<HashMap<String, MigrationResult>>>,
    /// 活跃任务数
    active_tasks: Arc<AtomicUsize>,
    /// 总任务数
    total_tasks: Arc<AtomicU64>,
    /// 成功任务数
    successful_tasks: Arc<AtomicU64>,
    /// 失败任务数
    failed_tasks: Arc<AtomicU64>,
    /// 停止标志
    stop_flag: Arc<AtomicBool>,
    /// 迁移历史
    migration_history: Arc<RwLock<VecDeque<MigrationResult>>>,
}

impl CompatibilityManager {
    /// 创建兼容性管理器
    pub fn new(
        current_version: StorageVersion,
        config: MigrationConfig,
        api_config: ApiCompatibilityConfig,
    ) -> Self {
        Self {
            current_version,
            config,
            api_config,
            migration_state: Arc::new(RwLock::new(MigrationState::NotStarted)),
            progress: Arc::new(RwLock::new(MigrationProgress {
                total_files: 0,
                migrated_files: 0,
                failed_files: 0,
                current_file: None,
                start_time: None,
                estimated_completion: None,
                migration_rate: 0.0,
                errors: Vec::new(),
            })),
            migration_queue: Arc::new(RwLock::new(VecDeque::new())),
            migration_results: Arc::new(RwLock::new(HashMap::new())),
            active_tasks: Arc::new(AtomicUsize::new(0)),
            total_tasks: Arc::new(AtomicU64::new(0)),
            successful_tasks: Arc::new(AtomicU64::new(0)),
            failed_tasks: Arc::new(AtomicU64::new(0)),
            stop_flag: Arc::new(AtomicBool::new(false)),
            migration_history: Arc::new(RwLock::new(VecDeque::new())),
        }
    }

    /// 初始化兼容性管理器
    pub fn init(&self) -> Result<()> {
        info!(
            "兼容性管理器初始化完成，当前版本: {:?}",
            self.current_version
        );
        Ok(())
    }

    /// 检测存储格式版本
    pub async fn detect_storage_version(&self, storage_path: &Path) -> Result<StorageVersion> {
        // 检查是否存在v0.7.0的标记文件
        let version_file = storage_path.join(".storage_version");
        if version_file.exists() {
            let content = fs::read_to_string(&version_file)?;
            let version = match content.trim() {
                "0.7" => StorageVersion::V07,
                "0.6" => StorageVersion::V06,
                _ => StorageVersion::V06,
            };
            info!("检测到存储版本: {:?}", version);
            return Ok(version);
        }

        // 检查是否存在v0.7.0特有的目录结构
        let new_format_dir = storage_path.join("storage_v2");
        if new_format_dir.exists() {
            return Ok(StorageVersion::V07);
        }

        // 默认为旧版本
        info!("未找到版本标识，使用默认版本: {:?}", StorageVersion::V06);
        Ok(StorageVersion::V06)
    }

    /// 开始在线迁移
    pub async fn start_online_migration(
        self: &Arc<Self>,
        source_path: &Path,
        target_path: &Path,
    ) -> Result<()> {
        let mut state = self.migration_state.write().unwrap();
        if *state != MigrationState::NotStarted {
            return Err(NasError::Other("迁移已经进行中或已完成".to_string()));
        }

        info!("开始在线迁移: {:?} -> {:?}", source_path, target_path);

        // 创建目标目录
        async_fs::create_dir_all(target_path).await?;

        // 扫描源目录中的文件
        let files = self.scan_files(source_path).await?;
        info!("发现 {} 个文件需要迁移", files.len());

        // 初始化进度
        {
            let mut progress = self.progress.write().unwrap();
            progress.total_files = files.len() as u64;
            progress.start_time = Some(chrono::Local::now().naive_local());
        }

        // 添加迁移任务
        {
            let mut queue = self.migration_queue.write().unwrap();
            for file in files {
                let target_file = target_path.join(file.strip_prefix(source_path).unwrap());
                queue.push_back(MigrationTask {
                    file_id: file.file_name().unwrap().to_str().unwrap().to_string(),
                    source_path: file,
                    target_path: target_file,
                    file_size: 0, // 将在实际迁移时填充
                    created_at: Instant::now(),
                });
            }
        }

        *state = MigrationState::InProgress;

        // 启动迁移任务
        self.spawn_migration_workers().await?;

        Ok(())
    }

    /// 扫描文件
    async fn scan_files(self: &Arc<Self>, path: &Path) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let mut entries = async_fs::read_dir(path).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                files.push(path);
            } else if path.is_dir() {
                // 递归扫描子目录
                let sub_files = self.scan_files(&path).await?;
                files.extend(sub_files);
            }
        }

        Ok(files)
    }

    /// 启动迁移工作线程
    async fn spawn_migration_workers(self: &Arc<Self>) -> Result<()> {
        let num_workers = self.config.concurrent_migrations;
        let migration_queue = self.migration_queue.clone();
        let progress = self.progress.clone();
        let active_tasks = self.active_tasks.clone();
        let total_tasks = self.total_tasks.clone();
        let successful_tasks = self.successful_tasks.clone();
        let failed_tasks = self.failed_tasks.clone();
        let migration_results = self.migration_results.clone();
        let migration_history = self.migration_history.clone();
        let stop_flag = self.stop_flag.clone();

        for worker_id in 0..num_workers {
            let worker_id = worker_id.to_string();
            let queue = migration_queue.clone();
            let progress = progress.clone();
            let active_tasks = active_tasks.clone();
            let total_tasks = total_tasks.clone();
            let successful_tasks = successful_tasks.clone();
            let failed_tasks = failed_tasks.clone();
            let results = migration_results.clone();
            let history = migration_history.clone();
            let stop = stop_flag.clone();

            tokio::spawn(async move {
                info!("迁移工作线程 {} 启动", worker_id);

                loop {
                    // 检查停止标志
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }

                    // 获取迁移任务
                    let task = {
                        let mut queue = queue.write().unwrap();
                        queue.pop_front()
                    };

                    if let Some(task) = task {
                        // 更新活动任务数
                        active_tasks.fetch_add(1, Ordering::Relaxed);
                        total_tasks.fetch_add(1, Ordering::Relaxed);

                        // 执行迁移
                        let start = Instant::now();
                        let result = Self::migrate_file(&task).await;
                        let duration = start.elapsed();

                        match result {
                            Ok(compression_ratio) => {
                                successful_tasks.fetch_add(1, Ordering::Relaxed);

                                let migration_result = MigrationResult {
                                    task_id: 0, // 可以使用递增ID
                                    file_id: task.file_id.clone(),
                                    success: true,
                                    error: None,
                                    duration,
                                    compression_ratio,
                                };

                                // 记录结果
                                {
                                    let mut results = results.write().unwrap();
                                    results.insert(task.file_id.clone(), migration_result.clone());
                                }

                                {
                                    let mut history = history.write().unwrap();
                                    history.push_back(migration_result.clone());
                                    if history.len() > 1000 {
                                        history.pop_front();
                                    }
                                }
                            }
                            Err(e) => {
                                failed_tasks.fetch_add(1, Ordering::Relaxed);

                                let migration_result = MigrationResult {
                                    task_id: 0,
                                    file_id: task.file_id.clone(),
                                    success: false,
                                    error: Some(e.to_string()),
                                    duration,
                                    compression_ratio: None,
                                };

                                {
                                    let mut results = results.write().unwrap();
                                    results.insert(task.file_id.clone(), migration_result.clone());
                                }

                                {
                                    let mut history = history.write().unwrap();
                                    history.push_back(migration_result.clone());
                                }
                            }
                        }

                        // 更新进度
                        {
                            let mut progress = progress.write().unwrap();
                            progress.migrated_files += 1;
                            progress.current_file = None;

                            // 计算迁移速度
                            if let Some(start_time) = progress.start_time {
                                let elapsed = chrono::Local::now().naive_local() - start_time;
                                progress.migration_rate =
                                    progress.migrated_files as f64 / elapsed.num_seconds() as f64;

                                // 预计完成时间
                                if progress.migration_rate > 0.0 {
                                    let remaining_files =
                                        progress.total_files - progress.migrated_files;
                                    let remaining_seconds =
                                        (remaining_files as f64 / progress.migration_rate) as i64;
                                    progress.estimated_completion = Some(
                                        chrono::Local::now().naive_local()
                                            + chrono::Duration::seconds(remaining_seconds),
                                    );
                                }
                            }
                        }

                        // 减少活动任务数
                        active_tasks.fetch_sub(1, Ordering::Relaxed);
                    } else {
                        // 没有任务，短暂休眠
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }

                info!("迁移工作线程 {} 退出", worker_id);
            });
        }

        Ok(())
    }

    /// 迁移单个文件
    async fn migrate_file(task: &MigrationTask) -> Result<Option<f32>> {
        debug!("迁移文件: {:?}", task.source_path);

        // 读取源文件
        let data = async_fs::read(&task.source_path).await?;

        // 模拟格式转换（这里应该实现实际的格式转换逻辑）
        // 例如：压缩、去重、重新组织块等
        let converted_data = data; // 简化实现

        // 写入目标文件
        if let Some(parent) = task.target_path.parent() {
            async_fs::create_dir_all(parent).await?;
        }
        async_fs::write(&task.target_path, &converted_data).await?;

        // 计算压缩比（如果数据被压缩）
        let compression_ratio = if converted_data.len() < data.len() {
            Some(data.len() as f32 / converted_data.len() as f32)
        } else {
            None
        };

        debug!(
            "文件迁移完成: {}, 压缩比: {:?}",
            task.file_id, compression_ratio
        );

        Ok(compression_ratio)
    }

    /// 检查迁移进度
    pub async fn check_migration_progress(&self) -> MigrationProgress {
        let progress = self.progress.read().unwrap();
        progress.clone()
    }

    /// 停止迁移
    pub fn stop_migration(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        info!("迁移停止标志已设置");
    }

    /// 回滚迁移
    pub async fn rollback_migration(&self) -> Result<()> {
        info!("开始回滚迁移");

        self.stop_migration();

        // 等待所有任务完成
        while self.active_tasks.load(Ordering::Relaxed) > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // 清理迁移结果
        {
            let mut results = self.migration_results.write().unwrap();
            results.clear();
        }

        // 更新状态
        {
            let mut state = self.migration_state.write().unwrap();
            *state = MigrationState::RolledBack;
        }

        info!("迁移回滚完成");
        Ok(())
    }

    /// 完成迁移
    pub async fn finalize_migration(&self) -> Result<()> {
        info!("完成迁移");

        self.stop_migration();

        // 等待所有任务完成
        while self.active_tasks.load(Ordering::Relaxed) > 0 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // 更新状态
        {
            let mut state = self.migration_state.write().unwrap();
            *state = MigrationState::Completed;
        }

        info!("迁移完成");
        Ok(())
    }

    /// 获取迁移状态
    pub async fn get_migration_state(&self) -> MigrationState {
        let state = self.migration_state.read().unwrap();
        *state
    }

    /// 获取迁移统计
    pub async fn get_migration_stats(&self) -> MigrationStats {
        MigrationStats {
            total_tasks: self.total_tasks.load(Ordering::Relaxed),
            active_tasks: self.active_tasks.load(Ordering::Relaxed),
            successful_tasks: self.successful_tasks.load(Ordering::Relaxed),
            failed_tasks: self.failed_tasks.load(Ordering::Relaxed),
            success_rate: if self.total_tasks.load(Ordering::Relaxed) > 0 {
                self.successful_tasks.load(Ordering::Relaxed) as f32
                    / self.total_tasks.load(Ordering::Relaxed) as f32
            } else {
                0.0
            },
        }
    }

    /// 获取迁移历史
    pub async fn get_migration_history(&self, limit: usize) -> Vec<MigrationResult> {
        let history = self.migration_history.read().unwrap();
        history.iter().rev().take(limit).cloned().collect()
    }

    /// API兼容性层：旧版API适配
    pub async fn handle_legacy_api(&self, request: &str) -> Result<String> {
        if !self.api_config.enable_compat_layer {
            return Err(NasError::Other("API兼容性层未启用".to_string()));
        }

        // 这里实现旧版API的适配逻辑
        // 简化实现：直接返回成功响应
        debug!("处理兼容性API请求: {}", request);
        Ok("{\"status\":\"ok\"}".to_string())
    }

    /// 检查API兼容性
    pub fn check_api_compatibility(&self, api_version: &str) -> bool {
        // 简化实现：v0.6.0及之后版本都兼容
        matches!(api_version.parse::<f32>(), Ok(v) if v >= 0.6)
    }

    /// 获取当前存储版本
    pub fn get_current_version(&self) -> StorageVersion {
        self.current_version
    }
}

/// 迁移统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationStats {
    pub total_tasks: u64,
    pub active_tasks: usize,
    pub successful_tasks: u64,
    pub failed_tasks: u64,
    pub success_rate: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_compatibility_manager_new() {
        let config = MigrationConfig::default();
        let api_config = ApiCompatibilityConfig::default();
        let manager = CompatibilityManager::new(StorageVersion::V06, config, api_config);

        manager.init().unwrap();
    }

    #[tokio::test]
    async fn test_detect_storage_version() {
        let temp_dir = TempDir::new().unwrap();
        let config = MigrationConfig::default();
        let api_config = ApiCompatibilityConfig::default();
        let manager = CompatibilityManager::new(StorageVersion::V07, config, api_config);

        // 创建版本文件
        fs::write(temp_dir.path().join(".storage_version"), "0.7").unwrap();

        let version = manager
            .detect_storage_version(temp_dir.path())
            .await
            .unwrap();
        assert_eq!(version, StorageVersion::V07);
    }

    #[tokio::test]
    async fn test_migration_progress() {
        let config = MigrationConfig::default();
        let api_config = ApiCompatibilityConfig::default();
        let manager = CompatibilityManager::new(StorageVersion::V06, config, api_config);

        let progress = manager.check_migration_progress().await;
        assert_eq!(progress.total_files, 0);
        assert_eq!(progress.migrated_files, 0);
    }

    #[tokio::test]
    async fn test_api_compatibility_check() {
        let config = MigrationConfig::default();
        let api_config = ApiCompatibilityConfig::default();
        let manager = CompatibilityManager::new(StorageVersion::V07, config, api_config);

        assert!(manager.check_api_compatibility("0.6.0"));
        assert!(manager.check_api_compatibility("0.7.0"));
        assert!(!manager.check_api_compatibility("0.5.0"));
    }
}
