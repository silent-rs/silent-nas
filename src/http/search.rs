//! 搜索 API 端点

use super::state::{AppState, SearchQuery, SearchSuggestQuery};
use http::StatusCode;
use serde_json::{Value, json};
use silent::SilentError;
use silent::extractor::{Configs as CfgExtractor, Query};

/// 搜索文件
pub async fn search_files(
    (Query(query), CfgExtractor(state)): (Query<SearchQuery>, CfgExtractor<AppState>),
) -> silent::Result<Value> {
    if query.q.trim().is_empty() {
        return Err(SilentError::business_error(
            StatusCode::BAD_REQUEST,
            "搜索查询不能为空",
        ));
    }

    // 执行搜索
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

    // 应用过滤
    let filtered_results = apply_filters(results, &query);

    // 应用排序
    let sorted_results = apply_sorting(filtered_results, &query);

    // 构建响应
    let response = json!({
        "query": query.q,
        "total": sorted_results.len(),
        "results": sorted_results,
        "pagination": {
            "limit": query.limit,
            "offset": query.offset,
            "has_more": sorted_results.len() == query.limit
        },
        "filters": {
            "file_type": query.file_type,
            "min_size": query.min_size,
            "max_size": query.max_size,
            "modified_after": query.modified_after,
            "modified_before": query.modified_before
        }
    });

    Ok(response)
}

/// 获取搜索统计
pub async fn get_search_stats(
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<Value> {
    let stats = state.search_engine.get_stats();

    // 获取增量索引统计
    let incremental_stats = state.search_engine.get_incremental_stats().await;

    let response = json!({
        "index": {
            "total_documents": stats.total_documents,
            "index_size": stats.index_size
        },
        "incremental": {
            "total_updates": incremental_stats.total_updates,
            "successful_updates": incremental_stats.successful_updates,
            "failed_updates": incremental_stats.failed_updates,
            "last_update": incremental_stats.last_update,
            "avg_update_time_ms": incremental_stats.avg_update_time_ms,
            "cache_hit_rate": incremental_stats.cache_hit_rate
        }
    });

    Ok(response)
}

/// 搜索建议（自动补全）
#[allow(dead_code)]
pub async fn search_suggest(
    (Query(query), CfgExtractor(_state)): (Query<SearchSuggestQuery>, CfgExtractor<AppState>),
) -> silent::Result<Value> {
    if query.q.trim().is_empty() {
        return Ok(json!({
            "query": query.q,
            "suggestions": Vec::<String>::new()
        }));
    }

    // 简化的搜索建议实现
    // 实际实现中可以从索引中获取热门搜索词或相关建议
    let suggestions: Vec<String> = vec![];

    Ok(json!({
        "query": query.q,
        "suggestions": suggestions
    }))
}

/// 重建搜索索引
#[allow(dead_code)]
pub async fn rebuild_search_index(
    CfgExtractor(_state): CfgExtractor<AppState>,
) -> silent::Result<Value> {
    // TODO: 实现重建索引逻辑
    // 需要从存储管理器获取所有文件并重新索引

    Ok(json!({
        "status": "success",
        "message": "索引重建功能尚未实现"
    }))
}

/// 应用过滤条件
fn apply_filters(
    results: Vec<crate::search::SearchResult>,
    query: &SearchQuery,
) -> Vec<crate::search::SearchResult> {
    results
        .into_iter()
        .filter(|result| {
            // 文件类型过滤
            if !query.file_type.is_empty() {
                // TODO: 需要从结果中获取文件类型
                // 目前的结果结构中没有文件类型字段
            }

            // 文件大小过滤
            if let Some(min_size) = query.min_size
                && result.size < min_size
            {
                return false;
            }
            if let Some(max_size) = query.max_size
                && result.size > max_size
            {
                return false;
            }

            // 修改时间过滤
            if let Some(after) = query.modified_after
                && result.modified_at < after
            {
                return false;
            }
            if let Some(before) = query.modified_before
                && result.modified_at > before
            {
                return false;
            }

            true
        })
        .collect()
}

/// 应用排序
fn apply_sorting(
    mut results: Vec<crate::search::SearchResult>,
    query: &SearchQuery,
) -> Vec<crate::search::SearchResult> {
    match query.sort_by.as_str() {
        "name" => {
            results.sort_by(|a, b| match query.sort_order.as_str() {
                "asc" => a.name.cmp(&b.name),
                _ => b.name.cmp(&a.name),
            });
        }
        "size" => {
            results.sort_by(|a, b| match query.sort_order.as_str() {
                "asc" => a.size.cmp(&b.size),
                _ => b.size.cmp(&a.size),
            });
        }
        "modified_at" => {
            results.sort_by(|a, b| match query.sort_order.as_str() {
                "asc" => a.modified_at.cmp(&b.modified_at),
                _ => b.modified_at.cmp(&a.modified_at),
            });
        }
        "score" => {
            // 默认按相关性分数排序
            results.sort_by(|a, b| match query.sort_order.as_str() {
                "asc" => a
                    .score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal),
                _ => b
                    .score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            });
        }
        _ => {
            // 未知排序字段，默认按分数降序
            results.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    }

    results
}
