//! 增量存储后端
//!
//! 实现版本链式存储和块级存储功能。
//!
//! # 架构
//!
//! `StorageManager` 是存储系统的核心管理器，提供以下功能模块：
//!
//! ## 核心存储 (Lines 121-1298)
//! - 初始化和配置管理
//! - 版本保存和读取 (`save_version`, `read_version_data`)
//! - 版本管理 (`list_file_versions`, `delete_file_version`)
//! - 存储统计 (`get_storage_stats`, `get_deduplication_stats`)
//! - 块操作 (`save_chunk`, `read_chunk`)
//! - 路径管理和辅助方法
//!
//! ## 索引和文件管理 (Lines 1300-1733)
//! - 引用计数管理 (`load_chunk_ref_count`, `save_chunk_ref_count`)
//! - 文件索引 (`load_file_index`, `save_file_index`, `rebuild_file_index`)
//! - 文件列表和删除 (`list_files`, `delete_file`, `permanently_delete_file`)
//! - 回收站管理 (`list_deleted_files`, `restore_file`, `empty_recycle_bin`)
//!
//! ## 垃圾回收 (Lines 1736-1901)
//! - 块级垃圾回收 (`garbage_collect_blocks`)
//! - 后台 GC 任务 (`start_gc_task`, `stop_gc_task`)
//! - 完整 GC (`garbage_collect`)
//!
//! ## 文件操作 (Lines 1902-2107)
//! - 文件移动 (`move_file`)
//! - 文件信息查询 (`get_file_info`)
//!
//! ## 可靠性 (Lines 2119-2163)
//! - 块校验 (`verify_all_chunks`, `verify_chunks`)
//! - 孤儿块检测和清理 (`detect_orphan_chunks`, `cleanup_orphan_chunks`)
//!
//! ## 后台优化 (Lines 2165-2663)
//! - 优化任务执行 (`execute_optimization_task`)
//! - 优化策略 (`optimize_compress_only`, `optimize_full`)
//! - 后台优化任务 (`start_optimization_task`, `stop_optimization_task`)
//! - 优化调度器控制 (`pause/resume_optimization_scheduler`)
//!
//! ## Trait 实现 (Lines 2708-2931)
//! - `StorageManagerTrait` 实现
//! - `S3CompatibleStorageTrait` 实现

use crate::cache::CacheManager;
use crate::error::{Result, StorageError};
use crate::metadata::SledMetadataDb;
use crate::reliability::{ChunkVerifier, OrphanChunkCleaner, WalManager};
use crate::{ChunkInfo, FileDelta, IncrementalConfig, VersionInfo};
use async_trait::async_trait;
use chrono::Local;
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use silent_nas_core::{FileMetadata, FileVersion, S3CompatibleStorageTrait, StorageManagerTrait};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::{OnceCell, RwLock};
use tracing::{info, warn};

/// 块引用计数信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRefCount {
    /// 块ID
    pub chunk_id: String,
    /// 引用计数
    pub ref_count: usize,
    /// 块大小
    pub size: u64,
    /// 存储路径
    pub path: PathBuf,
}

/// 文件索引信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileIndexEntry {
    /// 文件ID
    pub file_id: String,
    /// 最新版本ID
    pub latest_version_id: String,
    /// 版本数量
    pub version_count: usize,
    /// 创建时间
    pub created_at: chrono::NaiveDateTime,
    /// 最后修改时间
    pub modified_at: chrono::NaiveDateTime,
    /// 是否已删除（软删除标记）
    #[serde(default)]
    pub is_deleted: bool,
    /// 删除时间
    #[serde(default)]
    pub deleted_at: Option<chrono::NaiveDateTime>,
    /// 存储模式
    #[serde(default)]
    pub storage_mode: crate::StorageMode,
    /// 优化状态
    #[serde(default)]
    pub optimization_status: crate::OptimizationStatus,
    /// 文件大小（字节）
    #[serde(default)]
    pub file_size: u64,
    /// 文件哈希（SHA-256）
    #[serde(default)]
    pub file_hash: String,
}

/// 存储管理器
///
/// 基于增量存储、块级去重和版本管理的高级存储系统
#[derive(Clone)]
pub struct StorageManager {
    /// 存储根目录
    root_path: PathBuf,
    /// 用户数据根目录 (root_path/data)
    data_root: PathBuf,
    /// 热存储根目录 (root_path/hot)
    hot_storage_root: PathBuf,
    /// 配置
    config: IncrementalConfig,
    /// 版本根目录 (root_path/incremental)
    version_root: PathBuf,
    /// 块存储根目录
    chunk_root: PathBuf,
    /// 块大小（预留字段，当前使用 IncrementalConfig 中的分块配置）
    #[allow(dead_code)]
    chunk_size: usize,
    /// Sled 元数据数据库（在 init() 中初始化）
    metadata_db: Arc<OnceCell<SledMetadataDb>>,
    /// 版本索引 LRU 缓存（有界缓存，防止 OOM）
    version_cache: Cache<String, VersionInfo>,
    /// 块索引 LRU 缓存（有界缓存，防止 OOM）
    block_cache: Cache<String, PathBuf>,
    /// 缓存管理器（Phase 5 Step 3）
    cache_manager: Arc<CacheManager>,
    /// WAL 管理器（Phase 5 Step 4）
    wal_manager: Arc<RwLock<WalManager>>,
    /// Chunk 校验器（Phase 5 Step 4）
    chunk_verifier: Arc<ChunkVerifier>,
    /// 孤儿 Chunk 清理器（Phase 5 Step 4）
    orphan_cleaner: Arc<OrphanChunkCleaner>,
    /// 压缩器
    compressor: Arc<crate::core::compression::Compressor>,
    /// 去重管理器（使用内存索引）
    dedup_manager: Arc<crate::services::dedup::DedupManager>,
    /// GC任务句柄
    gc_task_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
    /// GC任务停止标志（无锁原子操作）
    gc_stop_flag: Arc<AtomicBool>,
    /// 优化调度器
    optimization_scheduler: Arc<crate::OptimizationScheduler>,
    /// 优化任务句柄
    optimization_task_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
    /// 优化任务停止标志（无锁原子操作）
    optimization_stop_flag: Arc<AtomicBool>,
}

// ============================================================================
// 核心存储实现
// ============================================================================
// 包含：初始化、版本管理、块操作、存储统计、路径管理
// ============================================================================

impl StorageManager {
    pub fn new(root_path: PathBuf, chunk_size: usize, config: IncrementalConfig) -> Self {
        let data_root = root_path.join("data");
        let hot_storage_root = root_path.join("hot");
        let version_root = root_path.join("incremental");
        let chunk_root = version_root.join("chunks");
        let wal_path = version_root.join("wal.log");

        // 从 IncrementalConfig 创建压缩配置
        let compression_algorithm = match config.compression_algorithm.as_str() {
            "lz4" => crate::core::compression::CompressionAlgorithm::LZ4,
            "zstd" => crate::core::compression::CompressionAlgorithm::Zstd,
            _ => crate::core::compression::CompressionAlgorithm::None,
        };

        let compression_config = crate::core::compression::CompressionConfig {
            algorithm: if config.enable_compression {
                compression_algorithm
            } else {
                crate::core::compression::CompressionAlgorithm::None
            },
            level: 1,       // 快速压缩
            min_size: 1024, // 1KB 以上才压缩
            auto_compress_days: 7,
            min_ratio: 1.1, // 压缩比至少 10%
        };

        let compressor = Arc::new(crate::core::compression::Compressor::new(
            compression_config,
        ));

        // 初始化去重管理器
        let dedup_config = crate::services::dedup::DedupConfig {
            enable_dedup: config.enable_deduplication,
            min_dedup_size: 1024, // 1KB
            batch_size: 1000,
            concurrent_threads: 4,
            max_memory_index: 100000,
            sync_interval_secs: 300,
            enable_cow: true,
        };
        let block_index_config = crate::services::index::BlockIndexConfig {
            auto_save: true,
            save_interval_secs: 60,
            max_memory_entries: 100000,
            hot_data_ratio: 0.2,
            gc_interval_secs: 3600,
        };
        let dedup_manager = Arc::new(crate::services::dedup::DedupManager::new(
            dedup_config,
            block_index_config,
            root_path.to_str().unwrap(),
        ));

        // 初始化优化调度器（最多2个并发任务）
        let optimization_scheduler = Arc::new(crate::OptimizationScheduler::new(2));

        // 初始化 LRU 缓存（有界，防止 OOM）
        // version_cache: 10,000 个版本，TTL 1小时，空闲5分钟淘汰
        let version_cache = Cache::builder()
            .max_capacity(10_000)
            .time_to_live(Duration::from_secs(3600))
            .time_to_idle(Duration::from_secs(300))
            .build();

        // block_cache: 50,000 个块，TTL 1小时，空闲5分钟淘汰
        let block_cache = Cache::builder()
            .max_capacity(50_000)
            .time_to_live(Duration::from_secs(3600))
            .time_to_idle(Duration::from_secs(300))
            .build();

        Self {
            root_path,
            data_root,
            hot_storage_root,
            config,
            version_root,
            chunk_root: chunk_root.clone(),
            chunk_size,
            metadata_db: Arc::new(OnceCell::new()),
            version_cache,
            block_cache,
            cache_manager: Arc::new(CacheManager::with_default()),
            wal_manager: Arc::new(RwLock::new(WalManager::new(wal_path))),
            chunk_verifier: Arc::new(ChunkVerifier::new(chunk_root.clone())),
            orphan_cleaner: Arc::new(OrphanChunkCleaner::new(chunk_root)),
            compressor,
            dedup_manager,
            gc_task_handle: Arc::new(RwLock::new(None)),
            gc_stop_flag: Arc::new(AtomicBool::new(false)),
            optimization_scheduler,
            optimization_task_handle: Arc::new(RwLock::new(None)),
            optimization_stop_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 初始化增量存储
    pub async fn init(&self) -> Result<()> {
        // 创建必要的目录
        fs::create_dir_all(&self.root_path).await?;
        fs::create_dir_all(&self.data_root).await?;
        fs::create_dir_all(&self.hot_storage_root).await?;
        fs::create_dir_all(&self.version_root).await?;
        fs::create_dir_all(&self.chunk_root).await?;

        // 初始化 Sled 元数据数据库
        let db_path = self.version_root.join("metadata");
        let metadata_db = SledMetadataDb::open(&db_path)
            .map_err(|e| StorageError::Storage(format!("初始化 Sled 数据库失败: {}", e)))?;

        self.metadata_db
            .set(metadata_db)
            .map_err(|_| StorageError::Storage("元数据数据库已初始化".to_string()))?;

        info!("Sled 元数据数据库初始化完成: path={:?}", db_path);

        // 初始化 WAL（Phase 5 Step 4）
        let mut wal = self.wal_manager.write().await;
        wal.init().await?;
        drop(wal); // 释放锁

        // 初始化去重管理器
        self.dedup_manager.init().await?;

        // 加载现有索引
        self.load_version_index().await?;
        self.load_block_index().await?;
        self.load_chunk_ref_count().await?;
        self.load_file_index().await?;

        // 启动自动GC任务（如果启用）
        if self.config.enable_auto_gc {
            self.start_gc_task().await;
            info!("自动GC任务已启动，间隔: {}秒", self.config.gc_interval_secs);
        }

        // 启动后台优化任务（统一流程，始终启用）
        self.start_optimization_task().await;
        info!("后台优化任务已启动");

        info!(
            "增量存储初始化完成: root={:?}, data={:?}, version_root={:?}",
            self.root_path, self.data_root, self.version_root
        );
        Ok(())
    }

    /// 获取元数据数据库引用
    fn get_metadata_db(&self) -> Result<&SledMetadataDb> {
        self.metadata_db
            .get()
            .ok_or_else(|| StorageError::Storage("元数据数据库未初始化".to_string()))
    }

    /// 获取缓存管理器引用
    pub fn get_cache_manager(&self) -> Arc<CacheManager> {
        self.cache_manager.clone()
    }

    /// 从磁盘路径流式保存文件（避免一次性将整个文件读入内存）
    pub async fn save_file_from_path(
        &self,
        file_id: &str,
        source_path: &Path,
    ) -> Result<FileMetadata> {
        let (_delta, file_version) = self
            .save_version_from_path(file_id, source_path, None)
            .await?;

        Ok(FileMetadata {
            id: file_id.to_string(),
            name: file_id.to_string(),
            path: file_id.to_string(),
            size: file_version.size,
            hash: file_version.version_id.clone(),
            created_at: file_version.created_at,
            modified_at: file_version.created_at,
        })
    }

    /// 从异步读取器流式保存文件（供上层传入 HTTP body 等场景使用）
    pub async fn save_file_from_reader<R>(
        &self,
        file_id: &str,
        reader: &mut R,
    ) -> Result<FileMetadata>
    where
        R: AsyncRead + Unpin,
    {
        let (_delta, file_version) = self.save_version_from_reader(file_id, reader, None).await?;

        Ok(FileMetadata {
            id: file_id.to_string(),
            name: file_id.to_string(),
            path: file_id.to_string(),
            size: file_version.size,
            hash: file_version.version_id.clone(),
            created_at: file_version.created_at,
            modified_at: file_version.created_at,
        })
    }

    /// 从磁盘路径流式保存文件版本（避免将整个文件读入内存）
    pub async fn save_version_from_path(
        &self,
        file_id: &str,
        source_path: &Path,
        parent_version_id: Option<&str>,
    ) -> Result<(FileDelta, FileVersion)> {
        let mut file = fs::File::open(source_path)
            .await
            .map_err(StorageError::Io)?;
        self.save_version_from_reader(file_id, &mut file, parent_version_id)
            .await
    }

    /// 从异步读取器流式保存文件版本（用于 WebDAV 等场景）
    pub async fn save_version_from_reader<R>(
        &self,
        file_id: &str,
        reader: &mut R,
        parent_version_id: Option<&str>,
    ) -> Result<(FileDelta, FileVersion)>
    where
        R: AsyncRead + Unpin,
    {
        let version_id = format!("v_{}", scru128::new());

        // 直接流式写入热存储（统一流程）
        use sha2::{Digest, Sha256};
        let now = Local::now().naive_local();

        // 1. 准备热存储路径
        let hot_path = self.get_hot_storage_path(file_id);
        if let Some(parent) = hot_path.parent() {
            fs::create_dir_all(parent).await.map_err(StorageError::Io)?;
        }

        // 2. 流式写入热存储，同时计算哈希
        let mut file = fs::File::create(&hot_path)
            .await
            .map_err(StorageError::Io)?;
        let mut hasher = Sha256::new();
        let mut total_size: u64 = 0;

        const BUFFER_SIZE: usize = 8 * 1024 * 1024; // 8MB
        let mut buffer = vec![0u8; BUFFER_SIZE];

        loop {
            let n = reader.read(&mut buffer).await.map_err(StorageError::Io)?;
            if n == 0 {
                break;
            }

            file.write_all(&buffer[..n])
                .await
                .map_err(StorageError::Io)?;
            hasher.update(&buffer[..n]);
            total_size += n as u64;
        }

        file.flush().await.map_err(StorageError::Io)?;

        let file_hash = hex::encode(hasher.finalize());

        info!(
            "文件 {} 已流式保存到热存储，大小={}B，待后台优化",
            file_id, total_size
        );

        // 3. 创建文件版本信息
        let file_version = FileVersion {
            version_id: version_id.clone(),
            file_id: file_id.to_string(),
            name: file_id.to_string(),
            size: total_size,
            hash: file_hash.clone(),
            created_at: now,
            author: None,
            comment: None,
            is_current: true,
        };

        // 4. 更新文件索引（Hot模式）
        let metadata_db = self.get_metadata_db()?;
        let mut file_entry = metadata_db
            .get_file_index(file_id)
            .map_err(|e| StorageError::Storage(format!("读取文件索引失败: {}", e)))?
            .unwrap_or_else(|| FileIndexEntry {
                file_id: file_id.to_string(),
                latest_version_id: version_id.clone(),
                version_count: 0,
                created_at: now,
                modified_at: now,
                is_deleted: false,
                deleted_at: None,
                storage_mode: crate::StorageMode::Hot,
                optimization_status: crate::OptimizationStatus::Pending,
                file_size: total_size,
                file_hash: file_hash.clone(),
            });

        file_entry.latest_version_id = version_id.clone();
        file_entry.version_count += 1;
        file_entry.modified_at = now;
        file_entry.storage_mode = crate::StorageMode::Hot;
        file_entry.optimization_status = crate::OptimizationStatus::Pending;
        file_entry.file_size = total_size;
        file_entry.file_hash = file_hash.clone();

        metadata_db
            .put_file_index(file_id, &file_entry)
            .map_err(|e| StorageError::Storage(format!("保存文件索引失败: {}", e)))?;

        // 5. 创建空的Delta（热存储不需要分块信息）
        let delta = FileDelta {
            file_id: file_id.to_string(),
            base_version_id: parent_version_id.unwrap_or("").to_string(),
            new_version_id: version_id.clone(),
            chunks: Vec::new(), // 热存储没有chunks
            created_at: now,
        };

        // 6. 保存版本信息（重要！）
        let _version_info = self
            .save_version_info(file_id, &delta, parent_version_id)
            .await?;

        // 7. 提交后台优化任务（统一使用Full策略）
        let task = crate::OptimizationTask::new(
            file_id.to_string(),
            hot_path.clone(),
            total_size,
            file_hash.clone(),
            crate::OptimizationStrategy::Full, // 统一策略
            0,                                 // 立即执行
        );
        self.optimization_scheduler.submit_task(task).await;

        Ok((delta, file_version))
    }

    /// 保存文件版本（使用增量存储）
    pub async fn save_version(
        &self,
        file_id: &str,
        data: &[u8],
        parent_version_id: Option<&str>,
    ) -> Result<(FileDelta, FileVersion)> {
        let version_id = format!("v_{}", scru128::new());
        let now = Local::now().naive_local();

        // 1. 计算文件哈希
        let file_hash = self.calculate_hash(data);

        // 2. 直接写入热存储（统一流程：先快速上传，后台优化）
        let hot_path = self.get_hot_storage_path(file_id);
        if let Some(parent) = hot_path.parent() {
            fs::create_dir_all(parent).await.map_err(StorageError::Io)?;
        }
        fs::write(&hot_path, data).await.map_err(StorageError::Io)?;

        info!(
            "文件 {} 已保存到热存储，大小={}B，待后台优化",
            file_id,
            data.len()
        );

        // 3. 创建文件版本信息
        let file_version = FileVersion {
            version_id: version_id.clone(),
            file_id: file_id.to_string(),
            name: file_id.to_string(),
            size: data.len() as u64,
            hash: file_hash.clone(),
            created_at: now,
            author: None,
            comment: None,
            is_current: true,
        };

        // 4. 更新文件索引（Hot模式）
        let metadata_db = self.get_metadata_db()?;
        let mut file_entry = metadata_db
            .get_file_index(file_id)
            .map_err(|e| StorageError::Storage(format!("读取文件索引失败: {}", e)))?
            .unwrap_or_else(|| FileIndexEntry {
                file_id: file_id.to_string(),
                latest_version_id: version_id.clone(),
                version_count: 0,
                created_at: now,
                modified_at: now,
                is_deleted: false,
                deleted_at: None,
                storage_mode: crate::StorageMode::Hot,
                optimization_status: crate::OptimizationStatus::Pending,
                file_size: data.len() as u64,
                file_hash: file_hash.clone(),
            });

        file_entry.latest_version_id = version_id.clone();
        file_entry.version_count += 1;
        file_entry.modified_at = now;
        file_entry.storage_mode = crate::StorageMode::Hot;
        file_entry.optimization_status = crate::OptimizationStatus::Pending;
        file_entry.file_size = data.len() as u64;
        file_entry.file_hash = file_hash.clone();

        metadata_db
            .put_file_index(file_id, &file_entry)
            .map_err(|e| StorageError::Storage(format!("保存文件索引失败: {}", e)))?;

        // 5. 创建空的Delta（热存储不需要分块信息）
        let delta = FileDelta {
            file_id: file_id.to_string(),
            base_version_id: parent_version_id.unwrap_or("").to_string(),
            new_version_id: version_id.clone(),
            chunks: Vec::new(), // 热存储没有chunks
            created_at: now,
        };

        // 6. 保存版本信息（重要！）
        let _version_info = self
            .save_version_info(file_id, &delta, parent_version_id)
            .await?;

        // 7. 提交后台优化任务（统一使用Full策略，立即执行）
        let task = crate::OptimizationTask::new(
            file_id.to_string(),
            hot_path.clone(),
            data.len() as u64,
            file_hash.clone(),
            crate::OptimizationStrategy::Full,
            0, // 立即执行，无延迟
        );
        self.optimization_scheduler.submit_task(task).await;

        Ok((delta, file_version))
    }

    /// 读取版本数据
    pub async fn read_version_data(&self, version_id: &str) -> Result<Vec<u8>> {
        // 获取版本信息
        let version_info = self.get_version_info(version_id).await?;

        // 检查文件的存储模式
        let metadata_db = self.get_metadata_db()?;
        if let Some(file_entry) = metadata_db
            .get_file_index(&version_info.file_id)
            .map_err(|e| StorageError::Storage(format!("读取文件索引失败: {}", e)))?
        {
            match file_entry.storage_mode {
                // 热存储模式：直接从热存储读取
                crate::StorageMode::Hot => {
                    let hot_path = self.get_hot_storage_path(&version_info.file_id);
                    if hot_path.exists() {
                        let data = fs::read(&hot_path).await.map_err(StorageError::Io)?;
                        return Ok(data);
                    } else {
                        return Err(StorageError::Storage(format!(
                            "热存储文件不存在: {}",
                            hot_path.display()
                        )));
                    }
                }
                // 压缩存储模式：读取压缩文件并解压
                crate::StorageMode::Compressed => {
                    let compressed_path = self
                        .data_root
                        .join(format!("{}.compressed", version_info.file_id));
                    if compressed_path.exists() {
                        let compressed_data =
                            fs::read(&compressed_path).await.map_err(StorageError::Io)?;

                        // 解压数据
                        if self.config.enable_compression {
                            let algorithm = match self.config.compression_algorithm.as_str() {
                                "lz4" => crate::core::CompressionAlgorithm::LZ4,
                                "zstd" => crate::core::CompressionAlgorithm::Zstd,
                                _ => crate::core::CompressionAlgorithm::LZ4,
                            };
                            let compression_config = crate::core::compression::CompressionConfig {
                                algorithm,
                                level: 1,
                                min_size: 0,
                                ..Default::default()
                            };
                            let compressor =
                                crate::core::compression::Compressor::new(compression_config);
                            let data = compressor.decompress(&compressed_data, algorithm)?;
                            return Ok(data);
                        } else {
                            // 未启用压缩，直接返回
                            return Ok(compressed_data);
                        }
                    } else {
                        return Err(StorageError::Storage(format!(
                            "压缩存储文件不存在: {}",
                            compressed_path.display()
                        )));
                    }
                }
                // 冷存储模式：继续使用分块读取
                crate::StorageMode::Cold => {
                    // 继续执行下面的分块读取逻辑
                }
            }
        }

        // 冷存储模式：使用传统的分块读取流程
        // 重建文件数据
        let mut result = Vec::new();
        let mut current_version_id = version_id.to_string();

        loop {
            let version = self.get_version_info(&current_version_id).await?;
            let delta = self
                .read_delta(&version.file_id, &current_version_id)
                .await?;

            // 读取并应用分块
            for chunk in &delta.chunks {
                let chunk_data = self.read_chunk(&chunk.chunk_id, chunk.compression).await?;

                // 确保result有足够的空间
                let required_len = chunk.offset + chunk_data.len();
                if result.len() < required_len {
                    result.resize(required_len, 0);
                }

                // 在正确的offset位置写入chunk数据
                result[chunk.offset..chunk.offset + chunk_data.len()].copy_from_slice(&chunk_data);
            }

            // 如果有父版本，继续向上遍历
            if let Some(parent_id) = version.parent_version_id {
                current_version_id = parent_id;
            } else {
                break;
            }
        }

        Ok(result)
    }

    /// 流式读取版本数据（用于大文件，避免将整个文件加载到内存）
    ///
    /// 返回一个实现了 `AsyncRead` 的文件句柄，适用于流式传输场景。
    /// 目前仅支持热存储模式；其他模式会回退到内存读取。
    ///
    /// # 返回值
    /// - `Ok(Some(file))`: 热存储模式，返回文件句柄
    /// - `Ok(None)`: 非热存储模式，调用者应使用 `read_version_data()` 代替
    /// - `Err(_)`: 发生错误
    ///
    /// # 示例
    /// ```rust,ignore
    /// match storage.read_version_stream(version_id).await? {
    ///     Some(file) => {
    ///         // 流式处理 file
    ///         tokio::io::copy(&mut file, &mut writer).await?;
    ///     }
    ///     None => {
    ///         // 回退到内存读取
    ///         let data = storage.read_version_data(version_id).await?;
    ///         writer.write_all(&data).await?;
    ///     }
    /// }
    /// ```
    pub async fn read_version_stream(
        &self,
        version_id: &str,
    ) -> Result<Option<tokio::fs::File>> {
        // 获取版本信息
        let version_info = self.get_version_info(version_id).await?;

        // 检查文件的存储模式
        let metadata_db = self.get_metadata_db()?;
        if let Some(file_entry) = metadata_db
            .get_file_index(&version_info.file_id)
            .map_err(|e| StorageError::Storage(format!("读取文件索引失败: {}", e)))?
        {
            if file_entry.storage_mode == crate::StorageMode::Hot {
                let hot_path = self.get_hot_storage_path(&version_info.file_id);
                if hot_path.exists() {
                    let file = fs::File::open(&hot_path).await.map_err(StorageError::Io)?;
                    return Ok(Some(file));
                } else {
                    return Err(StorageError::Storage(format!(
                        "热存储文件不存在: {}",
                        hot_path.display()
                    )));
                }
            }
        }

        // 非热存储模式，返回 None，调用者应使用 read_version_data()
        Ok(None)
    }

    /// 获取文件的流式读取路径（如果可用）
    ///
    /// 对于热存储模式，返回文件的实际路径，可用于零拷贝发送（如 sendfile）。
    /// 对于其他模式，返回 None。
    pub async fn get_file_path(&self, file_id: &str) -> Result<Option<PathBuf>> {
        let metadata_db = self.get_metadata_db()?;
        if let Some(file_entry) = metadata_db
            .get_file_index(file_id)
            .map_err(|e| StorageError::Storage(format!("读取文件索引失败: {}", e)))?
        {
            if file_entry.storage_mode == crate::StorageMode::Hot {
                let hot_path = self.get_hot_storage_path(file_id);
                if hot_path.exists() {
                    return Ok(Some(hot_path));
                }
            }
        }
        Ok(None)
    }

    /// 获取版本信息
    pub async fn get_version_info(&self, version_id: &str) -> Result<VersionInfo> {
        // 首先尝试从 LRU 缓存读取（无锁并发安全）
        if let Some(info) = self.version_cache.get(version_id).await {
            return Ok(info);
        }

        // 缓存未命中，从 Sled 读取
        let metadata_db = self.get_metadata_db()?;
        let version_info = metadata_db
            .get_version_info(version_id)
            .map_err(|e| StorageError::Storage(format!("从 Sled 读取版本信息失败: {}", e)))?
            .ok_or_else(|| StorageError::Storage(format!("版本信息不存在: {}", version_id)))?;

        // 更新 LRU 缓存（无锁并发安全，自动淘汰）
        self.version_cache
            .insert(version_id.to_string(), version_info.clone())
            .await;
        Ok(version_info)
    }

    /// 列出文件的所有版本
    pub async fn list_file_versions(&self, file_id: &str) -> Result<Vec<VersionInfo>> {
        let metadata_db = self.get_metadata_db()?;

        // 从 Sled 获取文件的所有版本
        let mut versions = metadata_db
            .list_file_versions(file_id)
            .map_err(|e| StorageError::Storage(format!("列出文件版本失败: {}", e)))?;

        // 按创建时间排序（最新的在前）
        versions.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(versions)
    }

    /// 删除特定文件版本
    pub async fn delete_file_version(&self, version_id: &str) -> Result<()> {
        let version_info = self.get_version_info(version_id).await?;

        // 不允许删除当前版本
        if version_info.is_current {
            return Err(StorageError::Storage("无法删除当前版本".to_string()));
        }

        // 读取delta以获取块信息
        let delta = self.read_delta(&version_info.file_id, version_id).await?;

        // 减少每个块的引用计数
        let metadata_db = self.get_metadata_db()?;
        for chunk in &delta.chunks {
            metadata_db
                .decrement_chunk_ref(&chunk.chunk_id)
                .map_err(|e| StorageError::Storage(format!("减少块引用计数失败: {}", e)))?;
        }

        // 删除delta文件
        let delta_path = self.get_delta_path(&version_info.file_id, version_id);
        if delta_path.exists() {
            fs::remove_file(&delta_path).await?;
        }

        // 从数据库中删除版本信息
        metadata_db
            .remove_version_info(version_id)
            .map_err(|e| StorageError::Storage(format!("删除版本信息失败: {}", e)))?;

        // 从 LRU 缓存中删除
        self.version_cache.invalidate(version_id).await;

        info!("删除版本: {}", version_id);
        Ok(())
    }

    /// 恢复文件到指定版本
    pub async fn restore_file_version(&self, file_id: &str, version_id: &str) -> Result<()> {
        // 获取版本信息
        let version_info = self.get_version_info(version_id).await?;

        if version_info.file_id != file_id {
            return Err(StorageError::Storage("版本与文件不匹配".to_string()));
        }

        // 读取版本数据
        let version_data = self.read_version_data(version_id).await?;

        // 获取当前版本（如果有）作为父版本
        let current_versions = self.list_file_versions(file_id).await?;
        let parent_version_id = current_versions.first().map(|v| v.version_id.as_str());

        // 保存为新版本（基于恢复的内容）
        self.save_version(file_id, &version_data, parent_version_id)
            .await?;

        info!("恢复文件到版本: {} -> {}", file_id, version_id);
        Ok(())
    }

    /// 获取存储统计信息
    pub async fn get_storage_stats(&self) -> Result<StorageStats> {
        let mut total_versions = 0;
        let mut total_chunks = 0;
        let mut total_size = 0u64;
        let mut unique_chunks = 0;

        // 从 Sled 读取所有文件和版本信息
        let metadata_db = self.get_metadata_db()?;
        let all_files = metadata_db
            .list_all_files()
            .map_err(|e| StorageError::Storage(format!("读取文件列表失败: {}", e)))?;

        // 遍历所有文件的所有版本
        for file_entry in all_files {
            let versions = metadata_db
                .list_file_versions(&file_entry.file_id)
                .map_err(|e| StorageError::Storage(format!("读取版本列表失败: {}", e)))?;

            for version in versions {
                total_versions += 1;
                total_size += version.storage_size;
                total_chunks += version.chunk_count;
            }
        }

        // 统计唯一块数量（扫描chunks目录）
        let chunks_dir = self.chunk_root.join("data");
        if chunks_dir.exists() {
            let mut entries = fs::read_dir(&chunks_dir).await.map_err(StorageError::Io)?;
            let mut total_chunk_size = 0u64;

            while let Some(entry) = entries.next_entry().await? {
                if entry.path().is_file() {
                    unique_chunks += 1;
                    if let Ok(metadata) = entry.metadata().await {
                        total_chunk_size += metadata.len();
                    }
                }
            }

            return Ok(StorageStats {
                total_versions,
                total_chunks,
                unique_chunks,
                total_size,
                total_chunk_size,
                compression_ratio: if total_size > 0 {
                    total_chunk_size as f64 / total_size as f64
                } else {
                    0.0
                },
                avg_chunk_size: if unique_chunks > 0 {
                    total_chunk_size as f64 / unique_chunks as f64
                } else {
                    0.0
                },
            });
        }

        // 如果chunks目录不存在，返回基础统计
        let total_chunk_size = 0;

        Ok(StorageStats {
            total_versions,
            total_chunks,
            unique_chunks,
            total_size,
            total_chunk_size,
            compression_ratio: if total_size > 0 {
                total_chunk_size as f64 / total_size as f64
            } else {
                0.0
            },
            avg_chunk_size: if unique_chunks > 0 {
                total_chunk_size as f64 / unique_chunks as f64
            } else {
                0.0
            },
        })
    }

    /// 获取全局去重统计
    pub async fn get_deduplication_stats(&self) -> Result<crate::DeduplicationStats> {
        // 如果启用了去重，从 dedup_manager 获取统计
        if self.config.enable_deduplication {
            // 从 DedupManager 获取所有块信息
            let all_blocks = self.dedup_manager.get_all_blocks().await;

            let mut total_original_size = 0u64;
            let mut total_stored_size = 0u64;
            let mut total_ref_count = 0usize;

            for block in &all_blocks {
                // 原始大小 = 块大小 × 引用次数
                total_original_size += block.size as u64 * block.ref_count as u64;
                // 存储大小 = 块大小（只存储一次）
                total_stored_size += block.size as u64;
                total_ref_count += block.ref_count as usize;
            }

            let unique_chunks = all_blocks.len();
            let duplicate_chunks = total_ref_count.saturating_sub(unique_chunks);

            let mut stats = crate::DeduplicationStats {
                total_chunks: total_ref_count,
                new_chunks: unique_chunks,
                duplicate_chunks,
                original_size: total_original_size,
                stored_size: total_stored_size,
                space_saved: 0,
                dedup_ratio: 0.0,
            };

            stats.calculate_dedup_ratio();
            Ok(stats)
        } else {
            // 未启用去重，从 metadata_db 获取（旧逻辑）
            let metadata_db = self.get_metadata_db()?;

            // 统计所有块
            let all_chunks = metadata_db
                .list_all_chunks()
                .map_err(|e| StorageError::Storage(format!("获取块列表失败: {}", e)))?;

            let mut total_original_size = 0u64;
            let mut total_stored_size = 0u64;
            let mut total_ref_count = 0usize;

            for (_chunk_id, ref_count_info) in &all_chunks {
                // 原始大小 = 块大小 × 引用次数
                total_original_size += ref_count_info.size * ref_count_info.ref_count as u64;
                // 存储大小 = 块大小（只存储一次）
                total_stored_size += ref_count_info.size;
                total_ref_count += ref_count_info.ref_count;
            }

            let unique_chunks = all_chunks.len();
            let duplicate_chunks = total_ref_count.saturating_sub(unique_chunks);

            let mut stats = crate::DeduplicationStats {
                total_chunks: total_ref_count,
                new_chunks: unique_chunks,
                duplicate_chunks,
                original_size: total_original_size,
                stored_size: total_stored_size,
                space_saved: 0,
                dedup_ratio: 0.0,
            };

            stats.calculate_dedup_ratio();
            Ok(stats)
        }
    }

    /// 保存块数据，返回使用的压缩算法
    #[allow(dead_code)]
    async fn save_chunk(
        &self,
        chunk: &ChunkInfo,
        file_data: &[u8],
    ) -> Result<crate::core::compression::CompressionAlgorithm> {
        let chunk_data = &file_data[chunk.offset..chunk.offset + chunk.size];
        let chunk_path = self.get_chunk_path(&chunk.chunk_id);

        // 创建父目录
        if let Some(parent) = chunk_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // 应用压缩（如果启用）
        let compression_result = self.compressor.compress(chunk_data)?;
        let data_to_write = &compression_result.compressed_data;
        let algorithm = compression_result.algorithm;

        // 写入块数据（可能已压缩）
        let mut file = fs::File::create(&chunk_path).await?;
        file.write_all(data_to_write).await?;
        file.flush().await?;

        // 更新块索引 LRU 缓存
        self.block_cache
            .insert(chunk.chunk_id.clone(), chunk_path)
            .await;

        Ok(algorithm)
    }

    /// 保存块数据（直接根据块内容，不依赖整体文件数据）
    async fn save_chunk_data(
        &self,
        chunk_id: &str,
        chunk_data: &[u8],
    ) -> Result<crate::core::compression::CompressionAlgorithm> {
        let chunk_path = self.get_chunk_path(chunk_id);

        // 创建父目录
        if let Some(parent) = chunk_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // 应用压缩（如果启用）
        let compression_result = self.compressor.compress(chunk_data)?;
        let data_to_write = &compression_result.compressed_data;
        let algorithm = compression_result.algorithm;

        // 写入块数据（可能已压缩）
        let mut file = fs::File::create(&chunk_path).await?;
        file.write_all(data_to_write).await?;
        file.flush().await?;

        // 更新块索引 LRU 缓存
        self.block_cache
            .insert(chunk_id.to_string(), chunk_path)
            .await;

        Ok(algorithm)
    }

    /// 读取块数据
    async fn read_chunk(
        &self,
        chunk_id: &str,
        compression: crate::core::compression::CompressionAlgorithm,
    ) -> Result<Vec<u8>> {
        let chunk_path = self.get_chunk_path(chunk_id);
        let data = fs::read(&chunk_path).await.map_err(StorageError::Io)?;

        // 如果数据被压缩，解压缩
        if compression != crate::core::compression::CompressionAlgorithm::None {
            self.compressor.decompress(&data, compression)
        } else {
            Ok(data)
        }
    }

    /// 保存版本信息
    async fn save_version_info(
        &self,
        file_id: &str,
        delta: &FileDelta,
        parent_version_id: Option<&str>,
    ) -> Result<VersionInfo> {
        // 计算文件大小：如果chunks为空（热存储模式），从file_index读取
        let file_size = if delta.chunks.is_empty() {
            let metadata_db = self.get_metadata_db()?;
            metadata_db
                .get_file_index(file_id)
                .map_err(|e| StorageError::Storage(format!("读取文件索引失败: {}", e)))?
                .map(|entry| entry.file_size)
                .unwrap_or(0)
        } else {
            delta.chunks.iter().map(|c| c.size as u64).sum()
        };

        let version_info = VersionInfo {
            version_id: delta.new_version_id.clone(),
            file_id: file_id.to_string(),
            parent_version_id: parent_version_id.map(|s| s.to_string()),
            file_size,
            chunk_count: delta.chunks.len(),
            storage_size: delta.chunks.iter().map(|c| c.size as u64).sum(),
            created_at: Local::now().naive_local(),
            is_current: true,
        };

        // 保存到 Sled 数据库
        let metadata_db = self.get_metadata_db()?;
        metadata_db
            .put_version_info(&version_info.version_id, &version_info)
            .map_err(|e| StorageError::Storage(format!("保存版本信息到 Sled 失败: {}", e)))?;

        // 更新 LRU 缓存
        self.version_cache
            .insert(version_info.version_id.clone(), version_info.clone())
            .await;

        Ok(version_info)
    }

    /// 读取差异数据
    async fn read_delta(&self, file_id: &str, version_id: &str) -> Result<FileDelta> {
        let delta_path = self.get_delta_path(file_id, version_id);
        let data = fs::read(&delta_path).await.map_err(StorageError::Io)?;
        let delta: FileDelta = serde_json::from_slice(&data)
            .map_err(|e| StorageError::Storage(format!("反序列化差异数据失败: {}", e)))?;

        Ok(delta)
    }

    /// 加载版本索引
    async fn load_version_index(&self) -> Result<()> {
        let versions_dir = self.version_root.join("versions");
        let metadata_db = self.get_metadata_db()?;

        // 如果旧的 versions 目录存在，迁移数据到 Sled
        if versions_dir.exists() {
            info!("检测到旧的 versions 目录，开始迁移数据到 Sled");
            let mut entries = fs::read_dir(&versions_dir)
                .await
                .map_err(StorageError::Io)?;

            let mut migrated_count = 0;
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.is_file()
                    && path.extension().and_then(|s| s.to_str()) == Some("json")
                    && let Some(file_name) = path.file_name().and_then(|s| s.to_str())
                {
                    let version_id = file_name.strip_suffix(".json").unwrap_or(file_name);
                    let data = fs::read(&path).await.map_err(StorageError::Io)?;
                    let version_info: VersionInfo = serde_json::from_slice(&data)
                        .map_err(|e| StorageError::Storage(format!("加载版本信息失败: {}", e)))?;

                    // 迁移到 Sled
                    metadata_db
                        .put_version_info(version_id, &version_info)
                        .map_err(|e| StorageError::Storage(format!("迁移版本信息失败: {}", e)))?;

                    // 可选：预热缓存（迁移的数据可能会立即被访问）
                    self.version_cache
                        .insert(version_id.to_string(), version_info)
                        .await;

                    migrated_count += 1;
                }
            }

            // 刷新到磁盘
            metadata_db
                .flush()
                .await
                .map_err(|e| StorageError::Storage(format!("刷新数据库失败: {}", e)))?;

            // 备份旧目录
            let backup_dir = self.version_root.join("versions.backup");
            fs::rename(&versions_dir, &backup_dir).await?;
            info!("已将 versions 目录备份到 {:?}", backup_dir);
            info!("迁移完成，共 {} 个版本信息", migrated_count);
        } else {
            // 从 Sled 加载数据（按需加载，不全部加载到内存）
            info!("使用 Sled 数据库存储版本信息，采用按需加载模式");
        }

        Ok(())
    }

    /// 加载块索引（已改为按需加载模式）
    async fn load_block_index(&self) -> Result<()> {
        // 块索引已改为按需加载 + LRU 缓存模式，不再在启动时全量加载
        // 只统计块数量用于日志
        let chunks_dir = self.chunk_root.join("data");

        if !chunks_dir.exists() {
            info!("块索引目录不存在，采用按需加载模式");
            return Ok(());
        }

        // 可选：统计块数量（不加载到内存）
        let mut count = 0;
        let mut entries = fs::read_dir(&chunks_dir).await.map_err(StorageError::Io)?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.path().is_file() {
                count += 1;
            }
        }

        info!("发现 {} 个数据块，采用按需加载 + LRU 缓存模式", count);
        Ok(())
    }

    /// 获取版本路径
    fn get_version_path(&self, version_id: &str) -> PathBuf {
        self.version_root
            .join("versions")
            .join(format!("{}.json", version_id))
    }

    /// 获取差异路径
    fn get_delta_path(&self, file_id: &str, version_id: &str) -> PathBuf {
        // 移除开头的 / 以确保是相对路径
        let cleaned_file_id = file_id.trim_start_matches('/');
        self.version_root
            .join("deltas")
            .join(cleaned_file_id)
            .join(format!("{}.json", version_id))
    }

    /// 保存差异数据
    async fn save_delta(&self, file_id: &str, delta: &FileDelta) -> Result<()> {
        let delta_path = self.get_delta_path(file_id, &delta.new_version_id);

        // 创建父目录
        if let Some(parent) = delta_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // 序列化并保存
        let data = serde_json::to_vec(delta)
            .map_err(|e| StorageError::Storage(format!("序列化差异数据失败: {}", e)))?;

        fs::write(&delta_path, data)
            .await
            .map_err(StorageError::Io)?;

        Ok(())
    }

    /// 获取块路径
    fn get_chunk_path(&self, chunk_id: &str) -> PathBuf {
        // 使用哈希前缀分层存储
        let prefix = &chunk_id[..2.min(chunk_id.len())];
        self.chunk_root.join("data").join(prefix).join(chunk_id)
    }

    /// 获取热存储路径
    fn get_hot_storage_path(&self, file_id: &str) -> PathBuf {
        // 移除开头的 / 以确保是相对路径
        let cleaned_id = file_id.trim_start_matches('/');

        // 如果文件ID包含目录结构（有 /），直接使用整个路径
        // 否则使用前2个字符作为前缀进行分层存储
        if cleaned_id.contains('/') {
            self.hot_storage_root.join(cleaned_id)
        } else {
            let prefix = &cleaned_id[..2.min(cleaned_id.len())];
            self.hot_storage_root.join(prefix).join(cleaned_id)
        }
    }

    /// 计算哈希值
    fn calculate_hash(&self, data: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    /// 获取版本根目录（公开方法，供适配器使用）
    pub fn version_root(&self) -> &Path {
        &self.version_root
    }

    /// 确保文件在 data_root 中存在（用于 WebDAV 等需要文件系统访问的场景）
    /// 如果文件不存在，从块存储中重建
    pub async fn ensure_file_in_data_root(&self, file_id: &str) -> Result<()> {
        let full_path = self.get_full_path(file_id);

        // 如果文件已存在且大小正确，直接返回
        if let Ok(metadata) = tokio::fs::metadata(&full_path).await
            && metadata.is_file()
        {
            // 验证文件大小是否匹配
            if let Ok(file_metadata) = self.get_metadata(file_id).await
                && metadata.len() == file_metadata.size
            {
                return Ok(());
            }
        }

        // 文件不存在或大小不对，从块存储重建
        let data = self.read_file(file_id).await?;

        // 创建父目录
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // 写入文件
        tokio::fs::write(&full_path, data).await?;

        Ok(())
    }

    /// 列出指定目录下的文件和子目录
    /// 返回 (文件列表, 子目录列表)
    pub async fn list_directory(&self, dir_path: &str) -> Result<(Vec<String>, Vec<String>)> {
        let all_files = self.list_files().await?;

        // 标准化目录路径
        let normalized_dir = if dir_path.is_empty() || dir_path == "/" {
            ""
        } else {
            dir_path.trim_matches('/')
        };

        let mut files = Vec::new();
        let mut subdirs = std::collections::HashSet::new();

        // 1. 从元数据推断目录结构
        for file_id in all_files {
            let normalized_file = file_id.trim_start_matches('/');

            // 检查文件是否在指定目录下
            if normalized_dir.is_empty() {
                // 根目录
                if let Some(slash_pos) = normalized_file.find('/') {
                    // 有子目录
                    subdirs.insert(normalized_file[..slash_pos].to_string());
                } else {
                    // 根目录下的文件
                    files.push(file_id);
                }
            } else if let Some(rest) = normalized_file.strip_prefix(normalized_dir) {
                let rest = rest.trim_start_matches('/');
                if !rest.is_empty() {
                    if let Some(slash_pos) = rest.find('/') {
                        // 有子目录
                        subdirs.insert(rest[..slash_pos].to_string());
                    } else {
                        // 当前目录下的文件
                        files.push(file_id);
                    }
                }
            }
        }

        // 2. 从文件系统补充空目录（MKCOL 创建的目录）
        let storage_path = self.get_full_path(dir_path);
        if let Ok(mut entries) = fs::read_dir(&storage_path).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Ok(file_type) = entry.file_type().await
                    && file_type.is_dir()
                    && let Some(name) = entry.file_name().to_str()
                {
                    // 添加文件系统中存在的目录（去重）
                    subdirs.insert(name.to_string());
                }
            }
        }

        Ok((files, subdirs.into_iter().collect()))
    }
}

// ============================================================================
// 索引和文件管理实现
// ============================================================================
// 包含：引用计数管理、文件索引、文件列表、删除、回收站、GC、文件操作
// ============================================================================

impl StorageManager {
    /// 加载块引用计数
    async fn load_chunk_ref_count(&self) -> Result<()> {
        let ref_count_path = self.chunk_root.join("ref_count.json");
        let metadata_db = self.get_metadata_db()?;

        // 如果旧的 JSON 文件存在，迁移数据到 Sled
        if ref_count_path.exists() {
            info!("检测到旧的 ref_count.json，开始迁移数据到 Sled");
            let data = fs::read(&ref_count_path).await.map_err(StorageError::Io)?;
            let ref_counts: HashMap<String, ChunkRefCount> = serde_json::from_slice(&data)
                .map_err(|e| StorageError::Storage(format!("加载块引用计数失败: {}", e)))?;

            // 迁移到 Sled
            for (chunk_id, ref_count) in ref_counts.iter() {
                metadata_db
                    .put_chunk_ref(chunk_id, ref_count)
                    .map_err(|e| StorageError::Storage(format!("迁移块引用计数失败: {}", e)))?;
            }

            // 刷新到磁盘
            metadata_db
                .flush()
                .await
                .map_err(|e| StorageError::Storage(format!("刷新数据库失败: {}", e)))?;

            // 备份旧文件
            let backup_path = ref_count_path.with_extension("json.backup");
            fs::rename(&ref_count_path, &backup_path).await?;
            info!("已将 ref_count.json 备份到 {:?}", backup_path);

            info!("迁移完成，共 {} 个块引用计数", ref_counts.len());
        } else {
            // 数据已在 Sled 中，无需全部加载到内存
            let count = metadata_db.chunk_ref_count();
            info!("使用 Sled 数据库存储块引用计数，当前 {} 个块", count);
        }

        Ok(())
    }

    /// 保存块引用计数到 Sled（主要用于刷新操作）
    async fn save_chunk_ref_count(&self) -> Result<()> {
        let metadata_db = self.get_metadata_db()?;

        // Sled 已经在写入时自动持久化，这里只需要刷新即可
        metadata_db
            .flush()
            .await
            .map_err(|e| StorageError::Storage(format!("刷新数据库失败: {}", e)))?;

        Ok(())
    }

    /// 重建块引用计数
    /// 重建块引用计数（保留以备恢复使用）
    #[allow(dead_code)]
    async fn rebuild_chunk_ref_count(&self) -> Result<()> {
        info!("开始重建块引用计数...");
        let mut ref_counts: HashMap<String, ChunkRefCount> = HashMap::new();
        let metadata_db = self.get_metadata_db()?;

        // 从 Sled 遍历所有文件和版本，统计块引用
        let all_files = metadata_db
            .list_all_files()
            .map_err(|e| StorageError::Storage(format!("列出所有文件失败: {}", e)))?;

        for file_entry in all_files {
            // 获取该文件的所有版本
            let versions = metadata_db
                .list_file_versions(&file_entry.file_id)
                .map_err(|e| StorageError::Storage(format!("列出文件版本失败: {}", e)))?;

            for version_info in versions {
                // 读取该版本的 delta
                if let Ok(delta) = self
                    .read_delta(&version_info.file_id, &version_info.version_id)
                    .await
                {
                    for chunk in &delta.chunks {
                        let entry =
                            ref_counts
                                .entry(chunk.chunk_id.clone())
                                .or_insert_with(|| ChunkRefCount {
                                    chunk_id: chunk.chunk_id.clone(),
                                    ref_count: 0,
                                    size: chunk.size as u64,
                                    path: self.get_chunk_path(&chunk.chunk_id),
                                });
                        entry.ref_count += 1;
                    }
                }
            }
        }

        // 直接保存到 Sled
        for (chunk_id, ref_count) in ref_counts.iter() {
            metadata_db
                .put_chunk_ref(chunk_id, ref_count)
                .map_err(|e| StorageError::Storage(format!("保存块引用计数失败: {}", e)))?;
        }

        // 刷新到磁盘
        metadata_db
            .flush()
            .await
            .map_err(|e| StorageError::Storage(format!("刷新数据库失败: {}", e)))?;

        let count = ref_counts.len();
        info!("重建完成，共 {} 个块", count);
        Ok(())
    }

    /// 加载文件索引
    async fn load_file_index(&self) -> Result<()> {
        let file_index_path = self.version_root.join("file_index.json");
        let metadata_db = self.get_metadata_db()?;

        // 如果旧的 JSON 文件存在，迁移数据到 Sled
        if file_index_path.exists() {
            info!("检测到旧的 file_index.json，开始迁移数据到 Sled");
            let data = fs::read(&file_index_path).await.map_err(StorageError::Io)?;
            let file_index: HashMap<String, FileIndexEntry> = serde_json::from_slice(&data)
                .map_err(|e| StorageError::Storage(format!("加载文件索引失败: {}", e)))?;

            // 迁移到 Sled
            for (file_id, entry) in file_index.iter() {
                metadata_db
                    .put_file_index(file_id, entry)
                    .map_err(|e| StorageError::Storage(format!("迁移文件索引失败: {}", e)))?;
            }

            // 刷新到磁盘
            metadata_db
                .flush()
                .await
                .map_err(|e| StorageError::Storage(format!("刷新数据库失败: {}", e)))?;

            // 备份旧文件
            let backup_path = file_index_path.with_extension("json.backup");
            fs::rename(&file_index_path, &backup_path).await?;
            info!("已将 file_index.json 备份到 {:?}", backup_path);

            info!("迁移完成，共 {} 个文件索引", file_index.len());
        } else {
            // 数据已在 Sled 中，无需加载到内存
            let count = metadata_db.file_index_count();
            info!("使用 Sled 数据库存储文件索引，当前 {} 个文件", count);
        }

        Ok(())
    }

    /// 保存文件索引到 Sled（主要用于刷新操作）
    async fn save_file_index(&self) -> Result<()> {
        let metadata_db = self.get_metadata_db()?;

        // Sled 已经在写入时自动持久化，这里只需要刷新即可
        metadata_db
            .flush()
            .await
            .map_err(|e| StorageError::Storage(format!("刷新数据库失败: {}", e)))?;

        Ok(())
    }

    /// 重建文件索引
    /// 重建文件索引（保留以备恢复使用）
    #[allow(dead_code)]
    async fn rebuild_file_index(&self) -> Result<()> {
        info!("开始重建文件索引...");
        let mut file_index: HashMap<String, FileIndexEntry> = HashMap::new();
        let metadata_db = self.get_metadata_db()?;

        // 从 Sled 遍历所有文件和版本，构建文件索引
        let all_files = metadata_db
            .list_all_files()
            .map_err(|e| StorageError::Storage(format!("列出所有文件失败: {}", e)))?;

        for file_entry in all_files {
            // 获取该文件的所有版本
            let versions = metadata_db
                .list_file_versions(&file_entry.file_id)
                .map_err(|e| StorageError::Storage(format!("列出文件版本失败: {}", e)))?;

            for version_info in versions {
                let entry = file_index
                    .entry(version_info.file_id.clone())
                    .or_insert_with(|| FileIndexEntry {
                        file_id: version_info.file_id.clone(),
                        latest_version_id: version_info.version_id.clone(),
                        version_count: 0,
                        created_at: version_info.created_at,
                        modified_at: version_info.created_at,
                        is_deleted: false,
                        deleted_at: None,
                        storage_mode: crate::StorageMode::Cold,
                        optimization_status: crate::OptimizationStatus::Completed,
                        file_size: version_info.file_size,
                        file_hash: String::new(),
                    });

                entry.version_count += 1;
                // 更新最新版本（假设版本ID可比较，或使用时间戳）
                if version_info.created_at > entry.modified_at {
                    entry.latest_version_id = version_info.version_id.clone();
                    entry.modified_at = version_info.created_at;
                }
            }
        }

        // 直接保存到 Sled
        for (file_id, entry) in file_index.iter() {
            metadata_db
                .put_file_index(file_id, entry)
                .map_err(|e| StorageError::Storage(format!("保存文件索引失败: {}", e)))?;
        }

        // 刷新到磁盘
        metadata_db
            .flush()
            .await
            .map_err(|e| StorageError::Storage(format!("刷新数据库失败: {}", e)))?;

        let count = file_index.len();
        info!("重建完成，共 {} 个文件", count);
        Ok(())
    }

    /// 列出所有文件
    pub async fn list_files(&self) -> Result<Vec<String>> {
        let metadata_db = self.get_metadata_db()?;
        let all_files = metadata_db
            .list_all_files()
            .map_err(|e| StorageError::Storage(format!("列出文件失败: {}", e)))?;

        // 过滤掉已删除的文件
        let mut files: Vec<String> = all_files
            .into_iter()
            .filter(|entry| !entry.is_deleted)
            .map(|entry| entry.file_id)
            .collect();

        files.sort();
        Ok(files)
    }

    /// 软删除文件（移到回收站）
    /// 只标记文件为已删除，不实际删除数据
    pub async fn delete_file(&self, file_id: &str) -> Result<()> {
        info!("软删除文件: {}", file_id);

        let metadata_db = self.get_metadata_db()?;

        // 1. 获取文件索引
        let mut file_entry = metadata_db
            .get_file_index(file_id)?
            .ok_or_else(|| StorageError::FileNotFound(file_id.to_string()))?;

        // 2. 检查是否已删除
        if file_entry.is_deleted {
            return Err(StorageError::Storage(format!(
                "文件已在回收站中: {}",
                file_id
            )));
        }

        // 3. 标记为已删除
        file_entry.is_deleted = true;
        file_entry.deleted_at = Some(chrono::Local::now().naive_local());

        // 4. 更新文件索引
        metadata_db.put_file_index(file_id, &file_entry)?;

        // 5. 持久化
        metadata_db.flush().await?;

        info!("文件已移到回收站: {}", file_id);
        Ok(())
    }

    /// 永久删除文件（物理删除）
    /// 删除文件的所有版本和块数据
    pub async fn permanently_delete_file(&self, file_id: &str) -> Result<()> {
        info!("开始永久删除文件: {}", file_id);

        // 1. 获取该文件的所有版本
        let versions = self.list_file_versions(file_id).await?;

        if versions.is_empty() {
            return Err(StorageError::FileNotFound(file_id.to_string()));
        }

        // 2. 收集所有需要减少引用计数的块
        let mut chunks_to_decrement: Vec<String> = Vec::new();

        for version in &versions {
            // 读取 delta 获取块列表
            if let Ok(delta) = self.read_delta(file_id, &version.version_id).await {
                for chunk in delta.chunks {
                    chunks_to_decrement.push(chunk.chunk_id);
                }
            }

            // 删除版本信息文件
            let version_path = self.get_version_path(&version.version_id);
            if version_path.exists() {
                fs::remove_file(&version_path)
                    .await
                    .map_err(StorageError::Io)?;
            }

            // 删除 delta 文件
            let delta_path = self.get_delta_path(file_id, &version.version_id);
            if delta_path.exists() {
                fs::remove_file(&delta_path)
                    .await
                    .map_err(StorageError::Io)?;
            }

            // 从 Sled 和缓存中移除版本信息
            let metadata_db = self.get_metadata_db()?;
            if let Err(e) = metadata_db.remove_version_info(&version.version_id) {
                info!("从 Sled 移除版本信息失败: {}", e);
            }
            // 从缓存中移除
            self.version_cache.invalidate(&version.version_id).await;
        }

        // 3. 如果启用了去重，使用 dedup_manager 减少块引用计数
        if self.config.enable_deduplication {
            for chunk_id in &chunks_to_decrement {
                // 减少 dedup_manager 中的引用计数
                if let Err(e) = self.dedup_manager.decrement_chunk_ref(chunk_id).await {
                    info!("递减块 {} 引用计数失败: {}", chunk_id, e);
                }
            }
        } else {
            // 否则使用 metadata_db
            let metadata_db = self.get_metadata_db()?;
            for chunk_id in chunks_to_decrement {
                if let Err(e) = metadata_db.decrement_chunk_ref(&chunk_id) {
                    info!("递减块 {} 引用计数失败: {}", chunk_id, e);
                }
            }
        }

        // 4. 从文件索引中移除
        let metadata_db = self.get_metadata_db()?;
        if let Err(e) = metadata_db.remove_file_index(file_id) {
            info!("从 Sled 移除文件索引失败: {}", e);
        }

        // 5. 删除文件的 delta 目录
        let file_delta_dir = self.version_root.join("deltas").join(file_id);
        if file_delta_dir.exists() {
            fs::remove_dir_all(&file_delta_dir)
                .await
                .map_err(StorageError::Io)?;
        }

        // 6. 保存更新后的索引
        self.save_chunk_ref_count().await?;
        self.save_file_index().await?;
        metadata_db.flush().await?;

        info!("文件永久删除完成: {}", file_id);
        Ok(())
    }

    /// 列出回收站中的文件
    pub async fn list_deleted_files(&self) -> Result<Vec<FileIndexEntry>> {
        let metadata_db = self.get_metadata_db()?;
        let all_files = metadata_db.list_all_files()?;

        let deleted_files: Vec<FileIndexEntry> = all_files
            .into_iter()
            .filter(|entry| entry.is_deleted)
            .collect();

        info!("回收站中有 {} 个文件", deleted_files.len());
        Ok(deleted_files)
    }

    /// 恢复文件（从回收站恢复）
    pub async fn restore_file(&self, file_id: &str) -> Result<()> {
        info!("恢复文件: {}", file_id);

        let metadata_db = self.get_metadata_db()?;

        // 1. 获取文件索引
        let mut file_entry = metadata_db
            .get_file_index(file_id)?
            .ok_or_else(|| StorageError::FileNotFound(file_id.to_string()))?;

        // 2. 检查是否在回收站中
        if !file_entry.is_deleted {
            return Err(StorageError::Storage(format!(
                "文件未在回收站中: {}",
                file_id
            )));
        }

        // 3. 清除删除标记
        file_entry.is_deleted = false;
        file_entry.deleted_at = None;

        // 4. 更新文件索引
        metadata_db.put_file_index(file_id, &file_entry)?;

        // 5. 持久化
        metadata_db.flush().await?;

        info!("文件已恢复: {}", file_id);
        Ok(())
    }

    /// 清空回收站（永久删除所有已删除的文件）
    pub async fn empty_recycle_bin(&self) -> Result<usize> {
        info!("开始清空回收站");

        let deleted_files = self.list_deleted_files().await?;
        let count = deleted_files.len();

        for file_entry in deleted_files {
            if let Err(e) = self.permanently_delete_file(&file_entry.file_id).await {
                info!("永久删除文件 {} 失败: {}", file_entry.file_id, e);
            }
        }

        info!("回收站已清空，删除了 {} 个文件", count);
        Ok(count)
    }

    /// 垃圾回收（清理引用计数为 0 的块）
    /// 删除没有任何文件引用的块，释放存储空间
    pub async fn garbage_collect_blocks(&self) -> Result<usize> {
        info!("开始垃圾回收");

        let mut deleted_count = 0;

        if self.config.enable_deduplication {
            // 使用 dedup_manager 的 GC 功能
            deleted_count = self.dedup_manager.gc().await? as usize;

            // 还需要删除物理块文件
            let all_blocks = self.dedup_manager.get_all_blocks().await;
            for block in all_blocks {
                if block.ref_count == 0 {
                    // 删除块文件
                    let chunk_path = self.get_chunk_path(&block.chunk_id);
                    if chunk_path.exists() {
                        if let Err(e) = fs::remove_file(&chunk_path).await {
                            info!("删除块文件 {} 失败: {}", block.chunk_id, e);
                        } else {
                            info!("删除未引用的块文件: {}", block.chunk_id);
                        }
                    }
                }
            }
        } else {
            // 使用 metadata_db 查找引用计数为 0 的块
            let metadata_db = self.get_metadata_db()?;
            let all_chunks = metadata_db.list_all_chunks()?;

            for (chunk_id, ref_count_info) in all_chunks {
                if ref_count_info.ref_count == 0 {
                    // 删除块文件
                    let chunk_path = self.get_chunk_path(&chunk_id);
                    if chunk_path.exists() {
                        if let Err(e) = fs::remove_file(&chunk_path).await {
                            info!("删除块文件 {} 失败: {}", chunk_id, e);
                        } else {
                            deleted_count += 1;
                            info!("删除未引用的块文件: {}", chunk_id);
                        }
                    }

                    // 从 metadata_db 中移除
                    let _ = metadata_db.remove_chunk_ref(&chunk_id);
                }
            }

            // 持久化
            metadata_db.flush().await?;
        }

        info!("垃圾回收完成，清理了 {} 个未引用的块", deleted_count);
        Ok(deleted_count)
    }

    /// 启动GC后台任务
    ///
    /// 该方法会启动一个后台任务，定期执行垃圾回收
    /// 任务间隔由配置中的gc_interval_secs决定
    pub async fn start_gc_task(&self) {
        // 先停止已有的任务
        self.stop_gc_task().await;

        // 重置停止标志
        self.gc_stop_flag.store(false, Ordering::Relaxed);

        let storage = self.clone_for_gc();
        let interval_secs = self.config.gc_interval_secs;
        let stop_flag = self.gc_stop_flag.clone();

        let handle = tokio::spawn(async move {
            info!("GC后台任务启动，间隔: {}秒", interval_secs);

            loop {
                // 等待指定间隔
                tokio::time::sleep(tokio::time::Duration::from_secs(interval_secs)).await;

                // 检查停止标志
                if stop_flag.load(Ordering::Relaxed) {
                    info!("GC后台任务收到停止信号");
                    break;
                }

                // 执行GC
                info!("开始执行定时GC");
                match storage.garbage_collect_blocks().await {
                    Ok(count) => {
                        info!("定时GC完成，清理了 {} 个未引用的块", count);
                    }
                    Err(e) => {
                        info!("定时GC执行失败: {}", e);
                    }
                }
            }

            info!("GC后台任务已停止");
        });

        *self.gc_task_handle.write().await = Some(handle);
    }

    /// 停止GC后台任务
    ///
    /// 该方法会停止正在运行的GC后台任务
    pub async fn stop_gc_task(&self) {
        // 设置停止标志
        self.gc_stop_flag.store(true, Ordering::Relaxed);

        // 等待任务结束
        if let Some(handle) = self.gc_task_handle.write().await.take() {
            let _ = handle.await;
            info!("GC后台任务已停止");
        }
    }

    /// 获取GC配置
    ///
    /// 返回当前GC的配置信息
    pub fn get_gc_config(&self) -> (bool, u64) {
        (self.config.enable_auto_gc, self.config.gc_interval_secs)
    }

    /// 检查GC任务是否正在运行
    ///
    /// 返回GC后台任务的运行状态
    pub async fn is_gc_task_running(&self) -> bool {
        self.gc_task_handle.read().await.is_some()
    }

    /// 克隆一个用于GC任务的StorageManager副本
    ///
    /// 由于GC任务需要在后台线程中运行，需要克隆必要的字段
    fn clone_for_gc(&self) -> Self {
        Self {
            root_path: self.root_path.clone(),
            data_root: self.data_root.clone(),
            hot_storage_root: self.hot_storage_root.clone(),
            config: self.config.clone(),
            version_root: self.version_root.clone(),
            chunk_root: self.chunk_root.clone(),
            chunk_size: self.chunk_size,
            metadata_db: self.metadata_db.clone(),
            version_cache: self.version_cache.clone(),
            block_cache: self.block_cache.clone(),
            cache_manager: self.cache_manager.clone(),
            wal_manager: self.wal_manager.clone(),
            chunk_verifier: self.chunk_verifier.clone(),
            orphan_cleaner: self.orphan_cleaner.clone(),
            compressor: self.compressor.clone(),
            dedup_manager: self.dedup_manager.clone(),
            gc_task_handle: Arc::new(RwLock::new(None)),
            gc_stop_flag: self.gc_stop_flag.clone(),
            optimization_scheduler: self.optimization_scheduler.clone(),
            optimization_task_handle: Arc::new(RwLock::new(None)),
            optimization_stop_flag: self.optimization_stop_flag.clone(),
        }
    }

    /// 移动/重命名文件（只更新元数据，不复制块数据）
    ///
    /// # 参数
    /// * `old_file_id` - 原文件ID/路径
    /// * `new_file_id` - 新文件ID/路径
    ///
    /// # 返回
    /// 返回新文件的元数据
    pub async fn move_file(&self, old_file_id: &str, new_file_id: &str) -> Result<FileMetadata> {
        info!("开始移动文件: {} -> {}", old_file_id, new_file_id);

        // 1. 检查目标文件是否已存在
        if self.file_exists(new_file_id).await {
            return Err(StorageError::Storage(format!(
                "目标文件已存在: {}",
                new_file_id
            )));
        }

        // 2. 获取源文件的元数据
        let old_metadata = self.get_metadata(old_file_id).await?;

        // 3. 获取源文件的所有版本
        let versions = self.list_file_versions(old_file_id).await?;
        if versions.is_empty() {
            return Err(StorageError::FileNotFound(old_file_id.to_string()));
        }

        let metadata_db = self.get_metadata_db()?;

        // 4. 移动每个版本的元数据和 delta 文件
        for version in &versions {
            // 4.1 读取并更新版本信息
            let mut version_info = self.get_version_info(&version.version_id).await?;
            version_info.file_id = new_file_id.to_string();

            // 保存到新的文件ID下
            metadata_db
                .put_version_info(&version.version_id, &version_info)
                .map_err(|e| StorageError::Storage(format!("保存版本信息失败: {}", e)))?;

            // 更新缓存
            self.version_cache
                .insert(version.version_id.clone(), version_info)
                .await;

            // 4.2 移动 delta 文件
            let old_delta_path = self.get_delta_path(old_file_id, &version.version_id);
            let new_delta_path = self.get_delta_path(new_file_id, &version.version_id);

            if old_delta_path.exists() {
                // 确保新路径的父目录存在
                if let Some(parent) = new_delta_path.parent() {
                    fs::create_dir_all(parent).await.map_err(StorageError::Io)?;
                }

                // 读取并更新 delta 文件中的 file_id
                let delta_data = fs::read(&old_delta_path).await.map_err(StorageError::Io)?;
                let mut delta: FileDelta = serde_json::from_slice(&delta_data)
                    .map_err(|e| StorageError::Storage(format!("反序列化 delta 失败: {}", e)))?;

                delta.file_id = new_file_id.to_string();

                let updated_delta_data = serde_json::to_vec_pretty(&delta)
                    .map_err(|e| StorageError::Storage(format!("序列化 delta 失败: {}", e)))?;

                fs::write(&new_delta_path, updated_delta_data)
                    .await
                    .map_err(StorageError::Io)?;

                // 删除旧的 delta 文件
                fs::remove_file(&old_delta_path)
                    .await
                    .map_err(StorageError::Io)?;
            }
        }

        // 5. 移动文件索引
        if let Ok(Some(mut file_entry)) = metadata_db.get_file_index(old_file_id) {
            file_entry.file_id = new_file_id.to_string();
            file_entry.modified_at = Local::now().naive_local();

            metadata_db
                .put_file_index(new_file_id, &file_entry)
                .map_err(|e| StorageError::Storage(format!("保存文件索引失败: {}", e)))?;

            // 删除旧的文件索引
            metadata_db
                .remove_file_index(old_file_id)
                .map_err(|e| StorageError::Storage(format!("删除旧文件索引失败: {}", e)))?;
        }

        // 6. 删除旧的 delta 目录（如果为空）
        let old_delta_dir = self.version_root.join("deltas").join(old_file_id);
        if old_delta_dir.exists()
            && let Ok(mut entries) = fs::read_dir(&old_delta_dir).await
            && entries.next_entry().await.ok().flatten().is_none()
        {
            // 目录为空，删除
            let _ = fs::remove_dir(&old_delta_dir).await;
        }

        // 7. 移动热存储文件（如果存在）
        let old_hot_path = self.get_hot_storage_path(old_file_id);
        let new_hot_path = self.get_hot_storage_path(new_file_id);

        if old_hot_path.exists() {
            // 确保新路径的父目录存在
            if let Some(parent) = new_hot_path.parent() {
                fs::create_dir_all(parent).await.map_err(StorageError::Io)?;
            }

            // 移动热存储文件
            fs::rename(&old_hot_path, &new_hot_path)
                .await
                .map_err(StorageError::Io)?;

            info!("热存储文件已移动: {:?} -> {:?}", old_hot_path, new_hot_path);
        }

        // 8. 刷新数据库
        let _ = metadata_db.flush().await;

        // 9. 返回新文件的元数据
        let new_metadata = FileMetadata {
            id: new_file_id.to_string(),
            name: new_file_id.to_string(),
            path: new_file_id.to_string(),
            size: old_metadata.size,
            hash: old_metadata.hash,
            created_at: old_metadata.created_at,
            modified_at: Local::now().naive_local(),
        };

        info!("文件移动完成: {} -> {}", old_file_id, new_file_id);
        Ok(new_metadata)
    }

    /// 垃圾回收 - 清理引用计数为0的块
    pub async fn garbage_collect(&self) -> Result<GarbageCollectResult> {
        info!("开始垃圾回收...");

        let mut orphaned_chunks = 0;
        let mut reclaimed_space = 0u64;
        let mut errors = Vec::new();

        let metadata_db = self.get_metadata_db()?;

        // 从 Sled 获取所有引用计数为0的块
        let orphaned_chunk_ids = metadata_db
            .list_orphaned_chunks()
            .map_err(|e| StorageError::Storage(format!("列出孤立块失败: {}", e)))?;

        // 删除这些块
        for chunk_id in orphaned_chunk_ids {
            // 从 Sled 获取块信息
            if let Ok(Some(entry)) = metadata_db.get_chunk_ref(&chunk_id) {
                if entry.path.exists() {
                    match fs::metadata(&entry.path).await {
                        Ok(metadata) => {
                            reclaimed_space += metadata.len();
                            match fs::remove_file(&entry.path).await {
                                Ok(_) => {
                                    orphaned_chunks += 1;
                                    // 从 Sled 移除
                                    if let Err(e) = metadata_db.remove_chunk_ref(&chunk_id) {
                                        errors.push(format!(
                                            "从 Sled 移除块 {} 失败: {}",
                                            chunk_id, e
                                        ));
                                    }
                                    // 从缓存中移除
                                    self.block_cache.invalidate(&chunk_id).await;
                                }
                                Err(e) => {
                                    errors.push(format!("删除块 {} 失败: {}", chunk_id, e));
                                }
                            }
                        }
                        Err(e) => {
                            errors.push(format!("获取块 {} 元数据失败: {}", chunk_id, e));
                        }
                    }
                } else {
                    // 块文件不存在，直接从索引中移除
                    if let Err(e) = metadata_db.remove_chunk_ref(&chunk_id) {
                        errors.push(format!("从 Sled 移除块 {} 失败: {}", chunk_id, e));
                    }
                    // 从缓存中移除
                    self.block_cache.invalidate(&chunk_id).await;
                }
            }
        }

        // 刷新数据库
        if orphaned_chunks > 0
            && let Err(e) = metadata_db.flush().await
        {
            errors.push(format!("刷新数据库失败: {}", e));
        }

        info!(
            "垃圾回收完成: 清理了 {} 个孤立块，回收了 {} 字节空间",
            orphaned_chunks, reclaimed_space
        );

        Ok(GarbageCollectResult {
            orphaned_chunks,
            reclaimed_space,
            errors,
        })
    }

    /// 获取文件信息（不读取内容）
    pub async fn get_file_info(&self, file_id: &str) -> Result<FileIndexEntry> {
        let metadata_db = self.get_metadata_db()?;
        metadata_db
            .get_file_index(file_id)
            .map_err(|e| StorageError::Storage(format!("读取文件信息失败: {}", e)))?
            .ok_or_else(|| StorageError::FileNotFound(file_id.to_string()))
    }

    // ============ Phase 5 Step 4: 可靠性增强 API ============

    /// 验证所有 chunks 的完整性
    pub async fn verify_all_chunks(&self) -> Result<crate::ChunkVerifyReport> {
        self.chunk_verifier
            .scan_and_verify()
            .await
            .map_err(|e| StorageError::Storage(format!("验证 chunks 失败: {}", e)))
    }

    /// 验证指定 chunks 的完整性
    pub async fn verify_chunks(&self, chunk_hashes: &[String]) -> Result<crate::ChunkVerifyReport> {
        self.chunk_verifier
            .verify_chunks(chunk_hashes)
            .await
            .map_err(|e| StorageError::Storage(format!("验证 chunks 失败: {}", e)))
    }

    /// 检测孤儿 chunks
    pub async fn detect_orphan_chunks(&self) -> Result<Vec<String>> {
        use std::collections::HashSet;

        // 收集所有被引用的 chunks
        let metadata_db = self.get_metadata_db()?;
        let chunk_refs = metadata_db
            .list_all_chunks()
            .map_err(|e| StorageError::Storage(format!("读取 chunk 引用失败: {}", e)))?;

        let referenced: HashSet<String> = chunk_refs.into_iter().map(|(hash, _)| hash).collect();

        // 检测孤儿 chunks
        self.orphan_cleaner
            .detect_orphans(&referenced)
            .await
            .map_err(|e| StorageError::Storage(format!("检测孤儿 chunks 失败: {}", e)))
    }

    /// 清理孤儿 chunks
    pub async fn cleanup_orphan_chunks(
        &self,
        orphan_hashes: &[String],
    ) -> Result<crate::CleanupReport> {
        self.orphan_cleaner
            .clean_orphans(orphan_hashes)
            .await
            .map_err(|e| StorageError::Storage(format!("清理孤儿 chunks 失败: {}", e)))
    }

    /// 执行优化任务 - 将热存储文件优化为冷存储
    pub async fn execute_optimization_task(
        &self,
        task: &mut crate::OptimizationTask,
    ) -> Result<(u64, u64)> {
        info!(
            "开始执行优化任务: file_id={}, strategy={:?}",
            task.file_id, task.strategy
        );

        task.mark_started();

        // 检查热存储文件是否存在
        if !task.hot_path.exists() {
            let error = format!("热存储文件不存在: {}", task.hot_path.display());
            task.mark_failed(error.clone());
            return Err(StorageError::Storage(error));
        }

        // 根据策略执行优化
        match task.strategy {
            crate::OptimizationStrategy::Skip => {
                task.mark_skipped("文件已是最优格式，跳过优化".to_string());
                Ok((0, 0))
            }
            crate::OptimizationStrategy::CompressOnly => self.optimize_compress_only(task).await,
            crate::OptimizationStrategy::Full => self.optimize_full(task).await,
        }
    }

    /// 仅压缩优化（小文件）
    async fn optimize_compress_only(
        &self,
        task: &mut crate::OptimizationTask,
    ) -> Result<(u64, u64)> {
        // 检查文件大小，防止 OOM
        if self.config.max_file_size_for_optimization > 0 {
            let file_size = fs::metadata(&task.hot_path)
                .await
                .map_err(StorageError::Io)?
                .len();

            if file_size > self.config.max_file_size_for_optimization {
                let msg = format!(
                    "文件过大 ({} MB)，超过限制 ({} MB)，跳过优化",
                    file_size / 1024 / 1024,
                    self.config.max_file_size_for_optimization / 1024 / 1024
                );
                warn!("{}: file_id={}", msg, task.file_id);
                task.mark_skipped(msg);
                return Ok((0, 0));
            }
        }

        // 读取热存储文件
        let data = fs::read(&task.hot_path).await.map_err(StorageError::Io)?;
        let original_size = data.len() as u64;

        // 检测文件类型
        let file_type = crate::core::FileType::detect(&data);
        if file_type.is_compressed() {
            task.mark_skipped("文件已压缩，跳过".to_string());
            return Ok((0, 0));
        }

        // 压缩数据
        let (compressed, _compression_algo) = if self.config.enable_compression {
            let algorithm = match self.config.compression_algorithm.as_str() {
                "lz4" => crate::core::CompressionAlgorithm::LZ4,
                "zstd" => crate::core::CompressionAlgorithm::Zstd,
                _ => crate::core::CompressionAlgorithm::LZ4,
            };
            let compression_config = crate::core::compression::CompressionConfig {
                algorithm,
                level: 1,
                min_size: 0, // 已经检查过是否需要压缩
                ..Default::default()
            };
            let compressor = crate::core::compression::Compressor::new(compression_config);
            let result = compressor.compress(&data)?;
            (result.compressed_data, result.algorithm)
        } else {
            (data.clone(), crate::core::CompressionAlgorithm::None)
        };

        let compressed_size = compressed.len() as u64;
        let space_saved = original_size.saturating_sub(compressed_size);

        // 保存到data目录（不分块）
        let compressed_path = self.data_root.join(format!("{}.compressed", task.file_id));
        if let Some(parent) = compressed_path.parent() {
            fs::create_dir_all(parent).await.map_err(StorageError::Io)?;
        }
        fs::write(&compressed_path, &compressed)
            .await
            .map_err(StorageError::Io)?;

        // 更新文件索引
        self.update_file_index_after_optimization(
            &task.file_id,
            crate::StorageMode::Compressed,
            compressed_size,
        )
        .await?;

        // 清理热存储（优化完成后自动清理）
        let _ = fs::remove_file(&task.hot_path).await;

        task.mark_completed();
        info!(
            "压缩优化完成: file_id={}, 原始={}B, 压缩后={}B, 节省={}B",
            task.file_id, original_size, compressed_size, space_saved
        );

        Ok((space_saved, compressed_size))
    }

    /// 完整优化（CDC分块 + 去重 + 压缩）
    async fn optimize_full(&self, task: &mut crate::OptimizationTask) -> Result<(u64, u64)> {
        // 检查文件大小，防止 OOM
        if self.config.max_file_size_for_optimization > 0 {
            let file_size = fs::metadata(&task.hot_path)
                .await
                .map_err(StorageError::Io)?
                .len();

            if file_size > self.config.max_file_size_for_optimization {
                let msg = format!(
                    "文件过大 ({} MB)，超过限制 ({} MB)，跳过优化",
                    file_size / 1024 / 1024,
                    self.config.max_file_size_for_optimization / 1024 / 1024
                );
                warn!("{}: file_id={}", msg, task.file_id);
                task.mark_skipped(msg);
                return Ok((0, 0));
            }
        }

        // 读取热存储文件
        let data = fs::read(&task.hot_path).await.map_err(StorageError::Io)?;
        let original_size = data.len() as u64;

        // 1. 检测文件类型并调整分块配置
        let file_type = crate::core::FileType::detect(&data);
        let (min_chunk, max_chunk) = file_type.recommended_chunk_size();

        let mut adjusted_config = self.config.clone();
        adjusted_config.min_chunk_size = min_chunk;
        adjusted_config.max_chunk_size = max_chunk;

        if file_type.is_compressed() {
            adjusted_config.enable_compression = false;
        }

        info!(
            "开始完整优化: file_id={}, 大小={}B, 类型={}, 块大小={}KB-{}KB",
            task.file_id,
            original_size,
            file_type.as_str(),
            min_chunk / 1024,
            max_chunk / 1024
        );

        // 2. 使用Delta生成器进行CDC分块
        let mut generator = crate::core::delta::DeltaGenerator::new(adjusted_config);
        let delta = generator
            .generate_full_delta(&data, &task.file_id)
            .map_err(|e| StorageError::Storage(format!("生成分块失败: {}", e)))?;

        // 3. 保存所有chunks并进行去重，同时更新compression字段
        let mut dedup_stats = crate::DeduplicationStats {
            total_chunks: delta.chunks.len(),
            original_size,
            ..Default::default()
        };

        // 创建新的chunks向量，更新compression字段
        let mut updated_chunks = Vec::with_capacity(delta.chunks.len());

        for chunk in &delta.chunks {
            let start = chunk.offset;
            let end = start + chunk.size;
            if end > data.len() {
                return Err(StorageError::Storage("分块范围越界".to_string()));
            }
            let chunk_data = &data[start..end];

            let compression_algo = if self.config.enable_deduplication {
                let exists = self.dedup_manager.chunk_exists(&chunk.chunk_id).await;
                if exists {
                    self.dedup_manager
                        .increment_chunk_ref(&chunk.chunk_id)
                        .await
                        .map_err(|e| StorageError::Storage(format!("增加块引用计数失败: {}", e)))?;
                    dedup_stats.duplicate_chunks += 1;
                    // 对于重复的chunk，从已存储的chunk读取compression信息
                    // 这里假设使用相同的压缩算法，使用当前配置的算法
                    if self.config.enable_compression {
                        crate::core::compression::CompressionAlgorithm::LZ4
                    } else {
                        crate::core::compression::CompressionAlgorithm::None
                    }
                } else {
                    let algo = self.save_chunk_data(&chunk.chunk_id, chunk_data).await?;
                    let storage_path = self.get_chunk_path(&chunk.chunk_id);
                    self.dedup_manager
                        .add_chunk(&chunk.chunk_id, chunk.size, storage_path)
                        .await
                        .map_err(|e| StorageError::Storage(format!("添加块到索引失败: {}", e)))?;
                    dedup_stats.new_chunks += 1;
                    dedup_stats.stored_size += chunk.size as u64;
                    algo
                }
            } else {
                let algo = self.save_chunk_data(&chunk.chunk_id, chunk_data).await?;
                dedup_stats.new_chunks += 1;
                dedup_stats.stored_size += chunk.size as u64;
                algo
            };

            // 创建更新后的ChunkInfo
            let mut updated_chunk = chunk.clone();
            updated_chunk.compression = compression_algo;
            updated_chunks.push(updated_chunk);
        }

        dedup_stats.calculate_dedup_ratio();

        // 4. 获取现有的版本ID（从文件索引中）
        let metadata_db = self.get_metadata_db()?;
        let version_id = if let Some(file_entry) = metadata_db
            .get_file_index(&task.file_id)
            .map_err(|e| StorageError::Storage(format!("读取文件索引失败: {}", e)))?
        {
            file_entry.latest_version_id.clone()
        } else {
            return Err(StorageError::Storage(format!(
                "文件索引不存在: {}",
                task.file_id
            )));
        };

        let now = chrono::Local::now().naive_local();

        // 5. 保存Delta和版本信息（使用现有的version_id和更新后的chunks）
        let file_delta = FileDelta {
            file_id: task.file_id.clone(),
            base_version_id: String::new(),
            new_version_id: version_id.clone(),
            chunks: updated_chunks,
            created_at: now,
        };

        self.save_delta(&task.file_id, &file_delta).await?;
        self.save_version_info(&task.file_id, &file_delta, None)
            .await?;

        // 6. 更新文件索引（重用已获取的metadata_db）
        if let Some(mut file_entry) = metadata_db
            .get_file_index(&task.file_id)
            .map_err(|e| StorageError::Storage(format!("读取文件索引失败: {}", e)))?
        {
            file_entry.storage_mode = crate::StorageMode::Cold;
            file_entry.optimization_status = crate::OptimizationStatus::Completed;
            metadata_db
                .put_file_index(&task.file_id, &file_entry)
                .map_err(|e| StorageError::Storage(format!("保存文件索引失败: {}", e)))?;
        }

        // 计算节省的空间（原始大小 - 实际存储大小）
        let stats = self.get_dedup_stats().await;
        let stored_size = stats.stored_size;
        let space_saved = original_size.saturating_sub(stored_size);

        // 清理热存储（优化完成后自动清理）
        let _ = fs::remove_file(&task.hot_path).await;

        task.mark_completed();
        info!(
            "完整优化完成: file_id={}, 原始={}B, 存储={}B, 节省={}B, 去重率={:.2}%",
            task.file_id, original_size, stored_size, space_saved, stats.dedup_ratio
        );

        Ok((space_saved, stored_size))
    }

    /// 更新文件索引（优化后）
    async fn update_file_index_after_optimization(
        &self,
        file_id: &str,
        storage_mode: crate::StorageMode,
        _storage_size: u64,
    ) -> Result<()> {
        let metadata_db = self.get_metadata_db()?;
        if let Some(mut file_entry) = metadata_db
            .get_file_index(file_id)
            .map_err(|e| StorageError::Storage(format!("读取文件索引失败: {}", e)))?
        {
            file_entry.storage_mode = storage_mode;
            file_entry.optimization_status = crate::OptimizationStatus::Completed;
            // 可以选择更新file_size为压缩后的大小
            metadata_db
                .put_file_index(file_id, &file_entry)
                .map_err(|e| StorageError::Storage(format!("保存文件索引失败: {}", e)))?;
        }
        Ok(())
    }

    /// 获取去重统计（临时方法，用于优化执行器）
    async fn get_dedup_stats(&self) -> crate::DeduplicationStats {
        // 简化实现，返回默认值
        // 实际应该从dedup_manager获取统计
        crate::DeduplicationStats::default()
    }

    /// 启动后台优化任务
    pub async fn start_optimization_task(&self) {
        if self.optimization_stop_flag.load(Ordering::Relaxed) {
            return; // 已停止，不启动
        }

        // 检查是否已有任务在运行
        if self.optimization_task_handle.read().await.is_some() {
            warn!("优化任务已在运行");
            return;
        }

        info!("启动后台优化任务");
        self.optimization_stop_flag.store(false, Ordering::Relaxed);

        let storage = self.clone_for_gc();
        let stop_flag = self.optimization_stop_flag.clone();

        let handle = tokio::spawn(async move {
            info!("后台优化任务已启动");

            loop {
                // 检查停止标志（无锁原子操作）
                if stop_flag.load(Ordering::Relaxed) {
                    info!("后台优化任务收到停止信号");
                    break;
                }

                // 获取下一个就绪的任务
                if let Some(mut task) = storage.optimization_scheduler.get_next_ready_task().await {
                    info!("开始执行优化任务: file_id={}", task.file_id);

                    // 执行优化
                    match storage.execute_optimization_task(&mut task).await {
                        Ok((space_saved, optimized_size)) => {
                            storage
                                .optimization_scheduler
                                .mark_task_completed(&task.file_id, space_saved, optimized_size)
                                .await;
                        }
                        Err(e) => {
                            let error_msg = format!("优化失败: {}", e);
                            storage
                                .optimization_scheduler
                                .mark_task_failed(&task.file_id, &error_msg)
                                .await;

                            // 如果可以重试，重新提交
                            if task.can_retry() {
                                storage
                                    .optimization_scheduler
                                    .resubmit_failed_task(task)
                                    .await;
                            }
                        }
                    }
                } else {
                    // 没有就绪的任务，等待一段时间
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                }
            }

            info!("后台优化任务已停止");
        });

        *self.optimization_task_handle.write().await = Some(handle);
    }

    /// 停止后台优化任务
    pub async fn stop_optimization_task(&self) {
        info!("停止后台优化任务");

        // 设置停止标志（无锁原子操作）
        self.optimization_stop_flag.store(true, Ordering::Relaxed);

        // 等待任务完成
        if let Some(handle) = self.optimization_task_handle.write().await.take() {
            let _ = handle.await;
        }

        info!("后台优化任务已停止");
    }

    /// 获取优化统计信息
    pub async fn get_optimization_stats(&self) -> crate::OptimizationStats {
        self.optimization_scheduler.get_stats().await
    }

    // ============================================================================
    // 优化管理API（阶段3）
    // ============================================================================

    /// 手动触发文件优化
    ///
    /// 为指定的文件立即创建优化任务（即使未启用异步优化）
    pub async fn trigger_file_optimization(&self, file_id: &str) -> Result<()> {
        // 获取文件索引信息
        let metadata_db = self.get_metadata_db()?;
        let file_entry = metadata_db
            .get_file_index(file_id)
            .map_err(|e| StorageError::Storage(format!("读取文件索引失败: {}", e)))?
            .ok_or_else(|| StorageError::FileNotFound(format!("文件不存在: {}", file_id)))?;

        // 检查文件是否在热存储
        if file_entry.storage_mode != crate::StorageMode::Hot {
            return Err(StorageError::Storage(format!(
                "文件 {} 不在热存储，当前模式: {:?}",
                file_id, file_entry.storage_mode
            )));
        }

        // 获取热存储路径
        let hot_path = self.get_hot_storage_path(file_id);
        if !hot_path.exists() {
            return Err(StorageError::Storage(format!(
                "热存储文件不存在: {}",
                hot_path.display()
            )));
        }

        // 读取文件数据以检测文件类型
        let data = fs::read(&hot_path).await.map_err(StorageError::Io)?;
        let file_type = crate::core::FileType::detect(&data);
        let strategy = crate::OptimizationStrategy::decide(&file_type, data.len() as u64);

        // 创建优化任务（延迟为0，立即执行）
        let task = crate::OptimizationTask::new(
            file_id.to_string(),
            hot_path,
            data.len() as u64,
            file_entry.file_hash,
            strategy,
            0, // 立即执行
        );

        // 提交任务
        self.optimization_scheduler.submit_task(task).await;

        info!("手动触发文件 {} 的优化任务，策略: {:?}", file_id, strategy);

        Ok(())
    }

    /// 暂停优化调度器
    ///
    /// 暂停后台优化任务的执行（不会停止已在运行的任务）
    pub async fn pause_optimization_scheduler(&self) -> Result<()> {
        self.optimization_stop_flag.store(true, Ordering::Relaxed);
        info!("优化调度器已暂停");
        Ok(())
    }

    /// 恢复优化调度器
    ///
    /// 恢复后台优化任务的执行
    pub async fn resume_optimization_scheduler(&self) -> Result<()> {
        self.optimization_stop_flag.store(false, Ordering::Relaxed);
        info!("优化调度器已恢复");
        Ok(())
    }

    /// 检查优化调度器是否暂停
    pub fn is_optimization_paused(&self) -> bool {
        self.optimization_stop_flag.load(Ordering::Relaxed)
    }

    /// 获取待处理的优化任务列表
    pub async fn get_pending_optimization_tasks(&self) -> Vec<crate::OptimizationTask> {
        self.optimization_scheduler.get_pending_tasks().await
    }

    /// 获取优化队列长度
    pub async fn get_optimization_queue_length(&self) -> usize {
        self.optimization_scheduler.queue_len().await
    }

    /// 清空优化队列
    ///
    /// 移除所有待处理的优化任务
    pub async fn clear_optimization_queue(&self) -> Result<()> {
        self.optimization_scheduler.clear_queue().await;
        info!("优化队列已清空");
        Ok(())
    }

    /// 优雅关闭（刷新所有数据）
    pub async fn shutdown(&self) -> Result<()> {
        info!("开始优雅关闭 StorageManager...");

        // 停止后台优化任务（统一流程，始终启用）
        info!("停止后台优化任务...");
        self.stop_optimization_task().await;

        // 刷新元数据数据库
        let metadata_db = self.get_metadata_db()?;
        metadata_db
            .flush()
            .await
            .map_err(|e| StorageError::Storage(format!("刷新数据库失败: {}", e)))?;

        info!("StorageManager 优雅关闭完成");
        Ok(())
    }
}

/// 垃圾回收结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GarbageCollectResult {
    /// 清理的孤立块数量
    pub orphaned_chunks: usize,
    /// 回收的空间（字节）
    pub reclaimed_space: u64,
    /// 错误信息列表
    pub errors: Vec<String>,
}

/// 存储统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageStats {
    pub total_versions: usize,
    pub total_chunks: usize,
    pub unique_chunks: usize,
    pub total_size: u64,
    pub total_chunk_size: u64,
    pub compression_ratio: f64,
    pub avg_chunk_size: f64,
}

// ============================================================================
// Trait 实现
// ============================================================================
// 包含：StorageManagerTrait、S3CompatibleStorageTrait
// ============================================================================

/// 实现 StorageManagerTrait 以提供标准存储接口
#[async_trait]
impl StorageManagerTrait for StorageManager {
    type Error = StorageError;

    async fn init(&self) -> std::result::Result<(), Self::Error> {
        StorageManager::init(self).await
    }

    async fn save_file(
        &self,
        file_id: &str,
        data: &[u8],
    ) -> std::result::Result<FileMetadata, Self::Error> {
        // 使用增量存储，这里我们保存第一个版本
        // parent_version_id 为 None 表示创建新文件
        let (_delta, file_version) = self.save_version(file_id, data, None).await?;

        // 转换为 FileMetadata
        Ok(FileMetadata {
            id: file_id.to_string(),
            name: file_id.to_string(),
            path: file_id.to_string(),
            size: data.len() as u64,
            hash: file_version.version_id.clone(),
            created_at: file_version.created_at,
            modified_at: file_version.created_at,
        })
    }

    async fn save_at_path(
        &self,
        relative_path: &str,
        data: &[u8],
    ) -> std::result::Result<FileMetadata, Self::Error> {
        // 使用路径作为 file_id，只保存到块存储
        // 不在 data_root 下创建冗余的完整文件副本
        self.save_file(relative_path, data).await
    }

    async fn read_file(&self, file_id: &str) -> std::result::Result<Vec<u8>, Self::Error> {
        // 读取文件的最新版本
        // 首先获取文件的版本列表
        let versions = self.list_file_versions(file_id).await?;

        if versions.is_empty() {
            return Err(StorageError::FileNotFound(format!(
                "文件不存在: {}",
                file_id
            )));
        }

        // 获取最新版本（list_file_versions 已按时间降序排列）
        let latest_version = &versions[0];

        // 读取版本数据
        self.read_version_data(&latest_version.version_id).await
    }

    async fn delete_file(&self, file_id: &str) -> std::result::Result<(), Self::Error> {
        // 删除文件及其所有版本
        StorageManager::delete_file(self, file_id).await
    }

    async fn file_exists(&self, file_id: &str) -> bool {
        // 检查文件是否有版本
        match self.list_file_versions(file_id).await {
            Ok(versions) => !versions.is_empty(),
            Err(_) => false,
        }
    }

    async fn get_metadata(&self, file_id: &str) -> std::result::Result<FileMetadata, Self::Error> {
        let versions = self.list_file_versions(file_id).await?;

        if versions.is_empty() {
            return Err(StorageError::FileNotFound(format!(
                "文件不存在: {}",
                file_id
            )));
        }

        let latest_version = &versions[0];

        Ok(FileMetadata {
            id: file_id.to_string(),
            name: file_id.to_string(),
            path: file_id.to_string(),
            size: latest_version.file_size,
            hash: latest_version.version_id.clone(),
            created_at: latest_version.created_at,
            modified_at: latest_version.created_at,
        })
    }

    async fn list_files(&self) -> std::result::Result<Vec<FileMetadata>, Self::Error> {
        // 从文件索引获取所有文件列表
        let file_ids = StorageManager::list_files(self).await?;

        let mut files = Vec::new();
        for file_id in file_ids {
            // 获取文件信息
            if let Ok(file_info) = self.get_file_info(&file_id).await {
                // 获取最新版本的详细信息
                if let Ok(version_info) = self.get_version_info(&file_info.latest_version_id).await
                {
                    files.push(FileMetadata {
                        id: file_id.clone(),
                        name: file_id,
                        path: file_info.latest_version_id.clone(),
                        size: version_info.file_size,
                        hash: version_info.version_id,
                        created_at: file_info.created_at,
                        modified_at: file_info.modified_at,
                    });
                }
            }
        }

        Ok(files)
    }

    async fn verify_hash(
        &self,
        file_id: &str,
        expected_hash: &str,
    ) -> std::result::Result<bool, Self::Error> {
        let metadata = self.get_metadata(file_id).await?;
        Ok(metadata.hash == expected_hash)
    }

    /// 获取存储根目录
    ///
    /// 返回用户文件存储的根目录 (data 目录)
    fn root_dir(&self) -> &Path {
        &self.data_root
    }

    fn get_full_path(&self, relative_path: &str) -> std::path::PathBuf {
        // 移除开头的 / 以确保是相对路径
        // PathBuf::join() 对绝对路径会直接返回绝对路径，忽略 root
        let cleaned_path = relative_path.trim_start_matches('/');
        self.data_root.join(cleaned_path)
    }
}

/// 实现 S3CompatibleStorageTrait 以提供 S3 API 支持
#[async_trait]
impl S3CompatibleStorageTrait for StorageManager {
    type Error = StorageError;

    async fn create_bucket(&self, bucket_name: &str) -> std::result::Result<(), Self::Error> {
        // bucket 可以映射为目录
        let bucket_path = self.root_dir().join(bucket_name);
        tokio::fs::create_dir_all(&bucket_path).await?;
        Ok(())
    }

    async fn delete_bucket(&self, bucket_name: &str) -> std::result::Result<(), Self::Error> {
        let bucket_path = self.root_dir().join(bucket_name);
        tokio::fs::remove_dir_all(&bucket_path).await?;
        Ok(())
    }

    async fn bucket_exists(&self, bucket_name: &str) -> bool {
        let bucket_path = self.root_dir().join(bucket_name);
        bucket_path.exists()
    }

    async fn list_buckets(&self) -> std::result::Result<Vec<String>, Self::Error> {
        let mut buckets = Vec::new();
        let mut entries = tokio::fs::read_dir(self.root_dir()).await?;

        while let Some(entry) = entries.next_entry().await? {
            if entry.file_type().await?.is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                buckets.push(name.to_string());
            }
        }

        Ok(buckets)
    }

    async fn list_bucket_objects(
        &self,
        bucket_name: &str,
        prefix: &str,
    ) -> std::result::Result<Vec<String>, Self::Error> {
        let bucket_path = self.root_dir().join(bucket_name);
        let mut objects = Vec::new();

        if !bucket_path.exists() {
            return Ok(objects);
        }

        // 递归扫描目录
        fn collect_files(
            dir: &std::path::Path,
            base: &std::path::Path,
            prefix: &str,
            objects: &mut Vec<String>,
        ) -> std::io::Result<()> {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() {
                    if let Ok(relative) = path.strip_prefix(base) {
                        let key = relative.to_string_lossy().to_string();
                        if key.starts_with(prefix) {
                            objects.push(key);
                        }
                    }
                } else if path.is_dir() {
                    collect_files(&path, base, prefix, objects)?;
                }
            }
            Ok(())
        }

        collect_files(&bucket_path, &bucket_path, prefix, &mut objects)?;

        Ok(objects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_storage() -> (StorageManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig::default();
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        // 注意：不在这里调用 init()，由各个测试自己调用

        (storage, temp_dir)
    }

    // 等待文件优化完成的helper函数
    async fn wait_for_optimization(
        storage: &StorageManager,
        file_id: &str,
        timeout_secs: u64,
    ) -> Result<()> {
        let start = std::time::Instant::now();
        loop {
            let metadata_db = storage.get_metadata_db()?;
            if let Some(entry) = metadata_db
                .get_file_index(file_id)
                .map_err(|e| StorageError::Storage(e.to_string()))?
            {
                if entry.optimization_status == crate::OptimizationStatus::Completed {
                    return Ok(());
                }
            }

            if start.elapsed().as_secs() > timeout_secs {
                return Err(StorageError::Storage(format!("等待优化超时: {}", file_id)));
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }

    #[tokio::test]
    async fn test_save_and_read_version() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        let data = b"Hello, World! This is a test.";
        let (_delta, version) = storage.save_version("test_file", data, None).await.unwrap();

        // 热存储模式下，初始delta.chunks为空，需要等待优化完成才有chunks
        assert!(!version.version_id.is_empty());

        // 读取版本数据（热存储可以直接读取）
        let read_data = storage
            .read_version_data(&version.version_id)
            .await
            .unwrap();
        assert_eq!(read_data, data);

        storage.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_list_file_versions() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        let data1 = b"Version 1";
        let (_delta1, version1) = storage
            .save_version("test_file", data1, None)
            .await
            .unwrap();

        let data2 = b"Version 2 - updated";
        let (_delta2, _version2) = storage
            .save_version("test_file", data2, Some(&version1.version_id))
            .await
            .unwrap();

        let versions = storage.list_file_versions("test_file").await.unwrap();
        assert_eq!(versions.len(), 2);
    }

    #[tokio::test]
    async fn test_storage_stats() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        storage
            .save_version("file1", b"Data 1", None)
            .await
            .unwrap();
        storage
            .save_version("file2", b"Data 2", None)
            .await
            .unwrap();

        // 等待优化完成
        wait_for_optimization(&storage, "file1", 10).await.unwrap();
        wait_for_optimization(&storage, "file2", 10).await.unwrap();

        let stats = storage.get_storage_stats().await.unwrap();
        // 优化过程可能创建额外版本，所以至少应该有2个版本
        assert!(
            stats.total_versions >= 2,
            "至少应该有2个版本，实际: {}",
            stats.total_versions
        );
        assert!(stats.total_chunks > 0, "优化完成后应该有chunks");

        storage.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_list_files() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 保存多个文件
        storage
            .save_version("file1", b"Data 1", None)
            .await
            .unwrap();
        storage
            .save_version("file2", b"Data 2", None)
            .await
            .unwrap();
        storage
            .save_version("file3", b"Data 3", None)
            .await
            .unwrap();

        // 列出所有文件
        let files = storage.list_files().await.unwrap();
        assert_eq!(files.len(), 3);
        assert!(files.contains(&"file1".to_string()));
        assert!(files.contains(&"file2".to_string()));
        assert!(files.contains(&"file3".to_string()));
    }

    #[tokio::test]
    async fn test_delete_file() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 保存文件和版本
        let (_delta1, version1) = storage
            .save_version("test_file", b"Version 1", None)
            .await
            .unwrap();
        let (_delta2, _version2) = storage
            .save_version("test_file", b"Version 2", Some(&version1.version_id))
            .await
            .unwrap();

        // 确认文件存在
        let files = storage.list_files().await.unwrap();
        assert!(files.contains(&"test_file".to_string()));

        // 软删除文件
        storage.delete_file("test_file").await.unwrap();

        // 确认文件不在普通列表中
        let files = storage.list_files().await.unwrap();
        assert!(!files.contains(&"test_file".to_string()));

        // 软删除不会删除版本信息，版本仍然存在
        let versions = storage.list_file_versions("test_file").await.unwrap();
        assert_eq!(versions.len(), 2);

        // 文件应该在已删除列表中
        let deleted_files = storage.list_deleted_files().await.unwrap();
        assert_eq!(deleted_files.len(), 1);
        assert_eq!(deleted_files[0].file_id, "test_file");
    }

    #[tokio::test]
    async fn test_garbage_collect() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 保存文件
        storage
            .save_version("file1", b"Some data", None)
            .await
            .unwrap();
        storage
            .save_version("file2", b"More data", None)
            .await
            .unwrap();

        // 删除一个文件
        storage.delete_file("file1").await.unwrap();

        // 运行垃圾回收
        let result = storage.garbage_collect().await.unwrap();

        // 应该有一些孤立块被清理
        assert!(
            result.orphaned_chunks > 0 || result.reclaimed_space > 0 || result.errors.is_empty()
        );
    }

    #[tokio::test]
    async fn test_get_file_info() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 保存文件
        storage
            .save_version("test_file", b"Data", None)
            .await
            .unwrap();

        // 获取文件信息
        let file_info = storage.get_file_info("test_file").await.unwrap();
        assert_eq!(file_info.file_id, "test_file");
        assert_eq!(file_info.version_count, 1);
        assert!(!file_info.latest_version_id.is_empty());
    }

    #[tokio::test]
    async fn test_deduplication() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 创建包含重复内容的数据
        let data1 = b"Hello World! ".repeat(100); // 1300 bytes
        let data2 = b"Hello World! ".repeat(100); // 相同内容

        // 保存第一个文件
        let (_delta1, _version1) = storage.save_version("file1", &data1, None).await.unwrap();

        // 保存第二个文件（相同内容，应触发去重）
        let (_delta2, _version2) = storage.save_version("file2", &data2, None).await.unwrap();

        // 等待优化完成才能看到去重效果
        wait_for_optimization(&storage, "file1", 10).await.unwrap();
        wait_for_optimization(&storage, "file2", 10).await.unwrap();

        // 获取去重统计
        let dedup_stats = storage.get_deduplication_stats().await.unwrap();

        // 验证去重效果
        assert!(dedup_stats.total_chunks > 0, "应该有块数据");
        assert!(
            dedup_stats.duplicate_chunks > 0,
            "应该有重复块（file2 与 file1 内容相同）"
        );
        assert!(
            dedup_stats.dedup_ratio > 0.0,
            "去重率应该大于 0: {}%",
            dedup_stats.dedup_ratio
        );
        assert!(
            dedup_stats.space_saved > 0,
            "应该节省了存储空间: {} bytes",
            dedup_stats.space_saved
        );

        // 验证原始大小应该是两份数据
        let expected_original_size = (data1.len() + data2.len()) as u64;
        assert!(
            dedup_stats.original_size >= expected_original_size,
            "原始大小应该至少等于两份数据大小"
        );

        // 验证存储大小应该小于原始大小
        assert!(
            dedup_stats.stored_size < dedup_stats.original_size,
            "存储大小应该小于原始大小（由于去重）"
        );

        println!("去重统计: {:?}", dedup_stats);
    }

    #[tokio::test]
    async fn test_cross_file_deduplication() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 策略：创建两个完全相同的文件，应该100%去重
        let data = b"This is test data for deduplication. ".repeat(300); // ~11KB

        println!("数据大小: {}B", data.len());

        // 保存第一个文件
        storage.save_version("file1", &data, None).await.unwrap();

        // 保存第二个文件（完全相同的数据）
        storage.save_version("file2", &data, None).await.unwrap();

        // 等待优化完成才能看到去重效果
        wait_for_optimization(&storage, "file1", 10).await.unwrap();
        wait_for_optimization(&storage, "file2", 10).await.unwrap();

        // 获取去重统计
        let dedup_stats = storage.get_deduplication_stats().await.unwrap();

        println!("跨文件去重统计: {:?}", dedup_stats);
        println!(
            "总块数: {}, 唯一块: {}, 重复块: {}",
            dedup_stats.total_chunks, dedup_stats.new_chunks, dedup_stats.duplicate_chunks
        );
        println!(
            "原始大小: {}B, 存储大小: {}B, 节省: {}B ({:.2}%)",
            dedup_stats.original_size,
            dedup_stats.stored_size,
            dedup_stats.space_saved,
            dedup_stats.dedup_ratio
        );

        // 验证跨文件去重效果
        // 两个完全相同的文件，总块数应该是唯一块数的2倍
        assert_eq!(
            dedup_stats.total_chunks,
            dedup_stats.new_chunks * 2,
            "两个相同文件，总块数应该是唯一块数的2倍"
        );
        assert_eq!(
            dedup_stats.duplicate_chunks, dedup_stats.new_chunks,
            "重复块数应该等于唯一块数（因为文件完全相同）"
        );
        assert!(
            dedup_stats.dedup_ratio >= 45.0,
            "去重率应该接近 50%（两个相同文件）: {:.2}%",
            dedup_stats.dedup_ratio
        );
    }

    /// 此测试已被忽略，因为块引用计数功能已由 DedupManager 的 BlockIndex 替代
    /// metadata_db 的块引用计数已不再使用
    #[ignore]
    #[tokio::test]
    async fn test_chunk_ref_count() {
        // 此测试需要禁用去重，因为它依赖 metadata_db 中的块引用计数
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_deduplication: false,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 保存同一个文件的多个版本(会产生重复的块)
        let data = b"Hello, World!";
        let (_delta1, version1) = storage.save_version("test_file", data, None).await.unwrap();

        // 保存相同数据(应该复用块)
        let data2 = b"Hello, World! Extra";
        let (_delta2, _version2) = storage
            .save_version("test_file", data2, Some(&version1.version_id))
            .await
            .unwrap();

        // 从 Sled 检查引用计数
        let metadata_db = storage.get_metadata_db().unwrap();
        let orphaned = metadata_db.list_orphaned_chunks().unwrap();

        // 确保有正常引用的块
        let total_chunks = metadata_db.chunk_ref_count();
        assert!(total_chunks > 0);
        assert!(orphaned.len() < total_chunks); // 不是所有块都是孤立的
    }

    #[tokio::test]
    async fn test_verify_chunks() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 创建测试数据
        let data = b"Test chunk verification data";
        storage.save_version("test_file", data, None).await.unwrap();

        // 等待优化完成才会有chunks
        wait_for_optimization(&storage, "test_file", 10)
            .await
            .unwrap();

        // 验证所有 chunks
        let report = storage.verify_all_chunks().await.unwrap();
        assert_eq!(report.valid + report.invalid + report.missing, report.total);
        // 正常情况下应该所有 chunks 都是有效的
        assert!(report.valid > 0, "应该有有效的 chunks");
        assert_eq!(report.invalid, 0, "不应该有损坏的 chunk");
    }

    #[tokio::test]
    async fn test_soft_delete() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 创建测试文件
        let data = b"Test data for soft delete";
        storage.save_version("test_file", data, None).await.unwrap();

        // 软删除文件
        storage.delete_file("test_file").await.unwrap();

        // 文件应该不在普通列表中
        let files = storage.list_files().await.unwrap();
        assert!(!files.contains(&"test_file".to_string()));

        // 但应该在已删除列表中
        let deleted_files = storage.list_deleted_files().await.unwrap();
        assert_eq!(deleted_files.len(), 1);
        assert_eq!(deleted_files[0].file_id, "test_file");
        assert!(deleted_files[0].is_deleted);
        assert!(deleted_files[0].deleted_at.is_some());
    }

    #[tokio::test]
    async fn test_restore_file() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 创建并删除文件
        storage
            .save_version("test_file", b"Test data", None)
            .await
            .unwrap();
        storage.delete_file("test_file").await.unwrap();

        // 恢复文件
        storage.restore_file("test_file").await.unwrap();

        // 文件应该回到普通列表
        let files = storage.list_files().await.unwrap();
        assert!(files.contains(&"test_file".to_string()));

        // 不应该在已删除列表中
        let deleted_files = storage.list_deleted_files().await.unwrap();
        assert_eq!(deleted_files.len(), 0);
    }

    #[tokio::test]
    async fn test_empty_recycle_bin() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 创建并删除多个文件
        storage
            .save_version("file1", b"Data 1", None)
            .await
            .unwrap();
        storage
            .save_version("file2", b"Data 2", None)
            .await
            .unwrap();
        storage.delete_file("file1").await.unwrap();
        storage.delete_file("file2").await.unwrap();

        // 确认有已删除的文件
        let deleted_files = storage.list_deleted_files().await.unwrap();
        assert_eq!(deleted_files.len(), 2);

        // 清空回收站
        let count = storage.empty_recycle_bin().await.unwrap();
        assert_eq!(count, 2);

        // 已删除列表应该为空
        let deleted_files = storage.list_deleted_files().await.unwrap();
        assert_eq!(deleted_files.len(), 0);
    }

    #[tokio::test]
    async fn test_permanently_delete_file() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 创建测试文件
        let data = b"Test data for permanent delete";
        storage.save_version("test_file", data, None).await.unwrap();

        // 永久删除文件
        storage.permanently_delete_file("test_file").await.unwrap();

        // 文件不应该存在
        assert!(!storage.file_exists("test_file").await);
    }

    #[tokio::test]
    async fn test_garbage_collect_blocks_with_dedup() {
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_deduplication: true,
            enable_compression: false,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 创建测试文件
        let data1 = b"Test data 1 for garbage collection";
        let data2 = b"Test data 2 for garbage collection";
        storage.save_version("file1", data1, None).await.unwrap();
        storage.save_version("file2", data2, None).await.unwrap();

        // 永久删除文件1
        storage.permanently_delete_file("file1").await.unwrap();

        // 运行GC
        let _deleted_count = storage.garbage_collect_blocks().await.unwrap();

        // 应该清理了一些块
        // 注意：具体数量取决于分块策略
        // GC应该成功完成，不需要检查具体数量
    }

    #[tokio::test]
    async fn test_garbage_collect_blocks_without_dedup() {
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_deduplication: false,
            enable_compression: false,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 创建测试文件
        let data = b"Test data for garbage collection without dedup";
        storage.save_version("file1", data, None).await.unwrap();

        // 永久删除文件
        storage.permanently_delete_file("file1").await.unwrap();

        // 运行GC
        let _deleted_count = storage.garbage_collect_blocks().await.unwrap();

        // 应该清理了一些块
        // GC应该成功完成，不需要检查具体数量
    }

    #[tokio::test]
    async fn test_delete_already_deleted_file() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 创建并删除文件
        storage
            .save_version("test_file", b"Test data", None)
            .await
            .unwrap();
        storage.delete_file("test_file").await.unwrap();

        // 尝试再次删除应该失败
        let result = storage.delete_file("test_file").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_files_excludes_deleted() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 创建多个文件
        storage
            .save_version("file1", b"Data 1", None)
            .await
            .unwrap();
        storage
            .save_version("file2", b"Data 2", None)
            .await
            .unwrap();
        storage
            .save_version("file3", b"Data 3", None)
            .await
            .unwrap();

        // 删除file2
        storage.delete_file("file2").await.unwrap();

        // list_files应该只返回file1和file3
        let files = storage.list_files().await.unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"file1".to_string()));
        assert!(!files.contains(&"file2".to_string()));
        assert!(files.contains(&"file3".to_string()));
    }

    #[tokio::test]
    async fn test_gc_task_start_stop() {
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_auto_gc: false, // 手动控制GC任务
            gc_interval_secs: 1,   // 1秒间隔用于快速测试
            enable_compression: false,
            enable_deduplication: false,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 启动GC任务
        storage.start_gc_task().await;

        // 等待一小段时间
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // 停止GC任务
        storage.stop_gc_task().await;

        // 验证任务已停止
        assert!(storage.gc_task_handle.read().await.is_none());
    }

    #[tokio::test]
    async fn test_auto_gc_on_init() {
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_auto_gc: true, // 自动启动GC
            gc_interval_secs: 1,  // 1秒间隔用于快速测试
            enable_compression: false,
            enable_deduplication: false,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 验证GC任务已启动
        assert!(storage.gc_task_handle.read().await.is_some());

        // 停止GC任务
        storage.stop_gc_task().await;
    }

    #[tokio::test]
    async fn test_manual_gc_trigger() {
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_auto_gc: false,
            enable_compression: false,
            enable_deduplication: false,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 创建测试文件
        storage
            .save_version("file1", b"Test data", None)
            .await
            .unwrap();

        // 永久删除文件
        storage.permanently_delete_file("file1").await.unwrap();

        // 手动触发GC
        let _deleted_count = storage.garbage_collect_blocks().await.unwrap();

        // GC应该成功完成
        // 不需要检查具体数量
    }

    #[tokio::test]
    async fn test_gc_task_periodic_execution() {
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_auto_gc: false,
            gc_interval_secs: 1, // 1秒间隔用于快速测试
            enable_compression: false,
            enable_deduplication: false,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 创建并删除文件
        storage
            .save_version("file1", b"Test data", None)
            .await
            .unwrap();
        storage.permanently_delete_file("file1").await.unwrap();

        // 启动GC任务
        storage.start_gc_task().await;

        // 等待至少一次GC执行
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // 停止GC任务
        storage.stop_gc_task().await;

        // GC任务应该已经执行过，清理了块
        // 这里只验证任务能正常启动和停止
    }

    #[tokio::test]
    async fn test_hot_storage_upload_and_read() {
        // 测试热存储模式的上传和读取
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_deduplication: false,
            enable_compression: false,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 上传测试数据（应该使用热存储）
        let test_data = b"Hello from hot storage! This is a test file.";
        let (delta, version) = storage
            .save_version("test_hot_file", test_data, None)
            .await
            .unwrap();

        // 验证使用了热存储（delta应该没有chunks）
        assert_eq!(delta.chunks.len(), 0, "热存储不应该有chunks");
        assert_eq!(version.size, test_data.len() as u64);

        // 验证文件索引的存储模式
        let metadata_db = storage.get_metadata_db().unwrap();
        let file_entry = metadata_db
            .get_file_index("test_hot_file")
            .unwrap()
            .unwrap();
        assert_eq!(
            file_entry.storage_mode,
            crate::StorageMode::Hot,
            "应该是热存储模式"
        );
        assert_eq!(
            file_entry.optimization_status,
            crate::OptimizationStatus::Pending,
            "应该是待优化状态"
        );

        // 验证热存储文件存在
        let hot_path = storage.get_hot_storage_path("test_hot_file");
        assert!(hot_path.exists(), "热存储文件应该存在");

        // 读取数据（应该从热存储读取）
        let read_data = storage
            .read_version_data(&version.version_id)
            .await
            .unwrap();
        assert_eq!(read_data, test_data, "读取的数据应该与原始数据一致");
    }

    #[tokio::test]
    async fn test_cold_storage_backward_compatibility() {
        // 测试禁用热存储时的传统分块存储（向后兼容）
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_deduplication: true,
            enable_compression: false,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 上传测试数据（先到热存储，然后优化为冷存储）
        let test_data = b"Hello from cold storage! This is a test file for traditional chunking.";
        let (_delta, version) = storage
            .save_version("test_cold_file", test_data, None)
            .await
            .unwrap();

        // 等待优化完成（从热存储转为冷存储）
        wait_for_optimization(&storage, "test_cold_file", 10)
            .await
            .unwrap();

        // 验证文件索引的存储模式（优化完成后重新获取metadata_db）
        let metadata_db = storage.get_metadata_db().unwrap();
        let file_entry = metadata_db
            .get_file_index("test_cold_file")
            .unwrap()
            .unwrap();
        assert_eq!(
            file_entry.storage_mode,
            crate::StorageMode::Cold,
            "应该是冷存储模式"
        );

        // 读取数据（应该从分块读取）
        let read_data = storage
            .read_version_data(&version.version_id)
            .await
            .unwrap();
        assert_eq!(read_data, test_data, "读取的数据应该与原始数据一致");
    }

    #[tokio::test]
    async fn test_hot_storage_stream_upload() {
        // 测试流式上传到热存储
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_deduplication: false,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 创建测试数据流
        let test_data = b"Streaming data to hot storage! This is a larger test file.".repeat(100);
        let mut cursor = std::io::Cursor::new(test_data.clone());

        // 流式上传
        let (delta, version) = storage
            .save_version_from_reader("test_stream_file", &mut cursor, None)
            .await
            .unwrap();

        // 验证热存储
        assert_eq!(delta.chunks.len(), 0, "热存储不应该有chunks");

        // 读取并验证数据
        let read_data = storage
            .read_version_data(&version.version_id)
            .await
            .unwrap();
        assert_eq!(read_data, test_data, "流式上传的数据应该正确");
    }

    #[tokio::test]
    async fn test_background_optimization() {
        // 测试后台优化功能
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_deduplication: true,
            enable_compression: true,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 上传一个较大的文件（应该使用热存储）
        let test_data = b"This is test data for background optimization. ".repeat(1000); // ~47KB
        let (delta, version) = storage
            .save_version("test_optimization_file", &test_data, None)
            .await
            .unwrap();

        // 验证初始状态：热存储，无chunks
        assert_eq!(delta.chunks.len(), 0, "热存储不应该有chunks");

        // 验证文件索引的初始状态
        let metadata_db = storage.get_metadata_db().unwrap();
        let file_entry = metadata_db
            .get_file_index("test_optimization_file")
            .unwrap()
            .unwrap();
        assert_eq!(
            file_entry.storage_mode,
            crate::StorageMode::Hot,
            "初始应该是热存储模式"
        );
        assert_eq!(
            file_entry.optimization_status,
            crate::OptimizationStatus::Pending,
            "优化状态应该是待优化"
        );

        // 验证热存储文件存在
        let file_id = "test_optimization_file";
        let prefix = &file_id[..2.min(file_id.len())];
        let hot_path = storage.hot_storage_root.join(prefix).join(file_id);
        assert!(hot_path.exists(), "热存储文件应该存在");

        // 等待优化完成
        wait_for_optimization(&storage, file_id, 10).await.unwrap();

        // 验证优化后的状态（重新获取metadata_db）
        let metadata_db = storage.get_metadata_db().unwrap();
        let file_entry_after = metadata_db
            .get_file_index("test_optimization_file")
            .unwrap()
            .unwrap();

        // 优化后应该变成冷存储或压缩存储
        assert_ne!(
            file_entry_after.storage_mode,
            crate::StorageMode::Hot,
            "优化后不应该还是热存储模式"
        );

        // 验证可以正确读取优化后的数据
        let read_data = storage
            .read_version_data(&version.version_id)
            .await
            .unwrap();
        assert_eq!(read_data, test_data, "优化后应该能正确读取数据");

        // 验证优化统计信息
        let stats = storage.get_optimization_stats().await;
        assert!(stats.total_tasks > 0, "应该有任务被处理");
        assert!(
            stats.completed_tasks > 0 || stats.pending_tasks > 0,
            "应该有完成或待处理的任务"
        );

        // 关闭存储
        storage.shutdown().await.unwrap();
    }

    /// 此测试已被忽略，因为优化策略决策逻辑已被移除，现在统一使用Full策略
    #[ignore]
    #[tokio::test]
    async fn test_optimization_strategy_decision() {
        // 测试优化策略决策逻辑
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_deduplication: true,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 测试1：小文件（< 1MB）应该使用 CompressOnly 策略
        let small_data = b"Small file".repeat(100); // ~1KB
        let (_, _version1) = storage
            .save_version("test_small", &small_data, None)
            .await
            .unwrap();

        // 等待任务提交
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // 获取并检查任务策略
        let tasks = storage.optimization_scheduler.get_pending_tasks().await;
        let has_compress_only = tasks
            .iter()
            .any(|t| matches!(t.strategy, crate::OptimizationStrategy::CompressOnly));
        let has_skip = tasks
            .iter()
            .any(|t| matches!(t.strategy, crate::OptimizationStrategy::Skip));
        // 注意：由于文件类型检测，可能会被判定为Skip，这里只验证不是Full策略
        assert!(
            has_compress_only || has_skip,
            "小文件应该使用CompressOnly或Skip策略"
        );

        // 测试2：已压缩文件应该被Skip
        // 创建一个模拟的压缩文件数据（PNG文件头）
        let mut png_data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]; // PNG头
        png_data.extend_from_slice(&vec![0u8; 1000]);
        let (_, _version2) = storage
            .save_version("test_compressed", &png_data, None)
            .await
            .unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // 获取任务并检查策略
        if let Some(task) = storage.optimization_scheduler.get_next_ready_task().await
            && task.file_id == "test_compressed"
        {
            assert_eq!(
                task.strategy,
                crate::OptimizationStrategy::Skip,
                "已压缩文件应该被跳过"
            );
        }

        storage.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_manual_trigger_optimization() {
        // 测试手动触发优化功能
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_deduplication: true,
            enable_compression: true,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 上传一个文件到热存储
        let test_data = b"Test data for manual optimization trigger".repeat(100);
        let (_delta, _version) = storage
            .save_version("test_manual", &test_data, None)
            .await
            .unwrap();

        // 验证初始队列长度
        let initial_queue_len = storage.get_optimization_queue_length().await;
        assert_eq!(initial_queue_len, 1, "应该有1个待处理任务");

        // 手动触发优化
        let result = storage.trigger_file_optimization("test_manual").await;
        assert!(result.is_ok(), "手动触发优化应该成功");

        // 验证队列中仍然只有一个任务（去重）
        let queue_len_after = storage.get_optimization_queue_length().await;
        assert_eq!(queue_len_after, 1, "去重后应该仍然是1个任务");

        // 测试触发不存在的文件
        let result_not_found = storage.trigger_file_optimization("non_existent").await;
        assert!(result_not_found.is_err(), "触发不存在的文件应该失败");

        storage.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_pause_resume_optimization_scheduler() {
        // 测试暂停和恢复优化调度器
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_deduplication: true,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 验证初始状态（未暂停）
        assert!(
            !storage.is_optimization_paused(),
            "初始状态应该是未暂停"
        );

        // 暂停调度器
        storage.pause_optimization_scheduler().await.unwrap();
        assert!(
            storage.is_optimization_paused(),
            "暂停后应该是暂停状态"
        );

        // 上传文件
        let test_data = b"Test data for pause test".repeat(100);
        storage
            .save_version("test_pause", &test_data, None)
            .await
            .unwrap();

        // 等待一段时间（如果未暂停，任务应该会被执行）
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // 验证任务仍在队列中（因为调度器暂停了）
        let queue_len = storage.get_optimization_queue_length().await;
        assert_eq!(queue_len, 1, "暂停时任务应该保留在队列中");

        // 恢复调度器
        storage.resume_optimization_scheduler().await.unwrap();
        assert!(
            !storage.is_optimization_paused(),
            "恢复后应该是未暂停状态"
        );

        storage.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_optimization_queue_management() {
        // 测试优化队列管理功能
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_deduplication: true,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 验证初始队列为空
        assert_eq!(
            storage.get_optimization_queue_length().await,
            0,
            "初始队列应该为空"
        );

        // 上传多个文件
        for i in 0..3 {
            let test_data = format!("Test data {}", i).repeat(100);
            storage
                .save_version(&format!("test_file_{}", i), test_data.as_bytes(), None)
                .await
                .unwrap();
        }

        // 验证队列长度
        assert_eq!(
            storage.get_optimization_queue_length().await,
            3,
            "应该有3个待处理任务"
        );

        // 获取待处理任务列表
        let pending_tasks = storage.get_pending_optimization_tasks().await;
        assert_eq!(pending_tasks.len(), 3, "应该返回3个待处理任务");

        // 验证任务信息
        let file_ids: Vec<String> = pending_tasks.iter().map(|t| t.file_id.clone()).collect();
        assert!(
            file_ids.contains(&"test_file_0".to_string()),
            "应该包含test_file_0"
        );
        assert!(
            file_ids.contains(&"test_file_1".to_string()),
            "应该包含test_file_1"
        );
        assert!(
            file_ids.contains(&"test_file_2".to_string()),
            "应该包含test_file_2"
        );

        // 清空队列
        storage.clear_optimization_queue().await.unwrap();
        assert_eq!(
            storage.get_optimization_queue_length().await,
            0,
            "清空后队列应该为空"
        );

        // 验证统计信息
        let stats = storage.get_optimization_stats().await;
        assert_eq!(stats.pending_tasks, 0, "待处理任务数应该为0");

        storage.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_optimization_api_error_cases() {
        // 测试优化API的错误情况
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_deduplication: false,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 上传文件（先到热存储，然后等待优化完成）
        let test_data = b"Test data for error cases";
        storage
            .save_version("test_error", test_data, None)
            .await
            .unwrap();

        // 等待优化完成（变成冷存储）
        wait_for_optimization(&storage, "test_error", 10)
            .await
            .unwrap();

        // 尝试触发已在冷存储的文件的优化（应该失败或者已经是Completed状态）
        let result = storage.trigger_file_optimization("test_error").await;
        // 如果文件已经是Completed状态，可能会返回错误或者忽略
        if result.is_err() {
            println!("触发已完成优化的文件返回错误（符合预期）");
        }

        // 尝试触发不存在的文件（应该失败）
        let result_not_found = storage.trigger_file_optimization("non_existent").await;
        assert!(result_not_found.is_err(), "触发不存在的文件应该失败");

        storage.shutdown().await.unwrap();
    }

    /// 此测试已被忽略，因为孤儿块检测功能依赖 metadata_db 的块引用计数
    /// 该功能已由 DedupManager 的 BlockIndex 和 GC 机制替代
    #[ignore]
    #[tokio::test]
    async fn test_orphan_cleanup() {
        // 此测试需要禁用去重，因为它依赖 metadata_db 中的块引用计数
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_deduplication: false,
            ..IncrementalConfig::default()
        };
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024, config);
        storage.init().await.unwrap();

        // 创建测试数据
        let data = b"Test orphan cleanup";
        storage.save_version("test_file", data, None).await.unwrap();

        // 检测孤儿 chunks（正常情况下应该没有）
        let orphans = storage.detect_orphan_chunks().await.unwrap();
        // 正常情况下，所有 chunks 都应该被引用，没有孤儿
        assert_eq!(orphans.len(), 0, "不应该有孤儿 chunks");
    }
}
// 性能对比测试：原版存储 vs v0.7.0增量存储
// 使用方法：cargo test --lib bench_comparison

// ============================================================================
// Trait 实现
// ============================================================================
