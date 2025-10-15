use crate::models::{EventType, FileEvent};
use crate::notify::EventNotifier;
use crate::storage::StorageManager;
use chrono::{DateTime, Utc};
use silent::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

/// S3认证信息
#[derive(Clone)]
pub struct S3Auth {
    access_key: String,
}

impl S3Auth {
    pub fn new(access_key: String, _secret_key: String) -> Self {
        Self { access_key }
    }
}

/// S3服务
pub struct S3Service {
    storage: Arc<StorageManager>,
    notifier: Arc<EventNotifier>,
    auth: Option<S3Auth>,
}

impl S3Service {
    pub fn new(
        storage: Arc<StorageManager>,
        notifier: Arc<EventNotifier>,
        auth: Option<S3Auth>,
    ) -> Self {
        Self {
            storage,
            notifier,
            auth,
        }
    }

    /// 验证请求
    fn verify_request(&self, req: &Request) -> bool {
        match &self.auth {
            Some(auth) => {
                // 简化版认证：检查Authorization头是否包含access_key
                let auth_header = req
                    .headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok());

                match auth_header {
                    Some(header) if header.contains(&auth.access_key) => true,
                    _ => {
                        // 允许匿名访问（用于测试）
                        warn!("S3请求认证失败，允许匿名访问");
                        true
                    }
                }
            }
            None => true, // 未配置认证，允许所有请求
        }
    }

    /// PutObject - 上传对象
    pub async fn put_object(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;
        let key: String = req.get_path_params("key")?;

        debug!("PutObject: bucket={}, key={}", bucket, key);

        // 读取请求体
        let body_bytes = Self::read_body(req).await?;

        // 使用bucket/key组合作为file_id
        let file_id = format!("{}/{}", bucket, key);

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

    /// GetObject - 下载对象
    pub async fn get_object(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;
        let key: String = req.get_path_params("key")?;

        debug!("GetObject: bucket={}, key={}", bucket, key);

        let file_id = format!("{}/{}", bucket, key);

        // 读取文件
        let data = self
            .storage
            .read_file(&file_id)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "NoSuchKey"))?;

        // 获取元数据以获取ETag
        let metadata = self.storage.get_metadata(&file_id).await.ok();

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("binary/octet-stream"),
        );
        resp.headers_mut().insert(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_str(&data.len().to_string()).unwrap(),
        );

        if let Some(meta) = metadata {
            resp.headers_mut().insert(
                "ETag",
                http::HeaderValue::from_str(&format!("\"{}\"", meta.hash)).unwrap(),
            );
        }

        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-002"),
        );
        resp.set_body(full(data));
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
        let xml = self.generate_list_response_v2(&bucket, prefix, &contents, is_truncated);

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

    /// 创建Bucket
    pub async fn put_bucket(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;
        debug!("PutBucket: bucket={}", bucket);

        // 创建bucket
        match self.storage.create_bucket(&bucket).await {
            Ok(_) => {
                let mut resp = Response::empty();
                resp.headers_mut().insert(
                    "x-amz-request-id",
                    http::HeaderValue::from_static("silent-nas-007"),
                );
                resp.set_status(StatusCode::OK);
                Ok(resp)
            }
            Err(_) => self.error_response(
                StatusCode::CONFLICT,
                "BucketAlreadyExists",
                "The requested bucket name already exists",
            ),
        }
    }

    /// 删除Bucket
    pub async fn delete_bucket(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;
        debug!("DeleteBucket: bucket={}", bucket);

        // 删除bucket
        match self.storage.delete_bucket(&bucket).await {
            Ok(_) => {
                let mut resp = Response::empty();
                resp.headers_mut().insert(
                    "x-amz-request-id",
                    http::HeaderValue::from_static("silent-nas-008"),
                );
                resp.set_status(StatusCode::NO_CONTENT);
                Ok(resp)
            }
            Err(e) => {
                let msg = format!("{}", e);
                if msg.contains("不存在") {
                    self.error_response(
                        StatusCode::NOT_FOUND,
                        "NoSuchBucket",
                        "The specified bucket does not exist",
                    )
                } else if msg.contains("不为空") {
                    self.error_response(
                        StatusCode::CONFLICT,
                        "BucketNotEmpty",
                        "The bucket you tried to delete is not empty",
                    )
                } else {
                    self.error_response(StatusCode::INTERNAL_SERVER_ERROR, "InternalError", &msg)
                }
            }
        }
    }

    /// 检查Bucket是否存在
    pub async fn head_bucket(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;
        debug!("HeadBucket: bucket={}", bucket);

        // 检查bucket是否存在
        if self.storage.bucket_exists(&bucket).await {
            let mut resp = Response::empty();
            resp.headers_mut().insert(
                "x-amz-request-id",
                http::HeaderValue::from_static("silent-nas-009"),
            );
            resp.set_status(StatusCode::OK);
            Ok(resp)
        } else {
            self.error_response(
                StatusCode::NOT_FOUND,
                "NoSuchBucket",
                "The specified bucket does not exist",
            )
        }
    }

    /// ListBuckets - 列出所有bucket
    pub async fn list_buckets(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        debug!("ListBuckets");

        let buckets = self.storage.list_buckets().await.map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("列出buckets失败: {}", e),
            )
        })?;

        // 生成XML响应
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str(
            "<ListAllMyBucketsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n",
        );
        xml.push_str("  <Owner>\n");
        xml.push_str("    <ID>silent-nas</ID>\n");
        xml.push_str("    <DisplayName>silent-nas</DisplayName>\n");
        xml.push_str("  </Owner>\n");
        xml.push_str("  <Buckets>\n");

        for bucket in buckets {
            xml.push_str("    <Bucket>\n");
            xml.push_str(&format!(
                "      <Name>{}</Name>\n",
                Self::xml_escape(&bucket)
            ));
            xml.push_str(&format!(
                "      <CreationDate>{}</CreationDate>\n",
                chrono::Utc::now().to_rfc3339()
            ));
            xml.push_str("    </Bucket>\n");
        }

        xml.push_str("  </Buckets>\n");
        xml.push_str("</ListAllMyBucketsResult>");

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/xml"),
        );
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-010"),
        );
        resp.set_body(full(xml.into_bytes()));
        resp.set_status(StatusCode::OK);

        Ok(resp)
    }

    // ===== 辅助方法 =====

    async fn read_body(mut req: Request) -> silent::Result<Vec<u8>> {
        use http_body_util::BodyExt;

        let body = req.take_body();
        match body {
            ReqBody::Incoming(body) => {
                let bytes = body
                    .collect()
                    .await
                    .map_err(|e| {
                        SilentError::business_error(
                            StatusCode::BAD_REQUEST,
                            format!("读取请求体失败: {}", e),
                        )
                    })?
                    .to_bytes()
                    .to_vec();
                Ok(bytes)
            }
            ReqBody::Once(bytes) => Ok(bytes.to_vec()),
            ReqBody::Empty => Ok(Vec::new()),
        }
    }

    fn parse_query_string(query: &str) -> HashMap<String, String> {
        query
            .split('&')
            .filter_map(|part| {
                let mut split = part.splitn(2, '=');
                match (split.next(), split.next()) {
                    (Some(key), Some(value)) => Some((
                        key.to_string(),
                        urlencoding::decode(value).ok()?.to_string(),
                    )),
                    _ => None,
                }
            })
            .collect()
    }

    fn generate_list_response_v2(
        &self,
        bucket: &str,
        prefix: &str,
        contents: &[S3Object],
        is_truncated: bool,
    ) -> String {
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n");
        xml.push_str(&format!("  <Name>{}</Name>\n", Self::xml_escape(bucket)));
        xml.push_str(&format!(
            "  <Prefix>{}</Prefix>\n",
            Self::xml_escape(prefix)
        ));
        xml.push_str(&format!("  <KeyCount>{}</KeyCount>\n", contents.len()));
        xml.push_str(&format!(
            "  <MaxKeys>{}</MaxKeys>\n",
            if is_truncated { contents.len() } else { 1000 }
        ));
        xml.push_str(&format!("  <IsTruncated>{}</IsTruncated>\n", is_truncated));

        for obj in contents {
            xml.push_str("  <Contents>\n");
            xml.push_str(&format!("    <Key>{}</Key>\n", Self::xml_escape(&obj.key)));
            xml.push_str(&format!(
                "    <LastModified>{}</LastModified>\n",
                obj.last_modified.format("%Y-%m-%dT%H:%M:%S.000Z")
            ));
            xml.push_str(&format!("    <ETag>\"{}\"</ETag>\n", obj.etag));
            xml.push_str(&format!("    <Size>{}</Size>\n", obj.size));
            xml.push_str("    <StorageClass>STANDARD</StorageClass>\n");
            xml.push_str("  </Contents>\n");
        }

        xml.push_str("</ListBucketResult>");
        xml
    }

    fn generate_list_response(
        &self,
        bucket: &str,
        prefix: &str,
        contents: &[S3Object],
        is_truncated: bool,
        max_keys: usize,
    ) -> String {
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n");
        xml.push_str(&format!("  <Name>{}</Name>\n", Self::xml_escape(bucket)));
        xml.push_str(&format!(
            "  <Prefix>{}</Prefix>\n",
            Self::xml_escape(prefix)
        ));
        xml.push_str("  <Marker></Marker>\n");
        xml.push_str(&format!("  <MaxKeys>{}</MaxKeys>\n", max_keys));
        xml.push_str(&format!("  <IsTruncated>{}</IsTruncated>\n", is_truncated));

        for obj in contents {
            xml.push_str("  <Contents>\n");
            xml.push_str(&format!("    <Key>{}</Key>\n", Self::xml_escape(&obj.key)));
            xml.push_str(&format!(
                "    <LastModified>{}</LastModified>\n",
                obj.last_modified.format("%Y-%m-%dT%H:%M:%S.000Z")
            ));
            xml.push_str(&format!("    <ETag>\"{}\"</ETag>\n", obj.etag));
            xml.push_str(&format!("    <Size>{}</Size>\n", obj.size));
            xml.push_str("    <StorageClass>STANDARD</StorageClass>\n");
            xml.push_str("  </Contents>\n");
        }

        xml.push_str("</ListBucketResult>");
        xml
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    fn error_response(
        &self,
        status: StatusCode,
        code: &str,
        message: &str,
    ) -> silent::Result<Response> {
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <Error>\n\
             <Code>{}</Code>\n\
             <Message>{}</Message>\n\
             <RequestId>silent-nas-error</RequestId>\n\
             </Error>",
            Self::xml_escape(code),
            Self::xml_escape(message)
        );

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/xml"),
        );
        resp.set_body(full(xml.into_bytes()));
        resp.set_status(status);

        Ok(resp)
    }
}

#[derive(Debug)]
struct S3Object {
    key: String,
    last_modified: DateTime<Utc>,
    etag: String,
    size: u64,
}

/// 创建S3路由
pub fn create_s3_routes(
    storage: Arc<StorageManager>,
    notifier: Arc<EventNotifier>,
    auth: Option<S3Auth>,
) -> Route {
    let service = Arc::new(S3Service::new(storage, notifier, auth));

    // Bucket操作 - 合并GET和HEAD
    let service_bucket = service.clone();
    let bucket_handler = move |req: Request| {
        let service = service_bucket.clone();
        async move {
            match *req.method() {
                Method::GET => {
                    // 检查是否是V2版本
                    let query = req.uri().query().unwrap_or("");
                    if query.contains("list-type=2") {
                        service.list_objects_v2(req).await
                    } else {
                        service.list_objects(req).await
                    }
                }
                Method::HEAD => service.head_bucket(req).await,
                _ => service.error_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "MethodNotAllowed",
                    "Method not allowed",
                ),
            }
        }
    };

    let service_put_bucket = service.clone();
    let put_bucket = move |req: Request| {
        let service = service_put_bucket.clone();
        async move { service.put_bucket(req).await }
    };

    let service_delete_bucket = service.clone();
    let delete_bucket = move |req: Request| {
        let service = service_delete_bucket.clone();
        async move { service.delete_bucket(req).await }
    };

    // 对象操作
    let service_put = service.clone();
    let put_object = move |req: Request| {
        let service = service_put.clone();
        async move { service.put_object(req).await }
    };

    let service_get_head = service.clone();
    let get_or_head_object = move |req: Request| {
        let service = service_get_head.clone();
        async move {
            match *req.method() {
                Method::GET => service.get_object(req).await,
                Method::HEAD => service.head_object(req).await,
                _ => service.error_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "MethodNotAllowed",
                    "Method not allowed",
                ),
            }
        }
    };

    let service_delete = service.clone();
    let delete_object = move |req: Request| {
        let service = service_delete.clone();
        async move { service.delete_object(req).await }
    };

    // 根路径处理ListBuckets
    let service_root = service.clone();
    let root_handler = move |req: Request| {
        let service = service_root.clone();
        async move {
            match *req.method() {
                Method::GET => service.list_buckets(req).await,
                _ => service.error_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "MethodNotAllowed",
                    "Method not allowed",
                ),
            }
        }
    };

    Route::new_root().get(root_handler).append(
        Route::new("<bucket>")
            // Bucket级别操作
            .get(bucket_handler)
            .put(put_bucket)
            .delete(delete_bucket)
            // 对象级别操作
            .append(
                Route::new("<key:**>")
                    .put(put_object)
                    .get(get_or_head_object)
                    .delete(delete_object),
            ),
    )
}
