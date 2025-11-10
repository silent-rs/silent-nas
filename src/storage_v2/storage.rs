//! 增量存储后端
//!
//! 实现版本链式存储和块级存储功能

use crate::error::{NasError, Result};
use crate::models::FileVersion;
use crate::storage::StorageManager;
use crate::storage_v2::{ChunkInfo, FileDelta, IncrementalConfig, VersionInfo};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::info;

/// 增量存储管理器
pub struct IncrementalStorage {
    /// 基础存储管理器
    storage: Arc<StorageManager>,
    /// 配置
    config: IncrementalConfig,
    /// 版本根目录
    version_root: PathBuf,
    /// 块存储根目录
    chunk_root: PathBuf,
    /// 版本索引缓存
    version_index: HashMap<String, VersionInfo>,
    /// 块索引缓存
    block_index: HashMap<String, PathBuf>,
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
            version_index: HashMap::new(),
            block_index: HashMap::new(),
        }
    }

    /// 初始化增量存储
    pub async fn init(&mut self) -> Result<()> {
        // 创建版本目录
        fs::create_dir_all(&self.version_root).await?;
        fs::create_dir_all(&self.chunk_root).await?;

        // 加载现有索引
        self.load_version_index().await?;
        self.load_block_index().await?;

        info!("增量存储初始化完成: {:?}", self.version_root);
        Ok(())
    }

    /// 保存文件版本（使用增量存储）
    pub async fn save_version(
        &mut self,
        file_id: &str,
        data: &[u8],
        parent_version_id: Option<&str>,
    ) -> Result<(FileDelta, FileVersion)> {
        let version_id = format!("v_{}", scru128::new());

        // 生成差异
        let base_data = if let Some(parent_id) = parent_version_id {
            self.read_version_data(parent_id).await?
        } else {
            Vec::new()
        };

        let mut generator = crate::storage_v2::delta::DeltaGenerator::new(self.config.clone());
        let delta =
            generator.generate_delta(&base_data, data, file_id, parent_version_id.unwrap_or(""))?;

        // 保存块数据
        for chunk in &delta.chunks {
            self.save_chunk(chunk, data).await?;
        }

        // 保存版本信息
        let _version_info = self
            .save_version_info(file_id, &delta, parent_version_id)
            .await?;

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
        if let Some(info) = self.version_index.get(version_id) {
            return Ok(info.clone());
        }

        // 从磁盘读取
        let version_path = self.get_version_path(version_id);
        let data = fs::read(&version_path).await.map_err(NasError::Io)?;
        let version_info: VersionInfo = serde_json::from_slice(&data)
            .map_err(|e| NasError::Other(format!("反序列化版本信息失败: {}", e)))?;

        Ok(version_info)
    }

    /// 列出文件的所有版本
    pub async fn list_file_versions(&self, file_id: &str) -> Result<Vec<VersionInfo>> {
        let mut versions = Vec::new();

        for version in self.version_index.values() {
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

        for version in self.version_index.values() {
            total_versions += 1;
            total_size += version.storage_size;
            total_chunks += version.chunk_count;
        }

        // 计算唯一块数量
        let mut chunk_sizes: HashMap<String, u64> = HashMap::new();
        for (chunk_id, chunk_path) in &self.block_index {
            if let Ok(metadata) = fs::metadata(chunk_path).await {
                chunk_sizes.insert(chunk_id.clone(), metadata.len());
                unique_chunks += 1;
            }
        }

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
    async fn save_chunk(&mut self, chunk: &ChunkInfo, file_data: &[u8]) -> Result<()> {
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
        self.block_index.insert(chunk.chunk_id.clone(), chunk_path);

        Ok(())
    }

    /// 读取块数据
    async fn read_chunk(&self, chunk_id: &str) -> Result<Vec<u8>> {
        let chunk_path = self.get_chunk_path(chunk_id);
        fs::read(&chunk_path).await.map_err(NasError::Io)
    }

    /// 保存版本信息
    async fn save_version_info(
        &mut self,
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
        let data = serde_json::to_vec(&version_info)
            .map_err(|e| NasError::Other(format!("序列化版本信息失败: {}", e)))?;

        fs::write(&version_path, data).await.map_err(NasError::Io)?;

        // 更新索引
        self.version_index
            .insert(version_info.version_id.clone(), version_info.clone());

        Ok(version_info)
    }

    /// 读取差异数据
    async fn read_delta(&self, file_id: &str, version_id: &str) -> Result<FileDelta> {
        let delta_path = self.get_delta_path(file_id, version_id);
        let data = fs::read(&delta_path).await.map_err(NasError::Io)?;
        let delta: FileDelta = serde_json::from_slice(&data)
            .map_err(|e| NasError::Other(format!("反序列化差异数据失败: {}", e)))?;

        Ok(delta)
    }

    /// 加载版本索引
    async fn load_version_index(&mut self) -> Result<()> {
        let versions_dir = self.version_root.join("versions");

        if !versions_dir.exists() {
            return Ok(());
        }

        let mut entries = fs::read_dir(&versions_dir).await.map_err(NasError::Io)?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file()
                && path.extension().and_then(|s| s.to_str()) == Some("json")
                && let Some(file_name) = path.file_name().and_then(|s| s.to_str())
            {
                let version_id = file_name.strip_suffix(".json").unwrap_or(file_name);
                let data = fs::read(&path).await.map_err(NasError::Io)?;
                let version_info: VersionInfo = serde_json::from_slice(&data)
                    .map_err(|e| NasError::Other(format!("加载版本信息失败: {}", e)))?;
                self.version_index
                    .insert(version_id.to_string(), version_info);
            }
        }

        info!("加载了 {} 个版本信息", self.version_index.len());
        Ok(())
    }

    /// 加载块索引
    async fn load_block_index(&mut self) -> Result<()> {
        let chunks_dir = self.chunk_root.join("data");

        if !chunks_dir.exists() {
            return Ok(());
        }

        let mut entries = fs::read_dir(&chunks_dir).await.map_err(NasError::Io)?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file()
                && let Some(file_name) = path.file_name().and_then(|s| s.to_str())
            {
                self.block_index.insert(file_name.to_string(), path);
            }
        }

        info!("加载了 {} 个块索引", self.block_index.len());
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
        let (mut storage, _temp) = create_test_storage().await;
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
        let (mut storage, _temp) = create_test_storage().await;
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
        let (mut storage, _temp) = create_test_storage().await;
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
}
