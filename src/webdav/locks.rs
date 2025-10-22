use silent::prelude::*;
// use tracing::warn;

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
