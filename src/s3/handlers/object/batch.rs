use crate::models::{EventType, FileEvent};
use crate::s3::service::S3Service;
use http::StatusCode;
use silent::prelude::*;
use silent_nas_core::StorageManager as StorageManagerTrait;
use tracing::debug;

impl S3Service {
    /// DeleteObjects - 批量删除对象
    pub async fn delete_objects(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;

        debug!("DeleteObjects: bucket={}", bucket);

        // 读取请求体XML
        let body_bytes = Self::read_body(req).await?;
        let body_str = String::from_utf8_lossy(&body_bytes);

        // 解析XML获取要删除的对象列表
        let keys = Self::parse_delete_objects_xml(&body_str);

        let mut deleted = Vec::new();
        let mut errors = Vec::new();

        // 批量删除对象
        for key in keys {
            let file_id = format!("{}/{}", bucket, key);
            match self.storage.delete_file(&file_id).await {
                Ok(_) => {
                    // 发送删除事件
                    let mut event = FileEvent::new(EventType::Deleted, file_id.clone(), None);
                    event.source_http_addr = Some(self.source_http_addr.clone());
                    if let Some(ref n) = self.notifier {
                        let _ = n.notify_deleted(event).await;
                    }
                    deleted.push(key);
                }
                Err(e) => {
                    debug!("删除失败: {} - {}", key, e);
                    errors.push((key, "InternalError", e.to_string()));
                }
            }
        }

        // 生成XML响应
        let xml = Self::generate_delete_result_xml(&deleted, &errors);

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/xml"),
        );
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-012"),
        );
        resp.set_body(full(xml.into_bytes()));
        resp.set_status(StatusCode::OK);

        Ok(resp)
    }
}
