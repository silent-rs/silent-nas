use super::{WebDavHandler, constants::*};
use http_body_util::BodyExt;
use silent::prelude::*;

impl WebDavHandler {
    /// PROPPATCH - 设置/移除自定义属性（简化实现）
    pub(super) async fn handle_proppatch(
        &self,
        path: &str,
        req: &mut Request,
    ) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        self.ensure_lock_ok(&path, req).await?;
        let body = req.take_body();
        let _xml = match body {
            ReqBody::Incoming(b) => b
                .collect()
                .await
                .map_err(|e| {
                    SilentError::business_error(
                        StatusCode::BAD_REQUEST,
                        format!("读取请求体失败: {}", e),
                    )
                })?
                .to_bytes()
                .to_vec(),
            ReqBody::Once(bytes) => bytes.to_vec(),
            ReqBody::Empty => Vec::new(),
        };

        // 简化：记录 PROPPATCH 时间戳为示例属性
        let mut props = self.props.write().await;
        let entry = props.entry(path.clone()).or_default();
        entry.insert(
            "prop:last-proppatch".to_string(),
            chrono::Local::now().naive_local().to_string(),
        );
        drop(props);
        self.persist_props().await;

        let mut resp = Response::text("");
        resp.set_status(StatusCode::MULTI_STATUS);
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static(CONTENT_TYPE_XML),
        );
        Ok(resp)
    }
}
