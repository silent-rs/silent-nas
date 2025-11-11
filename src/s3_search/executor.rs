//! SQL 查询执行器
//!
//! 执行解析后的 SQL 查询，返回结果

use crate::error::{NasError, Result};
use crate::search::SearchEngine;
use std::sync::Arc;
use std::time::Instant;

use super::SelectResult;
use super::parser::{Comparison, Condition, Literal, Operand, Operator, ParsedQuery, SelectClause};

/// 执行 SQL 查询
pub async fn execute_query(
    search_engine: &Arc<SearchEngine>,
    query: &ParsedQuery,
) -> Result<SelectResult> {
    let start_time = Instant::now();

    // 构建搜索查询字符串
    let search_query = build_search_query(query)?;

    // 执行搜索
    let results = search_engine
        .search(&search_query, 1000, 0)
        .await
        .map_err(|e| NasError::Storage(format!("搜索失败: {}", e)))?;

    // 处理搜索结果
    let output = format_search_results(&query.select, &results)?;

    // 计算统计信息
    let processing_time = start_time.elapsed().as_millis() as u64;
    let stats = super::QueryStats {
        records_scanned: results.len() as u64,
        records_returned: results.len() as u64,
        processing_time_ms: processing_time,
    };

    Ok(SelectResult {
        payload: output.clone(),
        bytes_scanned: results.len() as u64,
        bytes_returned: output.len() as u64,
        stats,
    })
}

/// 构建搜索查询字符串
fn build_search_query(query: &ParsedQuery) -> Result<String> {
    let mut parts = Vec::new();

    // 从 SELECT 子句中提取字段信息（如果指定了字段）
    if let SelectClause::Fields(fields) = &query.select {
        for field in fields {
            parts.push(field.name.clone());
        }
    }

    // 从 WHERE 子句中提取条件
    if let Some(where_clause) = &query.where_clause {
        let condition_str = build_condition_string(&where_clause.conditions)?;
        parts.push(condition_str);
    }

    Ok(parts.join(" "))
}

/// 构建条件字符串
fn build_condition_string(conditions: &[Condition]) -> Result<String> {
    let mut parts = Vec::new();

    for condition in conditions {
        match condition {
            Condition::Comparison(comp) => {
                let cond_str = build_comparison_string(comp)?;
                parts.push(cond_str);
            }
            Condition::And(conds) => {
                let and_str = build_condition_string(conds)?;
                parts.push(format!("({})", and_str));
            }
            Condition::Or(conds) => {
                let or_str = build_condition_string(conds)?;
                parts.push(format!("({})", or_str));
            }
            Condition::Not(cond) => {
                let not_str = build_condition_string(&[*cond.clone()])?;
                parts.push(format!("NOT ({})", not_str));
            }
        }
    }

    Ok(parts.join(" AND "))
}

/// 构建比较条件字符串
fn build_comparison_string(comp: &Comparison) -> Result<String> {
    let left_str = operand_to_string(&comp.left)?;
    let right_str = operand_to_string(&comp.right)?;

    let op_str = match comp.operator {
        Operator::Equal => "=".to_string(),
        Operator::NotEqual => "!=".to_string(),
        Operator::LessThan => "<".to_string(),
        Operator::LessThanOrEqual => "<=".to_string(),
        Operator::GreaterThan => ">".to_string(),
        Operator::GreaterThanOrEqual => ">=".to_string(),
        Operator::Like => "LIKE".to_string(),
        Operator::In => "IN".to_string(),
        Operator::Between => "BETWEEN".to_string(),
    };

    Ok(format!("{} {} {}", left_str, op_str, right_str))
}

/// 将操作数转换为字符串
fn operand_to_string(operand: &Operand) -> Result<String> {
    match operand {
        Operand::Field(name) => Ok(name.clone()),
        Operand::Literal(literal) => match literal {
            Literal::String(s) => Ok(format!("'{}'", s)),
            Literal::Number(n) => Ok(n.to_string()),
            Literal::Boolean(b) => Ok(b.to_string()),
            Literal::Null => Ok("NULL".to_string()),
        },
    }
}

/// 格式化搜索结果
fn format_search_results(
    select_clause: &SelectClause,
    results: &[crate::search::SearchResult],
) -> Result<String> {
    match select_clause {
        SelectClause::All => {
            // 返回 JSON 格式的结果
            let mut output = String::new();
            output.push_str("[\n");
            for (i, result) in results.iter().enumerate() {
                if i > 0 {
                    output.push_str(",\n");
                }
                output.push_str(&format!(
                    "  {{\n    \"name\": \"{}\",\n    \"path\": \"{}\",\n    \"size\": {},\n    \"modified_at\": {},\n    \"file_id\": \"{}\"\n  }}",
                    escape_json(&result.name),
                    escape_json(&result.path),
                    result.size,
                    result.modified_at,
                    escape_json(&result.file_id)
                ));
            }
            output.push_str("\n]");
            Ok(output)
        }
        SelectClause::Fields(fields) => {
            // 返回指定字段的 JSON 格式
            let mut output = String::new();
            output.push_str("[\n");
            for (i, result) in results.iter().enumerate() {
                if i > 0 {
                    output.push_str(",\n");
                }
                output.push_str("  {\n");
                for (j, field) in fields.iter().enumerate() {
                    if j > 0 {
                        output.push_str(",\n");
                    }
                    let value = match field.name.to_lowercase().as_str() {
                        "name" => format!("\"{}\"", escape_json(&result.name)),
                        "path" => format!("\"{}\"", escape_json(&result.path)),
                        "size" => result.size.to_string(),
                        "modified_at" => result.modified_at.to_string(),
                        "file_id" => format!("\"{}\"", escape_json(&result.file_id)),
                        _ => "\"\"".to_string(),
                    };
                    output.push_str(&format!("    \"{}\": {}", field.name, value));
                }
                output.push_str("\n  }");
            }
            output.push_str("\n]");
            Ok(output)
        }
    }
}

/// 转义 JSON 字符串
fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::SearchResult;

    #[test]
    fn test_operand_to_string() {
        // 字段
        let operand = Operand::Field("size".to_string());
        assert_eq!(operand_to_string(&operand).unwrap(), "size");

        // 字符串
        let operand = Operand::Literal(Literal::String("test".to_string()));
        assert_eq!(operand_to_string(&operand).unwrap(), "'test'");

        // 数字
        let operand = Operand::Literal(Literal::Number(123.45));
        assert_eq!(operand_to_string(&operand).unwrap(), "123.45");

        // 布尔值
        let operand = Operand::Literal(Literal::Boolean(true));
        assert_eq!(operand_to_string(&operand).unwrap(), "true");

        // NULL
        let operand = Operand::Literal(Literal::Null);
        assert_eq!(operand_to_string(&operand).unwrap(), "NULL");
    }

    #[test]
    fn test_build_comparison_string() {
        let comp = Comparison {
            left: Operand::Field("size".to_string()),
            operator: Operator::GreaterThan,
            right: Operand::Literal(Literal::Number(100.0)),
        };

        let result = build_comparison_string(&comp).unwrap();
        assert_eq!(result, "size > 100");
    }

    #[test]
    fn test_format_search_results_all() {
        let results = vec![
            SearchResult {
                file_id: "1".to_string(),
                path: "/test/file1.txt".to_string(),
                name: "file1.txt".to_string(),
                size: 1024,
                modified_at: 1634567890,
                score: 1.0,
            },
            SearchResult {
                file_id: "2".to_string(),
                path: "/test/file2.txt".to_string(),
                name: "file2.txt".to_string(),
                size: 2048,
                modified_at: 1634567891,
                score: 1.0,
            },
        ];

        let select_clause = SelectClause::All;
        let result = format_search_results(&select_clause, &results).unwrap();

        assert!(result.contains("file1.txt"));
        assert!(result.contains("file2.txt"));
    }

    #[test]
    fn test_format_search_results_fields() {
        let results = vec![SearchResult {
            file_id: "1".to_string(),
            path: "/test/file1.txt".to_string(),
            name: "file1.txt".to_string(),
            size: 1024,
            modified_at: 1634567890,
            score: 1.0,
        }];

        let fields = vec![
            crate::s3_search::parser::Field {
                name: "name".to_string(),
                alias: None,
            },
            crate::s3_search::parser::Field {
                name: "size".to_string(),
                alias: None,
            },
        ];
        let select_clause = SelectClause::Fields(fields);
        let result = format_search_results(&select_clause, &results).unwrap();

        assert!(result.contains("file1.txt"));
        assert!(result.contains("1024"));
    }

    #[test]
    fn test_escape_json() {
        let input = r#"test"quote\ntab	backslash\"#;
        let output = escape_json(input);
        assert!(output.contains(r#"\""#));
        assert!(output.contains(r#"\n"#));
        assert!(output.contains(r#"\t"#));
        assert!(output.contains(r#"\\"#));
    }
}
