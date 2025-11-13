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
            let mut fq_updates: Vec<(String, Option<String>)> = Vec::new();
            // 命名空间上下文栈
            let mut ns_stack: Vec<std::collections::HashMap<String, String>> = Vec::new();

            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Start(e)) => {
                        let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                        // 处理 xmlns 声明
                        let mut ns_map = ns_stack.last().cloned().unwrap_or_default();
                        for attr in e.attributes().with_checks(false).flatten() {
                            let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                            if key == "xmlns"
                                && let Ok(val) = String::from_utf8(attr.value.clone().into_owned())
                            {
                                ns_map.insert(String::new(), val);
                            } else if let Some(suffix) = key.strip_prefix("xmlns:")
                                && let Ok(val) = String::from_utf8(attr.value.into_owned())
                            {
                                ns_map.insert(suffix.to_string(), val);
                            }
                        }
                        ns_stack.push(ns_map);
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
                                        // 原始前缀键
                                        updates.push((key.clone(), current_text.clone()));
                                        // 生成命名空间完全限定键 ns:{uri}#{local}
                                        if let Some(ns_ctx) = ns_stack.last() {
                                            let (pref, local) =
                                                key.split_once(':').unwrap_or(("", key.as_str()));
                                            if let Some(uri) = ns_ctx.get(pref) {
                                                fq_updates.push((
                                                    format!("ns:{}#{}", uri, local),
                                                    current_text.take(),
                                                ));
                                            } else if let Some(uri) = ns_ctx.get("") {
                                                fq_updates.push((
                                                    format!("ns:{}#{}", uri, local),
                                                    current_text.take(),
                                                ));
                                            }
                                        }
                                    } else if in_remove {
                                        updates.push((key.clone(), None));
                                        if let Some(ns_ctx) = ns_stack.last() {
                                            let (pref, local) =
                                                key.split_once(':').unwrap_or(("", key.as_str()));
                                            if let Some(uri) = ns_ctx.get(pref) {
                                                fq_updates
                                                    .push((format!("ns:{}#{}", uri, local), None));
                                            } else if let Some(uri) = ns_ctx.get("") {
                                                fq_updates
                                                    .push((format!("ns:{}#{}", uri, local), None));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // 出栈当前元素的命名空间上下文
                        let _ = ns_stack.pop();
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
            if !updates.is_empty() || !fq_updates.is_empty() {
                let mut props = self.props.write().await;
                let entry = props.entry(path.clone()).or_default();
                // 只允许非 DAV: 命名空间的可写属性
                let mut reject_dav = false;
                let mut conflict = false;
                // 命名空间冲突检测：针对结构化键 ns:{URI}#{local}
                let mut seen_local: std::collections::HashMap<String, (String, Option<String>)> =
                    std::collections::HashMap::new();
                let parse_ns_key = |k: &str| -> Option<(String, String)> {
                    if let Some(rest) = k.strip_prefix("ns:")
                        && let Some((uri, local)) = rest.split_once('#')
                    {
                        return Some((uri.to_string(), local.to_string()));
                    }
                    None
                };
                // 先扫描现有条目，记录已存在的 (local -> uri,value)
                for (ek, ev) in entry.iter() {
                    if let Some((u, l)) = parse_ns_key(ek) {
                        seen_local.entry(l).or_insert((u, Some(ev.clone())));
                    }
                }
                // 再扫描本次结构化更新，若同 local 不同 uri 且值不同，标记冲突
                for (k, v) in fq_updates.iter() {
                    if let Some((u, l)) = parse_ns_key(k) {
                        if let Some((eu, ev)) = seen_local.get(&l)
                            && eu != &u
                            && ev.as_deref() != v.as_deref()
                        {
                            conflict = true;
                            break;
                        }
                        seen_local.insert(l, (u, v.clone()));
                    }
                }
                if conflict {
                    return Err(SilentError::business_error(
                        StatusCode::CONFLICT,
                        "命名空间冲突：同名属性存在不同URI且值不一致",
                    ));
                }
                for (k, v) in updates {
                    let key = k.trim().to_string();
                    if key.starts_with("D:") || key.starts_with("d:") {
                        reject_dav = true;
                        continue;
                    }
                    if let Some(val) = v {
                        Self::validate_prop_value(&key, &val)?;
                        // 限制属性值大小，防止滥用
                        let vshort = if val.len() > 4096 {
                            val[..4096].to_string()
                        } else {
                            val
                        };
                        entry.insert(key, vshort);
                    } else {
                        entry.remove(&key);
                    }
                }
                for (k, v) in fq_updates {
                    let key = k.trim().to_string();
                    if key.starts_with("ns:DAV:#") || key.starts_with("ns:dav:#") {
                        reject_dav = true;
                        continue;
                    }
                    if let Some(val) = v {
                        Self::validate_prop_value(&key, &val)?;
                        let vshort = if val.len() > 4096 {
                            val[..4096].to_string()
                        } else {
                            val
                        };
                        entry.insert(key, vshort);
                    } else {
                        entry.remove(&key);
                    }
                }
                if reject_dav {
                    return Err(SilentError::business_error(
                        StatusCode::CONFLICT,
                        "DAV: 命名空间属性为只读",
                    ));
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

        // 审计：记录属性变更
        self.append_change("prop:patch", &path);

        let mut resp = Response::text("");
        resp.set_status(StatusCode::MULTI_STATUS);
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static(CONTENT_TYPE_XML),
        );
        Ok(resp)
    }

    fn validate_prop_value(key: &str, val: &str) -> silent::Result<()> {
        // 简单类型约定：local 名以 ".bool" 结尾时必须为 true/false；以 ".int" 结尾时必须为整数
        let local = if let Some(rest) = key.strip_prefix("ns:") {
            rest.split('#').nth(1).unwrap_or(rest)
        } else {
            key
        };
        if local.ends_with(".bool") {
            let v = val.trim().to_ascii_lowercase();
            if v != "true" && v != "false" {
                return Err(SilentError::business_error(
                    StatusCode::BAD_REQUEST,
                    format!("属性 {} 期望布尔值", key),
                ));
            }
        }
        if local.ends_with(".int") && val.trim().parse::<i64>().is_err() {
            return Err(SilentError::business_error(
                StatusCode::BAD_REQUEST,
                format!("属性 {} 期望整数", key),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageManagerTrait;
    use std::sync::Arc;

    async fn build_handler() -> WebDavHandler {
        let dir = tempfile::tempdir().unwrap();
        let storage =
            crate::storage::StorageManager::new(dir.path().to_path_buf(), 4 * 1024 * 1024);
        let _ = crate::storage::init_global_storage(storage.clone());
        storage.init().await.unwrap();
        let syncm = crate::sync::crdt::SyncManager::new("node-test".to_string(), None);
        let ver = crate::version::VersionManager::new(
            std::sync::Arc::new(storage.clone()),
            Default::default(),
            dir.path().to_str().unwrap(),
        );
        let search_engine = Arc::new(
            crate::search::SearchEngine::new(
                dir.path().join("search_index"),
                dir.path().to_path_buf(),
            )
            .unwrap(),
        );
        WebDavHandler::new(
            None,
            syncm,
            "".into(),
            "http://127.0.0.1:8080".into(),
            ver,
            search_engine,
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
            // 记录命名空间完全限定键
            assert_eq!(entry.get("ns:urn:x-example#category").unwrap(), "interop");
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
            assert!(!entry.contains_key("ns:urn:x-example#category"));
            assert!(entry.contains_key("prop:last-proppatch"));
        }
    }
}
