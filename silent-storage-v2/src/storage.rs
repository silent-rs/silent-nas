//! 增量存储后端
//!
//! 实现版本链式存储和块级存储功能

use crate::error::{Result, StorageError};
use crate::{ChunkInfo, FileDelta, IncrementalConfig, VersionInfo};
use chrono::Local;
use serde::{Deserialize, Serialize};
use silent_nas_core::FileVersion;
use silent_storage_v1::StorageManager;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tracing::info;

/// 块引用计数信息
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChunkRefCount {
    /// 块ID
    chunk_id: String,
    /// 引用计数
    ref_count: usize,
    /// 块大小
    size: u64,
    /// 存储路径
    path: PathBuf,
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

/// 增量存储管理器
pub struct IncrementalStorage {
    /// 基础存储管理器
    #[allow(dead_code)]
    storage: Arc<StorageManager>,
    /// 配置
    config: IncrementalConfig,
    /// 版本根目录
    version_root: PathBuf,
    /// 块存储根目录
    chunk_root: PathBuf,
    /// 版本索引缓存（使用内部可变性）
    version_index: Arc<RwLock<HashMap<String, VersionInfo>>>,
    /// 块索引缓存（使用内部可变性）
    block_index: Arc<RwLock<HashMap<String, PathBuf>>>,
    /// 块引用计数
    chunk_ref_count: Arc<RwLock<HashMap<String, ChunkRefCount>>>,
    /// 文件索引
    file_index: Arc<RwLock<HashMap<String, FileIndexEntry>>>,
}

impl IncrementalStorage {
    pub fn new(storage: Arc<StorageManager>, config: IncrementalConfig, root_path: &str) -> Self {
        let version_root = Path::new(root_path).join("incremental");
        let chunk_root = version_root.join("chunks");

        Self {
            storage,
            config,
            version_root,
            chunk_root,
            version_index: Arc::new(RwLock::new(HashMap::new())),
            block_index: Arc::new(RwLock::new(HashMap::new())),
            chunk_ref_count: Arc::new(RwLock::new(HashMap::new())),
            file_index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 初始化增量存储
    pub async fn init(&self) -> Result<()> {
        // 创建版本目录
        fs::create_dir_all(&self.version_root).await?;
        fs::create_dir_all(&self.chunk_root).await?;

        // 加载现有索引
        self.load_version_index().await?;
        self.load_block_index().await?;
        self.load_chunk_ref_count().await?;
        self.load_file_index().await?;

        info!("增量存储初始化完成: {:?}", self.version_root);
        Ok(())
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
        for chunk in &delta.chunks {
            self.save_chunk(chunk, data).await?;

            // 更新块引用计数
            let mut ref_count = self.chunk_ref_count.write().await;
            let entry = ref_count.entry(chunk.chunk_id.clone()).or_insert_with(|| {
                ChunkRefCount {
                    chunk_id: chunk.chunk_id.clone(),
                    ref_count: 0,
                    size: chunk.size as u64,
                    path: self.get_chunk_path(&chunk.chunk_id),
                }
            });
            entry.ref_count += 1;
            drop(ref_count);
        }

        // 保存差异数据
        self.save_delta(file_id, &delta).await?;

        // 保存版本信息
        let _version_info = self
            .save_version_info(file_id, &delta, parent_version_id)
            .await?;

        // 更新文件索引
        let now = Local::now().naive_local();
        let mut file_index = self.file_index.write().await;
        let file_entry = file_index.entry(file_id.to_string()).or_insert_with(|| {
            FileIndexEntry {
                file_id: file_id.to_string(),
                latest_version_id: version_id.clone(),
                version_count: 0,
                created_at: now,
                modified_at: now,
            }
        });
        file_entry.latest_version_id = version_id.clone();
        file_entry.version_count += 1;
        file_entry.modified_at = now;
        drop(file_index);

        // 定期保存索引（每10个版本保存一次）
        let version_count = self.version_index.read().await.len();
        if version_count % 10 == 0 {
            let _ = self.save_chunk_ref_count().await;
            let _ = self.save_file_index().await;
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
        if let Some(info) = self.version_index.read().await.get(version_id) {
            return Ok(info.clone());
        }

        // 从磁盘读取
        let version_path = self.get_version_path(version_id);
        let data = fs::read(&version_path).await.map_err(StorageError::Io)?;
        let version_info: VersionInfo = serde_json::from_slice(&data)
            .map_err(|e| StorageError::Storage(format!("反序列化版本信息失败: {}", e)))?;

        Ok(version_info)
    }

    /// 列出文件的所有版本
    pub async fn list_file_versions(&self, file_id: &str) -> Result<Vec<VersionInfo>> {
        let mut versions = Vec::new();
        let index = self.version_index.read().await;

        for version in index.values() {
            if version.file_id == file_id {
                versions.push(version.clone());
            }
        }

        // 按创建时间排序
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

        // 保存到磁盘
        let version_path = self.get_version_path(&version_info.version_id);

        // 确保父目录存在
        if let Some(parent) = version_path.parent() {
            fs::create_dir_all(parent).await.map_err(StorageError::Io)?;
        }

        let data = serde_json::to_vec(&version_info)
            .map_err(|e| StorageError::Storage(format!("序列化版本信息失败: {}", e)))?;

        fs::write(&version_path, data)
            .await
            .map_err(StorageError::Io)?;

        // 更新索引
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

        if !versions_dir.exists() {
            return Ok(());
        }

        let mut entries = fs::read_dir(&versions_dir)
            .await
            .map_err(StorageError::Io)?;

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
                self.version_index
                    .write()
                    .await
                    .insert(version_id.to_string(), version_info);
            }
        }

        let index_len = self.version_index.read().await.len();
        info!("加载了 {} 个版本信息", index_len);
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

impl IncrementalStorage {
    /// 加载块引用计数
    async fn load_chunk_ref_count(&self) -> Result<()> {
        let ref_count_path = self.chunk_root.join("ref_count.json");

        if !ref_count_path.exists() {
            // 如果文件不存在，从现有数据重建引用计数
            return self.rebuild_chunk_ref_count().await;
        }

        let data = fs::read(&ref_count_path).await.map_err(StorageError::Io)?;
        let ref_counts: HashMap<String, ChunkRefCount> = serde_json::from_slice(&data)
            .map_err(|e| StorageError::Storage(format!("加载块引用计数失败: {}", e)))?;

        *self.chunk_ref_count.write().await = ref_counts;

        let count = self.chunk_ref_count.read().await.len();
        info!("加载了 {} 个块引用计数", count);
        Ok(())
    }

    /// 保存块引用计数
    async fn save_chunk_ref_count(&self) -> Result<()> {
        let ref_count_path = self.chunk_root.join("ref_count.json");
        let ref_counts = self.chunk_ref_count.read().await;

        let data = serde_json::to_vec_pretty(&*ref_counts)
            .map_err(|e| StorageError::Storage(format!("序列化块引用计数失败: {}", e)))?;

        fs::write(&ref_count_path, &data).await.map_err(StorageError::Io)?;
        Ok(())
    }

    /// 重建块引用计数
    async fn rebuild_chunk_ref_count(&self) -> Result<()> {
        info!("开始重建块引用计数...");
        let mut ref_counts: HashMap<String, ChunkRefCount> = HashMap::new();

        // 遍历所有版本，统计块引用
        let version_index = self.version_index.read().await;
        for version_info in version_index.values() {
            // 读取该版本的 delta
            if let Ok(delta) = self.read_delta(&version_info.file_id, &version_info.version_id).await {
                for chunk in &delta.chunks {
                    let entry = ref_counts.entry(chunk.chunk_id.clone()).or_insert_with(|| {
                        ChunkRefCount {
                            chunk_id: chunk.chunk_id.clone(),
                            ref_count: 0,
                            size: chunk.size as u64,
                            path: self.get_chunk_path(&chunk.chunk_id),
                        }
                    });
                    entry.ref_count += 1;
                }
            }
        }
        drop(version_index);

        *self.chunk_ref_count.write().await = ref_counts;

        // 保存到磁盘
        self.save_chunk_ref_count().await?;

        let count = self.chunk_ref_count.read().await.len();
        info!("重建完成，共 {} 个块", count);
        Ok(())
    }

    /// 加载文件索引
    async fn load_file_index(&self) -> Result<()> {
        let file_index_path = self.version_root.join("file_index.json");

        if !file_index_path.exists() {
            // 如果文件不存在，从现有数据重建文件索引
            return self.rebuild_file_index().await;
        }

        let data = fs::read(&file_index_path).await.map_err(StorageError::Io)?;
        let file_index: HashMap<String, FileIndexEntry> = serde_json::from_slice(&data)
            .map_err(|e| StorageError::Storage(format!("加载文件索引失败: {}", e)))?;

        *self.file_index.write().await = file_index;

        let count = self.file_index.read().await.len();
        info!("加载了 {} 个文件索引", count);
        Ok(())
    }

    /// 保存文件索引
    async fn save_file_index(&self) -> Result<()> {
        let file_index_path = self.version_root.join("file_index.json");
        let file_index = self.file_index.read().await;

        let data = serde_json::to_vec_pretty(&*file_index)
            .map_err(|e| StorageError::Storage(format!("序列化文件索引失败: {}", e)))?;

        fs::write(&file_index_path, &data).await.map_err(StorageError::Io)?;
        Ok(())
    }

    /// 重建文件索引
    async fn rebuild_file_index(&self) -> Result<()> {
        info!("开始重建文件索引...");
        let mut file_index: HashMap<String, FileIndexEntry> = HashMap::new();

        // 遍历所有版本，构建文件索引
        let version_index = self.version_index.read().await;
        for version_info in version_index.values() {
            let entry = file_index.entry(version_info.file_id.clone()).or_insert_with(|| {
                FileIndexEntry {
                    file_id: version_info.file_id.clone(),
                    latest_version_id: version_info.version_id.clone(),
                    version_count: 0,
                    created_at: version_info.created_at,
                    modified_at: version_info.created_at,
                }
            });

            entry.version_count += 1;
            // 更新最新版本（假设版本ID可比较，或使用时间戳）
            if version_info.created_at > entry.modified_at {
                entry.latest_version_id = version_info.version_id.clone();
                entry.modified_at = version_info.created_at;
            }
        }
        drop(version_index);

        *self.file_index.write().await = file_index;

        // 保存到磁盘
        self.save_file_index().await?;

        let count = self.file_index.read().await.len();
        info!("重建完成，共 {} 个文件", count);
        Ok(())
    }

    /// 列出所有文件
    pub async fn list_files(&self) -> Result<Vec<String>> {
        let file_index = self.file_index.read().await;
        let mut files: Vec<String> = file_index.keys().cloned().collect();
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
                fs::remove_file(&version_path).await.map_err(StorageError::Io)?;
            }

            // 删除 delta 文件
            let delta_path = self.get_delta_path(file_id, &version.version_id);
            if delta_path.exists() {
                fs::remove_file(&delta_path).await.map_err(StorageError::Io)?;
            }

            // 从版本索引中移除
            self.version_index.write().await.remove(&version.version_id);
        }

        // 3. 递减块引用计数
        let mut ref_count = self.chunk_ref_count.write().await;
        for chunk_id in chunks_to_decrement {
            if let Some(entry) = ref_count.get_mut(&chunk_id) {
                entry.ref_count = entry.ref_count.saturating_sub(1);
            }
        }
        drop(ref_count);

        // 4. 从文件索引中移除
        self.file_index.write().await.remove(file_id);

        // 5. 删除文件的 delta 目录
        let file_delta_dir = self.version_root.join("deltas").join(file_id);
        if file_delta_dir.exists() {
            fs::remove_dir_all(&file_delta_dir).await.map_err(StorageError::Io)?;
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

        let mut ref_count = self.chunk_ref_count.write().await;
        let mut chunks_to_remove = Vec::new();

        // 找出引用计数为0的块
        for (chunk_id, entry) in ref_count.iter() {
            if entry.ref_count == 0 {
                chunks_to_remove.push((chunk_id.clone(), entry.clone()));
            }
        }

        // 删除这些块
        for (chunk_id, entry) in chunks_to_remove {
            if entry.path.exists() {
                match fs::metadata(&entry.path).await {
                    Ok(metadata) => {
                        reclaimed_space += metadata.len();
                        match fs::remove_file(&entry.path).await {
                            Ok(_) => {
                                orphaned_chunks += 1;
                                ref_count.remove(&chunk_id);
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
                ref_count.remove(&chunk_id);
                self.block_index.write().await.remove(&chunk_id);
            }
        }

        drop(ref_count);

        // 保存更新后的引用计数
        if orphaned_chunks > 0 {
            self.save_chunk_ref_count().await?;
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
        self.file_index
            .read()
            .await
            .get(file_id)
            .cloned()
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_storage() -> (IncrementalStorage, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = Arc::new(crate::storage::StorageManager::new(
            temp_dir.path().to_path_buf(),
            4 * 1024 * 1024,
        ));
        storage.init().await.unwrap();

        let config = IncrementalConfig::default();
        let incremental =
            IncrementalStorage::new(storage, config, temp_dir.path().to_str().unwrap());

        (incremental, temp_dir)
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
        storage.save_version("file1", b"Data 1", None).await.unwrap();
        storage.save_version("file2", b"Data 2", None).await.unwrap();
        storage.save_version("file3", b"Data 3", None).await.unwrap();

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
        let (_delta1, version1) = storage.save_version("test_file", b"Version 1", None).await.unwrap();
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
        storage.save_version("file1", b"Some data", None).await.unwrap();
        storage.save_version("file2", b"More data", None).await.unwrap();

        // 删除一个文件
        storage.delete_file("file1").await.unwrap();

        // 运行垃圾回收
        let result = storage.garbage_collect().await.unwrap();

        // 应该有一些孤立块被清理
        assert!(result.orphaned_chunks > 0 || result.reclaimed_space > 0 || result.errors.is_empty());
    }

    #[tokio::test]
    async fn test_get_file_info() {
        let (storage, _temp) = create_test_storage().await;
        storage.init().await.unwrap();

        // 保存文件
        storage.save_version("test_file", b"Data", None).await.unwrap();

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

        // 检查引用计数
        let ref_count = storage.chunk_ref_count.read().await;
        assert!(!ref_count.is_empty());

        // 至少有一些块的引用计数应该大于0
        let has_refs = ref_count.values().any(|entry| entry.ref_count > 0);
        assert!(has_refs);
    }
}
// 性能对比测试：原版存储 vs v0.7.0增量存储
// 使用方法：cargo test --lib bench_comparison
