//! 缓存模块
//!
//! 提供高性能的LRU缓存，用于文件元数据和内容缓存

#![allow(dead_code)] // 这些结构体和方法将在后续集成时使用

use crate::models::FileMetadata;
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;

/// 文件元数据缓存
pub struct MetadataCache {
    cache: Cache<String, FileMetadata>,
}

impl MetadataCache {
    /// 创建元数据缓存
    /// - max_capacity: 最大缓存条目数
    /// - ttl: 缓存过期时间（秒）
    pub fn new(max_capacity: u64, ttl_secs: u64) -> Self {
        let cache = Cache::builder()
            .max_capacity(max_capacity)
            .time_to_live(Duration::from_secs(ttl_secs))
            .build();

        Self { cache }
    }

    /// 获取缓存的元数据
    pub async fn get(&self, file_id: &str) -> Option<FileMetadata> {
        self.cache.get(file_id).await
    }

    /// 缓存元数据
    pub async fn set(&self, file_id: String, metadata: FileMetadata) {
        self.cache.insert(file_id, metadata).await;
    }

    /// 删除缓存
    pub async fn remove(&self, file_id: &str) {
        self.cache.invalidate(file_id).await;
    }

    /// 清空所有缓存
    pub async fn clear(&self) {
        self.cache.invalidate_all();
    }

    /// 获取缓存统计信息
    pub fn stats(&self) -> CacheStats {
        let entry_count = self.cache.entry_count();

        // moka 不直接提供 hit/miss 统计，返回基础信息
        CacheStats {
            entry_count,
            hit_count: 0,
            miss_count: 0,
            hit_rate: 0.0,
        }
    }
}

/// 文件内容缓存
pub struct ContentCache {
    cache: Cache<String, Arc<Vec<u8>>>,
    max_size_bytes: u64,
}

impl ContentCache {
    /// 创建内容缓存
    /// - max_capacity: 最大缓存条目数
    /// - max_size_bytes: 最大缓存大小（字节）
    /// - ttl: 缓存过期时间（秒）
    pub fn new(max_capacity: u64, max_size_bytes: u64, ttl_secs: u64) -> Self {
        let cache = Cache::builder()
            .max_capacity(max_capacity)
            .time_to_live(Duration::from_secs(ttl_secs))
            .weigher(|_key: &String, value: &Arc<Vec<u8>>| -> u32 {
                // 权重基于内容大小
                value.len().min(u32::MAX as usize) as u32
            })
            .build();

        Self {
            cache,
            max_size_bytes,
        }
    }

    /// 获取缓存的文件内容
    pub async fn get(&self, file_id: &str) -> Option<Arc<Vec<u8>>> {
        self.cache.get(file_id).await
    }

    /// 缓存文件内容（如果大小允许）
    pub async fn set(&self, file_id: String, content: Vec<u8>) {
        // 只缓存不超过最大限制的文件
        if content.len() as u64 <= self.max_size_bytes {
            self.cache.insert(file_id, Arc::new(content)).await;
        }
    }

    /// 删除缓存
    pub async fn remove(&self, file_id: &str) {
        self.cache.invalidate(file_id).await;
    }

    /// 清空所有缓存
    pub async fn clear(&self) {
        self.cache.invalidate_all();
    }

    /// 获取缓存统计信息
    pub fn stats(&self) -> CacheStats {
        let entry_count = self.cache.entry_count();

        // moka 不直接提供 hit/miss 统计，返回基础信息
        CacheStats {
            entry_count,
            hit_count: 0,
            miss_count: 0,
            hit_rate: 0.0,
        }
    }
}

/// 搜索结果缓存
pub struct SearchCache {
    cache: Cache<String, Arc<serde_json::Value>>,
}

impl SearchCache {
    /// 创建搜索缓存
    /// - max_capacity: 最大缓存条目数
    /// - ttl: 缓存过期时间（秒）
    pub fn new(max_capacity: u64, ttl_secs: u64) -> Self {
        let cache = Cache::builder()
            .max_capacity(max_capacity)
            .time_to_live(Duration::from_secs(ttl_secs))
            .build();

        Self { cache }
    }

    /// 获取缓存的搜索结果
    pub async fn get(&self, query_key: &str) -> Option<Arc<serde_json::Value>> {
        self.cache.get(query_key).await
    }

    /// 缓存搜索结果
    pub async fn set(&self, query_key: String, results: serde_json::Value) {
        self.cache.insert(query_key, Arc::new(results)).await;
    }

    /// 清空所有缓存
    pub async fn clear(&self) {
        self.cache.invalidate_all();
    }

    /// 获取缓存统计信息
    pub fn stats(&self) -> CacheStats {
        let entry_count = self.cache.entry_count();

        // moka 不直接提供 hit/miss 统计，返回基础信息
        CacheStats {
            entry_count,
            hit_count: 0,
            miss_count: 0,
            hit_rate: 0.0,
        }
    }
}

/// 缓存统计信息
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// 缓存条目数
    pub entry_count: u64,
    /// 命中次数
    pub hit_count: u64,
    /// 未命中次数
    pub miss_count: u64,
    /// 命中率
    pub hit_rate: f64,
}

/// 应用缓存管理器
pub struct CacheManager {
    /// 元数据缓存
    pub metadata: MetadataCache,
    /// 文件内容缓存
    pub content: ContentCache,
    /// 搜索结果缓存
    pub search: SearchCache,
}

impl CacheManager {
    /// 创建缓存管理器
    pub fn new() -> Self {
        Self {
            // 元数据缓存：1000条，TTL 1小时
            metadata: MetadataCache::new(1000, 3600),
            // 内容缓存：100条，最大100MB，TTL 10分钟
            content: ContentCache::new(100, 100 * 1024 * 1024, 600),
            // 搜索缓存：500条，TTL 5分钟
            search: SearchCache::new(500, 300),
        }
    }

    /// 获取所有缓存的统计信息
    pub fn all_stats(&self) -> AllCacheStats {
        AllCacheStats {
            metadata: self.metadata.stats(),
            content: self.content.stats(),
            search: self.search.stats(),
        }
    }

    /// 清空所有缓存
    pub async fn clear_all(&self) {
        self.metadata.clear().await;
        self.content.clear().await;
        self.search.clear().await;
    }
}

impl Default for CacheManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 所有缓存的统计信息
#[derive(Debug, Clone)]
pub struct AllCacheStats {
    pub metadata: CacheStats,
    pub content: CacheStats,
    pub search: CacheStats,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    #[tokio::test]
    async fn test_metadata_cache() {
        let cache = MetadataCache::new(10, 60);

        let metadata = FileMetadata {
            id: "test-id".to_string(),
            name: "test.txt".to_string(),
            path: "/test.txt".to_string(),
            size: 1024,
            hash: "test-hash".to_string(),
            created_at: Local::now().naive_local(),
            modified_at: Local::now().naive_local(),
        };

        // 测试设置和获取
        cache.set("test-id".to_string(), metadata.clone()).await;
        let cached = cache.get("test-id").await;
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().id, "test-id");

        // 测试删除
        cache.remove("test-id").await;
        let cached = cache.get("test-id").await;
        assert!(cached.is_none());
    }

    #[tokio::test]
    async fn test_content_cache() {
        let cache = ContentCache::new(10, 1024 * 1024, 60);

        let content = b"Hello, World!".to_vec();

        // 测试设置和获取
        cache.set("test-file".to_string(), content.clone()).await;
        let cached = cache.get("test-file").await;
        assert!(cached.is_some());
        assert_eq!(*cached.unwrap(), content);

        // 测试删除
        cache.remove("test-file").await;
        let cached = cache.get("test-file").await;
        assert!(cached.is_none());
    }

    #[tokio::test]
    async fn test_search_cache() {
        let cache = SearchCache::new(10, 60);

        let results = serde_json::json!({
            "results": ["file1", "file2"],
            "total": 2
        });

        // 测试设置和获取
        cache.set("query-key".to_string(), results.clone()).await;
        let cached = cache.get("query-key").await;
        assert!(cached.is_some());
        assert_eq!(*cached.unwrap(), results);
    }

    #[tokio::test]
    async fn test_cache_manager() {
        let manager = CacheManager::new();

        // 测试元数据缓存
        let metadata = FileMetadata {
            id: "test-id".to_string(),
            name: "test.txt".to_string(),
            path: "/test.txt".to_string(),
            size: 1024,
            hash: "test-hash".to_string(),
            created_at: Local::now().naive_local(),
            modified_at: Local::now().naive_local(),
        };
        manager
            .metadata
            .set("test-id".to_string(), metadata.clone())
            .await;

        // 验证可以读取
        let cached = manager.metadata.get("test-id").await;
        assert!(cached.is_some());

        // 测试内容缓存
        let content = b"test content".to_vec();
        manager
            .content
            .set("test-file".to_string(), content.clone())
            .await;

        // 验证可以读取
        let cached_content = manager.content.get("test-file").await;
        assert!(cached_content.is_some());

        // 获取统计信息（不依赖具体数值）
        let _stats = manager.all_stats();

        // 清空所有缓存
        manager.clear_all().await;
    }

    #[tokio::test]
    async fn test_cache_stats() {
        let cache = MetadataCache::new(10, 60);

        let metadata = FileMetadata {
            id: "test-id".to_string(),
            name: "test.txt".to_string(),
            path: "/test.txt".to_string(),
            size: 1024,
            hash: "test-hash".to_string(),
            created_at: Local::now().naive_local(),
            modified_at: Local::now().naive_local(),
        };

        // 设置缓存
        cache.set("test-id".to_string(), metadata).await;

        // 触发命中
        let cached = cache.get("test-id").await;
        assert!(cached.is_some());

        // 验证stats方法可以调用
        let _stats = cache.stats();
        // moka的entry_count可能异步更新，不强依赖具体数值
    }
}
