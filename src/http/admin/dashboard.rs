//! 仪表盘 API

use super::super::state::AppState;
use serde::{Deserialize, Serialize};
use silent::extractor::Configs as CfgExtractor;
use silent::prelude::*;

/// 系统概览数据
#[derive(Debug, Serialize, Deserialize)]
pub struct SystemOverview {
    /// 文件总数
    pub file_count: u64,
    /// 用户总数
    pub user_count: u64,
    /// 存储使用情况
    pub storage: StorageUsage,
    /// 在线节点数
    pub online_nodes: u64,
}

/// 存储使用情况
#[derive(Debug, Serialize, Deserialize)]
pub struct StorageUsage {
    /// 总容量（字节）
    pub total_bytes: u64,
    /// 已用空间（字节）
    pub used_bytes: u64,
    /// 可用空间（字节）
    pub available_bytes: u64,
    /// 使用率（百分比）
    pub usage_percent: f64,
}

/// 性能指标
#[derive(Debug, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    /// CPU 使用率（百分比）
    pub cpu_usage: f64,
    /// 内存使用率（百分比）
    pub memory_usage: f64,
    /// 网络流量
    pub network: NetworkTraffic,
    /// 请求 QPS
    pub request_qps: f64,
}

/// 网络流量
#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkTraffic {
    /// 上传速率（字节/秒）
    pub upload_bytes_per_sec: u64,
    /// 下载速率（字节/秒）
    pub download_bytes_per_sec: u64,
}

/// 最近活动
#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct RecentActivity {
    /// 活动ID
    pub id: String,
    /// 用户名
    pub username: String,
    /// 操作类型
    pub action: String,
    /// 文件路径
    pub file_path: Option<String>,
    /// 时间戳
    pub timestamp: i64,
}

/// GET /api/admin/dashboard/overview
/// 获取系统概览数据
pub async fn get_overview(
    _req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    // 获取存储统计信息(一次性获取所有数据,避免重复调用)
    let storage_stats = state.storage.get_storage_stats().await;

    // 获取文件总数 - 从存储引擎获取实际文件数(不包括已删除的)
    let file_count = match state.storage.list_files().await {
        Ok(files) => files.len() as u64,
        Err(_) => 0,
    };

    // 获取用户总数
    let user_count = if let Some(ref auth_manager) = state.auth_manager {
        match auth_manager.list_users().await {
            Ok(users) => users.len() as u64,
            Err(_) => 0,
        }
    } else {
        0
    };

    // 获取存储使用情况
    let storage = match storage_stats {
        Ok(stats) => {
            // total_chunk_size: 实际占用的磁盘空间(去重和压缩后的块文件大小)
            let used = stats.total_chunk_size;

            // 获取文件系统信息
            // 在 Unix 系统上使用 statvfs 获取文件系统统计信息
            #[cfg(unix)]
            let (total, available) = {
                use std::os::unix::fs::MetadataExt;
                use std::path::Path;

                // 尝试获取 storage 目录的文件系统信息
                let storage_path = Path::new("./storage");
                if let Ok(metadata) = std::fs::metadata(storage_path) {
                    // 获取文件系统 ID,但我们无法直接获取容量
                    // 这里使用一个合理的估算值
                    // 对于生产环境,应该使用 nix crate 的 statvfs
                    let fs_id = metadata.dev();
                    tracing::debug!("Storage filesystem ID: {}", fs_id);

                    // 临时方案: 使用合理的默认值
                    // 假设至少有 100GB 可用空间
                    let estimated_total = 100 * 1024 * 1024 * 1024u64;
                    let estimated_available = estimated_total.saturating_sub(used);
                    (estimated_total, estimated_available)
                } else {
                    (0, 0)
                }
            };

            // 在非 Unix 系统上使用默认值
            #[cfg(not(unix))]
            let (total, available) = {
                let estimated_total = 100 * 1024 * 1024 * 1024u64;
                let estimated_available = estimated_total.saturating_sub(used);
                (estimated_total, estimated_available)
            };

            StorageUsage {
                total_bytes: total,
                used_bytes: used,
                available_bytes: available,
                usage_percent: if total > 0 {
                    used as f64 / total as f64
                } else {
                    0.0
                },
            }
        }
        Err(_) => StorageUsage {
            total_bytes: 0,
            used_bytes: 0,
            available_bytes: 0,
            usage_percent: 0.0,
        },
    };

    // 获取在线节点数（TODO: 从节点管理器获取）
    let online_nodes = 1u64; // 当前节点

    let overview = SystemOverview {
        file_count,
        user_count,
        storage,
        online_nodes,
    };

    Ok(serde_json::to_value(overview).unwrap())
}

/// GET /api/admin/dashboard/metrics
/// 获取性能指标
pub async fn get_metrics(
    _req: Request,
    _state: CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    // TODO: 实现真实的性能指标收集
    // 这里返回模拟数据
    let metrics = PerformanceMetrics {
        cpu_usage: 0.0,
        memory_usage: 0.0,
        network: NetworkTraffic {
            upload_bytes_per_sec: 0,
            download_bytes_per_sec: 0,
        },
        request_qps: 0.0,
    };

    Ok(serde_json::to_value(metrics).unwrap())
}

/// GET /api/admin/dashboard/activities
/// 获取最近活动
pub async fn get_activities(
    _req: Request,
    CfgExtractor(_state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    // TODO: 实现从审计日志获取最近活动
    // 暂时返回空数组
    Ok(serde_json::json!([]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_usage() {
        let storage = StorageUsage {
            total_bytes: 1000,
            used_bytes: 600,
            available_bytes: 400,
            usage_percent: 60.0,
        };

        assert_eq!(storage.total_bytes, 1000);
        assert_eq!(storage.used_bytes, 600);
        assert_eq!(storage.usage_percent, 60.0);
    }

    #[test]
    fn test_system_overview_serialization() {
        let overview = SystemOverview {
            file_count: 100,
            user_count: 5,
            storage: StorageUsage {
                total_bytes: 1000,
                used_bytes: 600,
                available_bytes: 400,
                usage_percent: 60.0,
            },
            online_nodes: 1,
        };

        let json = serde_json::to_value(&overview).unwrap();
        assert!(json.is_object());
        assert_eq!(json["file_count"], 100);
        assert_eq!(json["user_count"], 5);
    }
}
