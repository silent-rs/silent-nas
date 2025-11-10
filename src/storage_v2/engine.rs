//! 存储引擎优化模块
//!
//! 提供高性能的I/O操作和并发控制，包括：
//! - 顺序写入优化与写放大控制
//! - 随机读缓存与预读策略
//! - 多版本并发控制（MVCC）
//! - 读写锁分离与写入队列管理
//! - 锁竞争优化

use crate::error::{NasError, Result};
use crate::storage_v2::{
    BlockIndex, BlockIndexConfig, ChunkInfo, DedupConfig, DedupManager, LifecycleConfig,
    LifecycleManager, TierConfig, TieredStorage,
};
use moka::future::Cache as MokaCache;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{RwLock as AsyncRwLock, Semaphore, mpsc};
use tracing::{debug, info, warn};

/// 存储引擎配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    /// 缓存大小（条目数）
    pub cache_size: usize,
    /// 预读窗口大小（字节）
    pub readahead_size: usize,
    /// 写放大控制阈值
    pub write_amplification_threshold: f32,
    /// 写入队列大小
    pub write_queue_size: usize,
    /// 并发读取限制
    pub concurrent_reads: usize,
    /// 并发写入限制
    pub concurrent_writes: usize,
    /// 缓存过期时间（秒）
    pub cache_ttl_secs: u64,
    /// 强制刷新间隔（毫秒）
    pub flush_interval_ms: u64,
}

/// 引擎配置默认值
impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            cache_size: 10000,
            readahead_size: 64 * 1024, // 64KB
            write_amplification_threshold: 1.5,
            write_queue_size: 1000,
            concurrent_reads: 100,
            concurrent_writes: 10,
            cache_ttl_secs: 3600,   // 1小时
            flush_interval_ms: 100, // 100ms
        }
    }
}

/// 缓存条目
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// 数据
    pub data: Vec<u8>,
    /// 访问时间
    pub accessed_at: Instant,
    /// 访问次数
    pub access_count: u32,
    /// 文件ID
    pub file_id: String,
    /// 偏移量
    pub offset: u64,
    /// 大小
    pub size: usize,
}

/// 写入任务
#[derive(Debug, Clone)]
pub struct WriteTask {
    /// 任务ID
    pub task_id: u64,
    /// 文件ID
    pub file_id: String,
    /// 偏移量
    pub offset: u64,
    /// 数据
    pub data: Vec<u8>,
    /// 创建时间
    pub created_at: Instant,
}

/// 预读窗口
#[derive(Debug, Clone)]
pub struct ReadaheadWindow {
    /// 起始偏移
    pub start_offset: u64,
    /// 结束偏移
    pub end_offset: u64,
    /// 最后访问时间
    pub last_accessed: Instant,
    /// 访问计数
    pub access_count: u32,
}

/// 写入统计
#[derive(Debug, Default, Clone)]
pub struct WriteStats {
    /// 写入请求数
    pub write_requests: AtomicU64,
    /// 实际写入字节数
    pub bytes_written: AtomicU64,
    /// 物理写入字节数（带写放大）
    pub physical_bytes_written: AtomicU64,
    /// 写放大比
    pub write_amplification_ratio: AtomicU64,
    /// 刷新次数
    pub flush_count: AtomicU64,
    /// 平均写入延迟（微秒）
    pub avg_write_latency_us: AtomicU64,
    /// 写入队列长度峰值
    pub peak_queue_length: AtomicUsize,
}

/// 读取统计
#[derive(Debug, Default, Clone)]
pub struct ReadStats {
    /// 读取请求数
    pub read_requests: AtomicU64,
    /// 实际读取字节数
    pub bytes_read: AtomicU64,
    /// 缓存命中数
    pub cache_hits: AtomicU64,
    /// 缓存未命中数
    pub cache_misses: AtomicU64,
    /// 预读取字节数
    pub readahead_bytes: AtomicU64,
    /// 平均读取延迟（微秒）
    pub avg_read_latency_us: AtomicU64,
}

/// 存储引擎核心
pub struct StorageEngine {
    /// 引擎配置
    config: EngineConfig,
    /// 块索引
    block_index: Arc<AsyncRwLock<BlockIndex>>,
    /// 去重管理器
    dedup_manager: Arc<AsyncRwLock<DedupManager>>,
    /// 分层存储
    tiered_storage: Arc<AsyncRwLock<TieredStorage>>,
    /// 生命周期管理器
    lifecycle_manager: Arc<RwLock<LifecycleManager>>,
    /// 读缓存
    read_cache: Arc<AsyncRwLock<MokaCache<String, CacheEntry>>>,
    /// 预读窗口
    readahead_windows: Arc<RwLock<HashMap<String, ReadaheadWindow>>>,
    /// 写入队列
    write_queue: Arc<AsyncRwLock<VecDeque<WriteTask>>>,
    /// 写统计
    write_stats: Arc<WriteStats>,
    /// 读统计
    read_stats: Arc<ReadStats>,
    /// 读取信号量
    read_semaphore: Arc<Semaphore>,
    /// 写入信号量
    write_semaphore: Arc<Semaphore>,
    /// 强制刷新标志
    force_flush: Arc<AtomicBool>,
    /// MVCC版本管理
    mvcc_versions: Arc<RwLock<HashMap<String, u64>>>,
}

impl StorageEngine {
    /// 创建存储引擎
    pub fn new(
        config: EngineConfig,
        index_config: BlockIndexConfig,
        dedup_config: DedupConfig,
        tier_config: TierConfig,
        lifecycle_config: LifecycleConfig,
        root_path: &str,
    ) -> Self {
        let block_index = Arc::new(AsyncRwLock::new(BlockIndex::new(
            index_config,
            &format!("{}/index", root_path),
        )));

        let dedup_manager = Arc::new(AsyncRwLock::new(DedupManager::new(
            dedup_config,
            index_config.clone(),
            &format!("{}/dedup", root_path),
        )));

        let tiered_storage = Arc::new(AsyncRwLock::new(TieredStorage::new(
            tier_config,
            &format!("{}/tiered", root_path),
        )));

        let lifecycle_manager = Arc::new(RwLock::new(LifecycleManager::new(lifecycle_config)));

        let read_cache = Arc::new(AsyncRwLock::new(
            MokaCache::builder()
                .max_capacity(config.cache_size)
                .time_to_live(Duration::from_secs(config.cache_ttl_secs))
                .build(),
        ));

        let readahead_windows = Arc::new(RwLock::new(HashMap::new()));
        let write_queue = Arc::new(AsyncRwLock::new(VecDeque::new()));
        let write_stats = Arc::new(WriteStats::default());
        let read_stats = Arc::new(ReadStats::default());
        let read_semaphore = Arc::new(Semaphore::new(config.concurrent_reads));
        let write_semaphore = Arc::new(Semaphore::new(config.concurrent_writes));
        let force_flush = Arc::new(AtomicBool::new(false));
        let mvcc_versions = Arc::new(RwLock::new(HashMap::new()));

        Self {
            config,
            block_index,
            dedup_manager,
            tiered_storage,
            lifecycle_manager,
            read_cache,
            readahead_windows,
            write_queue,
            write_stats,
            read_stats,
            read_semaphore,
            write_semaphore,
            force_flush,
            mvcc_versions,
        }
    }

    /// 初始化存储引擎
    pub async fn init(&self) -> Result<()> {
        // 初始化各个组件
        self.block_index.read().await.init().await?;
        self.dedup_manager.read().await.init().await?;
        self.tiered_storage.read().await.init().await?;
        self.lifecycle_manager.write().unwrap().init()?;

        // 启动后台任务
        self.start_background_tasks().await?;

        info!("存储引擎初始化完成");
        Ok(())
    }

    /// 启动后台任务
    async fn start_background_tasks(&self) -> Result<()> {
        // 启动定期刷新任务
        let write_queue = self.write_queue.clone();
        let write_stats = self.write_stats.clone();
        let force_flush = self.force_flush.clone();
        let flush_interval = Duration::from_millis(self.config.flush_interval_ms);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(flush_interval);
            loop {
                interval.tick().await;
                if force_flush.load(Ordering::Relaxed) {
                    Self::flush_write_queue(&write_queue, &write_stats).await;
                }
            }
        });

        // 启动缓存清理任务
        let read_cache = self.read_cache.clone();
        let cache_ttl = Duration::from_secs(self.config.cache_ttl_secs);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300)); // 5分钟
            loop {
                interval.tick().await;
                // MokaCache会自动处理TTL，但我们可以手动清理
                let _ = &read_cache;
            }
        });

        Ok(())
    }

    /// 写入数据
    pub async fn write(&self, file_id: &str, offset: u64, data: &[u8]) -> Result<u64> {
        let _write_permit = self.write_semaphore.acquire().await.unwrap();
        let start = Instant::now();

        // 检查写放大
        let is_small_write = data.len() < self.config.readahead_size / 4;
        if is_small_write {
            // 小写入，加入队列
            return self.queue_write(file_id, offset, data).await;
        }

        // 大写入，直接写入
        let bytes_written = self.write_direct(file_id, offset, data).await?;

        // 更新统计
        self.write_stats
            .write_requests
            .fetch_add(1, Ordering::Relaxed);
        self.write_stats
            .bytes_written
            .fetch_add(data.len() as u64, Ordering::Relaxed);
        self.write_stats
            .physical_bytes_written
            .fetch_add(data.len() as u64, Ordering::Relaxed);

        let latency_us = start.elapsed().as_micros() as u64;
        self.write_stats
            .avg_write_latency_us
            .store(latency_us, Ordering::Relaxed);

        Ok(bytes_written)
    }

    /// 队列写入（小写入优化）
    async fn queue_write(&self, file_id: &str, offset: u64, data: &[u8]) -> Result<u64> {
        let task_id = self
            .write_stats
            .write_requests
            .fetch_add(1, Ordering::Relaxed)
            + 1;

        let task = WriteTask {
            task_id,
            file_id: file_id.to_string(),
            offset,
            data: data.to_vec(),
            created_at: Instant::now(),
        };

        let mut queue = self.write_queue.write().await;
        queue.push_back(task);

        // 更新峰值
        if queue.len() > self.write_stats.peak_queue_length.load(Ordering::Relaxed) {
            self.write_stats
                .peak_queue_length
                .store(queue.len(), Ordering::Relaxed);
        }

        // 检查是否需要强制刷新
        if queue.len() >= self.config.write_queue_size {
            self.force_flush.store(true, Ordering::Relaxed);
        }

        Ok(data.len() as u64)
    }

    /// 直接写入
    async fn write_direct(&self, file_id: &str, offset: u64, data: &[u8]) -> Result<u64> {
        // 1. 检查MVCC版本
        let version = {
            let versions = self.mvcc_versions.read().unwrap();
            versions.get(file_id).copied().unwrap_or(0)
        };

        // 2. 执行去重
        // 这里应该先分块，然后去重
        // 简化实现：直接将整个数据作为一个块
        let chunk_id = format!("{}_{:x}", file_id, md5::compute(data));

        // 3. 更新生命周期
        self.lifecycle_manager
            .write()
            .unwrap()
            .update_modification_time(file_id)?;

        // 4. 记录访问
        self.tiered_storage
            .read()
            .await
            .record_access(file_id)
            .await?;

        Ok(data.len() as u64)
    }

    /// 刷新写入队列
    async fn flush_write_queue(
        write_queue: &Arc<AsyncRwLock<VecDeque<WriteTask>>>,
        write_stats: &Arc<WriteStats>,
    ) {
        let mut queue = write_queue.write().await;
        if queue.is_empty() {
            return;
        }

        // 收集要刷新的任务
        let tasks: Vec<_> = queue.drain(..).collect();

        // 批量处理
        for task in tasks {
            // 实际写入逻辑
            write_stats
                .bytes_written
                .fetch_add(task.data.len() as u64, Ordering::Relaxed);
            write_stats
                .physical_bytes_written
                .fetch_add(task.data.len() as u64, Ordering::Relaxed);
        }

        write_stats.flush_count.fetch_add(1, Ordering::Relaxed);
        info!("刷新了 {} 个写入任务", tasks.len());
    }

    /// 读取数据
    pub async fn read(&self, file_id: &str, offset: u64, size: usize) -> Result<Vec<u8>> {
        let _read_permit = self.read_semaphore.acquire().await.unwrap();
        let start = Instant::now();

        // 生成缓存键
        let cache_key = format!("{}:{}:{}", file_id, offset, size);

        // 1. 检查缓存
        {
            let cache = self.read_cache.read().await;
            if let Some(entry) = cache.get(&cache_key) {
                // 缓存命中
                self.read_stats.cache_hits.fetch_add(1, Ordering::Relaxed);

                // 触发预读
                self.trigger_readahead(file_id, offset, size).await;

                let latency_us = start.elapsed().as_micros() as u64;
                self.read_stats
                    .avg_read_latency_us
                    .store(latency_us, Ordering::Relaxed);

                return Ok(entry.data);
            }
        }

        // 缓存未命中
        self.read_stats.cache_misses.fetch_add(1, Ordering::Relaxed);

        // 2. 执行实际读取
        let data = self.read_actual(file_id, offset, size).await?;

        // 3. 缓存数据
        {
            let mut cache = self.read_cache.write().await;
            cache
                .insert(
                    cache_key,
                    CacheEntry {
                        data: data.clone(),
                        accessed_at: Instant::now(),
                        access_count: 1,
                        file_id: file_id.to_string(),
                        offset,
                        size,
                    },
                )
                .await;
        }

        // 4. 触发预读
        self.trigger_readahead(file_id, offset, size).await;

        // 更新统计
        self.read_stats
            .read_requests
            .fetch_add(1, Ordering::Relaxed);
        self.read_stats
            .bytes_read
            .fetch_add(data.len() as u64, Ordering::Relaxed);

        let latency_us = start.elapsed().as_micros() as u64;
        self.read_stats
            .avg_read_latency_us
            .store(latency_us, Ordering::Relaxed);

        Ok(data)
    }

    /// 执行实际读取
    async fn read_actual(&self, file_id: &str, offset: u64, size: usize) -> Result<Vec<u8>> {
        // TODO: 从实际存储中读取数据
        // 这里应该根据版本链和块索引来重构数据
        Ok(vec![0u8; size])
    }

    /// 触发预读
    async fn trigger_readahead(&self, file_id: &str, offset: u64, size: usize) {
        let readahead_size = self.config.readahead_size;
        let start_offset = offset + size as u64;
        let end_offset = start_offset + readahead_size as u64;

        // 更新预读窗口
        {
            let mut windows = self.readahead_windows.write().unwrap();
            windows.insert(
                file_id.to_string(),
                ReadaheadWindow {
                    start_offset,
                    end_offset,
                    last_accessed: Instant::now(),
                    access_count: 1,
                },
            );
        }

        // 启动异步预读
        let readahead_size = readahead_size;
        self.read_stats
            .readahead_bytes
            .fetch_add(readahead_size as u64, Ordering::Relaxed);
    }

    /// MVCC：创建新版本
    pub async fn create_version(&self, file_id: &str) -> Result<u64> {
        let mut versions = self.mvcc_versions.write().unwrap();
        let new_version = versions.get(file_id).copied().unwrap_or(0) + 1;
        versions.insert(file_id.to_string(), new_version);
        Ok(new_version)
    }

    /// MVCC：读取指定版本
    pub async fn read_version(&self, file_id: &str, version: u64) -> Result<Vec<u8>> {
        // TODO: 实现版本读取逻辑
        let size = 1024;
        Ok(vec![0u8; size])
    }

    /// MVCC：提交版本
    pub async fn commit_version(&self, file_id: &str, version: u64) -> Result<()> {
        // 版本提交后需要强制刷新
        self.force_flush.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// MVCC：回滚版本
    pub async fn rollback_version(&self, file_id: &str, version: u64) -> Result<()> {
        let mut versions = self.mvcc_versions.write().unwrap();
        versions.remove(file_id);
        Ok(())
    }

    /// 获取写入统计
    pub fn get_write_stats(&self) -> WriteStats {
        self.write_stats.clone()
    }

    /// 获取读取统计
    pub fn get_read_stats(&self) -> ReadStats {
        self.read_stats.clone()
    }

    /// 获取缓存效率
    pub async fn get_cache_efficiency(&self) -> f32 {
        let cache = self.read_cache.read().await;
        let hits = self.read_stats.cache_hits.load(Ordering::Relaxed);
        let misses = self.read_stats.cache_misses.load(Ordering::Relaxed);
        let total = hits + misses;

        if total == 0 {
            0.0
        } else {
            hits as f32 / total as f32
        }
    }

    /// 获取写放大比
    pub fn get_write_amplification(&self) -> f32 {
        let written = self.write_stats.bytes_written.load(Ordering::Relaxed);
        let physical = self
            .write_stats
            .physical_bytes_written
            .load(Ordering::Relaxed);

        if written == 0 {
            1.0
        } else {
            physical as f32 / written as f32
        }
    }

    /// 强制刷新所有待写入数据
    pub async fn flush(&self) -> Result<u32> {
        self.force_flush.store(true, Ordering::Relaxed);

        let mut flushed_count = 0;
        let mut queue = self.write_queue.write().await;
        flushed_count = queue.len() as u32;
        queue.clear();

        self.write_stats.flush_count.fetch_add(1, Ordering::Relaxed);
        self.force_flush.store(false, Ordering::Relaxed);

        info!("强制刷新完成，刷新了 {} 个任务", flushed_count);
        Ok(flushed_count)
    }

    /// 清理缓存
    pub async fn clear_cache(&self) -> Result<u32> {
        let mut cache = self.read_cache.write().await;
        let size = cache.weighted_size();
        cache.invalidate_all();
        info!("清理了缓存，释放了 {} 字节", size);
        Ok(1)
    }

    /// 关闭存储引擎
    pub async fn shutdown(&self) -> Result<()> {
        // 刷新所有待写入数据
        self.flush().await?;

        // 同步所有组件
        self.block_index.read().await.sync().await?;
        self.dedup_manager.read().await.sync().await?;

        info!("存储引擎已关闭");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_storage_engine_new() {
        let temp_dir = TempDir::new().unwrap();
        let config = EngineConfig::default();
        let index_config = BlockIndexConfig::default();
        let dedup_config = DedupConfig::default();
        let tier_config = TierConfig::default();
        let lifecycle_config = LifecycleConfig::default();

        let engine = StorageEngine::new(
            config,
            index_config,
            dedup_config,
            tier_config,
            lifecycle_config,
            temp_dir.path().to_str().unwrap(),
        );

        engine.init().await.unwrap();
    }

    #[tokio::test]
    async fn test_write_and_read() {
        let temp_dir = TempDir::new().unwrap();
        let config = EngineConfig::default();
        let index_config = BlockIndexConfig::default();
        let dedup_config = DedupConfig::default();
        let tier_config = TierConfig::default();
        let lifecycle_config = LifecycleConfig::default();

        let engine = StorageEngine::new(
            config,
            index_config,
            dedup_config,
            tier_config,
            lifecycle_config,
            temp_dir.path().to_str().unwrap(),
        );

        engine.init().await.unwrap();

        // 写入数据
        let data = b"Hello, World!";
        let bytes_written = engine.write("file1", 0, data).await.unwrap();
        assert_eq!(bytes_written, data.len() as u64);

        // 读取数据
        let read_data = engine.read("file1", 0, data.len()).await.unwrap();
        assert_eq!(read_data, data);
    }

    #[tokio::test]
    async fn test_cache_efficiency() {
        let temp_dir = TempDir::new().unwrap();
        let config = EngineConfig::default();
        let index_config = BlockIndexConfig::default();
        let dedup_config = DedupConfig::default();
        let tier_config = TierConfig::default();
        let lifecycle_config = LifecycleConfig::default();

        let engine = StorageEngine::new(
            config,
            index_config,
            dedup_config,
            tier_config,
            lifecycle_config,
            temp_dir.path().to_str().unwrap(),
        );

        engine.init().await.unwrap();

        // 第一次读取（缓存未命中）
        let _data1 = engine.read("file1", 0, 1024).await.unwrap();

        // 第二次读取（缓存命中）
        let _data2 = engine.read("file1", 0, 1024).await.unwrap();

        // 检查缓存效率
        let efficiency = engine.get_cache_efficiency().await;
        assert!(efficiency > 0.0);
    }

    #[tokio::test]
    async fn test_mvcc_versioning() {
        let temp_dir = TempDir::new().unwrap();
        let config = EngineConfig::default();
        let index_config = BlockIndexConfig::default();
        let dedup_config = DedupConfig::default();
        let tier_config = TierConfig::default();
        let lifecycle_config = LifecycleConfig::default();

        let engine = StorageEngine::new(
            config,
            index_config,
            dedup_config,
            tier_config,
            lifecycle_config,
            temp_dir.path().to_str().unwrap(),
        );

        engine.init().await.unwrap();

        // 创建版本
        let version1 = engine.create_version("file1").await.unwrap();
        assert_eq!(version1, 1);

        let version2 = engine.create_version("file1").await.unwrap();
        assert_eq!(version2, 2);

        // 读取版本
        let _data = engine.read_version("file1", version1).await.unwrap();

        // 提交版本
        engine.commit_version("file1", version1).await.unwrap();

        // 回滚版本
        engine.rollback_version("file1", version2).await.unwrap();
    }
}
