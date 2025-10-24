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
