use crate::s3::models::{MultipartUpload, PartInfo};
use crate::s3::service::S3Service;
use chrono::Utc;
use http::StatusCode;
use sha2::{Digest, Sha256};
use silent::prelude::*;
use std::collections::HashMap;
use tracing::debug;

impl S3Service {
    /// InitiateMultipartUpload - 初始化分片上传
    pub async fn initiate_multipart_upload(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;
        let key: String = req.get_path_params("key")?;

        debug!("InitiateMultipartUpload: bucket={}, key={}", bucket, key);

        // 生成upload ID
        let upload_id = scru128::new_string().to_string();

        // 创建multipart upload记录
        let upload = MultipartUpload {
            upload_id: upload_id.clone(),
            bucket: bucket.clone(),
            key: key.clone(),
            initiated: Utc::now(),
            parts: HashMap::new(),
        };

        // 保存到内存中
        {
            let mut uploads = self.multipart_uploads.write().unwrap();
            uploads.insert(upload_id.clone(), upload);
        }

        // 返回XML响应
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <InitiateMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n\
               <Bucket>{}</Bucket>\n\
               <Key>{}</Key>\n\
               <UploadId>{}</UploadId>\n\
             </InitiateMultipartUploadResult>",
            bucket, key, upload_id
        );

        let mut resp = Response::empty();
        resp.set_body(full(xml));
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/xml"),
        );
        resp.set_status(StatusCode::OK);

        Ok(resp)
    }

    /// UploadPart - 上传分片
    pub async fn upload_part(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;
        let key: String = req.get_path_params("key")?;

        // 从查询参数获取partNumber和uploadId
        let query = req.uri().query().unwrap_or("");
        let params: HashMap<String, String> = query
            .split('&')
            .filter_map(|s| {
                let mut parts = s.split('=');
                Some((parts.next()?.to_string(), parts.next()?.to_string()))
            })
            .collect();

        let part_number: u32 = params
            .get("partNumber")
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                SilentError::business_error(StatusCode::BAD_REQUEST, "Missing partNumber")
            })?;

        let upload_id = params.get("uploadId").ok_or_else(|| {
            SilentError::business_error(StatusCode::BAD_REQUEST, "Missing uploadId")
        })?;

        debug!(
            "UploadPart: bucket={}, key={}, partNumber={}, uploadId={}",
            bucket, key, part_number, upload_id
        );

        // 读取分片数据
        let body_bytes = Self::read_body(req).await?;

        // 计算ETag（使用SHA256）
        let mut hasher = Sha256::new();
        hasher.update(&body_bytes);
        let etag = format!("{:x}", hasher.finalize());

        // 保存分片信息
        {
            let mut uploads = self.multipart_uploads.write().unwrap();
            let upload = uploads.get_mut(upload_id).ok_or_else(|| {
                SilentError::business_error(StatusCode::NOT_FOUND, "NoSuchUpload")
            })?;

            let part_info = PartInfo {
                part_number,
                etag: etag.clone(),
                size: body_bytes.len() as u64,
                data: body_bytes,
            };

            upload.parts.insert(part_number, part_info);
        }

        // 返回响应
        let mut resp = Response::empty();
        resp.headers_mut().insert(
            "ETag",
            http::HeaderValue::from_str(&format!("\"{}\"", etag)).unwrap(),
        );
        resp.set_status(StatusCode::OK);

        Ok(resp)
    }

    /// CompleteMultipartUpload - 完成分片上传
    pub async fn complete_multipart_upload(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;
        let key: String = req.get_path_params("key")?;

        // 从查询参数获取uploadId
        let query = req.uri().query().unwrap_or("");
        let params: HashMap<String, String> = query
            .split('&')
            .filter_map(|s| {
                let mut parts = s.split('=');
                Some((parts.next()?.to_string(), parts.next()?.to_string()))
            })
            .collect();

        let upload_id = params.get("uploadId").ok_or_else(|| {
            SilentError::business_error(StatusCode::BAD_REQUEST, "Missing uploadId")
        })?;

        debug!(
            "CompleteMultipartUpload: bucket={}, key={}, uploadId={}",
            bucket, key, upload_id
        );

        // 读取请求体（包含part列表）
        let _body = Self::read_body(req).await?;
        // TODO: 解析XML获取part列表，这里简化处理，使用所有已上传的part

        // 获取并合并所有分片
        let (merged_data, final_etag) = {
            let mut uploads = self.multipart_uploads.write().unwrap();
            let upload = uploads.remove(upload_id).ok_or_else(|| {
                SilentError::business_error(StatusCode::NOT_FOUND, "NoSuchUpload")
            })?;

            // 按part_number排序
            let mut parts: Vec<_> = upload.parts.values().collect();
            parts.sort_by_key(|p| p.part_number);

            // 合并数据
            let mut merged = Vec::new();
            for part in parts {
                merged.extend_from_slice(&part.data);
            }

            // 计算最终ETag
            let mut hasher = Sha256::new();
            hasher.update(&merged);
            let etag = format!("{:x}", hasher.finalize());

            (merged, etag)
        };

        // 保存完整文件
        let file_id = format!("{}/{}", bucket, key);
        let metadata = self
            .storage
            .save_file(&file_id, &merged_data)
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("保存文件失败: {}", e),
                )
            })?;

        // 发送事件
        let event = crate::models::FileEvent::new(
            crate::models::EventType::Created,
            file_id.clone(),
            Some(metadata.clone()),
        );
        let _ = self.notifier.notify_created(event).await;

        // 返回XML响应
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <CompleteMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n\
               <Location>http://{}/{}/{}</Location>\n\
               <Bucket>{}</Bucket>\n\
               <Key>{}</Key>\n\
               <ETag>\"{}\"</ETag>\n\
             </CompleteMultipartUploadResult>",
            "127.0.0.1:9000", bucket, key, bucket, key, final_etag
        );

        let mut resp = Response::empty();
        resp.set_body(full(xml));
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/xml"),
        );
        resp.set_status(StatusCode::OK);

        Ok(resp)
    }

    /// AbortMultipartUpload - 中止分片上传
    pub async fn abort_multipart_upload(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;
        let key: String = req.get_path_params("key")?;

        // 从查询参数获取uploadId
        let query = req.uri().query().unwrap_or("");
        let params: HashMap<String, String> = query
            .split('&')
            .filter_map(|s| {
                let mut parts = s.split('=');
                Some((parts.next()?.to_string(), parts.next()?.to_string()))
            })
            .collect();

        let upload_id = params.get("uploadId").ok_or_else(|| {
            SilentError::business_error(StatusCode::BAD_REQUEST, "Missing uploadId")
        })?;

        debug!(
            "AbortMultipartUpload: bucket={}, key={}, uploadId={}",
            bucket, key, upload_id
        );

        // 删除上传记录
        {
            let mut uploads = self.multipart_uploads.write().unwrap();
            uploads.remove(upload_id).ok_or_else(|| {
                SilentError::business_error(StatusCode::NOT_FOUND, "NoSuchUpload")
            })?;
        }

        // 返回204 No Content
        let mut resp = Response::empty();
        resp.set_status(StatusCode::NO_CONTENT);

        Ok(resp)
    }
}
