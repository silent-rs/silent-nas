use crate::models::{EventType, FileEvent};
use crate::s3::service::S3Service;
use http::StatusCode;
use silent::prelude::*;
use tracing::debug;

#[allow(clippy::collapsible_if)]
impl S3Service {
    pub async fn put_object(&self, req: Request) -> silent::Result<Response> {
        // 检查key是否为空，如果为空说明这是bucket创建请求（被路由错误匹配到这里）
        // 这种情况发生在路径如 /test-bucket 时，<key:**> 通配符匹配了空路径
        let key: String = req.get_path_params("key")?;
        if key.is_empty() {
            debug!("Empty key detected in put_object, redirecting to put_bucket");
            return self.put_bucket(req).await;
        }

        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;

        debug!("PutObject: bucket={}, key={}", bucket, key);

        // 使用bucket/key组合作file_id
        let file_id = format!("{}/{}", bucket, key);

        // 检查条件请求头 - If-Match
        if let Some(if_match) = req.headers().get("If-Match") {
            if let Ok(header_value) = if_match.to_str() {
                if let Ok(existing_meta) = self.storage.get_metadata(&file_id).await {
                    let etag = format!("\"{}\"", existing_meta.hash);
                    if header_value != "*" && !header_value.split(',').any(|tag| tag.trim() == etag)
                    {
                        return self.error_response(
                            StatusCode::PRECONDITION_FAILED,
                            "PreconditionFailed",
                            "Precondition failed",
                        );
                    }
                } else if header_value != "*" {
                    return self.error_response(
                        StatusCode::PRECONDITION_FAILED,
                        "PreconditionFailed",
                        "Precondition failed",
                    );
                }
            }
        }

        // 检查条件请求头 - If-None-Match
        if let Some(if_none_match) = req.headers().get("If-None-Match") {
            if let Ok(header_value) = if_none_match.to_str() {
                if let Ok(existing_meta) = self.storage.get_metadata(&file_id).await {
                    let etag = format!("\"{}\"", existing_meta.hash);
                    if header_value == "*" || header_value.split(',').any(|tag| tag.trim() == etag)
                    {
                        return self.error_response(
                            StatusCode::PRECONDITION_FAILED,
                            "PreconditionFailed",
                            "Precondition failed",
                        );
                    }
                }
            }
        }

        // 读取请求体
        let body_bytes = Self::read_body(req).await?;

        // 保存文件
        let metadata = self
            .storage
            .save_file(&file_id, &body_bytes)
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("保存文件失败: {}", e),
                )
            })?;

        // 发送事件
        let mut event = FileEvent::new(EventType::Created, file_id.clone(), Some(metadata.clone()));
        event.source_http_addr = Some(self.source_http_addr.clone());
        if let Some(ref n) = self.notifier {
            let _ = n.notify_created(event).await;
        }

        // 返回响应
        let mut resp = Response::empty();
        resp.headers_mut().insert(
            "ETag",
            http::HeaderValue::from_str(&format!("\"{}\"", metadata.hash)).unwrap(),
        );
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-001"),
        );
        resp.set_status(StatusCode::OK);

        Ok(resp)
    }

    /// GetObject - 获取对象
    pub async fn get_object(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;
        let key: String = req.get_path_params("key")?;

        debug!("GetObject: bucket={}, key={}", bucket, key);

        let file_id = format!("{}/{}", bucket, key);

        // 先获取元数据以支持条件请求
        let metadata = self
            .storage
            .get_metadata(&file_id)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "NoSuchKey"))?;

        // 检查If-None-Match
        if let Some(if_none_match) = req.headers().get("If-None-Match") {
            if let Ok(header_value) = if_none_match.to_str() {
                let etag = format!("\"{}\"", metadata.hash);
                if header_value == "*" || header_value.split(',').any(|tag| tag.trim() == etag) {
                    let mut resp = Response::empty();
                    resp.headers_mut()
                        .insert("ETag", http::HeaderValue::from_str(&etag).unwrap());
                    resp.set_status(StatusCode::NOT_MODIFIED);
                    return Ok(resp);
                }
            }
        }

        // 检查If-Match
        if let Some(if_match) = req.headers().get("If-Match") {
            if let Ok(header_value) = if_match.to_str() {
                let etag = format!("\"{}\"", metadata.hash);
                if header_value != "*" && !header_value.split(',').any(|tag| tag.trim() == etag) {
                    return self.error_response(
                        StatusCode::PRECONDITION_FAILED,
                        "PreconditionFailed",
                        "Precondition failed",
                    );
                }
            }
        }

        // 检查If-Modified-Since
        if let Some(if_modified_since) = req.headers().get("If-Modified-Since") {
            if let Ok(header_value) = if_modified_since.to_str() {
                if let Ok(since_time) = chrono::DateTime::parse_from_rfc2822(header_value) {
                    let file_modified = metadata.modified_at.and_utc();
                    if file_modified <= since_time {
                        let mut resp = Response::empty();
                        resp.headers_mut().insert(
                            "Last-Modified",
                            http::HeaderValue::from_str(&file_modified.to_rfc2822()).unwrap(),
                        );
                        resp.set_status(StatusCode::NOT_MODIFIED);
                        return Ok(resp);
                    }
                }
            }
        }

        // 读取完整文件
        let data = self
            .storage
            .read_file(&file_id)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "NoSuchKey"))?;
        let file_size = data.len() as u64;

        // 检查Range请求
        let range_header = req.headers().get("range").and_then(|v| v.to_str().ok());

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("binary/octet-stream"),
        );

        // 添加ETag和Last-Modified
        resp.headers_mut().insert(
            "ETag",
            http::HeaderValue::from_str(&format!("\"{}\"", metadata.hash)).unwrap(),
        );
        resp.headers_mut().insert(
            "Last-Modified",
            http::HeaderValue::from_str(&metadata.modified_at.and_utc().to_rfc2822()).unwrap(),
        );

        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-002"),
        );
        resp.headers_mut()
            .insert("Accept-Ranges", http::HeaderValue::from_static("bytes"));

        // 添加用户元数据支持（示例）
        Self::add_user_metadata(&mut resp);

        // 处理Range请求
        if let Some(range_str) = range_header {
            if let Some((start, end)) = Self::parse_range(range_str, file_size) {
                let range_data = data[start..=end].to_vec();
                let range_len = range_data.len();

                resp.headers_mut().insert(
                    http::header::CONTENT_LENGTH,
                    http::HeaderValue::from_str(&range_len.to_string()).unwrap(),
                );
                resp.headers_mut().insert(
                    "Content-Range",
                    http::HeaderValue::from_str(&format!("bytes {}-{}/{}", start, end, file_size))
                        .unwrap(),
                );
                resp.set_body(full(range_data));
                resp.set_status(StatusCode::PARTIAL_CONTENT);

                debug!("Range request: {}-{}/{}", start, end, file_size);
            } else {
                // Range格式无效，返回416
                resp.headers_mut().insert(
                    "Content-Range",
                    http::HeaderValue::from_str(&format!("bytes */{}", file_size)).unwrap(),
                );
                resp.set_status(StatusCode::RANGE_NOT_SATISFIABLE);
                return Ok(resp);
            }
        } else {
            // 正常完整响应
            resp.headers_mut().insert(
                http::header::CONTENT_LENGTH,
                http::HeaderValue::from_str(&data.len().to_string()).unwrap(),
            );
            resp.set_body(full(data));
            resp.set_status(StatusCode::OK);
        }

        Ok(resp)
    }

    /// CopyObject - 复制对象
    pub async fn copy_object(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let dest_bucket: String = req.get_path_params("bucket")?;
        let dest_key: String = req.get_path_params("key")?;

        // 获取源对象路径 from x-amz-copy-source header
        let copy_source = req
            .headers()
            .get("x-amz-copy-source")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                SilentError::business_error(StatusCode::BAD_REQUEST, "缺少x-amz-copy-source头")
            })?;

        // 解析源路径 (格式: /source-bucket/source-key)
        let source_path = copy_source.trim_start_matches('/');
        let source_parts: Vec<&str> = source_path.splitn(2, '/').collect();

        if source_parts.len() != 2 {
            return self.error_response(
                StatusCode::BAD_REQUEST,
                "InvalidArgument",
                "Invalid copy source format",
            );
        }

        let source_file_id = format!("{}/{}", source_parts[0], source_parts[1]);
        let dest_file_id = format!("{}/{}", dest_bucket, dest_key);

        debug!("CopyObject: from {} to {}", source_file_id, dest_file_id);

        // 读取源文件
        let data = self
            .storage
            .read_file(&source_file_id)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "源对象不存在"))?;

        // 保存到目标位置
        let metadata = self
            .storage
            .save_file(&dest_file_id, &data)
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("复制失败: {}", e),
                )
            })?;

        // 发送事件
        let mut event = FileEvent::new(EventType::Created, dest_file_id, Some(metadata.clone()));
        event.source_http_addr = Some(self.source_http_addr.clone());
        if let Some(ref n) = self.notifier {
            let _ = n.notify_created(event).await;
        }

        // 生成CopyObjectResult XML响应
        let last_modified = metadata.modified_at.and_utc().to_rfc3339();
        let etag = format!("\"{}\"", metadata.hash);

        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <CopyObjectResult>\n\
               <LastModified>{}</LastModified>\n\
               <ETag>{}</ETag>\n\
             </CopyObjectResult>",
            last_modified, etag
        );

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/xml"),
        );
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-011"),
        );
        resp.set_body(full(xml.into_bytes()));
        resp.set_status(StatusCode::OK);

        Ok(resp)
    }

    /// DeleteObject - 删除对象
    pub async fn delete_object(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;
        let key: String = req.get_path_params("key")?;

        debug!("DeleteObject: bucket={}, key={}", bucket, key);

        let file_id = format!("{}/{}", bucket, key);

        // 删除文件
        let _ = self.storage.delete_file(&file_id).await;

        // 发送事件
        let mut event = FileEvent::new(EventType::Deleted, file_id, None);
        event.source_http_addr = Some(self.source_http_addr.clone());
        if let Some(ref n) = self.notifier {
            let _ = n.notify_deleted(event).await;
        }

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-003"),
        );
        resp.set_status(StatusCode::NO_CONTENT);

        Ok(resp)
    }
    pub async fn head_object(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;
        let key: String = req.get_path_params("key")?;

        debug!("HeadObject: bucket={}, key={}", bucket, key);

        let file_id = format!("{}/{}", bucket, key);

        // 获取元数据
        let metadata = self
            .storage
            .get_metadata(&file_id)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "NoSuchKey"))?;

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_str(&metadata.size.to_string()).unwrap(),
        );
        resp.headers_mut().insert(
            "ETag",
            http::HeaderValue::from_str(&format!("\"{}\"", metadata.hash)).unwrap(),
        );
        resp.headers_mut().insert(
            "Last-Modified",
            http::HeaderValue::from_str(
                &metadata
                    .modified_at
                    .and_utc()
                    .format("%a, %d %b %Y %H:%M:%S GMT")
                    .to_string(),
            )
            .unwrap(),
        );
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-004"),
        );

        // 添加用户元数据支持（示例）
        Self::add_user_metadata(&mut resp);

        resp.set_status(StatusCode::OK);

        Ok(resp)
    }
}
