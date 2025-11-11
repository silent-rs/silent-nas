//! 统一搜索接口
//!
//! 提供跨协议的统一搜索功能，整合：
//! - WebDAV SEARCH 方法
//! - S3 Select 查询
//! - 本地文件系统搜索
//! - REST API 搜索
//!
//! 支持搜索结果聚合和权限控制

pub mod aggregator;

use crate::error::Result;
use crate::s3_search::S3SearchEngine;
use crate::search::SearchEngine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// 统一搜索请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedSearchRequest {
    /// 搜索查询字符串
    pub query: String,
    /// 搜索类型
    pub search_type: SearchType,
    /// 数据源
    pub sources: Vec<SearchSource>,
    /// 分页参数
    pub pagination: Pagination,
    /// 过滤条件
    pub filters: Vec<SearchFilter>,
    /// 排序规则
    pub sort: Option<SortSpec>,
}

/// 搜索类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SearchType {
    /// 全文搜索
    FullText,
    /// 结构化查询
    Structured,
    /// SQL 查询
    SQL,
}

/// 搜索源
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSource {
    /// 源类型
    pub source_type: SourceType,
    /// 源标识
    pub identifier: String,
    /// 权限验证
    pub credentials: Option<SearchCredentials>,
}

/// 源类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SourceType {
    /// 本地文件系统
    Local,
    /// WebDAV 服务器
    WebDAV,
    /// S3 存储
    S3,
    /// HTTP API
    HTTP,
}

/// 搜索凭据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchCredentials {
    /// 用户名
    pub username: String,
    /// 密码或令牌
    pub token: String,
    /// 权限范围
    pub scope: Vec<String>,
}

/// 分页参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagination {
    /// 页码（从1开始）
    pub page: usize,
    /// 每页大小
    pub page_size: usize,
    /// 偏移量
    pub offset: usize,
}

/// 搜索过滤
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchFilter {
    /// 过滤字段
    pub field: String,
    /// 操作符
    pub operator: FilterOperator,
    /// 过滤值
    pub value: FilterValue,
}

/// 过滤操作符
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FilterOperator {
    /// 等于
    Equal,
    /// 不等于
    NotEqual,
    /// 包含
    Contains,
    /// 大于
    GreaterThan,
    /// 小于
    LessThan,
    /// 在范围内
    InRange,
    /// 匹配模式
    Like,
}

/// 过滤值
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FilterValue {
    /// 字符串值
    String(String),
    /// 数值
    Number(f64),
    /// 布尔值
    Boolean(bool),
    /// 数组
    Array(Vec<String>),
}

/// 排序规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortSpec {
    /// 排序字段
    pub field: String,
    /// 排序方向
    pub direction: SortDirection,
}

/// 排序方向
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SortDirection {
    /// 升序
    Asc,
    /// 降序
    Desc,
}

/// 统一搜索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedSearchResult {
    /// 搜索结果列表
    pub results: Vec<SearchResultItem>,
    /// 总结果数
    pub total_count: usize,
    /// 当前页结果数
    pub current_count: usize,
    /// 分页信息
    pub pagination: PaginationInfo,
    /// 统计信息
    pub stats: SearchStats,
}

/// 搜索结果项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultItem {
    /// 结果 ID
    pub id: String,
    /// 结果类型
    pub result_type: ResultType,
    /// 源信息
    pub source: SearchSource,
    /// 标题
    pub title: String,
    /// 描述
    pub description: Option<String>,
    /// URL 或路径
    pub url: String,
    /// 相关性分数
    pub score: f32,
    /// 元数据
    pub metadata: HashMap<String, String>,
    /// 创建时间
    pub created_at: Option<i64>,
    /// 修改时间
    pub modified_at: Option<i64>,
}

/// 结果类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResultType {
    /// 文件
    File,
    /// 目录
    Directory,
    /// 文档
    Document,
    /// 图片
    Image,
    /// 视频
    Video,
    /// 音频
    Audio,
    /// 其他
    Other,
}

/// 分页信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationInfo {
    /// 当前页码
    pub current_page: usize,
    /// 每页大小
    pub page_size: usize,
    /// 总页数
    pub total_pages: usize,
    /// 是否有下一页
    pub has_next: bool,
    /// 是否有上一页
    pub has_previous: bool,
}

/// 搜索统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchStats {
    /// 搜索耗时（毫秒）
    pub search_time_ms: u64,
    /// 搜索的数据源数量
    pub sources_count: usize,
    /// 每个源的结果数
    pub results_by_source: HashMap<String, usize>,
    /// 搜索条件解析时间
    pub parse_time_ms: u64,
    /// 结果聚合时间
    pub aggregate_time_ms: u64,
}

/// 统一搜索引擎
pub struct UnifiedSearchEngine {
    /// 本地搜索引擎
    local_search: Arc<SearchEngine>,
    /// WebDAV 处理器（使用 dyn trait 避免循环依赖）
    webdav_handler: Option<Arc<dyn std::any::Any + Send + Sync>>,
    /// S3 搜索引擎
    s3_search: Option<Arc<S3SearchEngine>>,
}

impl UnifiedSearchEngine {
    /// 创建新的统一搜索引擎
    pub fn new(
        local_search: Arc<SearchEngine>,
        webdav_handler: Option<Arc<dyn std::any::Any + Send + Sync>>,
        s3_search: Option<Arc<S3SearchEngine>>,
    ) -> Self {
        Self {
            local_search,
            webdav_handler,
            s3_search,
        }
    }

    /// 执行统一搜索
    pub async fn search(&self, request: &UnifiedSearchRequest) -> Result<UnifiedSearchResult> {
        use std::time::Instant;

        let start_time = Instant::now();
        let parse_start = Instant::now();

        // 解析搜索条件
        let parsed_query = self.parse_search_request(request)?;
        let parse_time = parse_start.elapsed().as_millis() as u64;

        // 根据搜索源执行搜索
        let aggregate_start = Instant::now();
        let mut all_results = Vec::new();
        let mut results_by_source = HashMap::new();

        for source in &request.sources {
            let source_results = self
                .search_source(source, &parsed_query, &request.filters)
                .await?;
            let source_id = format!("{:?}", source.source_type);
            results_by_source.insert(source_id, source_results.len());
            all_results.extend(source_results);
        }

        let aggregate_time = aggregate_start.elapsed().as_millis() as u64;

        // 排序和分页
        let sorted_results = self.sort_results(all_results, &request.sort);
        let paginated_results = self.paginate_results(&sorted_results, &request.pagination);

        // 构建响应
        let total_count = sorted_results.len();
        let current_count = paginated_results.len();
        // 使用向上取整计算总页数
        let total_pages = total_count.div_ceil(request.pagination.page_size);

        let search_time = start_time.elapsed().as_millis() as u64;

        Ok(UnifiedSearchResult {
            results: paginated_results,
            total_count,
            current_count,
            pagination: PaginationInfo {
                current_page: request.pagination.page,
                page_size: request.pagination.page_size,
                total_pages,
                has_next: request.pagination.page < total_pages,
                has_previous: request.pagination.page > 1,
            },
            stats: SearchStats {
                search_time_ms: search_time,
                sources_count: request.sources.len(),
                results_by_source,
                parse_time_ms: parse_time,
                aggregate_time_ms: aggregate_time,
            },
        })
    }

    /// 解析搜索请求
    fn parse_search_request(&self, request: &UnifiedSearchRequest) -> Result<ParsedUnifiedQuery> {
        // 简化实现：直接返回查询字符串
        Ok(ParsedUnifiedQuery {
            query: request.query.clone(),
            search_type: request.search_type.clone(),
        })
    }

    /// 在指定数据源中搜索
    async fn search_source(
        &self,
        source: &SearchSource,
        query: &ParsedUnifiedQuery,
        _filters: &[SearchFilter],
    ) -> Result<Vec<SearchResultItem>> {
        match source.source_type {
            SourceType::Local => {
                // 使用本地搜索引擎
                let results = self.local_search.search(&query.query, 1000, 0).await?;

                Ok(results
                    .into_iter()
                    .map(|r| SearchResultItem {
                        id: r.file_id,
                        result_type: ResultType::File,
                        source: source.clone(),
                        title: r.name,
                        description: None,
                        url: r.path,
                        score: r.score,
                        metadata: HashMap::new(),
                        created_at: None,
                        modified_at: Some(r.modified_at),
                    })
                    .collect())
            }
            SourceType::WebDAV => {
                // 使用 WebDAV 搜索
                if self.webdav_handler.is_some() {
                    // TODO: 实现 WebDAV 搜索
                    Ok(Vec::new())
                } else {
                    Ok(Vec::new())
                }
            }
            SourceType::S3 => {
                // 使用 S3 搜索
                if let Some(ref _s3_search) = self.s3_search {
                    // TODO: 实现 S3 搜索
                    Ok(Vec::new())
                } else {
                    Ok(Vec::new())
                }
            }
            SourceType::HTTP => {
                // 使用 HTTP API 搜索
                // TODO: 实现 HTTP API 搜索
                Ok(Vec::new())
            }
        }
    }

    /// 排序结果
    fn sort_results(
        &self,
        results: Vec<SearchResultItem>,
        sort: &Option<SortSpec>,
    ) -> Vec<SearchResultItem> {
        if let Some(sort_spec) = sort {
            let mut results = results;
            match sort_spec.direction {
                SortDirection::Asc => {
                    results.sort_by(|a, b| {
                        a.metadata
                            .get(&sort_spec.field)
                            .cmp(&b.metadata.get(&sort_spec.field))
                    });
                }
                SortDirection::Desc => {
                    results.sort_by(|a, b| {
                        b.metadata
                            .get(&sort_spec.field)
                            .cmp(&a.metadata.get(&sort_spec.field))
                    });
                }
            }
            results
        } else {
            results
        }
    }

    /// 分页结果
    fn paginate_results(
        &self,
        results: &[SearchResultItem],
        pagination: &Pagination,
    ) -> Vec<SearchResultItem> {
        let start = pagination.offset;
        let end = std::cmp::min(start + pagination.page_size, results.len());
        results[start..end].to_vec()
    }
}

/// 解析后的统一查询
#[derive(Debug, Clone)]
struct ParsedUnifiedQuery {
    /// 查询字符串
    query: String,
    /// 搜索类型
    #[allow(dead_code)]
    search_type: SearchType,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use crate::unified_search::SourceType;

    #[test]
    fn test_unified_search_request_creation() {
        let request = UnifiedSearchRequest {
            query: "document".to_string(),
            search_type: SearchType::FullText,
            sources: vec![SearchSource {
                source_type: super::SourceType::Local,
                identifier: "local".to_string(),
                credentials: None,
            }],
            pagination: Pagination {
                page: 1,
                page_size: 20,
                offset: 0,
            },
            filters: vec![],
            sort: None,
        };

        assert_eq!(request.query, "document");
        assert!(matches!(request.search_type, SearchType::FullText));
        assert_eq!(request.sources.len(), 1);
    }

    #[test]
    fn test_search_result_item_creation() {
        let item = SearchResultItem {
            id: "test-id".to_string(),
            result_type: ResultType::File,
            source: SearchSource {
                source_type: super::SourceType::Local,
                identifier: "test".to_string(),
                credentials: None,
            },
            title: "test.txt".to_string(),
            description: Some("Test file".to_string()),
            url: "/files/test.txt".to_string(),
            score: 1.0,
            metadata: HashMap::new(),
            created_at: Some(1634567890),
            modified_at: Some(1634567890),
        };

        assert_eq!(item.id, "test-id");
        assert_eq!(item.title, "test.txt");
        assert!(item.description.is_some());
    }

    #[test]
    fn test_pagination_info_creation() {
        let pagination = PaginationInfo {
            current_page: 1,
            page_size: 20,
            total_pages: 5,
            has_next: true,
            has_previous: false,
        };

        assert_eq!(pagination.current_page, 1);
        assert_eq!(pagination.total_pages, 5);
        assert!(pagination.has_next);
        assert!(!pagination.has_previous);
    }
}
