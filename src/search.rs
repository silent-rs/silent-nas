use crate::error::{NasError, Result};
use crate::models::FileMetadata;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tantivy::schema::*;
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, doc};
use tokio::sync::RwLock;
use tracing::{debug, info};

/// 搜索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// 文件 ID
    pub file_id: String,
    /// 文件路径
    pub path: String,
    /// 文件名
    pub name: String,
    /// 文件大小
    pub size: u64,
    /// 修改时间
    pub modified_at: i64,
    /// 相关性分数
    pub score: f32,
}

/// 搜索引擎
pub struct SearchEngine {
    /// 索引
    index: Arc<Index>,
    /// 索引读取器
    reader: Arc<IndexReader>,
    /// 索引写入器
    writer: Arc<RwLock<IndexWriter>>,
    /// Schema 字段
    schema_fields: SchemaFields,
}

/// Schema 字段定义
#[derive(Clone)]
struct SchemaFields {
    file_id: Field,
    path: Field,
    name: Field,
    size: Field,
    modified_at: Field,
    #[allow(dead_code)]
    content: Field,
}

impl SearchEngine {
    /// 创建新的搜索引擎
    pub fn new(index_path: PathBuf) -> Result<Self> {
        // 创建索引目录
        std::fs::create_dir_all(&index_path)
            .map_err(|e| NasError::Storage(format!("创建索引目录失败: {}", e)))?;

        // 定义 Schema
        let mut schema_builder = Schema::builder();

        let file_id = schema_builder.add_text_field("file_id", STRING | STORED);
        let path = schema_builder.add_text_field("path", TEXT | STORED);
        let name = schema_builder.add_text_field("name", TEXT | STORED);
        let size = schema_builder.add_u64_field("size", INDEXED | STORED);
        let modified_at = schema_builder.add_i64_field("modified_at", INDEXED | STORED);
        let content = schema_builder.add_text_field("content", TEXT);

        let schema = schema_builder.build();

        // 打开或创建索引
        let index = if index_path.join("meta.json").exists() {
            Index::open_in_dir(&index_path)
                .map_err(|e| NasError::Storage(format!("打开索引失败: {}", e)))?
        } else {
            Index::create_in_dir(&index_path, schema.clone())
                .map_err(|e| NasError::Storage(format!("创建索引失败: {}", e)))?
        };

        // 创建索引写入器
        let writer = index
            .writer(50_000_000) // 50MB buffer
            .map_err(|e| NasError::Storage(format!("创建索引写入器失败: {}", e)))?;

        // 创建索引读取器（使用 Manual 策略，手动控制重载）
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|e| NasError::Storage(format!("创建索引读取器失败: {}", e)))?;

        info!("搜索引擎已初始化: {:?}", index_path);

        Ok(Self {
            index: Arc::new(index),
            reader: Arc::new(reader),
            writer: Arc::new(RwLock::new(writer)),
            schema_fields: SchemaFields {
                file_id,
                path,
                name,
                size,
                modified_at,
                content,
            },
        })
    }

    /// 索引单个文件
    pub async fn index_file(&self, file_meta: &FileMetadata) -> Result<()> {
        let fields = &self.schema_fields;

        let doc = doc!(
            fields.file_id => file_meta.id.clone(),
            fields.path => file_meta.path.clone(),
            fields.name => file_meta.name.clone(),
            fields.size => file_meta.size,
            fields.modified_at => file_meta.modified_at.and_utc().timestamp(),
        );

        let writer = self.writer.write().await;
        writer
            .add_document(doc)
            .map_err(|e| NasError::Storage(format!("添加文档到索引失败: {}", e)))?;

        debug!("文件已索引: {} ({})", file_meta.name, file_meta.id);
        Ok(())
    }

    /// 批量索引文件
    pub async fn index_files(&self, files: &[FileMetadata]) -> Result<()> {
        let fields = &self.schema_fields;
        let writer = self.writer.write().await;

        for file_meta in files {
            let doc = doc!(
                fields.file_id => file_meta.id.clone(),
                fields.path => file_meta.path.clone(),
                fields.name => file_meta.name.clone(),
                fields.size => file_meta.size,
                fields.modified_at => file_meta.modified_at.and_utc().timestamp(),
            );

            writer
                .add_document(doc)
                .map_err(|e| NasError::Storage(format!("添加文档到索引失败: {}", e)))?;
        }

        info!("批量索引完成: {} 个文件", files.len());
        Ok(())
    }

    /// 提交索引更改
    pub async fn commit(&self) -> Result<()> {
        let mut writer = self.writer.write().await;
        writer
            .commit()
            .map_err(|e| NasError::Storage(format!("提交索引失败: {}", e)))?;
        drop(writer);

        // 手动重载索引读取器
        self.reader
            .reload()
            .map_err(|e| NasError::Storage(format!("重载索引失败: {}", e)))?;

        debug!("索引已提交并重载");
        Ok(())
    }

    /// 删除文件索引
    pub async fn delete_file(&self, file_id: &str) -> Result<()> {
        let fields = &self.schema_fields;
        let writer = self.writer.write().await;

        writer.delete_term(Term::from_field_text(fields.file_id, file_id));

        debug!("文件索引已删除: {}", file_id);
        Ok(())
    }

    /// 搜索文件
    pub async fn search(
        &self,
        query_str: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SearchResult>> {
        use tantivy::collector::TopDocs;
        use tantivy::query::QueryParser;

        let searcher = self.reader.searcher();
        let fields = &self.schema_fields;

        // 创建查询解析器，搜索 path 和 name 字段
        let query_parser = QueryParser::for_index(&self.index, vec![fields.path, fields.name]);

        let query = query_parser
            .parse_query(query_str)
            .map_err(|e| NasError::Storage(format!("解析搜索查询失败: {}", e)))?;

        // 执行搜索
        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit + offset))
            .map_err(|e| NasError::Storage(format!("搜索失败: {}", e)))?;

        // 转换结果
        let mut results = Vec::new();
        for (_score, doc_address) in top_docs.into_iter().skip(offset) {
            let retrieved_doc: TantivyDocument = searcher
                .doc(doc_address)
                .map_err(|e| NasError::Storage(format!("获取文档失败: {}", e)))?;

            let file_id = retrieved_doc
                .get_first(fields.file_id)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let path = retrieved_doc
                .get_first(fields.path)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let name = retrieved_doc
                .get_first(fields.name)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let size = retrieved_doc
                .get_first(fields.size)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            let modified_at = retrieved_doc
                .get_first(fields.modified_at)
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            results.push(SearchResult {
                file_id,
                path,
                name,
                size,
                modified_at,
                score: _score,
            });
        }

        debug!("搜索完成: 找到 {} 个结果", results.len());
        Ok(results)
    }

    /// 按文件名搜索
    pub async fn search_by_name(&self, name: &str, limit: usize) -> Result<Vec<SearchResult>> {
        self.search(name, limit, 0).await
    }

    /// 重建索引（从存储管理器获取所有文件）
    pub async fn rebuild_index(&self, files: &[FileMetadata]) -> Result<()> {
        info!("开始重建索引...");

        // 清空现有索引
        let mut writer = self.writer.write().await;
        writer
            .delete_all_documents()
            .map_err(|e| NasError::Storage(format!("清空索引失败: {}", e)))?;
        writer
            .commit()
            .map_err(|e| NasError::Storage(format!("提交清空失败: {}", e)))?;
        drop(writer);

        // 重新索引所有文件
        self.index_files(files).await?;
        self.commit().await?;

        info!("索引重建完成: {} 个文件", files.len());
        Ok(())
    }

    /// 获取索引统计信息
    pub fn get_stats(&self) -> IndexStats {
        let searcher = self.reader.searcher();
        let num_docs = searcher.num_docs() as usize;

        IndexStats {
            total_documents: num_docs,
            index_size: 0, // TODO: 计算索引大小
        }
    }
}

/// 索引统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub total_documents: usize,
    pub index_size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    fn create_test_metadata(id: &str, name: &str, path: &str) -> FileMetadata {
        FileMetadata {
            id: id.to_string(),
            name: name.to_string(),
            path: path.to_string(),
            size: 1024,
            hash: "test_hash".to_string(),
            created_at: Utc::now().naive_local(),
            modified_at: Utc::now().naive_local(),
        }
    }

    #[tokio::test]
    async fn test_search_engine_creation() {
        let temp_dir = TempDir::new().unwrap();
        let index_path = temp_dir.path().join("index");

        let engine = SearchEngine::new(index_path).unwrap();
        assert!(engine.get_stats().total_documents == 0);
    }

    #[tokio::test]
    async fn test_index_and_search() {
        let temp_dir = TempDir::new().unwrap();
        let index_path = temp_dir.path().join("index");

        let engine = SearchEngine::new(index_path).unwrap();

        // 索引测试文件
        let file1 = create_test_metadata("1", "test.txt", "/files/test.txt");
        let file2 = create_test_metadata("2", "report.pdf", "/documents/report.pdf");

        engine.index_file(&file1).await.unwrap();
        engine.index_file(&file2).await.unwrap();
        engine.commit().await.unwrap();

        // 搜索
        let results = engine.search("test", 10, 0).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "test.txt");
    }

    #[tokio::test]
    async fn test_delete_file() {
        let temp_dir = TempDir::new().unwrap();
        let index_path = temp_dir.path().join("index");

        let engine = SearchEngine::new(index_path).unwrap();

        let file = create_test_metadata("1", "test.txt", "/files/test.txt");
        engine.index_file(&file).await.unwrap();
        engine.commit().await.unwrap();

        // 删除
        engine.delete_file("1").await.unwrap();
        engine.commit().await.unwrap();

        // 搜索应该找不到
        let results = engine.search("test", 10, 0).await.unwrap();
        assert_eq!(results.len(), 0);
    }
}
