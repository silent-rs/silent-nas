use crate::notify::EventNotifier;
use crate::storage::StorageManager;
use crate::sync::crdt::SyncManager;
use async_trait::async_trait;
use silent::prelude::*;
use std::sync::Arc;

#[allow(unused_imports)]
use super::{constants::*, types::DavLock};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq)]
enum IfTermKind { LockToken(String), ETag(String) }
#[derive(Debug, Clone)]
struct IfTerm { negate: bool, kind: IfTermKind }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ChangeEntry {
    pub path: String,
    pub action: String, // created/modified/deleted
    pub ts: chrono::NaiveDateTime,
}

#[derive(Clone)]
pub struct WebDavHandler {
    pub storage: Arc<StorageManager>,
    pub notifier: Option<Arc<EventNotifier>>,
    #[allow(dead_code)]
    pub sync_manager: Arc<SyncManager>,
    pub base_path: String,
    pub source_http_addr: String,
    #[allow(dead_code)]
    pub version_manager: Arc<crate::version::VersionManager>,
    pub(super) locks: Arc<tokio::sync::RwLock<std::collections::HashMap<String, Vec<DavLock>>>>,
    pub(super) props: Arc<
        tokio::sync::RwLock<
            std::collections::HashMap<String, std::collections::HashMap<String, String>>,
        >,
    >,
}

impl WebDavHandler {
    pub fn new(
        storage: Arc<StorageManager>,
        notifier: Option<Arc<EventNotifier>>,
        sync_manager: Arc<SyncManager>,
        base_path: String,
        source_http_addr: String,
        version_manager: Arc<crate::version::VersionManager>,
    ) -> Self {
        let handler = Self {
            storage,
            notifier,
            sync_manager,
            base_path,
            source_http_addr,
            version_manager,
            locks: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            props: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        };
        handler.load_persistent_state();
        handler
    }

    pub(super) fn lock_token() -> String {
        format!("opaquelocktoken:{}", scru128::new_string())
    }

    pub(super) fn meta_dir(&self) -> std::path::PathBuf {
        self.storage.root_dir().join(".webdav")
    }
    pub(super) fn locks_file(&self) -> std::path::PathBuf {
        self.meta_dir().join("locks.json")
    }
    pub(super) fn props_file(&self) -> std::path::PathBuf {
        self.meta_dir().join("props.json")
    }
    pub(super) fn changelog_file(&self) -> std::path::PathBuf {
        self.meta_dir().join("changelog.json")
    }

    #[allow(clippy::collapsible_if)]
    fn load_persistent_state(&self) {
        let _ = std::fs::create_dir_all(self.meta_dir());
        if let Ok(bytes) = std::fs::read(self.locks_file())
            && let Ok(map) =
                serde_json::from_slice::<std::collections::HashMap<String, Vec<DavLock>>>(&bytes)
        {
            let rt = tokio::runtime::Handle::current();
            let locks = self.locks.clone();
            rt.spawn(async move {
                *locks.write().await = map;
            });
        }
        if let Ok(bytes) = std::fs::read(self.props_file())
            && let Ok(map) = serde_json::from_slice::<
                std::collections::HashMap<String, std::collections::HashMap<String, String>>,
            >(&bytes)
        {
            let rt = tokio::runtime::Handle::current();
            let props = self.props.clone();
            rt.spawn(async move {
                *props.write().await = map;
            });
        }
    }

    pub(super) async fn persist_locks(&self) {
        let map = self.locks.read().await.clone();
        let _ = std::fs::create_dir_all(self.meta_dir());
        if let Ok(bytes) = serde_json::to_vec_pretty(&map) {
            let _ = std::fs::write(self.locks_file(), bytes);
        }
    }

    pub(super) async fn persist_props(&self) {
        let map = self.props.read().await.clone();
        let _ = std::fs::create_dir_all(self.meta_dir());
        if let Ok(bytes) = serde_json::to_vec_pretty(&map) {
            let _ = std::fs::write(self.props_file(), bytes);
        }
    }

    pub(super) fn append_change(&self, action: &str, path: &str) {
        let _ = std::fs::create_dir_all(self.meta_dir());
        let mut list: Vec<ChangeEntry> = std::fs::read(self.changelog_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        list.push(ChangeEntry {
            path: path.to_string(),
            action: action.to_string(),
            ts: chrono::Local::now().naive_local(),
        });
        // 简单裁剪：最多 10000 条，超出丢弃最旧
        const MAX_LEN: usize = 10000;
        if list.len() > MAX_LEN {
            let drain = list.len() - MAX_LEN;
            let _ = list.drain(0..drain);
        }
        if let Ok(bytes) = serde_json::to_vec(&list) {
            let _ = std::fs::write(self.changelog_file(), bytes);
        }
    }

    pub(super) fn list_deleted_since(
        &self,
        prefix: &str,
        since: chrono::NaiveDateTime,
        limit: usize,
    ) -> Vec<String> {
        let list: Vec<ChangeEntry> = std::fs::read(self.changelog_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut out = Vec::new();
        for e in list.iter().filter(|e| e.action == "deleted" && e.ts > since) {
            if !prefix.is_empty() && prefix != "/" {
                if !e.path.starts_with(prefix) { continue; }
            }
            out.push(e.path.clone());
            if out.len() >= limit { break; }
        }
        out
    }

    pub(super) fn parse_timeout(req: &Request) -> i64 {
        if let Some(v) = req.headers().get("Timeout").and_then(|h| h.to_str().ok()) {
            if v.to_lowercase().contains("infinite") {
                return 3600;
            }
            if let Some(num) = v.split(['-', ',']).find_map(|s| s.parse::<i64>().ok()) {
                return num.clamp(1, 3600);
            }
        }
        60
    }

    pub(super) fn extract_if_lock_tokens(req: &Request) -> Vec<String> {
        let mut tokens = Vec::new();
        if let Some(val) = req.headers().get("If").and_then(|h| h.to_str().ok()) {
            let s = val.as_bytes();
            let needle = b"opaquelocktoken:";
            let mut i = 0;
            while i + needle.len() <= s.len() {
                if &s[i..i + needle.len()] == needle {
                    let start = i;
                    // 向后找到 > 作为结束
                    let mut j = i;
                    while j < s.len() && s[j] != b'>' as u8 { j += 1; }
                    let end = j.min(s.len());
                    if end > start {
                        if let Ok(tok) = std::str::from_utf8(&s[start..end]) {
                            tokens.push(tok.to_string());
                        }
                    }
                    i = end;
                } else {
                    i += 1;
                }
            }
        }
        tokens
    }

    /// 提取与指定路径相关的 If 令牌（支持资源标记与未标记列表）
    pub(super) fn extract_if_tokens_for_path(&self, path: &str, req: &Request) -> Vec<String> {
        let Some(header) = req.headers().get("If").and_then(|h| h.to_str().ok()) else {
            return Vec::new();
        };
        let mut tokens_by_tag: std::collections::HashMap<Option<String>, Vec<String>> = std::collections::HashMap::new();
        let mut current_tag: Option<String> = None;
        let mut i = 0usize;
        let bytes = header.as_bytes();
        while i < bytes.len() {
            match bytes[i] {
                b'<' => {
                    // 资源标签
                    let start = i + 1;
                    let mut j = start;
                    while j < bytes.len() && bytes[j] != b'>' { j += 1; }
                    if j < bytes.len() {
                        if let Ok(s) = std::str::from_utf8(&bytes[start..j]) {
                            current_tag = Some(s.to_string());
                        }
                        i = j + 1;
                        continue;
                    } else { break; }
                }
                b'(' => {
                    // 括号内列表：收集令牌
                    let mut j = i + 1;
                    let content_start = j;
                    let mut depth = 1;
                    while j < bytes.len() {
                        if bytes[j] == b'(' { depth += 1; }
                        if bytes[j] == b')' { depth -= 1; if depth == 0 { break; } }
                        j += 1;
                    }
                    let end = j;
                    if end <= bytes.len() {
                        let segment = &header[content_start..end];
                        let mut toks = Vec::new();
                        // 从段中提取 token
                        let needle = "opaquelocktoken:";
                        let mut k = 0usize;
                        while let Some(pos) = segment[k..].find(needle) {
                            let abs = k + pos;
                            let after = &segment[abs..];
                            if let Some(close) = after.find('>') {
                                toks.push(after[..close].to_string());
                                k = abs + close;
                            } else { break; }
                        }
                        tokens_by_tag.entry(current_tag.clone()).or_default().extend(toks);
                    }
                    i = end.saturating_add(1);
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        // 选择与路径匹配的资源标签，或未标记的
        let target = self.build_full_href(path);
        let mut out = Vec::new();
        if let Some(v) = tokens_by_tag.get(&None) { out.extend(v.clone()); }
        for (tag, toks) in &tokens_by_tag {
            if let Some(t) = tag {
                if t == &target || t.ends_with(&target) { out.extend(toks.clone()); }
            }
        }
        out
    }

    // 辅助类型定义移动到模块级（impl 内不支持定义）

    fn current_etag(&self, path: &str) -> Option<String> {
        let full = self.storage.get_full_path(path);
        if let Ok(meta) = std::fs::metadata(full) {
            let len = meta.len();
            let ts = meta
                .modified().ok()?
                .duration_since(std::time::UNIX_EPOCH).ok()?
                .as_secs();
            Some(format!("\"{}-{}\"", len, ts))
        } else { None }
    }

    fn parse_if_header_full(&self, header: &str) -> std::collections::HashMap<Option<String>, Vec<Vec<IfTerm>>> {
        use std::collections::HashMap;
        let mut map: HashMap<Option<String>, Vec<Vec<IfTerm>>> = HashMap::new();
        let bytes = header.as_bytes();
        let mut i = 0usize;
        let mut current_tag: Option<String> = None;
        fn is_ws(b: u8) -> bool { matches!(b, b' ' | b'\t' | b'\r' | b'\n') }
        while i < bytes.len() {
            if is_ws(bytes[i]) { i += 1; continue; }
            match bytes[i] {
                b'<' => {
                    // 资源标签 <...>
                    let start = i + 1;
                    let mut j = start;
                    while j < bytes.len() && bytes[j] != b'>' { j += 1; }
                    if j < bytes.len() {
                        if let Ok(s) = std::str::from_utf8(&bytes[start..j]) {
                            current_tag = Some(s.to_string());
                        }
                        i = j + 1;
                    } else { break; }
                }
                b'(' => {
                    // 解析一个列表，直到配对 ')'
                    i += 1;
                    let mut terms: Vec<IfTerm> = Vec::new();
                    loop {
                        while i < bytes.len() && is_ws(bytes[i]) { i += 1; }
                        if i >= bytes.len() { break; }
                        if bytes[i] == b')' { i += 1; break; }
                        // 可选 Not
                        let mut negate = false;
                        if bytes[i..].len() >= 3 {
                            // 匹配大小写不敏感 "Not"
                            let s = &header[i..];
                            if s.to_ascii_lowercase().starts_with("not") {
                                // 下一个必须是空白或 '<' '"' '('
                                negate = true;
                                i += 3;
                                while i < bytes.len() && is_ws(bytes[i]) { i += 1; }
                            }
                        }
                        if i >= bytes.len() { break; }
                        match bytes[i] {
                            b'<' => {
                                // 锁令牌
                                let start = i + 1;
                                let mut j = start;
                                while j < bytes.len() && bytes[j] != b'>' { j += 1; }
                                if j < bytes.len() {
                                    if let Ok(tok) = std::str::from_utf8(&bytes[start..j]) {
                                        terms.push(IfTerm { negate, kind: IfTermKind::LockToken(tok.to_string()) });
                                    }
                                    i = j + 1;
                                } else { break; }
                            }
                            b'"' => {
                                // ETag（含双引号）
                                let start = i;
                                i += 1;
                                let mut j = i;
                                while j < bytes.len() && bytes[j] != b'"' { j += 1; }
                                if j < bytes.len() {
                                    // 包含引号
                                    if let Ok(et) = std::str::from_utf8(&bytes[start..=j]) {
                                        terms.push(IfTerm { negate, kind: IfTermKind::ETag(et.to_string()) });
                                    }
                                    i = j + 1;
                                } else { break; }
                            }
                            _ => {
                                // 未知 token，跳过到下一个空白或 ')'
                                while i < bytes.len() && !is_ws(bytes[i]) && bytes[i] != b')' { i += 1; }
                            }
                        }
                    }
                    if !terms.is_empty() {
                        map.entry(current_tag.clone()).or_default().push(terms);
                    }
                }
                _ => { i += 1; }
            }
        }
        map
    }

    async fn eval_if_header_for_path(&self, path: &str, req: &Request) -> bool {
        let header = match req.headers().get("If").and_then(|h| h.to_str().ok()) { Some(s) => s, None => return false };
        let conds = self.parse_if_header_full(header);
        // 收集相关条件：未标记和与 path 匹配的标记
        let target = self.build_full_href(path);
        let mut lists: Vec<Vec<IfTerm>> = Vec::new();
        if let Some(v) = conds.get(&None) { lists.extend_from_slice(v); }
        for (tag, v) in &conds {
            if let Some(t) = tag {
                if t == &target || t.ends_with(&target) { lists.extend_from_slice(v); }
            }
        }
        if lists.is_empty() { return false; }
        // 收集当前锁令牌集（精确路径 + 祖先 depth=infinity）
        let mut tokens: Vec<String> = Vec::new();
        let locks = self.locks.read().await;
        if let Some(list) = locks.get(path) { for l in list.iter().filter(|l| !l.is_expired()) { tokens.push(l.token.clone()); } }
        // 祖先
        let mut prefix = String::new();
        for seg in path.split('/').filter(|s| !s.is_empty()) {
            prefix.push('/'); prefix.push_str(seg);
            if &prefix == path { break; }
            if let Some(list) = locks.get(&prefix) {
                for l in list.iter().filter(|l| !l.is_expired() && l.depth_infinity) {
                    tokens.push(l.token.clone());
                }
            }
        }
        drop(locks);
        let etag_now = self.current_etag(path);

        // 评估：OR(AND(terms))
        'outer: for terms in lists {
            for t in terms {
                let ok = match t.kind {
                    IfTermKind::LockToken(ref tok) => tokens.iter().any(|x| x == tok),
                    IfTermKind::ETag(ref val) => etag_now.as_deref() == Some(val.as_str()),
                };
                let final_ok = if t.negate { !ok } else { ok };
                if !final_ok { continue 'outer; }
            }
            // 每个列表全部通过
            return true;
        }
        false
    }

    pub(super) async fn ensure_lock_ok(&self, path: &str, req: &Request) -> silent::Result<()> {
        // 若存在锁（本资源或祖先 Depth: infinity），需要 If 条件满足
        let locks = self.locks.read().await;
        let mut has_lock = false;
        if let Some(list) = locks.get(path) {
            has_lock |= list.iter().any(|l| !l.is_expired());
        }
        // 祖先
        let mut prefix = String::new();
        for seg in path.split('/').filter(|s| !s.is_empty()) {
            prefix.push('/'); prefix.push_str(seg);
            if &prefix == path { break; }
            if let Some(list) = locks.get(&prefix) {
                if list.iter().any(|l| !l.is_expired() && l.depth_infinity) {
                    has_lock = true;
                    break;
                }
            }
        }
        drop(locks);
        if has_lock {
            if self.eval_if_header_for_path(path, req).await { return Ok(()); }
            return Err(SilentError::business_error(StatusCode::LOCKED, "资源被锁定或条件不满足"));
        }
        Ok(())
    }

    pub(super) fn decode_path(path: &str) -> silent::Result<String> {
        urlencoding::decode(path)
            .map(|s| s.to_string())
            .map_err(|e| {
                SilentError::business_error(StatusCode::BAD_REQUEST, format!("路径解码失败: {}", e))
            })
    }

    pub(super) fn build_full_href(&self, relative_path: &str) -> String {
        // Finder 期望 href 为相对路径（不含 schema/host），目录以尾斜杠结尾
        // base_path 作为相对前缀（通常为空字符串）
        let mut path = format!("{}{}", &self.base_path, relative_path);
        if !path.starts_with('/') {
            path = format!("/{}", path);
        }
        path
    }
}

#[async_trait]
impl Handler for WebDavHandler {
    async fn call(&self, mut req: Request) -> silent::Result<Response> {
        let method = req.method().clone();
        let uri_path = req.uri().path().to_string();
        let relative_path = uri_path
            .strip_prefix(&self.base_path)
            .unwrap_or(&uri_path)
            .to_string();
        tracing::debug!("WebDAV {} {}", method, relative_path);
        match method.as_str() {
            "OPTIONS" => self.handle_options().await,
            "PROPFIND" => self.handle_propfind(&relative_path, &req).await,
            "PROPPATCH" => self.handle_proppatch(&relative_path, &mut req).await,
            "HEAD" => self.handle_head(&relative_path, &req).await,
            "GET" => self.handle_get(&relative_path, &req).await,
            "PUT" => self.handle_put(&relative_path, &mut req).await,
            "DELETE" => self.handle_delete(&relative_path).await,
            "MKCOL" => self.handle_mkcol(&relative_path).await,
            "MOVE" => self.handle_move(&relative_path, &req).await,
            "COPY" => self.handle_copy(&relative_path, &req).await,
            "LOCK" => self.handle_lock(&relative_path, &mut req).await,
            "UNLOCK" => self.handle_unlock(&relative_path, &req).await,
            "VERSION-CONTROL" => self.handle_version_control(&relative_path).await,
            "REPORT" => self.handle_report(&relative_path, &mut req).await,
            _ => Err(SilentError::business_error(
                StatusCode::METHOD_NOT_ALLOWED,
                "不支持的方法",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_token_format() {
        let token = WebDavHandler::lock_token();
        assert!(token.starts_with("opaquelocktoken:"));
        // scru128 由 [0-9a-z] 和分隔符组成，一般长度固定
        assert!(token.len() > 20);
    }

    #[test]
    fn test_decode_path_ok() {
        let s = WebDavHandler::decode_path("/dir/%E4%B8%AD%E6%96%87.txt").unwrap();
        assert_eq!(s, "/dir/中文.txt");
    }

    #[tokio::test]
    async fn test_build_full_href_rules() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(StorageManager::new(
            dir.path().to_path_buf(),
            4 * 1024 * 1024,
        ));
        storage.init().await.unwrap();
        let syncm = SyncManager::new("node-test".into(), storage.clone(), None);
        let ver = crate::version::VersionManager::new(
            storage.clone(),
            Default::default(),
            dir.path().to_str().unwrap(),
        );
        let handler = WebDavHandler::new(
            storage,
            None,
            syncm,
            "".into(),
            "http://127.0.0.1:8080".into(),
            ver,
        );
        assert_eq!(handler.build_full_href("/"), "/");
        assert_eq!(handler.build_full_href("/a/b"), "/a/b");
        assert_eq!(handler.build_full_href("a/b"), "/a/b");
    }

    #[test]
    fn test_parse_timeout() {
        let mut req = Request::empty();
        req.headers_mut()
            .insert("Timeout", http::HeaderValue::from_static("Second-120"));
        assert_eq!(WebDavHandler::parse_timeout(&req), 120);

        let mut req2 = Request::empty();
        req2.headers_mut()
            .insert("Timeout", http::HeaderValue::from_static("Infinite"));
        assert_eq!(WebDavHandler::parse_timeout(&req2), 3600);
    }
}
