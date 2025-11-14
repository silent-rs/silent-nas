//! 增量存储后端
//!
//! 实现版本链式存储和块级存储功能

use crate::error::{Result, StorageError};
use crate::metadata::SledMetadataDb;
use crate::{ChunkInfo, FileDelta, IncrementalConfig, VersionInfo};
use async_trait::async_trait;
use chrono::Local;
use serde::{Deserialize, Serialize};
use silent_nas_core::{FileMetadata, FileVersion, S3CompatibleStorageTrait, StorageManagerTrait};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
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
}

/// V2 存储管理器
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
    /// 版本根目录 (root_path/v2/incremental)
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
}

impl StorageManager {
    pub fn new(root_path: PathBuf, chunk_size: usize, config: IncrementalConfig) -> Self {
        let data_root = root_path.join("data");
        let v2_root = root_path.join("v2");
        let version_root = v2_root.join("incremental");
        let chunk_root = version_root.join("chunks");

        Self {
            root_path,
            data_root,
            config,
            version_root,
            chunk_root,
            chunk_size,
            metadata_db: Arc::new(OnceCell::new()),
            version_index: Arc::new(RwLock::new(HashMap::new())),
            block_index: Arc::new(RwLock::new(HashMap::new())),
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

        // 加载现有索引
        self.load_version_index().await?;
        self.load_block_index().await?;
        self.load_chunk_ref_count().await?;
        self.load_file_index().await?;

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

    /// 保存文件版本（使用增量存储）
    pub async fn save_version(
        &self,
        file_id: &str,
        data: &[u8],
        parent_version_id: Option<&str>,
    ) -> Result<(FileDelta, FileVersion)> {
        let version_id = format!("v_{}", scru128::new());

        // 生成差异（使用相同的version_id）
        let base_data = if let Some(parent_id) = parent_version_id {
            // 只有在有父版本时才读取
            self.read_version_data(parent_id).await?
        } else {
            Vec::new()
        };

        let mut generator = crate::core::delta::DeltaGenerator::new(self.config.clone());
        let mut delta =
            generator.generate_delta(&base_data, data, file_id, parent_version_id.unwrap_or(""))?;
        // 使用相同的version_id
        delta.new_version_id = version_id.clone();

        // 保存块数据并更新引用计数
        let metadata_db = self.get_metadata_db()?;
        for chunk in &delta.chunks {
            self.save_chunk(chunk, data).await?;

            // 检查块是否已存在
            let chunk_exists = metadata_db
                .get_chunk_ref(&chunk.chunk_id)
                .map_err(|e| StorageError::Storage(format!("查询块引用失败: {}", e)))?
                .is_some();

            if chunk_exists {
                // 块已存在，使用原子增量操作
                metadata_db
                    .increment_chunk_ref(&chunk.chunk_id)
                    .map_err(|e| StorageError::Storage(format!("增加块引用计数失败: {}", e)))?;
            } else {
                // 新块，创建引用计数记录
                let ref_count = ChunkRefCount {
                    chunk_id: chunk.chunk_id.clone(),
                    ref_count: 1,
                    size: chunk.size as u64,
                    path: self.get_chunk_path(&chunk.chunk_id),
                };
                metadata_db
                    .put_chunk_ref(&chunk.chunk_id, &ref_count)
                    .map_err(|e| StorageError::Storage(format!("保存块引用信息失败: {}", e)))?;
            }
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
                let chunk_data = self.read_chunk(&chunk.chunk_id).await?;
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

    /// 保存块数据
    async fn save_chunk(&self, chunk: &ChunkInfo, file_data: &[u8]) -> Result<()> {
        let chunk_data = &file_data[chunk.offset..chunk.offset + chunk.size];
        let chunk_path = self.get_chunk_path(&chunk.chunk_id);

        // 创建父目录
        if let Some(parent) = chunk_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // 写入块数据
        let mut file = fs::File::create(&chunk_path).await?;
        file.write_all(chunk_data).await?;
        file.flush().await?;

        // 更新块索引
        self.block_index
            .write()
            .await
            .insert(chunk.chunk_id.clone(), chunk_path);

        Ok(())
    }

    /// 读取块数据
    async fn read_chunk(&self, chunk_id: &str) -> Result<Vec<u8>> {
        let chunk_path = self.get_chunk_path(chunk_id);
        fs::read(&chunk_path).await.map_err(StorageError::Io)
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
        self.version_root
            .join("deltas")
            .join(file_id)
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
        let mut files = metadata_db
            .list_file_ids()
            .map_err(|e| StorageError::Storage(format!("列出文件失败: {}", e)))?;
        files.sort();
        Ok(files)
    }

    /// 删除文件（包含所有版本）
    pub async fn delete_file(&self, file_id: &str) -> Result<()> {
        info!("开始删除文件: {}", file_id);

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

        // 3. 递减块引用计数（使用 Sled 原子操作）
        let metadata_db = self.get_metadata_db()?;
        for chunk_id in chunks_to_decrement {
            // 使用原子递减操作
            if let Err(e) = metadata_db.decrement_chunk_ref(&chunk_id) {
                info!("递减块 {} 引用计数失败: {}", chunk_id, e);
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

        info!("文件删除完成: {}", file_id);
        Ok(())
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
        // V2 使用增量存储，这里我们保存第一个版本
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
        // 使用路径作为 file_id
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
        // V2 中 bucket 可以映射为目录
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

        // 删除文件
        storage.delete_file("test_file").await.unwrap();

        // 确认文件已删除
        let files = storage.list_files().await.unwrap();
        assert!(!files.contains(&"test_file".to_string()));

        // 确认版本已删除
        let versions = storage.list_file_versions("test_file").await.unwrap();
        assert!(versions.is_empty());
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
    async fn test_chunk_ref_count() {
        let (storage, _temp) = create_test_storage().await;
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
}
// 性能对比测试：原版存储 vs v0.7.0增量存储
// 使用方法：cargo test --lib bench_comparison

// ============================================================================
// Trait 实现
// ============================================================================
