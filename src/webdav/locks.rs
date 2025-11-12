use silent::prelude::*;
// use http_body_util::BodyExt;

use super::{WebDavHandler, constants::*, types::DavLock};
use http_body_util::BodyExt;
use quick_xml::de::from_str as xml_from_str;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct LockInfo {
    #[serde(rename = "lockscope")]
    scope: Option<LockScope>,
    #[serde(rename = "owner")]
    owner: Option<LockOwner>,
}

#[derive(Debug, Deserialize)]
struct LockScope {
    #[serde(rename = "exclusive")]
    #[allow(dead_code)]
    exclusive: Option<String>,
    #[serde(rename = "shared")]
    shared: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LockOwner {
    #[serde(rename = "href")]
    href: Option<String>,
}

impl WebDavHandler {
    /// LOCK - 锁定资源（简化，支持独占锁）
    pub(super) async fn handle_lock(
        &self,
        path: &str,
        req: &mut Request,
    ) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        // 解析 Depth 与 body
        let depth_infinity = req
            .headers()
            .get("Depth")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.eq_ignore_ascii_case("infinity"))
            .unwrap_or(false);
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

        let mut exclusive = true;
        let mut owner: Option<String> = None;
        if !xml_bytes.is_empty() {
            let xml_str = String::from_utf8_lossy(&xml_bytes);
            // 允许容错解析：优先用 quick-xml 反序列化，失败则根据字符串包含判断
            if let Ok(info) = xml_from_str::<LockInfo>(&xml_str) {
                if let Some(sc) = info.scope {
                    exclusive = sc.shared.is_none();
                }
                owner = info.owner.and_then(|o| o.href);
            } else {
                if xml_str.contains("<shared") || xml_str.contains(":shared") {
                    exclusive = false;
                }
                if let Some(pos) = xml_str.find("<href>")
                    && let Some(end) = xml_str[pos..].find("</href>")
                {
                    owner = Some(xml_str[pos + 6..pos + end].to_string());
                }
            }
        }

        // 冲突矩阵：
        // - 请求独占：若存在任意未过期锁（共享或独占）则 423
        // - 请求共享：若存在未过期独占锁则 423；否则可并存
        let mut locks = self.locks.write().await;
        let active_list: Vec<DavLock> = locks
            .get(&path)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|l| !l.is_expired())
            .collect();
        let has_excl = active_list.iter().any(|l| l.exclusive);
        let has_any = !active_list.is_empty();
        if exclusive {
            if has_any {
                return Err(SilentError::business_error(
                    StatusCode::LOCKED,
                    "资源已被锁定",
                ));
            }
        } else if has_excl {
            return Err(SilentError::business_error(
                StatusCode::LOCKED,
                "资源已被独占锁定",
            ));
        }
        let token = Self::lock_token();
        let timeout = Self::parse_timeout(req);
        let info = DavLock::new(token.clone(), exclusive, timeout, owner, depth_infinity);
        let entry = locks.entry(path.clone()).or_default();
        entry.push(info);
        drop(locks);
        self.persist_locks().await;

        let scope_xml = if exclusive {
            "<D:exclusive/>"
        } else {
            "<D:shared/>"
        };
        let xml = format!(
            "{}<D:prop xmlns:D=\"DAV:\"><D:lockdiscovery><D:activelock><D:locktype><D:write/></D:locktype><D:lockscope>{}</D:lockscope><D:locktoken><D:href>{}</D:href></D:locktoken></D:activelock></D:lockdiscovery></D:prop>",
            XML_HEADER, scope_xml, token
        );
        let mut resp = Response::text(&xml);
        resp.headers_mut().insert(
            http::header::HeaderName::from_static("lock-token"),
            http::HeaderValue::from_str(&format!("<{}>", token)).unwrap(),
        );
        // 回写 Timeout 响应头
        resp.headers_mut().insert(
            http::header::HeaderName::from_static("timeout"),
            http::HeaderValue::from_str(&format!("Second-{}", timeout)).unwrap(),
        );
        resp.set_status(StatusCode::OK);
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static(CONTENT_TYPE_XML),
        );
        Ok(resp)
    }

    /// UNLOCK - 解除资源锁
    pub(super) async fn handle_unlock(
        &self,
        path: &str,
        req: &Request,
    ) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let token = req
            .headers()
            .get("Lock-Token")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .trim_matches(['<', '>']);
        if token.is_empty() {
            return Err(SilentError::business_error(
                StatusCode::BAD_REQUEST,
                "缺少 Lock-Token",
            ));
        }
        let mut locks = self.locks.write().await;
        if let Some(list) = locks.get_mut(&path) {
            let before = list.len();
            list.retain(|l| l.token != token);
            if list.len() == before {
                return Err(SilentError::business_error(
                    StatusCode::CONFLICT,
                    "锁令牌不匹配",
                ));
            }
            // 若清空则移除条目
            if list.is_empty() {
                locks.remove(&path);
            }
        }
        drop(locks);
        self.persist_locks().await;
        Ok(Response::empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[tokio::test]
    async fn test_lock_then_unlock_ok() {
        let handler = build_handler().await;

        // 发起 LOCK
        let mut req = Request::empty();
        req.headers_mut()
            .insert("Timeout", http::HeaderValue::from_static("Second-120"));
        let resp = handler.handle_lock("/doc.txt", &mut req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let lock_header = resp
            .headers()
            .get("Lock-Token")
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .to_string();
        assert!(lock_header.contains("opaquelocktoken:"));

        // 提取 token（去掉 <>）并 UNLOCK
        let _token = lock_header.trim_matches(['<', '>']).to_string();
        let mut unlock_req = Request::empty();
        unlock_req.headers_mut().insert(
            "Lock-Token",
            http::HeaderValue::from_str(&lock_header).unwrap(),
        );
        let uresp = handler
            .handle_unlock("/doc.txt", &unlock_req)
            .await
            .unwrap();
        assert_eq!(uresp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_unlock_token_mismatch() {
        let handler = build_handler().await;

        // 先上锁
        let mut req = Request::empty();
        let _ = handler.handle_lock("/a.txt", &mut req).await.unwrap();

        // 使用错误的 token 解锁
        let mut bad = Request::empty();
        bad.headers_mut().insert(
            "Lock-Token",
            http::HeaderValue::from_static("<opaquelocktoken:wrong-token>"),
        );
        let err = handler.handle_unlock("/a.txt", &bad).await.err().unwrap();
        assert_eq!(err.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn test_ensure_lock_ok_with_if_header() {
        let handler = build_handler().await;

        // 上锁，拿到 token
        let mut r = Request::empty();
        let resp = handler.handle_lock("/b.txt", &mut r).await.unwrap();
        let lock_header = resp
            .headers()
            .get("Lock-Token")
            .and_then(|v| v.to_str().ok())
            .unwrap()
            .to_string();
        let token = lock_header.trim_matches(['<', '>']).to_string();

        // If 头包含 token 时通过
        let mut ok_req = Request::empty();
        ok_req.headers_mut().insert(
            "If",
            http::HeaderValue::from_str(&format!("(<{}>)", token)).unwrap(),
        );
        handler.ensure_lock_ok("/b.txt", &ok_req).await.unwrap();

        // 无 If 或错误 token 返回 LOCKED
        let bad_req = Request::empty();
        let err = handler
            .ensure_lock_ok("/b.txt", &bad_req)
            .await
            .err()
            .unwrap();
        assert_eq!(err.status(), StatusCode::LOCKED);
    }
}
