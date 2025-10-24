use silent::prelude::*;
// use http_body_util::BodyExt;

use super::{WebDavHandler, constants::*, types::DavLock};

impl WebDavHandler {
    /// LOCK - 锁定资源（简化，支持独占锁）
    pub(super) async fn handle_lock(&self, path: &str, req: &Request) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let mut locks = self.locks.write().await;
        if let Some(l) = locks.get(&path)
            && !l.is_expired()
        {
            return Err(SilentError::business_error(
                StatusCode::LOCKED,
                "资源已被锁定",
            ));
        }
        // 简易共享锁检测（请求体包含 <shared/> 则视为共享，内部仍使用独占策略存储）
        // 简化共享锁检测：从 If 头或 UA 约定头中判断（占位实现）
        let _is_shared = req
            .headers()
            .get("X-WebDAV-Shared")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let token = Self::lock_token();
        let timeout = Self::parse_timeout(req);
        let info = DavLock::new_exclusive(token.clone(), timeout);
        locks.insert(path.clone(), info);
        drop(locks);
        self.persist_locks().await;

        let xml = format!(
            "{}<D:prop xmlns:D=\"DAV:\"><D:lockdiscovery><D:activelock><D:locktype><D:write/></D:locktype><D:lockscope><D:exclusive/></D:lockscope><D:locktoken><D:href>{}</D:href></D:locktoken></D:activelock></D:lockdiscovery></D:prop>",
            XML_HEADER, token
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
        if let Some(l) = locks.get(&path) {
            if l.token == token {
                locks.remove(&path);
            } else {
                return Err(SilentError::business_error(
                    StatusCode::CONFLICT,
                    "锁令牌不匹配",
                ));
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

    #[tokio::test]
    async fn test_lock_then_unlock_ok() {
        let handler = build_handler().await;

        // 发起 LOCK
        let mut req = Request::empty();
        req.headers_mut()
            .insert("Timeout", http::HeaderValue::from_static("Second-120"));
        let resp = handler.handle_lock("/doc.txt", &req).await.unwrap();
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
        let req = Request::empty();
        let _ = handler.handle_lock("/a.txt", &req).await.unwrap();

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
        let resp = handler
            .handle_lock("/b.txt", &Request::empty())
            .await
            .unwrap();
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
