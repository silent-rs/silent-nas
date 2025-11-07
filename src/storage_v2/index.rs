//! 全局块索引管理
//!
//! 用于跨文件去重的块级索引，支持：
//! - 块哈希映射
//! - 引用计数管理
//! - 内存热索引 + 磁盘持久化
use tracing::{info, warn};

use crate::error::{NasError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;

/// 块索引条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockIndexEntry {
    /// 块ID（SHA-256哈希）
    pub chunk_id: String,
    /// 块大小
    pub size: usize,
    /// 引用计数
    pub ref_count: u32,
    /// 块存储路径
    pub storage_path: PathBuf,
    /// 最后访问时间
    pub last_accessed: chrono::NaiveDateTime,
    /// 创建时间
    pub created_at: chrono::NaiveDateTime,
    /// 是否为热点数据
    pub is_hot: bool,
}

/// 块索引管理器
pub struct BlockIndex {
    /// 索引配置
    config: BlockIndexConfig,
    /// 内存热索引（常驻内存）
    hot_index: Arc<RwLock<HashMap<String, BlockIndexEntry>>>,
    /// 块根目录
    block_root: PathBuf,
    /// 索引文件路径
    index_path: PathBuf,
}

impl BlockIndex {
    pub fn new(config: BlockIndexConfig, root_path: &str) -> Self {
        let block_root = Path::new(root_path).join("blocks");
        let index_path = block_root.join("index.json");

        Self {
            config,
            hot_index: Arc::new(RwLock::new(HashMap::new())),
            block_root,
            index_path,
        }
    }

    /// 初始化块索引
    pub async fn init(&self) -> Result<()> {
        // 创建目录
        fs::create_dir_all(&self.block_root).await?;

        // 加载现有索引
        self.load_index().await?;

        info!("块索引初始化完成: {:?}", self.block_root);
        Ok(())
    }

    /// 添加块到索引
    pub async fn add_block(
        &self,
        chunk_id: &str,
        size: usize,
        storage_path: PathBuf,
    ) -> Result<BlockIndexEntry> {
        let mut index = self.hot_index.write().await;

        let now = chrono::Local::now().naive_local();

        let entry = if let Some(mut existing) = index.get_mut(chunk_id) {
            // 块已存在，增加引用计数
            existing.ref_count += 1;
            existing.last_accessed = now;
            existing.clone()
        } else {
            // 新块
            let entry = BlockIndexEntry {
                chunk_id: chunk_id.to_string(),
                size,
                ref_count: 1,
                storage_path: storage_path.clone(),
                last_accessed: now,
                created_at: now,
                is_hot: false,
            };
            index.insert(chunk_id.to_string(), entry.clone());
            entry
        };

        // 持久化索引
        self.save_index().await?;

        info!("添加块到索引: {}, ref_count: {}", chunk_id, entry.ref_count);
        Ok(entry)
    }

    /// 获取块信息
    pub async fn get_block(&self, chunk_id: &str) -> Option<BlockIndexEntry> {
        let mut index = self.hot_index.write().await;

        if let Some(entry) = index.get_mut(chunk_id) {
            // 更新访问时间
            entry.last_accessed = chrono::Local::now().naive_local();
            return Some(entry.clone());
        }

        None
    }

    /// 检查块是否存在
    pub async fn contains(&self, chunk_id: &str) -> bool {
        let index = self.hot_index.read().await;
        index.contains_key(chunk_id)
    }

    /// 增加引用计数
    pub async fn inc_ref(&self, chunk_id: &str) -> Result<u32> {
        let mut index = self.hot_index.write().await;

        if let Some(entry) = index.get_mut(chunk_id) {
            entry.ref_count += 1;
            entry.last_accessed = chrono::Local::now().naive_local();
            self.save_index().await?;
            return Ok(entry.ref_count);
        }

        Err(NasError::Other(format!("块不存在: {}", chunk_id)))
    }

    /// 减少引用计数
    pub async fn dec_ref(&self, chunk_id: &str) -> Result<u32> {
        let mut index = self.hot_index.write().await;

        if let Some(entry) = index.get_mut(chunk_id) {
            if entry.ref_count > 0 {
                entry.ref_count -= 1;
                entry.last_accessed = chrono::Local::now().naive_local();

                // 如果引用计数为0，标记为可删除
                if entry.ref_count == 0 {
                    info!("块引用计数为0: {}", chunk_id);
                }

                self.save_index().await?;
                return Ok(entry.ref_count);
            }
        }

        Err(NasError::Other(format!("块不存在或引用计数异常: {}", chunk_id)))
    }

    /// 移除块
    pub async fn remove_block(&self, chunk_id: &str) -> Result<()> {
        let mut index = self.hot_index.write().await;

        if let Some(entry) = index.remove(chunk_id) {
            // 实际删除文件由调用者处理
            self.save_index().await?;
            info!("从索引中移除块: {}", chunk_id);
        }

        Ok(())
    }

    /// 获取所有块信息
    pub async fn get_all_blocks(&self) -> Vec<BlockIndexEntry> {
        let index = self.hot_index.read().await;
        index.values().cloned().collect()
    }

    /// 获取统计信息
    pub async fn get_stats(&self) -> BlockIndexStats {
        let index = self.hot_index.read().await;

        let total_blocks = index.len();
        let hot_blocks = index.values().filter(|e| e.is_hot).count();
        let total_refs: u32 = index.values().map(|e| e.ref_count).sum();
        let total_size: usize = index.values().map(|e| e.size).sum();

        BlockIndexStats {
            total_blocks,
            hot_blocks,
            total_refs,
            total_size,
        }
    }

    /// 清理未引用的块
    pub async fn gc_unreferenced(&self) -> Result<Vec<String>> {
        let mut index = self.hot_index.write().await;
        let mut unreferenced = Vec::new();

        // 查找引用计数为0的块
        for (chunk_id, entry) in index.iter() {
            if entry.ref_count == 0 {
                unreferenced.push(chunk_id.clone());
            }
        }

        // 移除未引用的块
        for chunk_id in &unreferenced {
            index.remove(chunk_id);
        }

        if !unreferenced.is_empty() {
            self.save_index().await?;
            info!("清理了 {} 个未引用的块", unreferenced.len());
        }

        Ok(unreferenced)
    }

    /// 更新热数据标记
    pub async fn update_hot_flags(&self) -> Result<()> {
        let now = chrono::Local::now().naive_local();
        let hot_threshold = chrono::Duration::minutes(30); // 30分钟内访问的为热数据

        let mut index = self.hot_index.write().await;

        for entry in index.values_mut() {
            entry.is_hot = now.signed_duration_since(entry.last_accessed) < hot_threshold;
        }

        Ok(())
    }

    /// 加载索引
    async fn load_index(&self) -> Result<()> {
        if !self.index_path.exists() {
            info!("索引文件不存在，跳过加载");
            return Ok(());
        }

        let data = fs::read(&self.index_path).await.map_err(NasError::Io)?;
        let entries: Vec<BlockIndexEntry> = serde_json::from_slice(&data)
            .map_err(|e| NasError::Other(format!("反序列化索引失败: {}", e)))?;

        let mut index = self.hot_index.write().await;
        for entry in entries {
            index.insert(entry.chunk_id.clone(), entry);
        }

        info!("加载了 {} 个块索引", index.len());
        Ok(())
    }

    /// 保存索引
    async fn save_index(&self) -> Result<()> {
        let index = self.hot_index.read().await;
        let entries: Vec<BlockIndexEntry> = index.values().cloned().collect();

        let data = serde_json::to_vec_pretty(&entries)
            .map_err(|e| NasError::Other(format!("序列化索引失败: {}", e)))?;

        // 原子性写入
        let temp_path = self.index_path.with_extension("tmp");
        fs::write(&temp_path, data).await.map_err(NasError::Io)?;
        fs::rename(temp_path, &self.index_path).await.map_err(NasError::Io)?;

        Ok(())
    }

    /// 获取块存储路径
    pub fn get_block_path(&self, chunk_id: &str) -> PathBuf {
        let prefix = &chunk_id[..2.min(chunk_id.len())];
        self.block_root.join(prefix).join(chunk_id)
    }
}

/// 块索引配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockIndexConfig {
    /// 内存索引大小限制（条目数）
    pub max_memory_entries: usize,
    /// 热数据比例（0-1）
    pub hot_data_ratio: f64,
    /// 索引自动保存间隔（秒）
    pub save_interval_secs: u64,
    /// GC间隔（秒）
    pub gc_interval_secs: u64,
}

impl Default for BlockIndexConfig {
    fn default() -> Self {
        Self {
            max_memory_entries: 100000,
            hot_data_ratio: 0.2,
            save_interval_secs: 300,
            gc_interval_secs: 3600,
        }
    }
}

/// 块索引统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockIndexStats {
    pub total_blocks: usize,
    pub hot_blocks: usize,
    pub total_refs: u32,
    pub total_size: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_index() -> (BlockIndex, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = BlockIndexConfig::default();
        let index = BlockIndex::new(config, temp_dir.path().to_str().unwrap());
        (index, temp_dir)
    }

    #[tokio::test]
    async fn test_add_and_get_block() {
        let (index, _temp) = create_test_index().await;
        index.init().await.unwrap();

        let chunk_id = "test_chunk_123";
        let size = 1024;
        let storage_path = index.get_block_path(chunk_id);

        let entry = index.add_block(chunk_id, size, storage_path.clone()).await.unwrap();
        assert_eq!(entry.chunk_id, chunk_id);
        assert_eq!(entry.ref_count, 1);

        let retrieved = index.get_block(chunk_id).await.unwrap();
        assert_eq!(retrieved.chunk_id, chunk_id);
    }

    #[tokio::test]
    async fn test_inc_dec_ref() {
        let (index, _temp) = create_test_index().await;
        index.init().await.unwrap();

        let chunk_id = "test_chunk_456";
        let storage_path = index.get_block_path(chunk_id);
        index.add_block(chunk_id, 2048, storage_path).await.unwrap();

        let ref_count = index.inc_ref(chunk_id).await.unwrap();
        assert_eq!(ref_count, 2);

        let ref_count = index.dec_ref(chunk_id).await.unwrap();
        assert_eq!(ref_count, 1);
    }

    #[tokio::test]
    async fn test_contains() {
        let (index, _temp) = create_test_index().await;
        index.init().await.unwrap();

        assert!(!index.contains("nonexistent").await);

        let storage_path = index.get_block_path("exists");
        index.add_block("exists", 1024, storage_path).await.unwrap();

        assert!(index.contains("exists").await);
    }

    #[tokio::test]
    async fn test_get_stats() {
        let (index, _temp) = create_test_index().await;
        index.init().await.unwrap();

        let storage_path1 = index.get_block_path("chunk1");
        let storage_path2 = index.get_block_path("chunk2");

        index.add_block("chunk1", 1024, storage_path1).await.unwrap();
        index.add_block("chunk2", 2048, storage_path2).await.unwrap();

        let stats = index.get_stats().await;
        assert_eq!(stats.total_blocks, 2);
        assert_eq!(stats.total_refs, 2);
        assert_eq!(stats.total_size, 3072);
    }
}
