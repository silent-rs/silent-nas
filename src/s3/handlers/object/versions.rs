// S3 对象版本管理 API
use crate::s3::service::S3Service;
use http::StatusCode;
use silent::prelude::*;
use tracing::debug;

impl S3Service {
    /// ListObjectVersions - 列出对象的所有版本
    pub async fn list_object_versions(&self, req: Request) -> silent::Result<Response> {
        if !self.verify_request(&req) {
            return self.error_response(StatusCode::FORBIDDEN, "AccessDenied", "Access Denied");
        }

        let bucket: String = req.get_path_params("bucket")?;

        debug!("ListObjectVersions: bucket={}", bucket);

        // 检查bucket是否存在
        if !self.storage.bucket_exists(&bucket).await {
            return self.error_response(
                StatusCode::NOT_FOUND,
                "NoSuchBucket",
                "The specified bucket does not exist",
            );
        }

        // 检查bucket是否启用了版本控制
        if !self.versioning_manager.is_versioning_enabled(&bucket).await {
            // 如果未启用版本控制，返回简单的空列表
            let xml = self.build_empty_versions_response(&bucket);
            return self.send_xml_response(xml, "silent-nas-016");
        }

        // 解析查询参数
        let query = req.uri().query().unwrap_or("");
        let params = Self::parse_query_string(query);

        let prefix = params.get("prefix").map(|s| s.as_str()).unwrap_or("");
        let max_keys = params
            .get("max-keys")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1000);

        debug!(
            "ListObjectVersions params: prefix={}, max_keys={}",
            prefix, max_keys
        );

        // 列出bucket中的所有对象
        let objects = self
            .storage
            .list_bucket_objects(&bucket, prefix)
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("列出对象失败: {}", e),
                )
            })?;

        // 为每个对象获取版本信息
        let mut version_entries = Vec::new();

        for (idx, key) in objects.iter().enumerate() {
            if idx >= max_keys {
                break;
            }

            let file_id = format!("{}/{}", bucket, key);

            // 获取该文件的所有版本
            let versions = self
                .version_manager
                .list_versions(&file_id)
                .await
                .unwrap_or_default();

            // 如果有版本记录，添加到结果中
            if !versions.is_empty() {
                for version in versions {
                    version_entries.push((key.clone(), version));
                }
            } else {
                // 如果没有版本记录，尝试从存储中获取当前文件信息
                if let Ok(metadata) = self.storage.get_metadata(&file_id).await {
                    // 创建一个伪版本条目
                    version_entries.push((
                        key.clone(),
                        crate::models::FileVersion {
                            version_id: "null".to_string(),
                            file_id: file_id.clone(),
                            name: key.clone(),
                            size: metadata.size,
                            hash: metadata.hash,
                            created_at: metadata.created_at,
                            is_current: true,
                            author: None,
                            comment: None,
                        },
                    ));
                }
            }
        }

        // 生成XML响应
        let xml = self.build_versions_response(&bucket, prefix, &version_entries);

        self.send_xml_response(xml, "silent-nas-016")
    }

    /// 构建空的版本列表响应
    fn build_empty_versions_response(&self, bucket: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <ListVersionsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n\
               <Name>{}</Name>\n\
               <Prefix></Prefix>\n\
               <MaxKeys>1000</MaxKeys>\n\
               <IsTruncated>false</IsTruncated>\n\
             </ListVersionsResult>",
            Self::xml_escape(bucket)
        )
    }

    /// 构建版本列表响应
    fn build_versions_response(
        &self,
        bucket: &str,
        prefix: &str,
        entries: &[(String, crate::models::FileVersion)],
    ) -> String {
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<ListVersionsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n");
        xml.push_str(&format!("  <Name>{}</Name>\n", Self::xml_escape(bucket)));
        xml.push_str(&format!(
            "  <Prefix>{}</Prefix>\n",
            Self::xml_escape(prefix)
        ));
        xml.push_str(&format!(
            "  <MaxKeys>{}</MaxKeys>\n",
            entries.len().max(1000)
        ));
        xml.push_str("  <IsTruncated>false</IsTruncated>\n");

        for (key, version) in entries {
            xml.push_str("  <Version>\n");
            xml.push_str(&format!("    <Key>{}</Key>\n", Self::xml_escape(key)));
            xml.push_str(&format!(
                "    <VersionId>{}</VersionId>\n",
                Self::xml_escape(&version.version_id)
            ));
            xml.push_str(&format!(
                "    <IsLatest>{}</IsLatest>\n",
                version.is_current
            ));
            xml.push_str(&format!(
                "    <LastModified>{}</LastModified>\n",
                version.created_at.and_utc().to_rfc3339()
            ));
            xml.push_str(&format!(
                "    <ETag>&quot;{}&quot;</ETag>\n",
                Self::xml_escape(&version.hash)
            ));
            xml.push_str(&format!("    <Size>{}</Size>\n", version.size));
            xml.push_str("    <Owner>\n");
            xml.push_str(&format!(
                "      <ID>{}</ID>\n",
                Self::xml_escape(version.author.as_deref().unwrap_or("silent-nas"))
            ));
            xml.push_str(&format!(
                "      <DisplayName>{}</DisplayName>\n",
                Self::xml_escape(version.author.as_deref().unwrap_or("silent-nas"))
            ));
            xml.push_str("    </Owner>\n");
            xml.push_str("    <StorageClass>STANDARD</StorageClass>\n");
            xml.push_str("  </Version>\n");
        }

        xml.push_str("</ListVersionsResult>");
        xml
    }

    /// 发送XML响应
    fn send_xml_response(&self, xml: String, request_id: &str) -> silent::Result<Response> {
        let mut resp = Response::empty();
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/xml"),
        );
        resp.headers_mut().insert(
            "x-amz-request-id",
            http::HeaderValue::from_str(request_id).unwrap(),
        );
        resp.set_body(full(xml.into_bytes()));
        resp.set_status(StatusCode::OK);
        Ok(resp)
    }
}
