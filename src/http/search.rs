//! 搜索 API 端点

use super::state::{AppState, SearchQuery};
use http::StatusCode;
use silent::SilentError;
use silent::extractor::{Configs as CfgExtractor, Query};

/// 搜索文件
pub async fn search_files(
    (Query(query), CfgExtractor(state)): (Query<SearchQuery>, CfgExtractor<AppState>),
) -> silent::Result<serde_json::Value> {
    if query.q.is_empty() {
        return Err(SilentError::business_error(
            StatusCode::BAD_REQUEST,
            "搜索查询不能为空",
        ));
    }

    let results = state
        .search_engine
        .search(&query.q, query.limit, query.offset)
        .await
        .map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("搜索失败: {}", e),
            )
        })?;

    Ok(serde_json::to_value(results).unwrap())
}

/// 获取搜索统计
pub async fn get_search_stats(
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    let stats = state.search_engine.get_stats();
    Ok(serde_json::to_value(stats).unwrap())
}
