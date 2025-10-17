//! Prometheus Metrics API 端点

use crate::metrics;
use http::StatusCode;
use silent::SilentError;
use silent::prelude::*;

/// Prometheus metrics 端点
pub async fn get_metrics(_req: Request) -> silent::Result<Response> {
    match metrics::export_metrics() {
        Ok(metrics_text) => {
            let mut resp = Response::empty();
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("text/plain; version=0.0.4"),
            );
            resp.set_body(full(metrics_text.into_bytes()));
            Ok(resp)
        }
        Err(e) => Err(SilentError::business_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("导出metrics失败: {}", e),
        )),
    }
}
