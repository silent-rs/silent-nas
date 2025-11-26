//! 缓存管理模块
//!
//! 使用 moka 库实现高性能的 LRU 缓存，提升热数据访问性能

use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;

/// 缓存配置
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// 文件元信息缓存容量（条目数）
    pub file_metadata_capacity: u64,
    /// Chunk 索引缓存容量（条目数）
    pub chunk_index_capacity: u64,
    /// 热数据缓存容量（字节）
    pub hot_data_capacity: u64,
    /// 缓存过期时间（秒）
    pub ttl_seconds: u64,
    /// 空闲淘汰时间（秒）
    pub idle_seconds: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            file_metadata_capacity: 10_000,       // 10000 个文件
            chunk_index_capacity: 100_000,        // 100000 个 chunks
            hot_data_capacity: 100 * 1024 * 1024, // 100 MB
            ttl_seconds: 3600,                    // 1 小时
            idle_seconds: 300,                    // 5 分钟
        }
    }
}

/// 文件元信息缓存条目
#[derive(Debug, Clone)]
pub struct FileMetadataEntry {
    /// 文件 ID
    pub file_id: String,
    /// 文件名
    pub name: String,
    /// 文件大小
    pub size: u64,
    /// 最新版本 ID
    pub latest_version: String,
    /// 版本计数
    pub version_count: usize,
}

/// Chunk 索引缓存条目
#[derive(Debug, Clone)]
pub struct ChunkIndexEntry {
    /// Chunk 哈希
    pub hash: String,
    /// Chunk 大小
    pub size: usize,
    /// 引用计数
    pub ref_count: usize,
    /// 是否压缩
    pub compressed: bool,
}

/// 热数据缓存条目
#[derive(Debug, Clone)]
pub struct HotDataEntry {
    /// 数据内容
    pub data: Arc<Vec<u8>>,
    /// 数据大小（用于权重计算）
    pub size: u64,
}

/// 缓存管理器
pub struct CacheManager {
    /// 配置
    config: CacheConfig,
    /// 文件元信息缓存
    file_metadata_cache: Cache<String, FileMetadataEntry>,
    /// Chunk 索引缓存
    chunk_index_cache: Cache<String, ChunkIndexEntry>,
    /// 热数据缓存（使用权重限制总大小）
    hot_data_cache: Cache<String, HotDataEntry>,
}

impl CacheManager {
    /// 创建新的缓存管理器
    pub fn new(config: CacheConfig) -> Self {
        // 文件元信息缓存（按条目数限制）
        let file_metadata_cache = Cache::builder()
            .max_capacity(config.file_metadata_capacity)
            .time_to_live(Duration::from_secs(config.ttl_seconds))
            .time_to_idle(Duration::from_secs(config.idle_seconds))
            .build();

        // Chunk 索引缓存（按条目数限制）
        let chunk_index_cache = Cache::builder()
            .max_capacity(config.chunk_index_capacity)
            .time_to_live(Duration::from_secs(config.ttl_seconds))
            .time_to_idle(Duration::from_secs(config.idle_seconds))
            .build();

        // 热数据缓存（按总字节数限制）
        let hot_data_cache = Cache::builder()
            .max_capacity(config.hot_data_capacity)
            .weigher(|_key: &String, value: &HotDataEntry| value.size as u32)
            .time_to_live(Duration::from_secs(config.ttl_seconds))
            .time_to_idle(Duration::from_secs(config.idle_seconds))
            .build();

        Self {
            config,
            file_metadata_cache,
            chunk_index_cache,
            hot_data_cache,
        }
    }

    /// 使用默认配置创建
    pub fn with_default() -> Self {
        Self::new(CacheConfig::default())
    }

    // ==================== 文件元信息缓存 ====================

    /// 获取文件元信息
    pub async fn get_file_metadata(&self, file_id: &str) -> Option<FileMetadataEntry> {
        self.file_metadata_cache.get(file_id).await
    }

    /// 设置文件元信息
    pub async fn set_file_metadata(&self, file_id: String, entry: FileMetadataEntry) {
        self.file_metadata_cache.insert(file_id, entry).await;
    }

    /// 移除文件元信息
    pub async fn remove_file_metadata(&self, file_id: &str) {
        self.file_metadata_cache.invalidate(file_id).await;
    }

    /// 批量设置文件元信息
    pub async fn set_file_metadata_batch(&self, entries: Vec<(String, FileMetadataEntry)>) {
        for (file_id, entry) in entries {
            self.file_metadata_cache.insert(file_id, entry).await;
        }
    }

    // ==================== Chunk 索引缓存 ====================

    /// 获取 Chunk 索引
    pub async fn get_chunk_index(&self, chunk_hash: &str) -> Option<ChunkIndexEntry> {
        self.chunk_index_cache.get(chunk_hash).await
    }

    /// 设置 Chunk 索引
    pub async fn set_chunk_index(&self, chunk_hash: String, entry: ChunkIndexEntry) {
        self.chunk_index_cache.insert(chunk_hash, entry).await;
    }

    /// 移除 Chunk 索引
    pub async fn remove_chunk_index(&self, chunk_hash: &str) {
        self.chunk_index_cache.invalidate(chunk_hash).await;
    }

    /// 批量设置 Chunk 索引
    pub async fn set_chunk_index_batch(&self, entries: Vec<(String, ChunkIndexEntry)>) {
        for (chunk_hash, entry) in entries {
            self.chunk_index_cache.insert(chunk_hash, entry).await;
        }
    }

    // ==================== 热数据缓存 ====================

    /// 获取热数据
    pub async fn get_hot_data(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        self.hot_data_cache.get(key).await.map(|entry| entry.data)
    }

    /// 设置热数据
    pub async fn set_hot_data(&self, key: String, data: Vec<u8>) {
        let size = data.len() as u64;
        let entry = HotDataEntry {
            data: Arc::new(data),
            size,
        };
        self.hot_data_cache.insert(key, entry).await;
    }

    /// 移除热数据
    pub async fn remove_hot_data(&self, key: &str) {
        self.hot_data_cache.invalidate(key).await;
    }

    // ==================== 缓存统计 ====================

    /// 获取缓存统计信息
    pub async fn get_stats(&self) -> CacheStats {
        // 同步缓存（运行后台任务清理过期条目）
        self.file_metadata_cache.run_pending_tasks().await;
        self.chunk_index_cache.run_pending_tasks().await;
        self.hot_data_cache.run_pending_tasks().await;

        CacheStats {
            file_metadata_count: self.file_metadata_cache.entry_count(),
            chunk_index_count: self.chunk_index_cache.entry_count(),
            hot_data_count: self.hot_data_cache.entry_count(),
            hot_data_size: self.hot_data_cache.weighted_size(),
            config: self.config.clone(),
        }
    }

    /// 清空所有缓存
    pub async fn clear_all(&self) {
        self.file_metadata_cache.invalidate_all();
        self.chunk_index_cache.invalidate_all();
        self.hot_data_cache.invalidate_all();

        // 等待后台清理完成
        self.file_metadata_cache.run_pending_tasks().await;
        self.chunk_index_cache.run_pending_tasks().await;
        self.hot_data_cache.run_pending_tasks().await;
    }
}

/// 缓存统计信息
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// 文件元信息缓存条目数
    pub file_metadata_count: u64,
    /// Chunk 索引缓存条目数
    pub chunk_index_count: u64,
    /// 热数据缓存条目数
    pub hot_data_count: u64,
    /// 热数据缓存总大小（字节）
    pub hot_data_size: u64,
    /// 缓存配置
    pub config: CacheConfig,
}

impl CacheStats {
    /// 计算文件元信息缓存使用率
    pub fn file_metadata_usage_ratio(&self) -> f64 {
        if self.config.file_metadata_capacity == 0 {
            0.0
        } else {
            self.file_metadata_count as f64 / self.config.file_metadata_capacity as f64
        }
    }

    /// 计算 Chunk 索引缓存使用率
    pub fn chunk_index_usage_ratio(&self) -> f64 {
        if self.config.chunk_index_capacity == 0 {
            0.0
        } else {
            self.chunk_index_count as f64 / self.config.chunk_index_capacity as f64
        }
    }

    /// 计算热数据缓存使用率
    pub fn hot_data_usage_ratio(&self) -> f64 {
        if self.config.hot_data_capacity == 0 {
            0.0
        } else {
            self.hot_data_size as f64 / self.config.hot_data_capacity as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_manager_creation() {
        let manager = CacheManager::with_default();
        let stats = manager.get_stats().await;

        assert_eq!(stats.file_metadata_count, 0);
        assert_eq!(stats.chunk_index_count, 0);
        assert_eq!(stats.hot_data_count, 0);
    }

    #[tokio::test]
    async fn test_file_metadata_cache() {
        let manager = CacheManager::with_default();

        let entry = FileMetadataEntry {
            file_id: "file1".to_string(),
            name: "test.txt".to_string(),
            size: 1024,
            latest_version: "v1".to_string(),
            version_count: 1,
        };

        // 设置
        manager
            .set_file_metadata("file1".to_string(), entry.clone())
            .await;

        // 获取
        let cached = manager.get_file_metadata("file1").await;
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().name, "test.txt");

        // 移除
        manager.remove_file_metadata("file1").await;
        assert!(manager.get_file_metadata("file1").await.is_none());
    }

    #[tokio::test]
    async fn test_chunk_index_cache() {
        let manager = CacheManager::with_default();

        let entry = ChunkIndexEntry {
            hash: "abc123".to_string(),
            size: 4096,
            ref_count: 2,
            compressed: true,
        };

        // 设置
        manager
            .set_chunk_index("abc123".to_string(), entry.clone())
            .await;

        // 获取
        let cached = manager.get_chunk_index("abc123").await;
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().ref_count, 2);

        // 移除
        manager.remove_chunk_index("abc123").await;
        assert!(manager.get_chunk_index("abc123").await.is_none());
    }

    #[tokio::test]
    async fn test_hot_data_cache() {
        let manager = CacheManager::with_default();

        let data = vec![1, 2, 3, 4, 5];

        // 设置
        manager
            .set_hot_data("data1".to_string(), data.clone())
            .await;

        // 获取
        let cached = manager.get_hot_data("data1").await;
        assert!(cached.is_some());
        assert_eq!(*cached.unwrap(), data);

        // 移除
        manager.remove_hot_data("data1").await;
        assert!(manager.get_hot_data("data1").await.is_none());
    }

    #[tokio::test]
    async fn test_batch_operations() {
        let manager = CacheManager::with_default();

        // 批量设置文件元信息
        let entries = vec![
            (
                "file1".to_string(),
                FileMetadataEntry {
                    file_id: "file1".to_string(),
                    name: "test1.txt".to_string(),
                    size: 1024,
                    latest_version: "v1".to_string(),
                    version_count: 1,
                },
            ),
            (
                "file2".to_string(),
                FileMetadataEntry {
                    file_id: "file2".to_string(),
                    name: "test2.txt".to_string(),
                    size: 2048,
                    latest_version: "v1".to_string(),
                    version_count: 1,
                },
            ),
        ];

        manager.set_file_metadata_batch(entries).await;

        // 验证
        assert!(manager.get_file_metadata("file1").await.is_some());
        assert!(manager.get_file_metadata("file2").await.is_some());
    }

    #[tokio::test]
    async fn test_cache_stats() {
        let manager = CacheManager::with_default();

        // 添加一些数据
        manager
            .set_file_metadata(
                "file1".to_string(),
                FileMetadataEntry {
                    file_id: "file1".to_string(),
                    name: "test.txt".to_string(),
                    size: 1024,
                    latest_version: "v1".to_string(),
                    version_count: 1,
                },
            )
            .await;

        manager
            .set_hot_data("data1".to_string(), vec![1, 2, 3, 4, 5])
            .await;

        // 获取统计
        let stats = manager.get_stats().await;
        assert_eq!(stats.file_metadata_count, 1);
        assert_eq!(stats.hot_data_count, 1);
        assert!(stats.file_metadata_usage_ratio() > 0.0);
    }

    #[tokio::test]
    async fn test_clear_all() {
        let manager = CacheManager::with_default();

        // 添加数据
        manager
            .set_file_metadata(
                "file1".to_string(),
                FileMetadataEntry {
                    file_id: "file1".to_string(),
                    name: "test.txt".to_string(),
                    size: 1024,
                    latest_version: "v1".to_string(),
                    version_count: 1,
                },
            )
            .await;

        // 清空
        manager.clear_all().await;

        // 验证
        let stats = manager.get_stats().await;
        assert_eq!(stats.file_metadata_count, 0);
        assert_eq!(stats.chunk_index_count, 0);
        assert_eq!(stats.hot_data_count, 0);
    }
}
