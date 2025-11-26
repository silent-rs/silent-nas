use crate::s3::service::S3Service;
use http::StatusCode;
use silent::prelude::*;
use silent_nas_core::S3CompatibleStorageTrait;
use tracing::debug;

impl S3Service {
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

    /// GetBucketLocation - 获取bucket位置
    pub async fn get_bucket_location(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;

        debug!("GetBucketLocation: bucket={}", bucket);

        // 检查bucket是否存在
        if !self.storage.bucket_exists(&bucket).await {
            return self.error_response(
                StatusCode::NOT_FOUND,
                "NoSuchBucket",
                "The specified bucket does not exist",
            );
        }

        // 生成XML响应（默认返回us-east-1）
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                   <LocationConstraint xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">us-east-1</LocationConstraint>";

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/xml"),
        );
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-013"),
        );
        resp.set_body(full(xml.to_string().into_bytes()));
        resp.set_status(StatusCode::OK);

        Ok(resp)
    }

    /// GetBucketVersioning - 获取bucket版本控制状态
    pub async fn get_bucket_versioning(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;

        debug!("GetBucketVersioning: bucket={}", bucket);

        // 检查bucket是否存在
        if !self.storage.bucket_exists(&bucket).await {
            return self.error_response(
                StatusCode::NOT_FOUND,
                "NoSuchBucket",
                "The specified bucket does not exist",
            );
        }

        // 获取版本控制配置
        let versioning = self.versioning_manager.get_versioning(&bucket).await;
        let status = versioning.status.to_string();

        // 生成XML响应
        let xml = if status.is_empty() {
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <VersioningConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"/>"
                .to_string()
        } else {
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <VersioningConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n\
                   <Status>{}</Status>\n\
                 </VersioningConfiguration>",
                status
            )
        };

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/xml"),
        );
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-014"),
        );
        resp.set_body(full(xml.into_bytes()));
        resp.set_status(StatusCode::OK);

        Ok(resp)
    }

    /// PutBucketVersioning - 设置bucket版本控制状态
    pub async fn put_bucket_versioning(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;

        debug!("PutBucketVersioning: bucket={}", bucket);

        // 检查bucket是否存在
        if !self.storage.bucket_exists(&bucket).await {
            return self.error_response(
                StatusCode::NOT_FOUND,
                "NoSuchBucket",
                "The specified bucket does not exist",
            );
        }

        // 读取请求体
        let body = Self::read_body(req).await?;
        let body_str = String::from_utf8(body)
            .map_err(|_| SilentError::business_error(StatusCode::BAD_REQUEST, "请求体格式错误"))?;

        debug!("PutBucketVersioning body: {}", body_str);

        // 解析XML获取Status
        use crate::s3::versioning::VersioningStatus;
        let status = if body_str.contains("<Status>Enabled</Status>") {
            VersioningStatus::Enabled
        } else if body_str.contains("<Status>Suspended</Status>") {
            VersioningStatus::Suspended
        } else {
            return self.error_response(
                StatusCode::BAD_REQUEST,
                "MalformedXML",
                "Invalid versioning status",
            );
        };

        // 设置版本控制状态
        self.versioning_manager
            .set_versioning(&bucket, status)
            .await;

        debug!("Bucket versioning updated: {}", bucket);

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-015"),
        );
        resp.set_status(StatusCode::OK);

        Ok(resp)
    }
}
