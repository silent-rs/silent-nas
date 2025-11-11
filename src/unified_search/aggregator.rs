//! 搜索结果聚合器
//!
//! 负责聚合来自多个数据源的搜索结果，包括：
//! - 结果去重
//! - 结果排序
//! - 结果合并
//! - 性能优化

use super::{SearchFilter, SearchResultItem, SortSpec};
use std::collections::HashMap;

/// 搜索结果聚合器
pub struct SearchResultAggregator {
    /// 最大结果数限制
    max_results: usize,
    /// 去重策略
    deduplication_strategy: DeduplicationStrategy,
}

/// 去重策略
#[derive(Debug, Clone)]
pub enum DeduplicationStrategy {
    /// 按 ID 去重
    ById,
    /// 按 URL 去重
    ByUrl,
    /// 按内容哈希去重
    ByContentHash,
    /// 不去重
    None,
}

impl SearchResultAggregator {
    /// 创建新的聚合器
    pub fn new(max_results: usize, deduplication_strategy: DeduplicationStrategy) -> Self {
        Self {
            max_results,
            deduplication_strategy,
        }
    }

    /// 聚合搜索结果
    pub fn aggregate(
        &self,
        results: Vec<Vec<SearchResultItem>>,
        filters: &[SearchFilter],
        sort: &Option<SortSpec>,
    ) -> Vec<SearchResultItem> {
        // 1. 合并所有结果
        let mut all_results = Vec::new();
        for result_set in results {
            all_results.extend(result_set);
        }

        // 2. 应用过滤
        let filtered_results = self.apply_filters(all_results, filters);

        // 3. 去重（ById不需要保留高分，ByUrl和ByContentHash需要，None不处理）
        let deduped_results = match self.deduplication_strategy {
            DeduplicationStrategy::ById => self.deduplicate(filtered_results),
            DeduplicationStrategy::None => filtered_results,
            _ => self.deduplicate_keep_high_score(filtered_results),
        };

        // 4. 排序
        let sorted_results = self.sort_results(deduped_results, sort);

        // 5. 限制结果数量
        self.limit_results(sorted_results)
    }

    /// 应用过滤条件
    fn apply_filters(
        &self,
        results: Vec<SearchResultItem>,
        filters: &[SearchFilter],
    ) -> Vec<SearchResultItem> {
        if filters.is_empty() {
            return results;
        }

        results
            .into_iter()
            .filter(|item| self.matches_filters(item, filters))
            .collect()
    }

    /// 检查结果是否匹配过滤条件
    fn matches_filters(&self, item: &SearchResultItem, filters: &[SearchFilter]) -> bool {
        for filter in filters {
            if !self.matches_filter(item, filter) {
                return false;
            }
        }
        true
    }

    /// 检查单个过滤条件
    fn matches_filter(&self, item: &SearchResultItem, filter: &SearchFilter) -> bool {
        let value = self.get_field_value(item, &filter.field);

        match &filter.value {
            super::FilterValue::String(expected) => match filter.operator {
                super::FilterOperator::Equal => value == *expected,
                super::FilterOperator::Contains => value.contains(expected),
                super::FilterOperator::Like => value.contains(expected),
                _ => true,
            },
            super::FilterValue::Number(expected) => {
                if let Ok(actual) = value.parse::<f64>() {
                    match filter.operator {
                        super::FilterOperator::Equal => actual == *expected,
                        super::FilterOperator::GreaterThan => actual > *expected,
                        super::FilterOperator::LessThan => actual < *expected,
                        _ => true,
                    }
                } else {
                    false
                }
            }
            super::FilterValue::Boolean(expected) => {
                if let Ok(actual) = value.parse::<bool>() {
                    match filter.operator {
                        super::FilterOperator::Equal => actual == *expected,
                        _ => true,
                    }
                } else {
                    false
                }
            }
            super::FilterValue::Array(values) => match filter.operator {
                super::FilterOperator::InRange => values.contains(&value),
                _ => values.contains(&value),
            },
        }
    }

    /// 获取字段值
    fn get_field_value(&self, item: &SearchResultItem, field: &str) -> String {
        // 先从元数据中查找
        if let Some(value) = item.metadata.get(field) {
            return value.clone();
        }

        // 从预定义字段中查找
        match field.to_lowercase().as_str() {
            "id" => item.id.clone(),
            "title" => item.title.clone(),
            "url" => item.url.clone(),
            "score" => item.score.to_string(),
            "type" => format!("{:?}", item.result_type),
            "created_at" => item.created_at.map_or("".to_string(), |v| v.to_string()),
            "modified_at" => item.modified_at.map_or("".to_string(), |v| v.to_string()),
            _ => "".to_string(),
        }
    }

    /// 去重
    fn deduplicate(&self, results: Vec<SearchResultItem>) -> Vec<SearchResultItem> {
        match self.deduplication_strategy {
            DeduplicationStrategy::ById => {
                let mut seen = HashMap::new();
                results
                    .into_iter()
                    .filter(|item| {
                        if seen.contains_key(&item.id) {
                            false
                        } else {
                            seen.insert(item.id.clone(), true);
                            true
                        }
                    })
                    .collect()
            }
            DeduplicationStrategy::ByUrl => {
                let mut seen = HashMap::new();
                results
                    .into_iter()
                    .filter(|item| {
                        if seen.contains_key(&item.url) {
                            false
                        } else {
                            seen.insert(item.url.clone(), true);
                            true
                        }
                    })
                    .collect()
            }
            DeduplicationStrategy::ByContentHash => {
                // 简化实现：按 URL 去重
                let mut seen = HashMap::new();
                results
                    .into_iter()
                    .filter(|item| {
                        let hash = item.metadata.get("content_hash").unwrap_or(&item.url);
                        if seen.contains_key(hash) {
                            false
                        } else {
                            seen.insert(hash.to_string(), true);
                            true
                        }
                    })
                    .collect()
            }
            DeduplicationStrategy::None => results,
        }
    }

    /// 去重（保留高分）
    fn deduplicate_keep_high_score(&self, results: Vec<SearchResultItem>) -> Vec<SearchResultItem> {
        let mut map: HashMap<String, &SearchResultItem> = HashMap::new();

        for item in &results {
            let key = match self.deduplication_strategy {
                DeduplicationStrategy::ById => item.id.clone(),
                DeduplicationStrategy::ByUrl => item.url.clone(),
                DeduplicationStrategy::ByContentHash => item
                    .metadata
                    .get("content_hash")
                    .unwrap_or(&item.url)
                    .clone(),
                DeduplicationStrategy::None => continue,
            };

            if let Some(existing) = map.get(&key) {
                // 保留分数更高的结果
                if item.score > existing.score {
                    map.insert(key, item);
                }
            } else {
                map.insert(key, item);
            }
        }

        // 提取结果
        map.values().cloned().cloned().collect()
    }

    /// 排序结果
    fn sort_results(
        &self,
        results: Vec<SearchResultItem>,
        sort: &Option<SortSpec>,
    ) -> Vec<SearchResultItem> {
        if let Some(sort_spec) = sort {
            let mut results = results;
            let field = &sort_spec.field;

            results.sort_by(|a, b| {
                let a_value = self.get_field_value(a, field);
                let b_value = self.get_field_value(b, field);

                match sort_spec.direction {
                    super::SortDirection::Asc => a_value.cmp(&b_value),
                    super::SortDirection::Desc => b_value.cmp(&a_value),
                }
            });

            results
        } else {
            // 默认按分数降序排序
            let mut results = results;
            results.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            results
        }
    }

    /// 限制结果数量
    fn limit_results(&self, results: Vec<SearchResultItem>) -> Vec<SearchResultItem> {
        if results.len() > self.max_results {
            results.into_iter().take(self.max_results).collect()
        } else {
            results
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unified_search::{ResultType, SearchSource, SourceType};
    use std::collections::HashMap;

    fn create_test_item(id: &str, title: &str, url: &str, score: f32) -> SearchResultItem {
        SearchResultItem {
            id: id.to_string(),
            result_type: ResultType::File,
            source: SearchSource {
                source_type: SourceType::Local,
                identifier: "test".to_string(),
                credentials: None,
            },
            title: title.to_string(),
            description: None,
            url: url.to_string(),
            score,
            metadata: HashMap::new(),
            created_at: None,
            modified_at: None,
        }
    }

    #[test]
    fn test_aggregate_basic() {
        let aggregator = SearchResultAggregator::new(100, DeduplicationStrategy::ById);

        let results1 = vec![create_test_item("1", "file1.txt", "/files/1", 1.0)];
        let results2 = vec![create_test_item("2", "file2.txt", "/files/2", 2.0)];

        let aggregated = aggregator.aggregate(vec![results1, results2], &[], &None);

        assert_eq!(aggregated.len(), 2);
        assert_eq!(aggregated[0].id, "2"); // 按分数降序排序
        assert_eq!(aggregated[1].id, "1");
    }

    #[test]
    fn test_deduplicate_by_id() {
        let aggregator = SearchResultAggregator::new(100, DeduplicationStrategy::ById);

        let results = vec![
            create_test_item("1", "file1.txt", "/files/1", 1.0),
            create_test_item("1", "file1_dup.txt", "/files/1", 2.0), // 重复 ID
            create_test_item("2", "file2.txt", "/files/2", 3.0),
        ];

        let deduped = aggregator.aggregate(vec![results], &[], &None);

        assert_eq!(deduped.len(), 2);
        assert!(deduped.iter().any(|r| r.id == "1"));
        assert!(deduped.iter().any(|r| r.id == "2"));
    }

    #[test]
    fn test_deduplicate_by_url() {
        let aggregator = SearchResultAggregator::new(100, DeduplicationStrategy::ByUrl);

        let results = vec![
            create_test_item("1", "file1.txt", "/files/1", 1.0),
            create_test_item("2", "file1_dup.txt", "/files/1", 2.0), // 重复 URL
            create_test_item("3", "file2.txt", "/files/2", 3.0),
        ];

        let deduped = aggregator.aggregate(vec![results], &[], &None);

        assert_eq!(deduped.len(), 2);
        // 保留分数更高的结果
        assert!(deduped.iter().any(|r| r.id == "2"));
    }

    #[test]
    fn test_limit_results() {
        let aggregator = SearchResultAggregator::new(2, DeduplicationStrategy::None);

        let results = vec![
            create_test_item("1", "file1.txt", "/files/1", 1.0),
            create_test_item("2", "file2.txt", "/files/2", 2.0),
            create_test_item("3", "file3.txt", "/files/3", 3.0),
        ];

        let limited = aggregator.aggregate(vec![results], &[], &None);

        assert_eq!(limited.len(), 2);
        assert_eq!(limited[0].id, "3"); // 分数最高
        assert_eq!(limited[1].id, "2");
    }
}
