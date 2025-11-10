//! 跨文件块级去重模块
//!
//! 实现高效的块级去重功能，包括：
//! - 块哈希映射和查找
//! - 引用计数管理
//! - Copy-on-Write 机制
//! - 批量去重操作
//! - 并发去重控制

use crate::error::Result;
use crate::storage_v2::{BlockIndex, BlockIndexConfig};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::RwLock as AsyncRwLock;
use tracing::{debug, info};

/// 去重配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupConfig {
    /// 启用去重
    pub enable_dedup: bool,
    /// 最小去重块大小（字节）
    pub min_dedup_size: usize,
    /// 批处理大小
    pub batch_size: usize,
    /// 并发去重线程数
    pub concurrent_threads: usize,
    /// 内存索引大小限制
    pub max_memory_index: usize,
    /// 同步间隔（秒）
    pub sync_interval_secs: u64,
    /// 启用Copy-on-Write
    pub enable_cow: bool,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            enable_dedup: true,
            min_dedup_size: 1024, // 1KB
            batch_size: 1000,
            concurrent_threads: 4,
            max_memory_index: 100000,
            sync_interval_secs: 300, // 5分钟
            enable_cow: true,
        }
    }
}

/// 块引用信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockRef {
    /// 块ID
    pub chunk_id: String,
    /// 引用的文件ID
    pub file_ids: HashSet<String>,
    /// 引用计数
    pub ref_count: u32,
    /// 块大小
    pub size: u64,
    /// 存储路径
    pub storage_path: PathBuf,
    /// 最后访问时间
    pub last_accessed: chrono::NaiveDateTime,
    /// 创建时间
    pub created_at: chrono::NaiveDateTime,
}

/// 去重统计信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DedupStats {
    /// 总文件数
    pub total_files: u64,
    /// 总块数
    pub total_chunks: u64,
    /// 唯一块数
    pub unique_chunks: u64,
    /// 重复块数
    pub duplicate_chunks: u64,
    /// 去重节省的空间（字节）
    pub space_saved: u64,
    /// 去重率
    pub dedup_ratio: f32,
    /// 平均块大小
    pub avg_chunk_size: f64,
}

/// 去重结果
#[derive(Debug, Clone)]
pub struct DedupResult {
    /// 去重的文件数
    pub deduped_files: u32,
    /// 找到的重复块数
    pub duplicate_blocks: u32,
    /// 节省的空间
    pub space_saved: u64,
    /// 处理时间（毫秒）
    pub duration_ms: u64,
}

/// 块去重管理器
pub struct DedupManager {
    config: DedupConfig,
    /// 块索引
    block_index: Arc<AsyncRwLock<BlockIndex>>,
    /// 内存中的块引用映射
    block_refs: Arc<RwLock<HashMap<String, BlockRef>>>,
    /// 文件到块的映射
    file_to_chunks: Arc<RwLock<HashMap<String, HashSet<String>>>>,
    /// 统计信息
    stats: Arc<RwLock<DedupStats>>,
}

impl DedupManager {
    pub fn new(config: DedupConfig, index_config: BlockIndexConfig, root_path: &str) -> Self {
        let block_index = Arc::new(AsyncRwLock::new(BlockIndex::new(
            index_config,
            &format!("{}/dedup", root_path),
        )));

        Self {
            config,
            block_index,
            block_refs: Arc::new(RwLock::new(HashMap::new())),
            file_to_chunks: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(DedupStats::default())),
        }
    }

    /// 初始化去重管理器
    pub async fn init(&self) -> Result<()> {
        let block_index = self.block_index.read().await;
        block_index.init().await?;

        info!("去重管理器初始化完成");
        Ok(())
    }

    /// 处理文件的分块和去重
    pub async fn process_file(
        &self,
        file_id: &str,
        chunks: &[crate::storage_v2::ChunkInfo],
    ) -> Result<DedupResult> {
        let start = std::time::Instant::now();
        let mut deduped_blocks = 0;
        let mut space_saved = 0u64;

        if !self.config.enable_dedup {
            return Ok(DedupResult {
                deduped_files: 0,
                duplicate_blocks: 0,
                space_saved: 0,
                duration_ms: 0,
            });
        }

        let mut file_chunks = HashSet::new();
        #[allow(unused_mut)]
        let mut block_index = self.block_index.write().await;

        for chunk in chunks {
            // 跳过小于阈值的块
            if chunk.size < self.config.min_dedup_size {
                file_chunks.insert(chunk.chunk_id.clone());
                continue;
            }

            // 检查块是否已存在
            if block_index.contains(&chunk.chunk_id).await {
                // 块已存在，增加引用
                let ref_count = block_index.inc_ref(&chunk.chunk_id).await?;
                deduped_blocks += 1;
                space_saved += chunk.size as u64;

                // 更新内存中的引用信息
                let mut block_refs = self.block_refs.write().unwrap();
                if let Some(block_ref) = block_refs.get_mut(&chunk.chunk_id) {
                    block_ref.file_ids.insert(file_id.to_string());
                    block_ref.ref_count = ref_count;
                }

                debug!("块 {} 已存在，增加引用计数到 {}", chunk.chunk_id, ref_count);
            } else {
                // 新块，添加到索引
                let storage_path = block_index.get_block_path(&chunk.chunk_id);
                block_index
                    .add_block(&chunk.chunk_id, chunk.size, storage_path)
                    .await?;

                // 创建新的引用信息
                let mut block_refs = self.block_refs.write().unwrap();
                block_refs.insert(
                    chunk.chunk_id.clone(),
                    BlockRef {
                        chunk_id: chunk.chunk_id.clone(),
                        file_ids: HashSet::from([file_id.to_string()]),
                        ref_count: 1,
                        size: chunk.size as u64,
                        storage_path: block_index.get_block_path(&chunk.chunk_id),
                        last_accessed: chrono::Local::now().naive_local(),
                        created_at: chrono::Local::now().naive_local(),
                    },
                );
            }

            file_chunks.insert(chunk.chunk_id.clone());
        }

        // 更新文件到块的映射
        {
            let mut file_to_chunks = self.file_to_chunks.write().unwrap();
            file_to_chunks.insert(file_id.to_string(), file_chunks);
        }

        // 更新统计信息
        {
            // 先获取unique_chunks的值（避免在持有stats锁时await）
            let unique_chunks = self.block_index.read().await.get_stats().await.total_blocks as u64;

            let mut stats = self.stats.write().unwrap();
            stats.total_files += 1;
            stats.total_chunks += chunks.len() as u64;
            stats.unique_chunks = unique_chunks;
            stats.duplicate_chunks = stats.total_chunks.saturating_sub(stats.unique_chunks);
            stats.space_saved += space_saved;

            if stats.total_chunks > 0 {
                stats.dedup_ratio = stats.duplicate_chunks as f32 / stats.total_chunks as f32;
            }

            if stats.unique_chunks > 0 {
                stats.avg_chunk_size = stats.space_saved as f64 / stats.unique_chunks as f64;
            }
        }

        let duration = start.elapsed();

        info!(
            "文件 {} 去重完成: {} 个重复块, 节省 {} 字节, 耗时 {}ms",
            file_id,
            deduped_blocks,
            space_saved,
            duration.as_millis()
        );

        Ok(DedupResult {
            deduped_files: 1,
            duplicate_blocks: deduped_blocks,
            space_saved,
            duration_ms: duration.as_millis() as u64,
        })
    }

    /// 批量处理文件
    pub async fn batch_process_files(
        &self,
        files: Vec<(&str, Vec<crate::storage_v2::ChunkInfo>)>,
    ) -> Result<DedupResult> {
        let start = std::time::Instant::now();
        let mut total_deduped_files = 0;
        let mut total_duplicate_blocks = 0;
        let mut total_space_saved = 0u64;

        // 分批处理
        for chunk in files.chunks(self.config.batch_size) {
            let mut batch_results = Vec::new();

            // 并发处理每批文件
            for (file_id, chunks) in chunk {
                let result = self.process_file(file_id, chunks).await?;
                batch_results.push(result);
            }

            // 汇总结果
            for result in batch_results {
                total_deduped_files += result.deduped_files;
                total_duplicate_blocks += result.duplicate_blocks;
                total_space_saved += result.space_saved;
            }
        }

        let duration = start.elapsed();

        Ok(DedupResult {
            deduped_files: total_deduped_files,
            duplicate_blocks: total_duplicate_blocks,
            space_saved: total_space_saved,
            duration_ms: duration.as_millis() as u64,
        })
    }

    /// 检查文件是否包含指定块
    pub async fn has_block(&self, file_id: &str, chunk_id: &str) -> bool {
        let file_to_chunks = self.file_to_chunks.read().unwrap();
        if let Some(chunks) = file_to_chunks.get(file_id) {
            chunks.contains(chunk_id)
        } else {
            false
        }
    }

    /// 获取块引用信息
    pub async fn get_block_ref(&self, chunk_id: &str) -> Option<BlockRef> {
        let block_refs = self.block_refs.read().unwrap();
        block_refs.get(chunk_id).cloned()
    }

    /// 获取文件的所有块
    pub async fn get_file_chunks(&self, file_id: &str) -> Option<HashSet<String>> {
        let file_to_chunks = self.file_to_chunks.read().unwrap();
        file_to_chunks.get(file_id).cloned()
    }

    /// 删除文件并更新引用计数
    pub async fn remove_file(&self, file_id: &str) -> Result<u32> {
        let mut removed_blocks = 0;

        // 获取文件的块
        let file_chunks = {
            let mut file_to_chunks = self.file_to_chunks.write().unwrap();
            file_to_chunks.remove(file_id)
        };

        if let Some(chunks) = file_chunks {
            #[allow(unused_mut)]
            let mut block_index = self.block_index.write().await;

            for chunk_id in chunks {
                // 减少引用计数
                if let Ok(ref_count) = block_index.dec_ref(&chunk_id).await
                    && ref_count == 0
                {
                    // 引用计数为0，清理块
                    block_index.remove_block(&chunk_id).await?;

                    // 清理内存中的引用信息
                    let mut block_refs = self.block_refs.write().unwrap();
                    block_refs.remove(&chunk_id);

                    removed_blocks += 1;
                }
            }
        }

        info!("删除文件 {}, 清理了 {} 个未引用块", file_id, removed_blocks);
        Ok(removed_blocks)
    }

    /// 查找重复文件
    pub async fn find_duplicate_files(&self) -> Result<Vec<Vec<String>>> {
        let file_to_chunks = self.file_to_chunks.read().unwrap();
        let mut chunk_to_files: HashMap<String, HashSet<String>> = HashMap::new();

        // 构建块到文件的反向映射
        for (file_id, chunks) in file_to_chunks.iter() {
            for chunk_id in chunks {
                chunk_to_files
                    .entry(chunk_id.clone())
                    .or_default()
                    .insert(file_id.clone());
            }
        }

        // 查找共享相同块集的文件
        let mut duplicate_groups: HashMap<String, HashSet<String>> = HashMap::new();

        for (_chunk_id, file_ids) in chunk_to_files {
            if file_ids.len() > 1 {
                // 对文件ID列表排序，生成组合键
                let mut sorted_files: Vec<String> = file_ids.iter().cloned().collect();
                sorted_files.sort();

                let group_key = sorted_files.join("|");
                duplicate_groups
                    .entry(group_key)
                    .or_default()
                    .extend(sorted_files);
            }
        }

        Ok(duplicate_groups
            .values()
            .filter(|group| group.len() > 1)
            .map(|group| group.iter().cloned().collect())
            .collect())
    }

    /// 获取去重统计信息
    pub async fn get_stats(&self) -> DedupStats {
        let stats = self.stats.read().unwrap();
        stats.clone()
    }

    /// 执行GC - 清理未引用的块
    pub async fn gc(&self) -> Result<u32> {
        let block_index = self.block_index.read().await;
        let unreferenced = block_index.gc_unreferenced().await?;

        if !unreferenced.is_empty() {
            // 清理内存中的引用信息
            let mut block_refs = self.block_refs.write().unwrap();
            for chunk_id in &unreferenced {
                block_refs.remove(chunk_id);
            }

            info!("GC 清理了 {} 个未引用块", unreferenced.len());
        }

        Ok(unreferenced.len() as u32)
    }

    /// 同步索引到磁盘
    pub async fn sync(&self) -> Result<()> {
        let _block_index = self.block_index.read().await;
        // BlockIndex 内部会自动处理持久化
        debug!("去重索引已同步");
        Ok(())
    }

    /// 获取存储效率分析
    pub async fn get_efficiency_analysis(&self) -> Result<EfficiencyAnalysis> {
        let stats = self.get_stats().await;

        let total_size = stats.total_chunks * stats.avg_chunk_size as u64;
        let deduplicated_size =
            stats.total_chunks * stats.avg_chunk_size as u64 - stats.space_saved;

        let efficiency = if total_size > 0 {
            stats.space_saved as f32 / total_size as f32
        } else {
            0.0
        };

        Ok(EfficiencyAnalysis {
            total_size,
            deduplicated_size,
            space_saved: stats.space_saved,
            dedup_ratio: stats.dedup_ratio,
            efficiency_percentage: efficiency * 100.0,
            unique_chunks: stats.unique_chunks,
            duplicate_chunks: stats.duplicate_chunks,
        })
    }
}

/// 存储效率分析
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EfficiencyAnalysis {
    /// 原始总大小
    pub total_size: u64,
    /// 去重后大小
    pub deduplicated_size: u64,
    /// 节省的空间
    pub space_saved: u64,
    /// 去重比率
    pub dedup_ratio: f32,
    /// 效率百分比
    pub efficiency_percentage: f32,
    /// 唯一块数
    pub unique_chunks: u64,
    /// 重复块数
    pub duplicate_chunks: u64,
}

impl EfficiencyAnalysis {
    /// 格式化大小
    pub fn format_size(&self, bytes: u64) -> String {
        const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
        let mut size = bytes as f64;
        let mut unit_index = 0;

        while size >= 1024.0 && unit_index < UNITS.len() - 1 {
            size /= 1024.0;
            unit_index += 1;
        }

        format!("{:.2} {}", size, UNITS[unit_index])
    }

    /// 获取压缩比
    pub fn get_compression_ratio(&self) -> f32 {
        if self.deduplicated_size > 0 {
            self.total_size as f32 / self.deduplicated_size as f32
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_dedup_manager_new() {
        let temp_dir = TempDir::new().unwrap();
        let config = DedupConfig::default();
        let index_config = BlockIndexConfig::default();
        let manager = DedupManager::new(config, index_config, temp_dir.path().to_str().unwrap());

        manager.init().await.unwrap();

        let stats = manager.get_stats().await;
        assert_eq!(stats.total_files, 0);
        assert_eq!(stats.total_chunks, 0);
    }

    #[tokio::test]
    async fn test_process_file_new_blocks() {
        let temp_dir = TempDir::new().unwrap();
        let config = DedupConfig::default();
        let index_config = BlockIndexConfig::default();
        let manager = DedupManager::new(config, index_config, temp_dir.path().to_str().unwrap());
        manager.init().await.unwrap();

        let chunks = vec![
            crate::storage_v2::ChunkInfo {
                chunk_id: "chunk1".to_string(),
                offset: 0,
                size: 1024,
                weak_hash: 0,
                strong_hash: "hash1".to_string(),
            },
            crate::storage_v2::ChunkInfo {
                chunk_id: "chunk2".to_string(),
                offset: 1024,
                size: 2048,
                weak_hash: 0,
                strong_hash: "hash2".to_string(),
            },
        ];

        let result = manager.process_file("file1", &chunks).await.unwrap();

        assert_eq!(result.deduped_files, 1);
        assert_eq!(result.duplicate_blocks, 0); // 新块，无去重
        assert_eq!(result.space_saved, 0);
    }

    #[tokio::test]
    async fn test_process_file_duplicate_blocks() {
        let temp_dir = TempDir::new().unwrap();
        let config = DedupConfig::default();
        let index_config = BlockIndexConfig::default();
        let manager = DedupManager::new(config, index_config, temp_dir.path().to_str().unwrap());
        manager.init().await.unwrap();

        let chunks = vec![crate::storage_v2::ChunkInfo {
            chunk_id: "chunk1".to_string(),
            offset: 0,
            size: 1024,
            weak_hash: 0,
            strong_hash: "hash1".to_string(),
        }];

        // 第一次处理
        manager.process_file("file1", &chunks).await.unwrap();

        // 第二次处理相同块
        let result = manager.process_file("file2", &chunks).await.unwrap();

        assert_eq!(result.deduped_files, 1);
        assert_eq!(result.duplicate_blocks, 1); // 找到重复块
        assert_eq!(result.space_saved, 1024); // 节省1024字节
    }

    #[tokio::test]
    async fn test_remove_file() {
        let temp_dir = TempDir::new().unwrap();
        let config = DedupConfig::default();
        let index_config = BlockIndexConfig::default();
        let manager = DedupManager::new(config, index_config, temp_dir.path().to_str().unwrap());
        manager.init().await.unwrap();

        let chunks = vec![crate::storage_v2::ChunkInfo {
            chunk_id: "chunk1".to_string(),
            offset: 0,
            size: 1024,
            weak_hash: 0,
            strong_hash: "hash1".to_string(),
        }];

        manager.process_file("file1", &chunks).await.unwrap();
        let removed = manager.remove_file("file1").await.unwrap();

        assert_eq!(removed, 1); // 清理了1个块
    }

    #[tokio::test]
    async fn test_find_duplicate_files() {
        let temp_dir = TempDir::new().unwrap();
        let config = DedupConfig::default();
        let index_config = BlockIndexConfig::default();
        let manager = DedupManager::new(config, index_config, temp_dir.path().to_str().unwrap());
        manager.init().await.unwrap();

        let chunks = vec![crate::storage_v2::ChunkInfo {
            chunk_id: "chunk1".to_string(),
            offset: 0,
            size: 1024,
            weak_hash: 0,
            strong_hash: "hash1".to_string(),
        }];

        // 创建重复文件
        manager.process_file("file1", &chunks).await.unwrap();
        manager.process_file("file2", &chunks).await.unwrap();

        let duplicates = manager.find_duplicate_files().await.unwrap();
        assert!(!duplicates.is_empty());
    }

    #[tokio::test]
    async fn test_get_efficiency_analysis() {
        let temp_dir = TempDir::new().unwrap();
        let config = DedupConfig::default();
        let index_config = BlockIndexConfig::default();
        let manager = DedupManager::new(config, index_config, temp_dir.path().to_str().unwrap());
        manager.init().await.unwrap();

        let chunks = vec![crate::storage_v2::ChunkInfo {
            chunk_id: "chunk1".to_string(),
            offset: 0,
            size: 1024,
            weak_hash: 0,
            strong_hash: "hash1".to_string(),
        }];

        manager.process_file("file1", &chunks).await.unwrap();
        manager.process_file("file2", &chunks).await.unwrap();

        let analysis = manager.get_efficiency_analysis().await.unwrap();
        assert_eq!(analysis.space_saved, 1024);
        assert!(analysis.efficiency_percentage > 0.0);
    }
}
