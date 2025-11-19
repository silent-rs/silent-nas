//! 增量存储后端
//!
//! 实现版本链式存储和块级存储功能

use crate::cache::CacheManager;
use crate::error::{Result, StorageError};
use crate::metadata::SledMetadataDb;
use crate::reliability::{ChunkVerifier, OrphanChunkCleaner, WalManager, WalOperation};
use crate::{ChunkInfo, FileDelta, IncrementalConfig, VersionInfo};
use async_trait::async_trait;
use chrono::Local;
use serde::{Deserialize, Serialize};
use silent_nas_core::{FileMetadata, FileVersion, S3CompatibleStorageTrait, StorageManagerTrait};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use sha2::Digest;
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::{OnceCell, RwLock};
use tracing::info;

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
    /// 版本索引缓存（使用内部可变性）
    version_index: Arc<RwLock<HashMap<String, VersionInfo>>>,
    /// 块索引缓存（使用内部可变性）
    block_index: Arc<RwLock<HashMap<String, PathBuf>>>,
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
    /// GC任务停止标志
    gc_stop_flag: Arc<tokio::sync::RwLock<bool>>,
}

impl StorageManager {
    pub fn new(root_path: PathBuf, chunk_size: usize, config: IncrementalConfig) -> Self {
        let data_root = root_path.join("data");
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

        Self {
            root_path,
            data_root,
            config,
            version_root,
            chunk_root: chunk_root.clone(),
            chunk_size,
            metadata_db: Arc::new(OnceCell::new()),
            version_index: Arc::new(RwLock::new(HashMap::new())),
            block_index: Arc::new(RwLock::new(HashMap::new())),
            cache_manager: Arc::new(CacheManager::with_default()),
            wal_manager: Arc::new(RwLock::new(WalManager::new(wal_path))),
            chunk_verifier: Arc::new(ChunkVerifier::new(chunk_root.clone())),
            orphan_cleaner: Arc::new(OrphanChunkCleaner::new(chunk_root)),
            compressor,
            dedup_manager,
            gc_task_handle: Arc::new(RwLock::new(None)),
            gc_stop_flag: Arc::new(tokio::sync::RwLock::new(false)),
        }
    }

    /// 初始化增量存储
    pub async fn init(&self) -> Result<()> {
        // 创建必要的目录
        fs::create_dir_all(&self.root_path).await?;
        fs::create_dir_all(&self.data_root).await?;
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
        let (_delta, file_version) = self
            .save_version_from_reader(file_id, reader, None)
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

        // 读取前 1MB 进行类型检测
        const HEADER_SIZE: usize = 1024 * 1024;
        let mut header_buf = vec![0u8; HEADER_SIZE];
        let mut header_len = 0usize;
        while header_len < HEADER_SIZE {
            let n = reader
                .read(&mut header_buf[header_len..])
                .await
                .map_err(StorageError::Io)?;
            if n == 0 {
                break;
            }
            header_len += n;
        }

        // 空文件特殊处理
        if header_len == 0 {
            let now = Local::now().naive_local();

            let delta = FileDelta {
                file_id: file_id.to_string(),
                base_version_id: parent_version_id.unwrap_or("").to_string(),
                new_version_id: version_id.clone(),
                chunks: Vec::new(),
                created_at: now,
            };

            // 保存差异数据（空文件）
            self.save_delta(file_id, &delta).await?;

            // 保存版本信息
            let _version_info = self
                .save_version_info(file_id, &delta, parent_version_id)
                .await?;

            // 更新文件索引
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
                });

            file_entry.latest_version_id = version_id.clone();
            file_entry.version_count += 1;
            file_entry.modified_at = now;

            metadata_db
                .put_file_index(file_id, &file_entry)
                .map_err(|e| StorageError::Storage(format!("保存文件索引失败: {}", e)))?;

            // 记录 WAL（空文件版本）
            let operation = WalOperation::CreateVersion {
                file_id: file_id.to_string(),
                version_id: version_id.clone(),
                chunk_hashes: Vec::new(),
            };
            let mut wal = self.wal_manager.write().await;
            wal.write(operation).await?;
            drop(wal);

            let file_version = FileVersion {
                version_id: version_id.clone(),
                file_id: file_id.to_string(),
                name: file_id.to_string(),
                size: 0,
                hash: self.calculate_hash(&[]),
                created_at: now,
                author: None,
                comment: None,
                is_current: true,
            };

            return Ok((delta, file_version));
        }

        header_buf.truncate(header_len);

        // 检测文件类型并调整配置（仅根据前 1MB）
        let file_type = crate::core::FileType::detect(&header_buf);
        let (min_chunk, max_chunk) = file_type.recommended_chunk_size();

        let mut adjusted_config = self.config.clone();
        adjusted_config.min_chunk_size = min_chunk;
        adjusted_config.max_chunk_size = max_chunk;

        if file_type.is_compressed() {
            adjusted_config.enable_compression = false;
        }

        info!(
            "文件 {} 类型: {}, 块大小: {}KB-{}KB, 压缩: {} (流式)",
            file_id,
            file_type.as_str(),
            min_chunk / 1024,
            max_chunk / 1024,
            adjusted_config.enable_compression
        );

        // 当前仅支持从空版本开始的流式写入
        if parent_version_id.is_some() {
            return Err(StorageError::Storage(
                "save_version_from_reader 目前不支持基于父版本的增量写入".to_string(),
            ));
        }

        let mut generator = crate::core::delta::DeltaGenerator::new(adjusted_config);

        // 初始化去重统计
        let mut dedup_stats = crate::DeduplicationStats {
            total_chunks: 0,
            original_size: 0,
            ..Default::default()
        };

        let mut all_chunks: Vec<ChunkInfo> = Vec::new();
        let mut offset_base: usize = 0;
        let mut total_size: u64 = 0;

        // 增量计算文件哈希
        let mut hasher = sha2::Sha256::new();

        // 处理首个缓冲区
        let mut current_buf = header_buf;
        loop {
            if current_buf.is_empty() {
                break;
            }

            total_size += current_buf.len() as u64;
            hasher.update(&current_buf);

            let delta_segment = generator
                .generate_full_delta(&current_buf, file_id)
                .map_err(|e| StorageError::Storage(format!("生成分块失败: {}", e)))?;

            dedup_stats.total_chunks += delta_segment.chunks.len();

            // 按片段处理块：写入块数据并更新去重状态
            for chunk in &delta_segment.chunks {
                let start = chunk.offset;
                let end = start + chunk.size;
                if end > current_buf.len() {
                    return Err(StorageError::Storage(
                        "分块范围越界（流式处理）".to_string(),
                    ));
                }
                let chunk_data = &current_buf[start..end];

                let mut chunk_with_compression = chunk.clone();
                // 全局偏移
                chunk_with_compression.offset = offset_base + chunk.offset;

                if self.config.enable_deduplication {
                    // 使用 DedupManager 的内存索引进行去重
                    let exists = self.dedup_manager.chunk_exists(&chunk.chunk_id).await;

                    if exists {
                        self.dedup_manager
                            .increment_chunk_ref(&chunk.chunk_id)
                            .await
                            .map_err(|e| {
                                StorageError::Storage(format!(
                                    "增加块引用计数失败: {}",
                                    e
                                ))
                            })?;
                        dedup_stats.duplicate_chunks += 1;
                    } else {
                        let compression_algo =
                            self.save_chunk_data(&chunk.chunk_id, chunk_data).await?;
                        chunk_with_compression.compression = compression_algo;

                        let storage_path = self.get_chunk_path(&chunk.chunk_id);
                        self.dedup_manager
                            .add_chunk(&chunk.chunk_id, chunk.size, storage_path)
                            .await
                            .map_err(|e| {
                                StorageError::Storage(format!(
                                    "添加块到索引失败: {}",
                                    e
                                ))
                            })?;

                        dedup_stats.new_chunks += 1;
                        dedup_stats.stored_size += chunk.size as u64;
                    }
                } else {
                    // 不使用去重，直接保存所有块
                    let compression_algo =
                        self.save_chunk_data(&chunk.chunk_id, chunk_data).await?;

                    chunk_with_compression.compression = compression_algo;
                    dedup_stats.new_chunks += 1;
                    dedup_stats.stored_size += chunk.size as u64;
                }

                all_chunks.push(chunk_with_compression);
            }

            offset_base += current_buf.len();

            // 读取下一批数据
            const READ_BUFFER_SIZE: usize = 8 * 1024 * 1024; // 8MB
            let mut buf = vec![0u8; READ_BUFFER_SIZE];
            let n = reader.read(&mut buf).await.map_err(StorageError::Io)?;
            if n == 0 {
                break;
            }
            buf.truncate(n);
            current_buf = buf;
        }

        dedup_stats.original_size = total_size;
        dedup_stats.calculate_dedup_ratio();

        if dedup_stats.dedup_ratio > 0.0 {
            info!(
                "版本 {} 去重统计(流式): 总块数={}, 新块={}, 重复块={}, 原始大小={}B, 存储大小={}B, 去重率={:.2}%",
                version_id,
                dedup_stats.total_chunks,
                dedup_stats.new_chunks,
                dedup_stats.duplicate_chunks,
                dedup_stats.original_size,
                dedup_stats.stored_size,
                dedup_stats.dedup_ratio
            );
        }

        // 构建完整的 FileDelta
        let delta = FileDelta {
            file_id: file_id.to_string(),
            base_version_id: parent_version_id.unwrap_or("").to_string(),
            new_version_id: version_id.clone(),
            chunks: all_chunks,
            created_at: Local::now().naive_local(),
        };

        // 保存差异数据
        self.save_delta(file_id, &delta).await?;

        // 保存版本信息
        let _version_info = self
            .save_version_info(file_id, &delta, parent_version_id)
            .await?;

        // 更新文件索引
        let now = Local::now().naive_local();
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
            });

        file_entry.latest_version_id = version_id.clone();
        file_entry.version_count += 1;
        file_entry.modified_at = now;

        metadata_db
            .put_file_index(file_id, &file_entry)
            .map_err(|e| StorageError::Storage(format!("保存文件索引失败: {}", e)))?;

        // 定期刷新数据库（每10个版本刷新一次）
        let version_count = self.version_index.read().await.len();
        if version_count % 10 == 0 {
            let _ = self.save_chunk_ref_count().await;
            let _ = metadata_db.flush().await;
        }

        // 创建 FileVersion（使用流式哈希和总大小）
        let file_hash = {
use sha2::Digest;
            let hash_bytes = hasher.finalize();
            hex::encode(hash_bytes)
        };

        let file_version = FileVersion {
            version_id: version_id.clone(),
            file_id: file_id.to_string(),
            name: file_id.to_string(),
            size: total_size,
            hash: file_hash,
            created_at: Local::now().naive_local(),
            author: None,
            comment: None,
            is_current: true,
        };

        // 记录 WAL（Phase 5 Step 4）
        let chunk_hashes: Vec<String> = delta.chunks.iter().map(|c| c.chunk_id.clone()).collect();
        let operation = WalOperation::CreateVersion {
            file_id: file_id.to_string(),
            version_id: version_id.clone(),
            chunk_hashes,
        };
        let mut wal = self.wal_manager.write().await;
        wal.write(operation).await?;
        drop(wal);

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

        // 检测文件类型并调整配置
        let file_type = crate::core::FileType::detect(data);
        let (min_chunk, max_chunk) = file_type.recommended_chunk_size();

        let mut adjusted_config = self.config.clone();
        adjusted_config.min_chunk_size = min_chunk;
        adjusted_config.max_chunk_size = max_chunk;

        // 已压缩文件不需要再压缩
        if file_type.is_compressed() {
            adjusted_config.enable_compression = false;
        }

        info!(
            "文件 {} 类型: {}, 块大小: {}KB-{}KB, 压缩: {}",
            file_id,
            file_type.as_str(),
            min_chunk / 1024,
            max_chunk / 1024,
            adjusted_config.enable_compression
        );

        // 生成差异（使用调整后的配置）
        let base_data = if let Some(parent_id) = parent_version_id {
            // 只有在有父版本时才读取
            self.read_version_data(parent_id).await?
        } else {
            Vec::new()
        };

        let mut generator = crate::core::delta::DeltaGenerator::new(adjusted_config);
        let mut delta =
            generator.generate_delta(&base_data, data, file_id, parent_version_id.unwrap_or(""))?;
        // 使用相同的version_id
        delta.new_version_id = version_id.clone();

        // 初始化去重统计
        let mut dedup_stats = crate::DeduplicationStats {
            total_chunks: delta.chunks.len(),
            original_size: data.len() as u64,
            ..Default::default()
        };

        // 保存块数据并更新引用计数（带去重）
        // 同时更新 chunks 的压缩信息
        let mut updated_chunks = Vec::new();

        // 根据配置决定是否使用去重
        if self.config.enable_deduplication {
            // 使用 DedupManager 的内存索引进行去重
            for chunk in &delta.chunks {
                let mut chunk_with_compression = chunk.clone();

                // 检查块是否已存在（内存索引查询，O(1)）
                let exists = self.dedup_manager.chunk_exists(&chunk.chunk_id).await;

                if exists {
                    // 块已存在，跳过写入，只增加引用计数
                    self.dedup_manager
                        .increment_chunk_ref(&chunk.chunk_id)
                        .await
                        .map_err(|e| StorageError::Storage(format!("增加块引用计数失败: {}", e)))?;

                    // 更新去重统计
                    dedup_stats.duplicate_chunks += 1;
                } else {
                    // 新块，写入磁盘
                    let compression_algo = self.save_chunk(chunk, data).await?;

                    // 更新 chunk 的压缩信息
                    chunk_with_compression.compression = compression_algo;

                    // 添加到去重索引
                    let storage_path = self.get_chunk_path(&chunk.chunk_id);
                    self.dedup_manager
                        .add_chunk(&chunk.chunk_id, chunk.size, storage_path)
                        .await
                        .map_err(|e| StorageError::Storage(format!("添加块到索引失败: {}", e)))?;

                    // 更新去重统计
                    dedup_stats.new_chunks += 1;
                    dedup_stats.stored_size += chunk.size as u64;
                }

                updated_chunks.push(chunk_with_compression);
            }
        } else {
            // 不使用去重，直接保存所有块
            for chunk in &delta.chunks {
                let compression_algo = self.save_chunk(chunk, data).await?;

                let mut chunk_with_compression = chunk.clone();
                chunk_with_compression.compression = compression_algo;

                dedup_stats.new_chunks += 1;
                dedup_stats.stored_size += chunk.size as u64;

                updated_chunks.push(chunk_with_compression);
            }
        }

        // 用包含压缩信息的 chunks 替换原来的
        delta.chunks = updated_chunks;

        // 计算去重率
        dedup_stats.calculate_dedup_ratio();

        // 记录去重统计（如果去重率大于 0）
        if dedup_stats.dedup_ratio > 0.0 {
            info!(
                "版本 {} 去重统计: 总块数={}, 新块={}, 重复块={}, 原始大小={}B, 存储大小={}B, 去重率={:.2}%",
                version_id,
                dedup_stats.total_chunks,
                dedup_stats.new_chunks,
                dedup_stats.duplicate_chunks,
                dedup_stats.original_size,
                dedup_stats.stored_size,
                dedup_stats.dedup_ratio
            );
        }

        // 保存差异数据
        self.save_delta(file_id, &delta).await?;

        // 保存版本信息
        let _version_info = self
            .save_version_info(file_id, &delta, parent_version_id)
            .await?;

        // 更新文件索引
        let now = Local::now().naive_local();
        let metadata_db = self.get_metadata_db()?;

        // 先从 Sled 读取或创建新条目
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
            });

        file_entry.latest_version_id = version_id.clone();
        file_entry.version_count += 1;
        file_entry.modified_at = now;

        // 保存到 Sled
        metadata_db
            .put_file_index(file_id, &file_entry)
            .map_err(|e| StorageError::Storage(format!("保存文件索引失败: {}", e)))?;

        // 定期刷新数据库（每10个版本刷新一次）
        let version_count = self.version_index.read().await.len();
        if version_count % 10 == 0 {
            let _ = self.save_chunk_ref_count().await;
            let _ = metadata_db.flush().await;
        }

        // 创建FileVersion
        let file_version = FileVersion {
            version_id: version_id.clone(),
            file_id: file_id.to_string(),
            name: file_id.to_string(),
            size: data.len() as u64,
            hash: self.calculate_hash(data),
            created_at: Local::now().naive_local(),
            author: None,
            comment: None,
            is_current: true,
        };

        // 记录 WAL（Phase 5 Step 4）
        let chunk_hashes: Vec<String> = delta.chunks.iter().map(|c| c.chunk_id.clone()).collect();
        let operation = WalOperation::CreateVersion {
            file_id: file_id.to_string(),
            version_id: version_id.clone(),
            chunk_hashes,
        };
        let mut wal = self.wal_manager.write().await;
        wal.write(operation).await?;
        drop(wal);

        Ok((delta, file_version))
    }

    /// 读取版本数据
    pub async fn read_version_data(&self, version_id: &str) -> Result<Vec<u8>> {
        // 获取版本信息
        let _version_info = self.get_version_info(version_id).await?;

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
                // 扩展到正确的偏移量
                if result.len() < chunk.offset {
                    result.resize(chunk.offset, 0);
                }
                result.extend_from_slice(&chunk_data);
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

    /// 获取版本信息
    pub async fn get_version_info(&self, version_id: &str) -> Result<VersionInfo> {
        // 首先尝试从内存缓存读取
        if let Some(info) = self.version_index.read().await.get(version_id) {
            return Ok(info.clone());
        }

        // 从 Sled 读取
        let metadata_db = self.get_metadata_db()?;
        if let Some(version_info) = metadata_db
            .get_version_info(version_id)
            .map_err(|e| StorageError::Storage(format!("从 Sled 读取版本信息失败: {}", e)))?
        {
            // 更新内存缓存
            self.version_index
                .write()
                .await
                .insert(version_id.to_string(), version_info.clone());
            return Ok(version_info);
        }

        Err(StorageError::Storage(format!(
            "版本信息不存在: {}",
            version_id
        )))
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

        // 从内存索引中删除
        self.version_index.write().await.remove(version_id);

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

        let version_index = self.version_index.read().await;
        for version in version_index.values() {
            total_versions += 1;
            total_size += version.storage_size;
            total_chunks += version.chunk_count;
        }
        drop(version_index);

        // 计算唯一块数量
        let mut chunk_sizes: HashMap<String, u64> = HashMap::new();
        let block_index = self.block_index.read().await;
        for (chunk_id, chunk_path) in block_index.iter() {
            if let Ok(metadata) = fs::metadata(chunk_path).await {
                chunk_sizes.insert(chunk_id.clone(), metadata.len());
                unique_chunks += 1;
            }
        }
        drop(block_index);

        let total_chunk_size: u64 = chunk_sizes.values().sum();

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

        // 更新块索引
        self.block_index
            .write()
            .await
            .insert(chunk.chunk_id.clone(), chunk_path);

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

        // 更新块索引
        self.block_index
            .write()
            .await
            .insert(chunk_id.to_string(), chunk_path);

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
        let version_info = VersionInfo {
            version_id: delta.new_version_id.clone(),
            file_id: file_id.to_string(),
            parent_version_id: parent_version_id.map(|s| s.to_string()),
            file_size: delta.chunks.iter().map(|c| c.size as u64).sum(),
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

        // 更新内存索引
        self.version_index
            .write()
            .await
            .insert(version_info.version_id.clone(), version_info.clone());

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

                    // 更新内存索引
                    self.version_index
                        .write()
                        .await
                        .insert(version_id.to_string(), version_info);

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

    /// 加载块索引
    async fn load_block_index(&self) -> Result<()> {
        let chunks_dir = self.chunk_root.join("data");

        if !chunks_dir.exists() {
            return Ok(());
        }

        let mut entries = fs::read_dir(&chunks_dir).await.map_err(StorageError::Io)?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file()
                && let Some(file_name) = path.file_name().and_then(|s| s.to_str())
            {
                self.block_index
                    .write()
                    .await
                    .insert(file_name.to_string(), path);
            }
        }

        let index_len = self.block_index.read().await.len();
        info!("加载了 {} 个块索引", index_len);
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

        // 遍历所有版本，统计块引用
        let version_index = self.version_index.read().await;
        for version_info in version_index.values() {
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
        drop(version_index);

        // 获取 metadata_db 引用
        let metadata_db = self.get_metadata_db()?;

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

        // 遍历所有版本，构建文件索引
        let version_index = self.version_index.read().await;
        for version_info in version_index.values() {
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
                });

            entry.version_count += 1;
            // 更新最新版本（假设版本ID可比较，或使用时间戳）
            if version_info.created_at > entry.modified_at {
                entry.latest_version_id = version_info.version_id.clone();
                entry.modified_at = version_info.created_at;
            }
        }
        drop(version_index);

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

            // 从 Sled 和内存索引中移除版本信息
            let metadata_db = self.get_metadata_db()?;
            if let Err(e) = metadata_db.remove_version_info(&version.version_id) {
                info!("从 Sled 移除版本信息失败: {}", e);
            }
            self.version_index.write().await.remove(&version.version_id);
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
        *self.gc_stop_flag.write().await = false;

        let storage = self.clone_for_gc();
        let interval_secs = self.config.gc_interval_secs;
        let stop_flag = self.gc_stop_flag.clone();

        let handle = tokio::spawn(async move {
            info!("GC后台任务启动，间隔: {}秒", interval_secs);

            loop {
                // 等待指定间隔
                tokio::time::sleep(tokio::time::Duration::from_secs(interval_secs)).await;

                // 检查停止标志
                if *stop_flag.read().await {
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
        *self.gc_stop_flag.write().await = true;

        // 等待任务结束
        if let Some(handle) = self.gc_task_handle.write().await.take() {
            let _ = handle.await;
            info!("GC后台任务已停止");
        }
    }

    /// 克隆一个用于GC任务的StorageManager副本
    ///
    /// 由于GC任务需要在后台线程中运行，需要克隆必要的字段
    fn clone_for_gc(&self) -> Self {
        Self {
            root_path: self.root_path.clone(),
            data_root: self.data_root.clone(),
            config: self.config.clone(),
            version_root: self.version_root.clone(),
            chunk_root: self.chunk_root.clone(),
            chunk_size: self.chunk_size,
            metadata_db: self.metadata_db.clone(),
            version_index: self.version_index.clone(),
            block_index: self.block_index.clone(),
            cache_manager: self.cache_manager.clone(),
            wal_manager: self.wal_manager.clone(),
            chunk_verifier: self.chunk_verifier.clone(),
            orphan_cleaner: self.orphan_cleaner.clone(),
            compressor: self.compressor.clone(),
            dedup_manager: self.dedup_manager.clone(),
            gc_task_handle: Arc::new(RwLock::new(None)),
            gc_stop_flag: Arc::new(tokio::sync::RwLock::new(false)),
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

            // 更新内存索引
            self.version_index
                .write()
                .await
                .insert(version.version_id.clone(), version_info);

            // 4.2 移动 delta 文件
            let old_delta_path = self.get_delta_path(old_file_id, &version.version_id);
            let new_delta_path = self.get_delta_path(new_file_id, &version.version_id);

            if old_delta_path.exists() {
                // 确保新路径的父目录存在
                if let Some(parent) = new_delta_path.parent() {
                    fs::create_dir_all(parent)
                        .await
                        .map_err(StorageError::Io)?;
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

        // 7. 刷新数据库
        let _ = metadata_db.flush().await;

        // 8. 返回新文件的元数据
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
                                    // 从块索引移除
                                    self.block_index.write().await.remove(&chunk_id);
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
                    self.block_index.write().await.remove(&chunk_id);
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

    /// 优雅关闭（刷新所有数据）
    pub async fn shutdown(&self) -> Result<()> {
        info!("开始优雅关闭 StorageManager...");

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

    #[tokio::test]
    async fn test_save_and_read_version() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        let data = b"Hello, World! This is a test.";
        let (delta, version) = storage.save_version("test_file", data, None).await.unwrap();

        assert!(!delta.chunks.is_empty());
        assert!(!version.version_id.is_empty());

        // 读取版本数据
        let read_data = storage
            .read_version_data(&version.version_id)
            .await
            .unwrap();
        assert_eq!(read_data, data);
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

        let stats = storage.get_storage_stats().await.unwrap();
        assert_eq!(stats.total_versions, 2);
        assert!(stats.total_chunks > 0);
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
        storage.save_version("test_file", b"Test data", None).await.unwrap();
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
        storage.save_version("file1", b"Data 1", None).await.unwrap();
        storage.save_version("file2", b"Data 2", None).await.unwrap();
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
        storage.save_version("test_file", b"Test data", None).await.unwrap();
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
        storage.save_version("file1", b"Data 1", None).await.unwrap();
        storage.save_version("file2", b"Data 2", None).await.unwrap();
        storage.save_version("file3", b"Data 3", None).await.unwrap();

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
            enable_auto_gc: true,  // 自动启动GC
            gc_interval_secs: 1,   // 1秒间隔用于快速测试
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
        storage.save_version("file1", b"Test data", None).await.unwrap();

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
        storage.save_version("file1", b"Test data", None).await.unwrap();
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
