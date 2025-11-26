//! Bloom Filter for fast chunk existence checking
//!
//! 用于在文件系统检查之前快速判断块是否可能存在，减少不必要的磁盘 I/O。

use bloomfilter::Bloom;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Bloom Filter 管理器
///
/// 提供线程安全的 Bloom Filter 操作，用于快速判断块是否可能存在。
///
/// # 特性
/// - 假阳性率: ~0.1% (可配置)
/// - 容量: 1000万块（默认）
/// - 内存占用: ~12 MB
pub struct ChunkBloomFilter {
    /// Bloom Filter 实例（线程安全）
    bloom: Arc<RwLock<Bloom<String>>>,
    /// 预期元素数量
    expected_items: usize,
    /// 假阳性率
    false_positive_rate: f64,
}

impl ChunkBloomFilter {
    /// 创建新的 Bloom Filter
    ///
    /// # 参数
    /// - `expected_items`: 预期的块数量（默认 1000万）
    /// - `false_positive_rate`: 假阳性率（默认 0.1%）
    pub fn new(expected_items: usize, false_positive_rate: f64) -> Self {
        let bloom = Bloom::new_for_fp_rate(expected_items, false_positive_rate);

        Self {
            bloom: Arc::new(RwLock::new(bloom)),
            expected_items,
            false_positive_rate,
        }
    }

    /// 使用默认参数创建 Bloom Filter
    ///
    /// - 预期元素: 1000万块
    /// - 假阳性率: 0.1%
    /// - 内存占用: ~12 MB
    pub fn with_defaults() -> Self {
        Self::new(10_000_000, 0.001)
    }

    /// 添加块 ID 到 Bloom Filter
    pub async fn insert(&self, chunk_id: &str) {
        let mut bloom = self.bloom.write().await;
        bloom.set(&chunk_id.to_string());
    }

    /// 检查块 ID 是否可能存在
    ///
    /// # 返回值
    /// - `true`: 块**可能**存在（需要进一步检查文件系统）
    /// - `false`: 块**一定不**存在
    pub async fn contains(&self, chunk_id: &str) -> bool {
        let bloom = self.bloom.read().await;
        bloom.check(&chunk_id.to_string())
    }

    /// 批量添加块 ID
    pub async fn insert_batch(&self, chunk_ids: &[String]) {
        let mut bloom = self.bloom.write().await;
        for chunk_id in chunk_ids {
            bloom.set(chunk_id);
        }
    }

    /// 获取 Bloom Filter 统计信息
    pub async fn get_stats(&self) -> BloomFilterStats {
        let bloom = self.bloom.read().await;
        BloomFilterStats {
            expected_items: self.expected_items,
            false_positive_rate: self.false_positive_rate,
            bit_count: bloom.number_of_bits(),
            hash_count: bloom.number_of_hash_functions(),
            estimated_memory_bytes: bloom.number_of_bits() / 8,
        }
    }

    /// 清空 Bloom Filter
    pub async fn clear(&self) {
        let mut bloom = self.bloom.write().await;
        bloom.clear();
    }

    /// 重建 Bloom Filter（从块列表）
    pub async fn rebuild(&self, chunk_ids: Vec<String>) {
        let mut bloom = self.bloom.write().await;
        bloom.clear();
        for chunk_id in chunk_ids {
            bloom.set(&chunk_id);
        }
    }
}

/// Bloom Filter 统计信息
#[derive(Debug, Clone)]
pub struct BloomFilterStats {
    /// 预期元素数量
    pub expected_items: usize,
    /// 假阳性率
    pub false_positive_rate: f64,
    /// 位数组大小
    pub bit_count: u64,
    /// 哈希函数数量
    pub hash_count: u32,
    /// 估计内存占用（字节）
    pub estimated_memory_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bloom_filter_basic() {
        let bloom = ChunkBloomFilter::with_defaults();

        // 测试不存在的元素
        assert!(!bloom.contains("nonexistent").await);

        // 添加元素
        bloom.insert("chunk_123").await;

        // 测试存在的元素
        assert!(bloom.contains("chunk_123").await);

        // 测试不存在的元素
        assert!(!bloom.contains("chunk_456").await);
    }

    #[tokio::test]
    async fn test_bloom_filter_batch() {
        let bloom = ChunkBloomFilter::with_defaults();

        let chunks = vec![
            "chunk_1".to_string(),
            "chunk_2".to_string(),
            "chunk_3".to_string(),
        ];

        bloom.insert_batch(&chunks).await;

        assert!(bloom.contains("chunk_1").await);
        assert!(bloom.contains("chunk_2").await);
        assert!(bloom.contains("chunk_3").await);
        assert!(!bloom.contains("chunk_4").await);
    }

    #[tokio::test]
    async fn test_bloom_filter_clear() {
        let bloom = ChunkBloomFilter::with_defaults();

        bloom.insert("chunk_123").await;
        assert!(bloom.contains("chunk_123").await);

        bloom.clear().await;
        assert!(!bloom.contains("chunk_123").await);
    }

    #[tokio::test]
    async fn test_bloom_filter_stats() {
        let bloom = ChunkBloomFilter::with_defaults();
        let stats = bloom.get_stats().await;

        assert_eq!(stats.expected_items, 10_000_000);
        assert_eq!(stats.false_positive_rate, 0.001);
        assert!(stats.bit_count > 0);
        assert!(stats.hash_count > 0);
    }

    #[tokio::test]
    async fn test_bloom_filter_rebuild() {
        let bloom = ChunkBloomFilter::with_defaults();

        // 初始添加
        bloom.insert("chunk_1").await;
        bloom.insert("chunk_2").await;

        // 重建
        let new_chunks = vec!["chunk_3".to_string(), "chunk_4".to_string()];
        bloom.rebuild(new_chunks).await;

        // 旧元素不应存在
        assert!(!bloom.contains("chunk_1").await);
        assert!(!bloom.contains("chunk_2").await);

        // 新元素应存在
        assert!(bloom.contains("chunk_3").await);
        assert!(bloom.contains("chunk_4").await);
    }
}
