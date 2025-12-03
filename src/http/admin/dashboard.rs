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
    // 获取文件总数
    let file_count = state
        .sync_manager
        .get_all_sync_states()
        .await
        .into_iter()
        .filter(|s| !s.is_deleted())
        .count() as u64;

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
    let storage = match state.storage.get_storage_stats().await {
        Ok(stats) => {
            let used = stats.total_chunk_size;
            let total = stats.total_size;
            StorageUsage {
                total_bytes: total,
                used_bytes: used,
                available_bytes: total.saturating_sub(used),
                usage_percent: if total > 0 {
                    (used as f64 / total as f64) * 100.0
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
