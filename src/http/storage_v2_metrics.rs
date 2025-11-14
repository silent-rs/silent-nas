//! Storage V2 Prometheus 监控端点

use crate::http::AppState;
use http::StatusCode;
use silent::SilentError;
use silent::prelude::*;
use silent_storage::{HealthStatus, StorageMetrics};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Storage V2 指标状态
pub struct StorageV2MetricsState {
    /// 当前指标
    pub metrics: Arc<RwLock<StorageMetrics>>,
    /// 上次更新时间
    #[allow(dead_code)]
    pub last_update: Arc<RwLock<chrono::NaiveDateTime>>,
}

impl StorageV2MetricsState {
    /// 创建新的指标状态
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(RwLock::new(StorageMetrics::new())),
            last_update: Arc::new(RwLock::new(chrono::Local::now().naive_local())),
        }
    }

    /// 获取指标的 Prometheus 格式
    pub async fn get_prometheus_format(&self) -> String {
        let metrics = self.metrics.read().await;
        metrics.to_prometheus()
    }

    /// 更新指标
    #[allow(dead_code)]
    pub async fn update_metrics(&self, new_metrics: StorageMetrics) {
        let mut metrics = self.metrics.write().await;
        *metrics = new_metrics;

        let mut last_update = self.last_update.write().await;
        *last_update = chrono::Local::now().naive_local();
    }

    /// 获取健康状态
    pub async fn get_health_status(&self) -> HealthStatus {
        // TODO: 实现真实的健康检查逻辑
        // 目前返回健康状态
        HealthStatus::healthy()
    }
}

impl Default for StorageV2MetricsState {
    fn default() -> Self {
        Self::new()
    }
}

/// GET /metrics/storage-v2
/// 获取 Storage V2 的 Prometheus 指标
pub async fn get_storage_v2_metrics(req: Request) -> silent::Result<Response> {
    // 从 AppState 获取指标状态
    let app_state = req.extensions().get::<AppState>().cloned();

    if let Some(state) = app_state {
        let metrics_text = state.storage_v2_metrics.get_prometheus_format().await;

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
        );
        resp.set_body(full(metrics_text.into_bytes()));
        Ok(resp)
    } else {
        Err(SilentError::business_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Storage V2 metrics not available".to_string(),
        ))
    }
}

/// GET /metrics/storage-v2/health
/// 获取 Storage V2 的健康状态
pub async fn get_storage_v2_health(req: Request) -> silent::Result<Response> {
    // 从 AppState 获取指标状态
    let app_state = req.extensions().get::<AppState>().cloned();

    if let Some(state) = app_state {
        let health_status = state.storage_v2_metrics.get_health_status().await;

        let status_code = if health_status.healthy {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };

        let json_body = serde_json::to_string(&health_status).map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("JSON序列化失败: {}", e),
            )
        })?;

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        resp.set_status(status_code);
        resp.set_body(full(json_body.into_bytes()));
        Ok(resp)
    } else {
        Err(SilentError::business_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Storage V2 health check not available".to_string(),
        ))
    }
}

/// GET /metrics/storage-v2/json
/// 获取 Storage V2 的 JSON 格式指标（用于调试）
pub async fn get_storage_v2_metrics_json(req: Request) -> silent::Result<Response> {
    // 从 AppState 获取指标状态
    let app_state = req.extensions().get::<AppState>().cloned();

    if let Some(state) = app_state {
        let metrics = state.storage_v2_metrics.metrics.read().await;

        let json_body = serde_json::to_string_pretty(&*metrics).map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("JSON序列化失败: {}", e),
            )
        })?;

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        );
        resp.set_body(full(json_body.into_bytes()));
        Ok(resp)
    } else {
        Err(SilentError::business_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Storage V2 metrics not available".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_state_creation() {
        let state = StorageV2MetricsState::new();
        let metrics = state.metrics.read().await;
        assert_eq!(metrics.storage.total_files, 0);
    }

    #[tokio::test]
    async fn test_health_status() {
        let state = StorageV2MetricsState::new();
        let health = state.get_health_status().await;
        assert!(health.healthy);
        assert!(health.storage_available);
        assert!(health.database_available);
    }

    #[tokio::test]
    async fn test_update_metrics() {
        let state = StorageV2MetricsState::new();

        let mut new_metrics = StorageMetrics::new();
        new_metrics.storage.total_files = 100;
        new_metrics.storage.total_chunks = 1000;

        state.update_metrics(new_metrics).await;

        let metrics = state.metrics.read().await;
        assert_eq!(metrics.storage.total_files, 100);
        assert_eq!(metrics.storage.total_chunks, 1000);
    }

    #[tokio::test]
    async fn test_prometheus_format() {
        let state = StorageV2MetricsState::new();
        let prometheus = state.get_prometheus_format().await;

        assert!(prometheus.contains("storage_total_files"));
        assert!(prometheus.contains("storage_total_chunks"));
        assert!(prometheus.contains("TYPE"));
        assert!(prometheus.contains("HELP"));
    }
}
