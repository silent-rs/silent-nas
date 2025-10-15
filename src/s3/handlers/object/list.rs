use crate::s3::models::S3Object;
use crate::s3::service::S3Service;
use http::StatusCode;
use silent::prelude::*;
use tracing::debug;

impl S3Service {
    pub async fn list_objects_v2(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;

        // 解析查询参数
        let query_params = Self::parse_query_string(req.uri().query().unwrap_or(""));
        let prefix = query_params.get("prefix").map(|s| s.as_str()).unwrap_or("");
        let max_keys = query_params
            .get("max-keys")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1000);

        debug!(
            "ListObjectsV2: bucket={}, prefix={}, max_keys={}",
            bucket, prefix, max_keys
        );

        // 检查bucket是否存在
        if !self.storage.bucket_exists(&bucket).await {
            return self.error_response(
                StatusCode::NOT_FOUND,
                "NoSuchBucket",
                "The specified bucket does not exist",
            );
        }

        // 使用新的list_bucket_objects API
        let object_keys = self
            .storage
            .list_bucket_objects(&bucket, prefix)
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("列出对象失败: {}", e),
                )
            })?;

        // 构建对象列表
        let mut contents = Vec::new();
        for key in object_keys.iter().take(max_keys) {
            let file_id = format!("{}/{}", bucket, key);
            if let Ok(metadata) = self.storage.get_metadata(&file_id).await {
                contents.push(S3Object {
                    key: key.clone(),
                    last_modified: metadata.modified_at.and_utc(),
                    etag: metadata.hash,
                    size: metadata.size,
                });
            }
        }

        let is_truncated = contents.len() >= max_keys;

        // 生成XML响应
        let xml = self.generate_list_v2_response(&bucket, prefix, &contents, is_truncated);

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/xml"),
        );
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-005"),
        );
        resp.set_body(full(xml.into_bytes()));
        resp.set_status(StatusCode::OK);

        Ok(resp)
    }

    /// ListObjects - 列出对象（V1版本）
    pub async fn list_objects(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;

        let query_params = Self::parse_query_string(req.uri().query().unwrap_or(""));
        let prefix = query_params.get("prefix").map(|s| s.as_str()).unwrap_or("");
        let max_keys = query_params
            .get("max-keys")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1000);

        debug!(
            "ListObjects: bucket={}, prefix={}, max_keys={}",
            bucket, prefix, max_keys
        );

        // 检查bucket是否存在
        if !self.storage.bucket_exists(&bucket).await {
            return self.error_response(
                StatusCode::NOT_FOUND,
                "NoSuchBucket",
                "The specified bucket does not exist",
            );
        }

        // 使用新的list_bucket_objects API
        let object_keys = self
            .storage
            .list_bucket_objects(&bucket, prefix)
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("列出对象失败: {}", e),
                )
            })?;

        // 构建对象列表
        let mut contents = Vec::new();
        for key in object_keys.iter().take(max_keys) {
            let file_id = format!("{}/{}", bucket, key);
            if let Ok(metadata) = self.storage.get_metadata(&file_id).await {
                contents.push(S3Object {
                    key: key.clone(),
                    last_modified: metadata.modified_at.and_utc(),
                    etag: metadata.hash,
                    size: metadata.size,
                });
            }
        }

        let is_truncated = contents.len() >= max_keys;

        let xml = self.generate_list_response(&bucket, prefix, &contents, is_truncated, max_keys);

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/xml"),
        );
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-006"),
        );
        resp.set_body(full(xml.into_bytes()));
        resp.set_status(StatusCode::OK);

        Ok(resp)
    }
}
