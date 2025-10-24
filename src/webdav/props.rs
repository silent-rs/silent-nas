use super::{WebDavHandler, constants::*};
use http_body_util::BodyExt;
use quick_xml::Reader;
use quick_xml::events::Event;
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
        let xml_bytes = match body {
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
        // 解析 set/remove（简化 XML 解析）
        if !xml_bytes.is_empty() {
            let mut reader = Reader::from_reader(xml_bytes.as_slice());
            reader.config_mut().trim_text(true);
            let mut buf = Vec::new();
            let mut in_set = false;
            let mut in_remove = false;
            let mut current_name: Option<String> = None;
            let mut current_text: Option<String> = None;
            let mut updates: Vec<(String, Option<String>)> = Vec::new();

            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Start(e)) => {
                        let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                        match name.as_str() {
                            "set" | "D:set" => in_set = true,
                            "remove" | "D:remove" => in_remove = true,
                            n => {
                                if in_set || in_remove {
                                    current_name = Some(n.to_string());
                                    current_text = Some(String::new());
                                }
                            }
                        }
                    }
                    Ok(Event::End(e)) => {
                        let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                        match name.as_str() {
                            "set" | "D:set" => in_set = false,
                            "remove" | "D:remove" => in_remove = false,
                            _ => {
                                if let Some(key) = current_name.take() {
                                    if in_set {
                                        updates.push((key, current_text.take()));
                                    } else if in_remove {
                                        updates.push((key, None));
                                    }
                                }
                            }
                        }
                    }
                    Ok(Event::Text(t)) => {
                        if current_text.is_some() {
                            let v = String::from_utf8_lossy(&t.into_inner()).to_string();
                            if let Some(ref mut ct) = current_text {
                                ct.push_str(&v);
                            }
                        }
                    }
                    Ok(Event::Eof) => break,
                    Err(_) => break,
                    _ => {}
                }
                buf.clear();
            }
            if !updates.is_empty() {
                let mut props = self.props.write().await;
                let entry = props.entry(path.clone()).or_default();
                for (k, v) in updates {
                    let key = k.trim().to_string();
                    if let Some(val) = v {
                        entry.insert(key, val);
                    } else {
                        entry.remove(&key);
                    }
                }
            }
        }
        // 记录 PROPPATCH 时间戳
        {
            let mut props = self.props.write().await;
            let entry = props.entry(path.clone()).or_default();
            entry.insert(
                "prop:last-proppatch".to_string(),
                chrono::Local::now().naive_local().to_string(),
            );
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    async fn build_handler() -> WebDavHandler {
        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(crate::storage::StorageManager::new(
            dir.path().to_path_buf(),
            4 * 1024 * 1024,
        ));
        storage.init().await.unwrap();
        let syncm = crate::sync::crdt::SyncManager::new("node-test".into(), storage.clone(), None);
        let ver = crate::version::VersionManager::new(
            storage.clone(),
            Default::default(),
            dir.path().to_str().unwrap(),
        );
        WebDavHandler::new(
            storage,
            None,
            syncm,
            "".into(),
            "http://127.0.0.1:8080".into(),
            ver,
        )
    }

    fn make_request_with_body(method: &str, path: &str, body: &str) -> Request {
        let http_req = http::Request::builder()
            .method(method)
            .uri(path)
            .body(())
            .unwrap();
        let (parts, _b) = http_req.into_parts();
        Request::from_parts(parts, ReqBody::Once(body.as_bytes().to_vec().into()))
    }

    #[tokio::test]
    async fn test_proppatch_set_and_remove() {
        let handler = build_handler().await;
        let path = "/p.txt";

        // set 属性
        let set_xml = r#"
<D:propertyupdate xmlns:D="DAV:">
  <D:set><D:prop><Z:category xmlns:Z="urn:x-example">interop</Z:category></D:prop></D:set>
</D:propertyupdate>
"#;
        let mut req = make_request_with_body("PROPPATCH", path, set_xml);
        handler.handle_proppatch(path, &mut req).await.unwrap();
        {
            let props = handler.props.read().await;
            let entry = props.get(path).unwrap();
            // 记录的键为元素名（包含前缀）
            assert_eq!(entry.get("Z:category").unwrap(), "interop");
            assert!(entry.contains_key("prop:last-proppatch"));
        }

        // remove 属性
        let remove_xml = r#"
<D:propertyupdate xmlns:D="DAV:">
  <D:remove><D:prop><Z:category xmlns:Z="urn:x-example"></Z:category></D:prop></D:remove>
</D:propertyupdate>
"#;
        let mut req2 = make_request_with_body("PROPPATCH", path, remove_xml);
        handler.handle_proppatch(path, &mut req2).await.unwrap();
        {
            let props = handler.props.read().await;
            let entry = props.get(path).unwrap();
            assert!(!entry.contains_key("Z:category"));
            assert!(entry.contains_key("prop:last-proppatch"));
        }
    }
}
