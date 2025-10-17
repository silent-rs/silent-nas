use crate::notify::EventNotifier;
use crate::s3::auth::S3Auth;
use crate::s3::models::MultipartUpload;
use crate::s3::versioning::VersioningManager;
use crate::storage::StorageManager;
use crate::version::VersionManager;
use silent::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// S3服务
pub struct S3Service {
    pub(crate) storage: Arc<StorageManager>,
    pub(crate) notifier: Arc<EventNotifier>,
    pub(crate) auth: Option<S3Auth>,
    pub(crate) multipart_uploads: Arc<RwLock<HashMap<String, MultipartUpload>>>,
    pub(crate) source_http_addr: String,
    pub(crate) versioning_manager: Arc<VersioningManager>,
    pub(crate) version_manager: Arc<VersionManager>,
}

impl S3Service {
    pub fn new(
        storage: Arc<StorageManager>,
        notifier: Arc<EventNotifier>,
        auth: Option<S3Auth>,
        source_http_addr: String,
        versioning_manager: Arc<VersioningManager>,
        version_manager: Arc<VersionManager>,
    ) -> Self {
        Self {
            storage,
            notifier,
            auth,
            multipart_uploads: Arc::new(RwLock::new(HashMap::new())),
            source_http_addr,
            versioning_manager,
            version_manager,
        }
    }

    /// 验证请求
    pub(crate) fn verify_request(&self, req: &Request) -> bool {
        match &self.auth {
            Some(auth) => auth.verify_request(req),
            None => true, // 未配置认证，允许所有请求
        }
    }

    /// 读取请求体
    pub(crate) async fn read_body(mut req: Request) -> silent::Result<Vec<u8>> {
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

    /// 解析查询字符串
    pub(crate) fn parse_query_string(query: &str) -> HashMap<String, String> {
        query
            .split('&')
            .filter_map(|pair| {
                let parts: Vec<&str> = pair.split('=').collect();
                if parts.len() == 2 {
                    Some((parts[0].to_string(), parts[1].to_string()))
                } else {
                    None
                }
            })
            .collect()
    }

    /// 解析Range头，返回(start, end)，都是包含的
    pub(crate) fn parse_range(range_str: &str, file_size: u64) -> Option<(usize, usize)> {
        // 格式: "bytes=start-end" 或 "bytes=start-" 或 "bytes=-count"
        let range_str = range_str.trim();
        if !range_str.starts_with("bytes=") {
            return None;
        }

        let range = range_str.strip_prefix("bytes=")?;
        let parts: Vec<&str> = range.split('-').collect();

        if parts.len() != 2 {
            return None;
        }

        match (parts[0].trim(), parts[1].trim()) {
            ("", end_str) => {
                // bytes=-count: 最后count字节
                let count: u64 = end_str.parse().ok()?;
                let start = file_size.saturating_sub(count);
                Some((start as usize, (file_size - 1) as usize))
            }
            (start_str, "") => {
                // bytes=start-: 从start到结束
                let start: u64 = start_str.parse().ok()?;
                if start >= file_size {
                    return None;
                }
                Some((start as usize, (file_size - 1) as usize))
            }
            (start_str, end_str) => {
                // bytes=start-end: 指定范围
                let start: u64 = start_str.parse().ok()?;
                let mut end: u64 = end_str.parse().ok()?;

                if start >= file_size {
                    return None;
                }

                // end不能超过文件大小
                if end >= file_size {
                    end = file_size - 1;
                }

                if start > end {
                    return None;
                }

                Some((start as usize, end as usize))
            }
        }
    }

    /// 添加用户自定义元数据（示例实现）
    pub(crate) fn add_user_metadata(resp: &mut Response) {
        // 注：实际应用中应该从持久化存储读取
        // 这里仅为演示S3协议兼容性
        resp.headers_mut().insert(
            "x-amz-meta-author",
            http::HeaderValue::from_static("silent-nas"),
        );
        resp.headers_mut()
            .insert("x-amz-meta-version", http::HeaderValue::from_static("1.0"));
    }

    /// XML转义
    pub(crate) fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    /// 错误响应
    pub(crate) fn error_response(
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
