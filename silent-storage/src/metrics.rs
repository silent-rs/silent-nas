//! Prometheus 指标收集模块
//!
//! 该模块提供存储系统的实时监控指标，支持 Prometheus 格式导出

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// 存储指标
#[derive(Debug, Clone, Default)]
pub struct StorageMetrics {
    /// 存储统计
    pub storage: StorageStats,
    /// 去重统计
    pub deduplication: DeduplicationMetrics,
    /// 压缩统计
    pub compression: CompressionMetrics,
    /// 增量统计
    pub delta: DeltaMetrics,
    /// 性能统计
    pub performance: PerformanceMetrics,
    /// 操作计数
    pub operations: OperationCounters,
}

impl Serialize for StorageMetrics {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("StorageMetrics", 6)?;
        state.serialize_field("storage", &self.storage)?;
        state.serialize_field("deduplication", &self.deduplication)?;
        state.serialize_field("compression", &self.compression)?;
        state.serialize_field("delta", &self.delta)?;
        state.serialize_field("performance", &self.performance)?;
        state.serialize_field("operations", &self.operations)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for StorageMetrics {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct StorageMetricsHelper {
            storage: StorageStats,
            deduplication: DeduplicationMetrics,
            compression: CompressionMetrics,
            delta: DeltaMetrics,
            performance: PerformanceMetrics,
            operations: OperationCounters,
        }

        let helper = StorageMetricsHelper::deserialize(deserializer)?;
        Ok(Self {
            storage: helper.storage,
            deduplication: helper.deduplication,
            compression: helper.compression,
            delta: helper.delta,
            performance: helper.performance,
            operations: helper.operations,
        })
    }
}

/// 存储统计
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageStats {
    /// 总空间（字节）
    pub total_space: u64,
    /// 已用空间（字节）
    pub used_space: u64,
    /// 可用空间（字节）
    pub available_space: u64,
    /// 总文件数
    pub total_files: usize,
    /// 总版本数
    pub total_versions: usize,
    /// 总块数
    pub total_chunks: usize,
    /// 孤儿块数
    pub orphaned_chunks: usize,
}

impl StorageStats {
    /// 计算空间使用率
    pub fn usage_ratio(&self) -> f64 {
        if self.total_space == 0 {
            0.0
        } else {
            self.used_space as f64 / self.total_space as f64
        }
    }

    /// 格式化为 Prometheus 指标
    pub fn to_prometheus(&self) -> String {
        format!(
            "# HELP storage_total_space_bytes Total storage space in bytes\n\
             # TYPE storage_total_space_bytes gauge\n\
             storage_total_space_bytes {}\n\
             # HELP storage_used_space_bytes Used storage space in bytes\n\
             # TYPE storage_used_space_bytes gauge\n\
             storage_used_space_bytes {}\n\
             # HELP storage_available_space_bytes Available storage space in bytes\n\
             # TYPE storage_available_space_bytes gauge\n\
             storage_available_space_bytes {}\n\
             # HELP storage_total_files Total number of files\n\
             # TYPE storage_total_files gauge\n\
             storage_total_files {}\n\
             # HELP storage_total_versions Total number of versions\n\
             # TYPE storage_total_versions gauge\n\
             storage_total_versions {}\n\
             # HELP storage_total_chunks Total number of chunks\n\
             # TYPE storage_total_chunks gauge\n\
             storage_total_chunks {}\n\
             # HELP storage_orphaned_chunks Number of orphaned chunks\n\
             # TYPE storage_orphaned_chunks gauge\n\
             storage_orphaned_chunks {}\n\
             # HELP storage_usage_ratio Storage usage ratio (0.0-1.0)\n\
             # TYPE storage_usage_ratio gauge\n\
             storage_usage_ratio {}\n",
            self.total_space,
            self.used_space,
            self.available_space,
            self.total_files,
            self.total_versions,
            self.total_chunks,
            self.orphaned_chunks,
            self.usage_ratio()
        )
    }
}

/// 去重统计
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeduplicationMetrics {
    /// 总块数
    pub total_chunks: usize,
    /// 唯一块数
    pub unique_chunks: usize,
    /// 重复块数
    pub duplicate_chunks: usize,
    /// 原始数据大小（字节）
    pub original_size: u64,
    /// 实际存储大小（字节）
    pub stored_size: u64,
    /// 节省空间（字节）
    pub space_saved: u64,
}

impl DeduplicationMetrics {
    /// 计算去重率
    pub fn dedup_ratio(&self) -> f64 {
        if self.total_chunks == 0 {
            0.0
        } else {
            self.duplicate_chunks as f64 / self.total_chunks as f64
        }
    }

    /// 计算空间节省率
    pub fn space_saving_ratio(&self) -> f64 {
        if self.original_size == 0 {
            0.0
        } else {
            self.space_saved as f64 / self.original_size as f64
        }
    }

    /// 格式化为 Prometheus 指标
    pub fn to_prometheus(&self) -> String {
        format!(
            "# HELP dedup_total_chunks Total number of chunks\n\
             # TYPE dedup_total_chunks counter\n\
             dedup_total_chunks {}\n\
             # HELP dedup_unique_chunks Number of unique chunks\n\
             # TYPE dedup_unique_chunks gauge\n\
             dedup_unique_chunks {}\n\
             # HELP dedup_duplicate_chunks Number of duplicate chunks\n\
             # TYPE dedup_duplicate_chunks counter\n\
             dedup_duplicate_chunks {}\n\
             # HELP dedup_original_size_bytes Original data size in bytes\n\
             # TYPE dedup_original_size_bytes counter\n\
             dedup_original_size_bytes {}\n\
             # HELP dedup_stored_size_bytes Actual stored size in bytes\n\
             # TYPE dedup_stored_size_bytes counter\n\
             dedup_stored_size_bytes {}\n\
             # HELP dedup_space_saved_bytes Space saved by deduplication in bytes\n\
             # TYPE dedup_space_saved_bytes counter\n\
             dedup_space_saved_bytes {}\n\
             # HELP dedup_ratio Deduplication ratio (0.0-1.0)\n\
             # TYPE dedup_ratio gauge\n\
             dedup_ratio {}\n\
             # HELP dedup_space_saving_ratio Space saving ratio (0.0-1.0)\n\
             # TYPE dedup_space_saving_ratio gauge\n\
             dedup_space_saving_ratio {}\n",
            self.total_chunks,
            self.unique_chunks,
            self.duplicate_chunks,
            self.original_size,
            self.stored_size,
            self.space_saved,
            self.dedup_ratio(),
            self.space_saving_ratio()
        )
    }
}

/// 压缩统计
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompressionMetrics {
    /// 压缩前总大小（字节）
    pub uncompressed_size: u64,
    /// 压缩后总大小（字节）
    pub compressed_size: u64,
    /// 节省空间（字节）
    pub space_saved: u64,
    /// LZ4 压缩次数
    pub lz4_compressions: usize,
    /// Zstd 压缩次数
    pub zstd_compressions: usize,
    /// 跳过压缩次数
    pub skipped_compressions: usize,
}

impl CompressionMetrics {
    /// 计算压缩比
    pub fn compression_ratio(&self) -> f64 {
        if self.compressed_size == 0 {
            0.0
        } else {
            self.uncompressed_size as f64 / self.compressed_size as f64
        }
    }

    /// 计算空间节省率
    pub fn space_saving_ratio(&self) -> f64 {
        if self.uncompressed_size == 0 {
            0.0
        } else {
            self.space_saved as f64 / self.uncompressed_size as f64
        }
    }

    /// 格式化为 Prometheus 指标
    pub fn to_prometheus(&self) -> String {
        format!(
            "# HELP compression_uncompressed_size_bytes Total uncompressed size in bytes\n\
             # TYPE compression_uncompressed_size_bytes counter\n\
             compression_uncompressed_size_bytes {}\n\
             # HELP compression_compressed_size_bytes Total compressed size in bytes\n\
             # TYPE compression_compressed_size_bytes counter\n\
             compression_compressed_size_bytes {}\n\
             # HELP compression_space_saved_bytes Space saved by compression in bytes\n\
             # TYPE compression_space_saved_bytes counter\n\
             compression_space_saved_bytes {}\n\
             # HELP compression_lz4_total Total number of LZ4 compressions\n\
             # TYPE compression_lz4_total counter\n\
             compression_lz4_total {}\n\
             # HELP compression_zstd_total Total number of Zstd compressions\n\
             # TYPE compression_zstd_total counter\n\
             compression_zstd_total {}\n\
             # HELP compression_skipped_total Total number of skipped compressions\n\
             # TYPE compression_skipped_total counter\n\
             compression_skipped_total {}\n\
             # HELP compression_ratio Compression ratio\n\
             # TYPE compression_ratio gauge\n\
             compression_ratio {}\n\
             # HELP compression_space_saving_ratio Space saving ratio (0.0-1.0)\n\
             # TYPE compression_space_saving_ratio gauge\n\
             compression_space_saving_ratio {}\n",
            self.uncompressed_size,
            self.compressed_size,
            self.space_saved,
            self.lz4_compressions,
            self.zstd_compressions,
            self.skipped_compressions,
            self.compression_ratio(),
            self.space_saving_ratio()
        )
    }
}

/// 增量存储统计
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeltaMetrics {
    /// 全量版本数
    pub full_versions: usize,
    /// 增量版本数
    pub delta_versions: usize,
    /// Delta 原始大小（字节）
    pub delta_original_size: u64,
    /// Delta 存储大小（字节）
    pub delta_stored_size: u64,
    /// 节省空间（字节）
    pub space_saved: u64,
}

impl DeltaMetrics {
    /// 计算 Delta 率
    pub fn delta_ratio(&self) -> f64 {
        let total = self.full_versions + self.delta_versions;
        if total == 0 {
            0.0
        } else {
            self.delta_versions as f64 / total as f64
        }
    }

    /// 计算空间节省率
    pub fn space_saving_ratio(&self) -> f64 {
        if self.delta_original_size == 0 {
            0.0
        } else {
            self.space_saved as f64 / self.delta_original_size as f64
        }
    }

    /// 格式化为 Prometheus 指标
    pub fn to_prometheus(&self) -> String {
        format!(
            "# HELP delta_full_versions_total Total number of full versions\n\
             # TYPE delta_full_versions_total counter\n\
             delta_full_versions_total {}\n\
             # HELP delta_incremental_versions_total Total number of delta versions\n\
             # TYPE delta_incremental_versions_total counter\n\
             delta_incremental_versions_total {}\n\
             # HELP delta_original_size_bytes Original delta size in bytes\n\
             # TYPE delta_original_size_bytes counter\n\
             delta_original_size_bytes {}\n\
             # HELP delta_stored_size_bytes Actual delta stored size in bytes\n\
             # TYPE delta_stored_size_bytes counter\n\
             delta_stored_size_bytes {}\n\
             # HELP delta_space_saved_bytes Space saved by delta in bytes\n\
             # TYPE delta_space_saved_bytes counter\n\
             delta_space_saved_bytes {}\n\
             # HELP delta_ratio Delta version ratio (0.0-1.0)\n\
             # TYPE delta_ratio gauge\n\
             delta_ratio {}\n\
             # HELP delta_space_saving_ratio Space saving ratio (0.0-1.0)\n\
             # TYPE delta_space_saving_ratio gauge\n\
             delta_space_saving_ratio {}\n",
            self.full_versions,
            self.delta_versions,
            self.delta_original_size,
            self.delta_stored_size,
            self.space_saved,
            self.delta_ratio(),
            self.space_saving_ratio()
        )
    }
}

/// 性能统计
#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    /// 读操作平均延迟（微秒）
    pub read_latency_us: Arc<AtomicU64>,
    /// 写操作平均延迟（微秒）
    pub write_latency_us: Arc<AtomicU64>,
    /// 删除操作平均延迟（微秒）
    pub delete_latency_us: Arc<AtomicU64>,
    /// 读吞吐量（字节/秒）
    pub read_throughput_bps: Arc<AtomicU64>,
    /// 写吞吐量（字节/秒）
    pub write_throughput_bps: Arc<AtomicU64>,
}

impl Serialize for PerformanceMetrics {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("PerformanceMetrics", 5)?;
        state.serialize_field(
            "read_latency_us",
            &self.read_latency_us.load(Ordering::Relaxed),
        )?;
        state.serialize_field(
            "write_latency_us",
            &self.write_latency_us.load(Ordering::Relaxed),
        )?;
        state.serialize_field(
            "delete_latency_us",
            &self.delete_latency_us.load(Ordering::Relaxed),
        )?;
        state.serialize_field(
            "read_throughput_bps",
            &self.read_throughput_bps.load(Ordering::Relaxed),
        )?;
        state.serialize_field(
            "write_throughput_bps",
            &self.write_throughput_bps.load(Ordering::Relaxed),
        )?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for PerformanceMetrics {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct PerformanceMetricsHelper {
            read_latency_us: u64,
            write_latency_us: u64,
            delete_latency_us: u64,
            read_throughput_bps: u64,
            write_throughput_bps: u64,
        }

        let helper = PerformanceMetricsHelper::deserialize(deserializer)?;
        Ok(Self {
            read_latency_us: Arc::new(AtomicU64::new(helper.read_latency_us)),
            write_latency_us: Arc::new(AtomicU64::new(helper.write_latency_us)),
            delete_latency_us: Arc::new(AtomicU64::new(helper.delete_latency_us)),
            read_throughput_bps: Arc::new(AtomicU64::new(helper.read_throughput_bps)),
            write_throughput_bps: Arc::new(AtomicU64::new(helper.write_throughput_bps)),
        })
    }
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self {
            read_latency_us: Arc::new(AtomicU64::new(0)),
            write_latency_us: Arc::new(AtomicU64::new(0)),
            delete_latency_us: Arc::new(AtomicU64::new(0)),
            read_throughput_bps: Arc::new(AtomicU64::new(0)),
            write_throughput_bps: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl PerformanceMetrics {
    /// 更新读延迟
    pub fn update_read_latency(&self, latency_us: u64) {
        self.read_latency_us.store(latency_us, Ordering::Relaxed);
    }

    /// 更新写延迟
    pub fn update_write_latency(&self, latency_us: u64) {
        self.write_latency_us.store(latency_us, Ordering::Relaxed);
    }

    /// 更新删除延迟
    pub fn update_delete_latency(&self, latency_us: u64) {
        self.delete_latency_us.store(latency_us, Ordering::Relaxed);
    }

    /// 更新读吞吐量
    pub fn update_read_throughput(&self, bytes_per_sec: u64) {
        self.read_throughput_bps
            .store(bytes_per_sec, Ordering::Relaxed);
    }

    /// 更新写吞吐量
    pub fn update_write_throughput(&self, bytes_per_sec: u64) {
        self.write_throughput_bps
            .store(bytes_per_sec, Ordering::Relaxed);
    }

    /// 格式化为 Prometheus 指标
    pub fn to_prometheus(&self) -> String {
        format!(
            "# HELP perf_read_latency_microseconds Average read latency in microseconds\n\
             # TYPE perf_read_latency_microseconds gauge\n\
             perf_read_latency_microseconds {}\n\
             # HELP perf_write_latency_microseconds Average write latency in microseconds\n\
             # TYPE perf_write_latency_microseconds gauge\n\
             perf_write_latency_microseconds {}\n\
             # HELP perf_delete_latency_microseconds Average delete latency in microseconds\n\
             # TYPE perf_delete_latency_microseconds gauge\n\
             perf_delete_latency_microseconds {}\n\
             # HELP perf_read_throughput_bytes_per_second Read throughput in bytes per second\n\
             # TYPE perf_read_throughput_bytes_per_second gauge\n\
             perf_read_throughput_bytes_per_second {}\n\
             # HELP perf_write_throughput_bytes_per_second Write throughput in bytes per second\n\
             # TYPE perf_write_throughput_bytes_per_second gauge\n\
             perf_write_throughput_bytes_per_second {}\n",
            self.read_latency_us.load(Ordering::Relaxed),
            self.write_latency_us.load(Ordering::Relaxed),
            self.delete_latency_us.load(Ordering::Relaxed),
            self.read_throughput_bps.load(Ordering::Relaxed),
            self.write_throughput_bps.load(Ordering::Relaxed)
        )
    }
}

/// 操作计数器
#[derive(Debug, Clone)]
pub struct OperationCounters {
    /// 创建操作次数
    pub create_count: Arc<AtomicUsize>,
    /// 读取操作次数
    pub read_count: Arc<AtomicUsize>,
    /// 更新操作次数
    pub update_count: Arc<AtomicUsize>,
    /// 删除操作次数
    pub delete_count: Arc<AtomicUsize>,
    /// 复制操作次数
    pub copy_count: Arc<AtomicUsize>,
    /// 垃圾回收次数
    pub gc_count: Arc<AtomicUsize>,
    /// 错误次数
    pub error_count: Arc<AtomicUsize>,
}

impl Serialize for OperationCounters {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("OperationCounters", 7)?;
        state.serialize_field("create_count", &self.create_count.load(Ordering::Relaxed))?;
        state.serialize_field("read_count", &self.read_count.load(Ordering::Relaxed))?;
        state.serialize_field("update_count", &self.update_count.load(Ordering::Relaxed))?;
        state.serialize_field("delete_count", &self.delete_count.load(Ordering::Relaxed))?;
        state.serialize_field("copy_count", &self.copy_count.load(Ordering::Relaxed))?;
        state.serialize_field("gc_count", &self.gc_count.load(Ordering::Relaxed))?;
        state.serialize_field("error_count", &self.error_count.load(Ordering::Relaxed))?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for OperationCounters {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct OperationCountersHelper {
            create_count: usize,
            read_count: usize,
            update_count: usize,
            delete_count: usize,
            copy_count: usize,
            gc_count: usize,
            error_count: usize,
        }

        let helper = OperationCountersHelper::deserialize(deserializer)?;
        Ok(Self {
            create_count: Arc::new(AtomicUsize::new(helper.create_count)),
            read_count: Arc::new(AtomicUsize::new(helper.read_count)),
            update_count: Arc::new(AtomicUsize::new(helper.update_count)),
            delete_count: Arc::new(AtomicUsize::new(helper.delete_count)),
            copy_count: Arc::new(AtomicUsize::new(helper.copy_count)),
            gc_count: Arc::new(AtomicUsize::new(helper.gc_count)),
            error_count: Arc::new(AtomicUsize::new(helper.error_count)),
        })
    }
}

impl Default for OperationCounters {
    fn default() -> Self {
        Self {
            create_count: Arc::new(AtomicUsize::new(0)),
            read_count: Arc::new(AtomicUsize::new(0)),
            update_count: Arc::new(AtomicUsize::new(0)),
            delete_count: Arc::new(AtomicUsize::new(0)),
            copy_count: Arc::new(AtomicUsize::new(0)),
            gc_count: Arc::new(AtomicUsize::new(0)),
            error_count: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl OperationCounters {
    /// 增加创建计数
    pub fn inc_create(&self) {
        self.create_count.fetch_add(1, Ordering::Relaxed);
    }

    /// 增加读取计数
    pub fn inc_read(&self) {
        self.read_count.fetch_add(1, Ordering::Relaxed);
    }

    /// 增加更新计数
    pub fn inc_update(&self) {
        self.update_count.fetch_add(1, Ordering::Relaxed);
    }

    /// 增加删除计数
    pub fn inc_delete(&self) {
        self.delete_count.fetch_add(1, Ordering::Relaxed);
    }

    /// 增加复制计数
    pub fn inc_copy(&self) {
        self.copy_count.fetch_add(1, Ordering::Relaxed);
    }

    /// 增加垃圾回收计数
    pub fn inc_gc(&self) {
        self.gc_count.fetch_add(1, Ordering::Relaxed);
    }

    /// 增加错误计数
    pub fn inc_error(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
    }

    /// 格式化为 Prometheus 指标
    pub fn to_prometheus(&self) -> String {
        format!(
            "# HELP ops_create_total Total number of create operations\n\
             # TYPE ops_create_total counter\n\
             ops_create_total {}\n\
             # HELP ops_read_total Total number of read operations\n\
             # TYPE ops_read_total counter\n\
             ops_read_total {}\n\
             # HELP ops_update_total Total number of update operations\n\
             # TYPE ops_update_total counter\n\
             ops_update_total {}\n\
             # HELP ops_delete_total Total number of delete operations\n\
             # TYPE ops_delete_total counter\n\
             ops_delete_total {}\n\
             # HELP ops_copy_total Total number of copy operations\n\
             # TYPE ops_copy_total counter\n\
             ops_copy_total {}\n\
             # HELP ops_gc_total Total number of garbage collection operations\n\
             # TYPE ops_gc_total counter\n\
             ops_gc_total {}\n\
             # HELP ops_error_total Total number of errors\n\
             # TYPE ops_error_total counter\n\
             ops_error_total {}\n",
            self.create_count.load(Ordering::Relaxed),
            self.read_count.load(Ordering::Relaxed),
            self.update_count.load(Ordering::Relaxed),
            self.delete_count.load(Ordering::Relaxed),
            self.copy_count.load(Ordering::Relaxed),
            self.gc_count.load(Ordering::Relaxed),
            self.error_count.load(Ordering::Relaxed)
        )
    }
}

impl StorageMetrics {
    /// 创建新的指标实例
    pub fn new() -> Self {
        Self::default()
    }

    /// 格式化为 Prometheus 指标格式
    pub fn to_prometheus(&self) -> String {
        let mut output = String::new();
        output.push_str(&self.storage.to_prometheus());
        output.push('\n');
        output.push_str(&self.deduplication.to_prometheus());
        output.push('\n');
        output.push_str(&self.compression.to_prometheus());
        output.push('\n');
        output.push_str(&self.delta.to_prometheus());
        output.push('\n');
        output.push_str(&self.performance.to_prometheus());
        output.push('\n');
        output.push_str(&self.operations.to_prometheus());
        output
    }
}

/// 健康状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// 是否健康
    pub healthy: bool,
    /// 状态消息
    pub message: String,
    /// 检查时间
    pub checked_at: NaiveDateTime,
    /// 存储可用
    pub storage_available: bool,
    /// 数据库可用
    pub database_available: bool,
}

impl HealthStatus {
    /// 创建健康状态
    pub fn healthy() -> Self {
        Self {
            healthy: true,
            message: "All systems operational".to_string(),
            checked_at: chrono::Local::now().naive_local(),
            storage_available: true,
            database_available: true,
        }
    }

    /// 创建不健康状态
    pub fn unhealthy(message: String) -> Self {
        Self {
            healthy: false,
            message,
            checked_at: chrono::Local::now().naive_local(),
            storage_available: false,
            database_available: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_stats() {
        let stats = StorageStats {
            total_space: 1000,
            used_space: 600,
            available_space: 400,
            total_files: 10,
            total_versions: 20,
            total_chunks: 100,
            orphaned_chunks: 5,
        };

        assert_eq!(stats.usage_ratio(), 0.6);
        assert!(
            stats
                .to_prometheus()
                .contains("storage_total_space_bytes 1000")
        );
    }

    #[test]
    fn test_dedup_metrics() {
        let dedup = DeduplicationMetrics {
            total_chunks: 100,
            unique_chunks: 60,
            duplicate_chunks: 40,
            original_size: 1000,
            stored_size: 600,
            space_saved: 400,
        };

        assert_eq!(dedup.dedup_ratio(), 0.4);
        assert_eq!(dedup.space_saving_ratio(), 0.4);
        assert!(dedup.to_prometheus().contains("dedup_ratio 0.4"));
    }

    #[test]
    fn test_compression_metrics() {
        let compression = CompressionMetrics {
            uncompressed_size: 1000,
            compressed_size: 250,
            space_saved: 750,
            lz4_compressions: 10,
            zstd_compressions: 5,
            skipped_compressions: 2,
        };

        assert_eq!(compression.compression_ratio(), 4.0);
        assert_eq!(compression.space_saving_ratio(), 0.75);
        assert!(compression.to_prometheus().contains("compression_ratio 4"));
    }

    #[test]
    fn test_operation_counters() {
        let ops = OperationCounters::default();

        ops.inc_create();
        ops.inc_read();
        ops.inc_read();
        ops.inc_update();

        assert_eq!(ops.create_count.load(Ordering::Relaxed), 1);
        assert_eq!(ops.read_count.load(Ordering::Relaxed), 2);
        assert_eq!(ops.update_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_health_status() {
        let healthy = HealthStatus::healthy();
        assert!(healthy.healthy);
        assert!(healthy.storage_available);

        let unhealthy = HealthStatus::unhealthy("Test error".to_string());
        assert!(!unhealthy.healthy);
        assert_eq!(unhealthy.message, "Test error");
    }
}
