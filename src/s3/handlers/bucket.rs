use crate::s3::service::S3Service;
use http::StatusCode;
use silent::prelude::*;
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

        // 生成XML响应（默认未启用版本控制）
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                   <VersioningConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"/>";

        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/xml"),
        );
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_static("silent-nas-014"),
        );
        resp.set_body(full(xml.to_string().into_bytes()));
        resp.set_status(StatusCode::OK);

        Ok(resp)
    }
}
