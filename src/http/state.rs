//! HTTP 服务器状态和配置

use crate::audit::AuditLogger;
use crate::auth::AuthManager;
use crate::http::StorageV2MetricsState;
use crate::notify::EventNotifier;
use crate::search::SearchEngine;
use crate::storage::StorageManager;
#[cfg(not(test))]
use crate::sync::crdt::SyncManager;
#[cfg(not(test))]
use crate::sync::incremental::IncrementalSyncHandler;
use serde::Deserialize;
use std::sync::Arc;

// 测试时的占位符
#[cfg(test)]
use crate::sync::crdt::SyncManager;
#[cfg(test)]
use crate::sync::incremental::IncrementalSyncHandler;

/// 应用共享状态
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<StorageManager>,
    pub notifier: Option<Arc<EventNotifier>>,
    pub sync_manager: Arc<SyncManager>,
    pub search_engine: Arc<SearchEngine>,
    pub inc_sync_handler: Arc<IncrementalSyncHandler>,
    pub source_http_addr: Arc<String>,
    pub audit_logger: Option<Arc<AuditLogger>>,
    pub auth_manager: Option<Arc<AuthManager>>,
    pub storage_v2_metrics: Arc<StorageV2MetricsState>,
}

/// 搜索查询参数
#[derive(Debug, Deserialize, Default)]
pub struct SearchQuery {
    #[serde(default)]
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
    /// 文件类型过滤（如：text, html, code, pdf）
    #[serde(default)]
    pub file_type: Vec<String>,
    /// 最小文件大小（字节）
    #[serde(default)]
    pub min_size: Option<u64>,
    /// 最大文件大小（字节）
    #[serde(default)]
    pub max_size: Option<u64>,
    /// 修改时间范围 - 开始时间戳
    #[serde(default)]
    pub modified_after: Option<i64>,
    /// 修改时间范围 - 结束时间戳
    #[serde(default)]
    pub modified_before: Option<i64>,
    /// 排序字段（name, size, modified_at, score）
    #[serde(default = "default_sort_by")]
    pub sort_by: String,
    /// 排序方向（asc, desc）
    #[serde(default = "default_sort_order")]
    pub sort_order: String,
    /// 是否包含内容搜索
    #[serde(default = "default_search_content")]
    #[allow(dead_code)]
    pub search_content: bool,
}

fn default_limit() -> usize {
    20
}

fn default_sort_by() -> String {
    "score".to_string()
}

fn default_sort_order() -> String {
    "desc".to_string()
}

fn default_search_content() -> bool {
    true
}

/// 搜索建议查询参数
#[derive(Debug, Deserialize)]
pub struct SearchSuggestQuery {
    #[serde(default)]
    pub q: String,
    #[serde(default = "default_suggest_limit")]
    #[allow(dead_code)]
    pub limit: usize,
}

fn default_suggest_limit() -> usize {
    10
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_query_deserialization() {
        let json = r#"{
            "q": "test query",
            "limit": 50,
            "offset": 10,
            "file_type": ["text", "pdf"],
            "min_size": 1024,
            "max_size": 10240,
            "modified_after": 1234567890,
            "modified_before": 1234567900,
            "sort_by": "name",
            "sort_order": "asc",
            "search_content": false
        }"#;

        let query: SearchQuery = serde_json::from_str(json).unwrap();
        assert_eq!(query.q, "test query");
        assert_eq!(query.limit, 50);
        assert_eq!(query.offset, 10);
        assert_eq!(query.file_type, vec!["text", "pdf"]);
        assert_eq!(query.min_size, Some(1024));
        assert_eq!(query.max_size, Some(10240));
        assert_eq!(query.modified_after, Some(1234567890));
        assert_eq!(query.modified_before, Some(1234567900));
        assert_eq!(query.sort_by, "name");
        assert_eq!(query.sort_order, "asc");
        assert!(!query.search_content);
    }

    #[test]
    fn test_search_query_partial_deserialization() {
        let json = r#"{"q": "test"}"#;
        let query: SearchQuery = serde_json::from_str(json).unwrap();

        assert_eq!(query.q, "test");
        assert_eq!(query.limit, 20); // 默认值
        assert_eq!(query.offset, 0); // 默认值
        assert_eq!(query.sort_by, "score"); // 默认值
        assert_eq!(query.sort_order, "desc"); // 默认值
        assert!(query.search_content); // 默认值
    }

    #[test]
    fn test_search_suggest_query_deserialization() {
        let json = r#"{"q": "test", "limit": 5}"#;
        let query: SearchSuggestQuery = serde_json::from_str(json).unwrap();

        assert_eq!(query.q, "test");
        assert_eq!(query.limit, 5);
    }

    #[test]
    fn test_search_suggest_query_default_limit() {
        let json = r#"{"q": "test"}"#;
        let query: SearchSuggestQuery = serde_json::from_str(json).unwrap();

        assert_eq!(query.q, "test");
        assert_eq!(query.limit, 10); // 默认值
    }

    #[test]
    fn test_default_functions() {
        assert_eq!(default_limit(), 20);
        assert_eq!(default_sort_by(), "score");
        assert_eq!(default_sort_order(), "desc");
        assert!(default_search_content());
        assert_eq!(default_suggest_limit(), 10);
    }
}
