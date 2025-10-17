//! Prometheus Metrics 模块
//!
//! 提供应用程序的各项监控指标

#![allow(dead_code)] // 这些函数将在后续集成时使用

use lazy_static::lazy_static;
use prometheus::{
    CounterVec, Encoder, Gauge, HistogramVec, IntCounterVec, IntGauge, TextEncoder,
    register_counter_vec, register_gauge, register_histogram_vec, register_int_counter_vec,
    register_int_gauge,
};

lazy_static! {
    // ============ HTTP 指标 ============
    /// HTTP 请求总数
    pub static ref HTTP_REQUESTS_TOTAL: IntCounterVec = register_int_counter_vec!(
        "http_requests_total",
        "Total number of HTTP requests",
        &["method", "path", "status"]
    )
    .unwrap();

    /// HTTP 请求延迟（秒）
    pub static ref HTTP_REQUEST_DURATION_SECONDS: HistogramVec = register_histogram_vec!(
        "http_request_duration_seconds",
        "HTTP request duration in seconds",
        &["method", "path"],
        vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
    )
    .unwrap();

    /// HTTP 当前活跃连接数
    pub static ref HTTP_REQUESTS_IN_FLIGHT: IntGauge = register_int_gauge!(
        "http_requests_in_flight",
        "Current number of HTTP requests being processed"
    )
    .unwrap();

    // ============ 文件操作指标 ============
    /// 文件操作总数
    pub static ref FILE_OPERATIONS_TOTAL: IntCounterVec = register_int_counter_vec!(
        "file_operations_total",
        "Total number of file operations",
        &["operation"] // upload, download, delete, list
    )
    .unwrap();

    /// 文件传输字节数
    pub static ref FILE_BYTES_TRANSFERRED: IntCounterVec = register_int_counter_vec!(
        "file_bytes_transferred_total",
        "Total bytes transferred in file operations",
        &["direction"] // sent, received
    )
    .unwrap();

    /// 当前文件总数
    pub static ref FILE_COUNT_TOTAL: IntGauge = register_int_gauge!(
        "file_count_total",
        "Total number of files in storage"
    )
    .unwrap();

    /// 存储使用字节数
    pub static ref STORAGE_BYTES_USED: IntGauge = register_int_gauge!(
        "storage_bytes_used",
        "Total bytes used in storage"
    )
    .unwrap();

    // ============ 搜索指标 ============
    /// 搜索查询总数
    pub static ref SEARCH_QUERIES_TOTAL: IntCounterVec = register_int_counter_vec!(
        "search_queries_total",
        "Total number of search queries",
        &["status"] // success, error
    )
    .unwrap();

    /// 搜索查询延迟（秒）
    pub static ref SEARCH_QUERY_DURATION_SECONDS: HistogramVec = register_histogram_vec!(
        "search_query_duration_seconds",
        "Search query duration in seconds",
        &[],
        vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0]
    )
    .unwrap();

    /// 搜索结果总数
    pub static ref SEARCH_RESULTS_TOTAL: CounterVec = register_counter_vec!(
        "search_results_total",
        "Total number of search results returned",
        &[]
    )
    .unwrap();

    // ============ 同步指标 ============
    /// 同步操作总数
    pub static ref SYNC_OPERATIONS_TOTAL: IntCounterVec = register_int_counter_vec!(
        "sync_operations_total",
        "Total number of sync operations",
        &["type", "status"] // type: full/incremental, status: success/error
    )
    .unwrap();

    /// 同步传输字节数
    pub static ref SYNC_BYTES_TRANSFERRED: IntCounterVec = register_int_counter_vec!(
        "sync_bytes_transferred_total",
        "Total bytes transferred in sync operations",
        &["type"] // full, incremental
    )
    .unwrap();

    /// 同步冲突总数
    pub static ref SYNC_CONFLICTS_TOTAL: IntCounterVec = register_int_counter_vec!(
        "sync_conflicts_total",
        "Total number of sync conflicts",
        &["resolution"] // auto, manual, pending
    )
    .unwrap();

    // ============ 缓存指标 ============
    /// 缓存命中率
    pub static ref CACHE_HIT_RATE: Gauge = register_gauge!(
        "cache_hit_rate",
        "Cache hit rate (0.0 to 1.0)"
    )
    .unwrap();

    /// 缓存大小（字节）
    pub static ref CACHE_SIZE_BYTES: IntGauge = register_int_gauge!(
        "cache_size_bytes",
        "Total cache size in bytes"
    )
    .unwrap();

    /// 缓存条目数
    pub static ref CACHE_ENTRIES: IntGauge = register_int_gauge!(
        "cache_entries",
        "Total number of cache entries"
    )
    .unwrap();

    // ============ 系统指标 ============
    /// 当前活跃连接数
    pub static ref ACTIVE_CONNECTIONS: IntGauge = register_int_gauge!(
        "active_connections",
        "Current number of active connections"
    )
    .unwrap();
}

/// 导出 Prometheus metrics
pub fn export_metrics() -> Result<String, Box<dyn std::error::Error>> {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = vec![];
    encoder.encode(&metric_families, &mut buffer)?;
    Ok(String::from_utf8(buffer)?)
}

/// 记录 HTTP 请求
pub fn record_http_request(method: &str, path: &str, status: u16, duration: f64) {
    HTTP_REQUESTS_TOTAL
        .with_label_values(&[method, path, &status.to_string()])
        .inc();
    HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&[method, path])
        .observe(duration);
}

/// 记录文件操作
pub fn record_file_operation(operation: &str) {
    FILE_OPERATIONS_TOTAL.with_label_values(&[operation]).inc();
}

/// 记录文件传输
pub fn record_file_transfer(direction: &str, bytes: u64) {
    FILE_BYTES_TRANSFERRED
        .with_label_values(&[direction])
        .inc_by(bytes);
}

/// 更新存储统计
pub fn update_storage_stats(file_count: i64, bytes_used: i64) {
    FILE_COUNT_TOTAL.set(file_count);
    STORAGE_BYTES_USED.set(bytes_used);
}

/// 记录搜索查询
pub fn record_search_query(status: &str, duration: f64, result_count: usize) {
    SEARCH_QUERIES_TOTAL.with_label_values(&[status]).inc();
    SEARCH_QUERY_DURATION_SECONDS
        .with_label_values(&[])
        .observe(duration);
    SEARCH_RESULTS_TOTAL
        .with_label_values(&[])
        .inc_by(result_count as f64);
}

/// 记录同步操作
pub fn record_sync_operation(sync_type: &str, status: &str, bytes: u64) {
    SYNC_OPERATIONS_TOTAL
        .with_label_values(&[sync_type, status])
        .inc();
    SYNC_BYTES_TRANSFERRED
        .with_label_values(&[sync_type])
        .inc_by(bytes);
}

/// 记录同步冲突
pub fn record_sync_conflict(resolution: &str) {
    SYNC_CONFLICTS_TOTAL.with_label_values(&[resolution]).inc();
}

/// 更新缓存统计
pub fn update_cache_stats(hit_rate: f64, size_bytes: i64, entries: i64) {
    CACHE_HIT_RATE.set(hit_rate);
    CACHE_SIZE_BYTES.set(size_bytes);
    CACHE_ENTRIES.set(entries);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_http_request() {
        record_http_request("GET", "/api/files", 200, 0.05);
        // 验证 metrics 可以正常记录
    }

    #[test]
    fn test_record_file_operation() {
        record_file_operation("upload");
        record_file_operation("download");
        record_file_operation("delete");
    }

    #[test]
    fn test_export_metrics() {
        // 先记录一些指标
        record_http_request("GET", "/test", 200, 0.05);

        let result = export_metrics();
        assert!(result.is_ok());
        let metrics_text = result.unwrap();
        assert!(!metrics_text.is_empty());
        // 应该包含 Prometheus 格式的指标
        assert!(metrics_text.contains("http_requests_total"));
    }

    #[test]
    fn test_update_storage_stats() {
        update_storage_stats(100, 1024 * 1024);
        assert_eq!(FILE_COUNT_TOTAL.get(), 100);
        assert_eq!(STORAGE_BYTES_USED.get(), 1024 * 1024);
    }

    #[test]
    fn test_cache_stats() {
        update_cache_stats(0.85, 10 * 1024 * 1024, 1000);
        assert_eq!(CACHE_HIT_RATE.get(), 0.85);
        assert_eq!(CACHE_SIZE_BYTES.get(), 10 * 1024 * 1024);
        assert_eq!(CACHE_ENTRIES.get(), 1000);
    }
}
