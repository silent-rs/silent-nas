use crate::models::{EventType, FileEvent};
use crate::s3::models::S3Object;
use crate::s3::service::S3Service;
use http::StatusCode;
use silent::prelude::*;
use tracing::debug;

#[allow(clippy::collapsible_if)]
impl S3Service {
    /// PutObject - 上传对象
    pub async fn put_object(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;
        let key: String = req.get_path_params("key")?;

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
        let event = FileEvent::new(EventType::Created, file_id.clone(), Some(metadata.clone()));
        let _ = self.notifier.notify_created(event).await;

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
        let event = FileEvent::new(EventType::Created, dest_file_id, Some(metadata.clone()));
        let _ = self.notifier.notify_created(event).await;

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
        let event = FileEvent::new(EventType::Deleted, file_id, None);
        let _ = self.notifier.notify_deleted(event).await;

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-003"),
        );
        resp.set_status(StatusCode::NO_CONTENT);

        Ok(resp)
    }

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
                    let event = FileEvent::new(EventType::Deleted, file_id.clone(), None);
                    let _ = self.notifier.notify_deleted(event).await;
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

    /// HeadObject - 获取对象元数据
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

    /// ListObjectsV2 - 列出对象
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
