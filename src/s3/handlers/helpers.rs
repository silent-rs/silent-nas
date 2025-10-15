use crate::s3::models::S3Object;
use crate::s3::service::S3Service;

impl S3Service {
    /// 生成ListObjectsV2响应的XML
    pub(crate) fn generate_list_v2_response(
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

    /// 生成ListObjects (V1) 响应的XML
    pub(crate) fn generate_list_response(
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

    /// 解析DeleteObjects请求的XML
    pub(crate) fn parse_delete_objects_xml(xml: &str) -> Vec<String> {
        let mut keys = Vec::new();

        // 简单的XML解析，查找<Key>标签
        for line in xml.lines() {
            let line = line.trim();
            if line.starts_with("<Key>") && line.ends_with("</Key>") {
                let key = line
                    .trim_start_matches("<Key>")
                    .trim_end_matches("</Key>")
                    .to_string();
                keys.push(key);
            }
        }

        keys
    }

    /// 生成DeleteObjects响应的XML
    pub(crate) fn generate_delete_result_xml(
        deleted: &[String],
        errors: &[(String, &str, String)],
    ) -> String {
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<DeleteResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n");

        // 成功删除的对象
        for key in deleted {
            xml.push_str("  <Deleted>\n");
            xml.push_str(&format!("    <Key>{}</Key>\n", Self::xml_escape(key)));
            xml.push_str("  </Deleted>\n");
        }

        // 删除失败的对象
        for (key, code, message) in errors {
            xml.push_str("  <Error>\n");
            xml.push_str(&format!("    <Key>{}</Key>\n", Self::xml_escape(key)));
            xml.push_str(&format!("    <Code>{}</Code>\n", Self::xml_escape(code)));
            xml.push_str(&format!(
                "    <Message>{}</Message>\n",
                Self::xml_escape(message)
            ));
            xml.push_str("  </Error>\n");
        }

        xml.push_str("</DeleteResult>");
        xml
    }
}
