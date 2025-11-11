//! SQL 查询解析器
//!
//! 解析 S3 Select 兼容的 SQL 查询语句，提取查询条件和字段

use crate::error::{NasError, Result};

/// SQL 查询语句的抽象表示
#[derive(Debug, Clone)]
pub struct ParsedQuery {
    /// SELECT 子句
    pub select: SelectClause,
    /// FROM 子句
    pub from: FromClause,
    /// WHERE 子句（可选）
    pub where_clause: Option<WhereClause>,
    /// LIMIT 子句（可选）
    pub limit: Option<u64>,
}

/// SELECT 子句
#[derive(Debug, Clone)]
pub enum SelectClause {
    /// 选择所有字段
    All,
    /// 选择指定字段
    Fields(Vec<Field>),
}

/// 字段
#[derive(Debug, Clone)]
pub struct Field {
    /// 字段名
    pub name: String,
    /// 别名（可选）
    pub alias: Option<String>,
}

/// FROM 子句
#[derive(Debug, Clone)]
pub struct FromClause {
    /// 数据源类型
    pub source_type: String,
}

/// WHERE 子句
#[derive(Debug, Clone)]
pub struct WhereClause {
    /// 条件表达式
    pub conditions: Vec<Condition>,
}

/// 条件表达式
#[derive(Debug, Clone)]
pub enum Condition {
    /// 比较条件
    Comparison(Comparison),
    /// 逻辑条件（AND）
    And(Vec<Condition>),
    /// 逻辑条件（OR）
    Or(Vec<Condition>),
    /// 逻辑条件（NOT）
    Not(Box<Condition>),
}

/// 比较操作符
#[derive(Debug, Clone)]
pub enum Operator {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    Like,
    In,
    Between,
}

/// 比较条件
#[derive(Debug, Clone)]
pub struct Comparison {
    /// 左操作数
    pub left: Operand,
    /// 操作符
    pub operator: Operator,
    /// 右操作数
    pub right: Operand,
}

/// 操作数
#[derive(Debug, Clone)]
pub enum Operand {
    /// 字段
    Field(String),
    /// 字面量值
    Literal(Literal),
}

/// 字面量值
#[derive(Debug, Clone)]
pub enum Literal {
    /// 字符串
    String(String),
    /// 数字
    Number(f64),
    /// 布尔值
    Boolean(bool),
    /// NULL
    Null,
}

/// 解析 SQL 查询语句
pub fn parse_sql(sql: &str) -> Result<ParsedQuery> {
    let sql = sql.trim();

    // 基本的 SQL 解析（简化实现）
    if !sql.to_uppercase().starts_with("SELECT") {
        return Err(NasError::Other(format!("无效的 SQL 语句: {}", sql)));
    }

    // 解析 SELECT 子句
    let select = parse_select_clause(sql)?;

    // 解析 FROM 子句
    let from = parse_from_clause(sql)?;

    // 解析 WHERE 子句（可选）
    let where_clause = parse_where_clause(sql)?;

    // 解析 LIMIT 子句（可选）
    let limit = parse_limit_clause(sql)?;

    Ok(ParsedQuery {
        select,
        from,
        where_clause,
        limit,
    })
}

/// 解析 SELECT 子句
fn parse_select_clause(sql: &str) -> Result<SelectClause> {
    let sql = sql.trim();

    // 查找 SELECT 和 FROM 之间的内容
    if let Some(from_pos) = sql.to_uppercase().find(" FROM ") {
        let select_part = &sql[7..from_pos].trim();

        if *select_part == "*" {
            return Ok(SelectClause::All);
        }

        // 解析字段列表
        let fields = select_part
            .split(',')
            .map(|field| {
                let field = field.trim();
                if let Some(alias_pos) = field.to_uppercase().find(" AS ") {
                    // 处理别名
                    let (name, alias) = field.split_at(alias_pos);
                    Ok(Field {
                        name: name.trim().to_string(),
                        alias: Some(alias[3..].trim().to_string()),
                    })
                } else {
                    Ok(Field {
                        name: field.to_string(),
                        alias: None,
                    })
                }
            })
            .collect::<Result<Vec<Field>>>()?;

        Ok(SelectClause::Fields(fields))
    } else {
        Err(NasError::Other("缺少 FROM 子句".to_string()))
    }
}

/// 解析 FROM 子句
fn parse_from_clause(sql: &str) -> Result<FromClause> {
    let sql = sql.trim();

    // 查找 FROM 关键字
    if let Some(from_pos) = sql.to_uppercase().find(" FROM ") {
        let from_part_start = from_pos + 6;
        let from_part =
            if let Some(where_pos) = sql[from_part_start..].to_uppercase().find(" WHERE ") {
                &sql[from_part_start..from_part_start + where_pos]
            } else if let Some(limit_pos) = sql[from_part_start..].to_uppercase().find(" LIMIT ") {
                &sql[from_part_start..from_part_start + limit_pos]
            } else {
                &sql[from_part_start..]
            }
            .trim();

        if from_part.is_empty() {
            return Err(NasError::Other("FROM 子句不能为空".to_string()));
        }

        Ok(FromClause {
            source_type: from_part.to_string(),
        })
    } else {
        Err(NasError::Other("缺少 FROM 子句".to_string()))
    }
}

/// 解析 WHERE 子句（可选）
fn parse_where_clause(sql: &str) -> Result<Option<WhereClause>> {
    let sql = sql.trim();

    // 查找 WHERE 关键字
    if let Some(where_pos) = sql.to_uppercase().find(" WHERE ") {
        let where_part_start = where_pos + 7;
        let where_part =
            if let Some(limit_pos) = sql[where_part_start..].to_uppercase().find(" LIMIT ") {
                &sql[where_part_start..where_part_start + limit_pos]
            } else {
                &sql[where_part_start..]
            }
            .trim();

        if where_part.is_empty() {
            return Err(NasError::Other("WHERE 子句不能为空".to_string()));
        }

        // 解析简单条件
        let conditions = parse_conditions(where_part)?;

        Ok(Some(WhereClause { conditions }))
    } else {
        Ok(None)
    }
}

/// 解析条件列表
fn parse_conditions(where_part: &str) -> Result<Vec<Condition>> {
    let conditions = parse_comparison_conditions(where_part)?;
    Ok(conditions)
}

/// 解析比较条件
fn parse_comparison_conditions(where_part: &str) -> Result<Vec<Condition>> {
    let mut conditions = Vec::new();
    let remaining = where_part.trim();

    // 简单的条件解析（处理 "AND" 连接的条件）
    if remaining.to_uppercase().contains(" AND ") {
        for part in remaining.split(" AND ") {
            let condition = parse_single_condition(part.trim())?;
            conditions.push(condition);
        }
    } else {
        let condition = parse_single_condition(remaining)?;
        conditions.push(condition);
    }

    Ok(conditions)
}

/// 解析单个条件
fn parse_single_condition(condition: &str) -> Result<Condition> {
    let condition = condition.trim();

    // 解析比较操作符
    let operators = [
        ("!=", Operator::NotEqual),
        ("<>", Operator::NotEqual),
        ("<=", Operator::LessThanOrEqual),
        (">=", Operator::GreaterThanOrEqual),
        ("=", Operator::Equal),
        ("<", Operator::LessThan),
        (">", Operator::GreaterThan),
        (" LIKE ", Operator::Like),
    ];

    for (op_str, operator) in &operators {
        if condition.to_uppercase().contains(op_str) {
            let parts: Vec<&str> = if op_str.trim() == "" {
                // 特殊处理 LIKE 空格
                let like_pos = condition.to_uppercase().find("LIKE").unwrap();
                let left = &condition[..like_pos];
                let right = &condition[like_pos + 4..];
                vec![left.trim(), right.trim()]
            } else {
                let op_pos = condition.to_uppercase().find(op_str).unwrap();
                let left = &condition[..op_pos];
                let right = &condition[op_pos + op_str.len()..];
                vec![left.trim(), right.trim()]
            };

            if parts.len() == 2 {
                let left_operand = parse_operand(parts[0])?;
                let right_operand = parse_operand(parts[1])?;

                return Ok(Condition::Comparison(Comparison {
                    left: left_operand,
                    operator: operator.clone(),
                    right: right_operand,
                }));
            }
        }
    }

    Err(NasError::Other(format!("无法解析条件: {}", condition)))
}

/// 解析操作数
fn parse_operand(operand: &str) -> Result<Operand> {
    let operand = operand.trim();

    // 字符串字面量（单引号）
    if operand.starts_with('\'') && operand.ends_with('\'') {
        let value = &operand[1..operand.len() - 1];
        return Ok(Operand::Literal(Literal::String(value.to_string())));
    }

    // 数字
    if let Ok(num) = operand.parse::<f64>() {
        return Ok(Operand::Literal(Literal::Number(num)));
    }

    // 布尔值
    if operand.to_uppercase() == "TRUE" {
        return Ok(Operand::Literal(Literal::Boolean(true)));
    }
    if operand.to_uppercase() == "FALSE" {
        return Ok(Operand::Literal(Literal::Boolean(false)));
    }

    // NULL
    if operand.to_uppercase() == "NULL" {
        return Ok(Operand::Literal(Literal::Null));
    }

    // 字段名
    Ok(Operand::Field(operand.to_string()))
}

/// 解析 LIMIT 子句（可选）
fn parse_limit_clause(sql: &str) -> Result<Option<u64>> {
    let sql = sql.trim();

    // 查找 LIMIT 关键字
    if let Some(limit_pos) = sql.to_uppercase().find(" LIMIT ") {
        let limit_part = &sql[limit_pos + 7..].trim();

        if let Ok(limit) = limit_part.parse::<u64>() {
            Ok(Some(limit))
        } else {
            Err(NasError::Other("无效的 LIMIT 值".to_string()))
        }
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_select() {
        let sql = "SELECT * FROM s3object";
        let result = parse_sql(sql).unwrap();

        match result.select {
            SelectClause::All => {}
            _ => panic!("应该选择所有字段"),
        }
        assert_eq!(result.from.source_type, "s3object");
        assert!(result.where_clause.is_none());
    }

    #[test]
    fn test_parse_select_with_where() {
        let sql = "SELECT * FROM s3object WHERE size > 100";
        let result = parse_sql(sql).unwrap();

        assert!(result.where_clause.is_some());
        let where_clause = result.where_clause.unwrap();
        assert_eq!(where_clause.conditions.len(), 1);
    }

    #[test]
    fn test_parse_select_with_fields() {
        let sql = "SELECT name, size FROM s3object";
        let result = parse_sql(sql).unwrap();

        match result.select {
            SelectClause::Fields(fields) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].name, "name");
                assert_eq!(fields[1].name, "size");
            }
            _ => panic!("应该选择指定字段"),
        }
    }

    #[test]
    fn test_parse_select_with_alias() {
        let sql = "SELECT name AS file_name FROM s3object";
        let result = parse_sql(sql).unwrap();

        match result.select {
            SelectClause::Fields(fields) => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].name, "name");
                assert_eq!(fields[0].alias, Some("file_name".to_string()));
            }
            _ => panic!("应该选择指定字段"),
        }
    }

    #[test]
    fn test_parse_select_with_limit() {
        let sql = "SELECT * FROM s3object LIMIT 10";
        let result = parse_sql(sql).unwrap();

        assert_eq!(result.limit, Some(10));
    }

    #[test]
    fn test_parse_operand() {
        // 字符串
        let operand = parse_operand("'test'").unwrap();
        assert!(matches!(operand, Operand::Literal(Literal::String(_))));

        // 数字
        let operand = parse_operand("123.45").unwrap();
        assert!(matches!(operand, Operand::Literal(Literal::Number(_))));

        // 布尔值
        let operand = parse_operand("TRUE").unwrap();
        assert!(matches!(operand, Operand::Literal(Literal::Boolean(true))));

        // 字段
        let operand = parse_operand("size").unwrap();
        assert!(matches!(operand, Operand::Field(_)));
    }
}
