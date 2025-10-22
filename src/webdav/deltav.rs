use super::{WebDavHandler, constants::*};
use silent::prelude::*;

impl WebDavHandler {
    /// VERSION-CONTROL - 启用版本控制（简化为标记属性）
    pub(super) async fn handle_version_control(&self, path: &str) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let mut props = self.props.write().await;
        let entry = props.entry(path).or_default();
        entry.insert("dav:version-controlled".to_string(), "true".to_string());
        Ok(Response::empty())
    }

    /// REPORT - 返回版本列表（简化 XML）
    pub(super) async fn handle_report(&self, path: &str) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let files = self.storage.list_files().await.map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("列出文件失败: {}", e),
            )
        })?;
        let file_id = files.iter().find(|m| m.path == path).map(|m| m.id.clone());
        if file_id.is_none() {
            return Err(SilentError::business_error(
                StatusCode::NOT_FOUND,
                "文件未找到",
            ));
        }
        let file_id = file_id.unwrap();
        let versions = self
            .version_manager
            .list_versions(&file_id)
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("获取版本失败: {}", e),
                )
            })?;

        let mut xml = String::new();
        xml.push_str(XML_HEADER);
        xml.push_str("<D:multistatus xmlns:D=\"DAV:\">");
        for v in versions {
            xml.push_str(&format!(
                "<D:response><D:href>{}</D:href><D:propstat><D:prop><D:version-name>{}</D:version-name><D:version-created>{}</D:version-created></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
                self.build_full_href(&path), v.version_id, v.created_at
            ));
        }
        xml.push_str(XML_MULTISTATUS_END);

        let mut resp = Response::text(&xml);
        resp.set_status(StatusCode::MULTI_STATUS);
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static(CONTENT_TYPE_XML),
        );
        Ok(resp)
    }
}
