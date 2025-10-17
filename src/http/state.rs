//! HTTP 服务器状态和配置

use crate::notify::EventNotifier;
use crate::search::SearchEngine;
use crate::storage::StorageManager;
use crate::version::VersionManager;
use serde::Deserialize;
use std::sync::Arc;

// 当作为 main.rs 的子模块时，使用 super 访问同级模块
#[cfg(not(test))]
use crate::sync::crdt::SyncManager;
#[cfg(not(test))]
use crate::sync::incremental::IncrementalSyncHandler;

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
    pub version_manager: Arc<VersionManager>,
    pub search_engine: Arc<SearchEngine>,
    pub inc_sync_handler: Arc<IncrementalSyncHandler>,
    pub source_http_addr: Arc<String>,
}

/// 搜索查询参数
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    20
}
