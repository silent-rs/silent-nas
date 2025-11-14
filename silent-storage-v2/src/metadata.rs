//! Sled 元数据数据库封装
//!
//! 提供统一的元数据存储接口，替代 JSON 文件

use crate::VersionInfo;
use crate::error::{Result, StorageError};
use crate::storage::{ChunkRefCount, FileIndexEntry};
use serde::de::DeserializeOwned;
use std::path::Path;
use tracing::{debug, info};

/// Sled 数据库封装
///
/// 用于存储三种类型的元数据：
/// - 文件索引（file_index）
/// - 版本索引（version_index）
/// - 块引用计数（chunk_ref_count）
pub struct SledMetadataDb {
    /// Sled 数据库实例
    db: sled::Db,

    /// 文件索引树
    file_index_tree: sled::Tree,

    /// 版本索引树
    version_index_tree: sled::Tree,

    /// 块引用计数树
    chunk_ref_tree: sled::Tree,
}

impl SledMetadataDb {
    /// 打开或创建 Sled 数据库
    ///
    /// # 参数
    /// * `db_path` - 数据库路径
    pub fn open<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let db = sled::open(&db_path)
            .map_err(|e| StorageError::Database(format!("打开 Sled 数据库失败: {}", e)))?;

        // 打开三个独立的树
        let file_index_tree = db
            .open_tree("file_index")
            .map_err(|e| StorageError::Database(format!("打开 file_index 树失败: {}", e)))?;

        let version_index_tree = db
            .open_tree("version_index")
            .map_err(|e| StorageError::Database(format!("打开 version_index 树失败: {}", e)))?;

        let chunk_ref_tree = db
            .open_tree("chunk_ref_count")
            .map_err(|e| StorageError::Database(format!("打开 chunk_ref_count 树失败: {}", e)))?;

        info!("Sled 数据库初始化完成: {:?}", db_path.as_ref());

        Ok(Self {
            db,
            file_index_tree,
            version_index_tree,
            chunk_ref_tree,
        })
    }

    /// 刷新数据到磁盘
    pub async fn flush(&self) -> Result<()> {
        self.db
            .flush_async()
            .await
            .map_err(|e| StorageError::Database(format!("刷新数据库失败: {}", e)))?;
        Ok(())
    }

    // ========== 文件索引操作 ==========

    /// 保存文件索引条目
    pub fn put_file_index(&self, file_id: &str, entry: &FileIndexEntry) -> Result<()> {
        let value = serde_json::to_vec(entry).map_err(|e| StorageError::Serialization(e))?;

        self.file_index_tree
            .insert(file_id.as_bytes(), value)
            .map_err(|e| StorageError::Database(format!("插入文件索引失败: {}", e)))?;

        debug!("保存文件索引: {}", file_id);
        Ok(())
    }

    /// 获取文件索引条目
    pub fn get_file_index(&self, file_id: &str) -> Result<Option<FileIndexEntry>> {
        self.get_value(&self.file_index_tree, file_id)
    }

    /// 删除文件索引条目
    pub fn remove_file_index(&self, file_id: &str) -> Result<()> {
        self.file_index_tree
            .remove(file_id.as_bytes())
            .map_err(|e| StorageError::Database(format!("删除文件索引失败: {}", e)))?;

        debug!("删除文件索引: {}", file_id);
        Ok(())
    }

    /// 列出所有文件 ID
    pub fn list_file_ids(&self) -> Result<Vec<String>> {
        let mut file_ids = Vec::new();

        for item in self.file_index_tree.iter() {
            let (key, _) =
                item.map_err(|e| StorageError::Database(format!("遍历文件索引失败: {}", e)))?;

            let file_id = String::from_utf8_lossy(&key).to_string();
            file_ids.push(file_id);
        }

        Ok(file_ids)
    }

    /// 获取文件索引数量
    pub fn file_index_count(&self) -> usize {
        self.file_index_tree.len()
    }

    // ========== 版本索引操作 ==========

    /// 保存版本信息
    pub fn put_version_info(&self, version_id: &str, info: &VersionInfo) -> Result<()> {
        let value = serde_json::to_vec(info).map_err(|e| StorageError::Serialization(e))?;

        self.version_index_tree
            .insert(version_id.as_bytes(), value)
            .map_err(|e| StorageError::Database(format!("插入版本信息失败: {}", e)))?;

        debug!("保存版本信息: {}", version_id);
        Ok(())
    }

    /// 获取版本信息
    pub fn get_version_info(&self, version_id: &str) -> Result<Option<VersionInfo>> {
        self.get_value(&self.version_index_tree, version_id)
    }

    /// 删除版本信息
    pub fn remove_version_info(&self, version_id: &str) -> Result<()> {
        self.version_index_tree
            .remove(version_id.as_bytes())
            .map_err(|e| StorageError::Database(format!("删除版本信息失败: {}", e)))?;

        debug!("删除版本信息: {}", version_id);
        Ok(())
    }

    /// 列出指定文件的所有版本
    pub fn list_file_versions(&self, file_id: &str) -> Result<Vec<VersionInfo>> {
        let mut versions = Vec::new();

        for item in self.version_index_tree.iter() {
            let (_, value) =
                item.map_err(|e| StorageError::Database(format!("遍历版本索引失败: {}", e)))?;

            let version_info: VersionInfo =
                serde_json::from_slice(&value).map_err(|e| StorageError::Serialization(e))?;

            if version_info.file_id == file_id {
                versions.push(version_info);
            }
        }

        // 按创建时间降序排序
        versions.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(versions)
    }

    /// 获取版本索引数量
    pub fn version_index_count(&self) -> usize {
        self.version_index_tree.len()
    }

    // ========== 块引用计数操作 ==========

    /// 保存块引用计数
    pub fn put_chunk_ref(&self, chunk_id: &str, ref_count: &ChunkRefCount) -> Result<()> {
        let value = serde_json::to_vec(ref_count).map_err(|e| StorageError::Serialization(e))?;

        self.chunk_ref_tree
            .insert(chunk_id.as_bytes(), value)
            .map_err(|e| StorageError::Database(format!("插入块引用计数失败: {}", e)))?;

        debug!(
            "保存块引用计数: {} (ref_count={})",
            chunk_id, ref_count.ref_count
        );
        Ok(())
    }

    /// 获取块引用计数
    pub fn get_chunk_ref(&self, chunk_id: &str) -> Result<Option<ChunkRefCount>> {
        self.get_value(&self.chunk_ref_tree, chunk_id)
    }

    /// 删除块引用计数
    pub fn remove_chunk_ref(&self, chunk_id: &str) -> Result<()> {
        self.chunk_ref_tree
            .remove(chunk_id.as_bytes())
            .map_err(|e| StorageError::Database(format!("删除块引用计数失败: {}", e)))?;

        debug!("删除块引用计数: {}", chunk_id);
        Ok(())
    }

    /// 原子性增加块引用计数
    pub fn increment_chunk_ref(&self, chunk_id: &str) -> Result<usize> {
        self.update_chunk_ref_atomic(chunk_id, |count| count + 1)
    }

    /// 原子性减少块引用计数
    pub fn decrement_chunk_ref(&self, chunk_id: &str) -> Result<usize> {
        self.update_chunk_ref_atomic(chunk_id, |count| count.saturating_sub(1))
    }

    /// 原子性更新块引用计数
    fn update_chunk_ref_atomic<F>(&self, chunk_id: &str, update_fn: F) -> Result<usize>
    where
        F: Fn(usize) -> usize,
    {
        let result = self
            .chunk_ref_tree
            .update_and_fetch(chunk_id.as_bytes(), |old_value| {
                let mut ref_count = if let Some(bytes) = old_value {
                    serde_json::from_slice::<ChunkRefCount>(bytes).ok()?
                } else {
                    return None; // 块不存在
                };

                ref_count.ref_count = update_fn(ref_count.ref_count);

                serde_json::to_vec(&ref_count).ok()
            })
            .map_err(|e| StorageError::Database(format!("原子更新块引用计数失败: {}", e)))?;

        match result {
            Some(bytes) => {
                let ref_count: ChunkRefCount =
                    serde_json::from_slice(&bytes).map_err(|e| StorageError::Serialization(e))?;
                Ok(ref_count.ref_count)
            }
            None => Err(StorageError::Chunk(format!("块不存在: {}", chunk_id))),
        }
    }

    /// 列出所有引用计数为 0 的块
    pub fn list_orphaned_chunks(&self) -> Result<Vec<String>> {
        let mut orphaned = Vec::new();

        for item in self.chunk_ref_tree.iter() {
            let (key, value) =
                item.map_err(|e| StorageError::Database(format!("遍历块引用计数失败: {}", e)))?;

            let ref_count: ChunkRefCount =
                serde_json::from_slice(&value).map_err(|e| StorageError::Serialization(e))?;

            if ref_count.ref_count == 0 {
                let chunk_id = String::from_utf8_lossy(&key).to_string();
                orphaned.push(chunk_id);
            }
        }

        Ok(orphaned)
    }

    /// 获取块引用计数总数
    pub fn chunk_ref_count(&self) -> usize {
        self.chunk_ref_tree.len()
    }

    // ========== 通用辅助方法 ==========

    /// 从树中获取并反序列化值
    fn get_value<T: DeserializeOwned>(&self, tree: &sled::Tree, key: &str) -> Result<Option<T>> {
        match tree.get(key.as_bytes()) {
            Ok(Some(bytes)) => {
                let value =
                    serde_json::from_slice(&bytes).map_err(|e| StorageError::Serialization(e))?;
                Ok(Some(value))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(StorageError::Database(format!("读取数据失败: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_db() -> (SledMetadataDb, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db = SledMetadataDb::open(temp_dir.path().join("test.db")).unwrap();
        (db, temp_dir)
    }

    #[test]
    fn test_file_index_operations() {
        let (db, _temp) = create_test_db();
        let now = Local::now().naive_local();

        let entry = FileIndexEntry {
            file_id: "test_file".to_string(),
            latest_version_id: "v1".to_string(),
            version_count: 1,
            created_at: now,
            modified_at: now,
        };

        // 保存
        db.put_file_index("test_file", &entry).unwrap();

        // 读取
        let retrieved = db.get_file_index("test_file").unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().file_id, "test_file");

        // 列出
        let files = db.list_file_ids().unwrap();
        assert_eq!(files.len(), 1);
        assert!(files.contains(&"test_file".to_string()));

        // 删除
        db.remove_file_index("test_file").unwrap();
        assert!(db.get_file_index("test_file").unwrap().is_none());
    }

    #[test]
    fn test_version_index_operations() {
        let (db, _temp) = create_test_db();
        let now = Local::now().naive_local();

        let version = VersionInfo {
            version_id: "v1".to_string(),
            file_id: "test_file".to_string(),
            parent_version_id: None,
            file_size: 1024,
            chunk_count: 1,
            storage_size: 1024,
            created_at: now,
            is_current: true,
        };

        // 保存
        db.put_version_info("v1", &version).unwrap();

        // 读取
        let retrieved = db.get_version_info("v1").unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().version_id, "v1");

        // 列出文件版本
        let versions = db.list_file_versions("test_file").unwrap();
        assert_eq!(versions.len(), 1);

        // 删除
        db.remove_version_info("v1").unwrap();
        assert!(db.get_version_info("v1").unwrap().is_none());
    }

    #[test]
    fn test_chunk_ref_operations() {
        let (db, _temp) = create_test_db();

        let ref_count = ChunkRefCount {
            chunk_id: "chunk1".to_string(),
            ref_count: 5,
            size: 1024,
            path: PathBuf::from("/tmp/chunk1"),
        };

        // 保存
        db.put_chunk_ref("chunk1", &ref_count).unwrap();

        // 读取
        let retrieved = db.get_chunk_ref("chunk1").unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().ref_count, 5);

        // 原子增加
        let new_count = db.increment_chunk_ref("chunk1").unwrap();
        assert_eq!(new_count, 6);

        // 原子减少
        let new_count = db.decrement_chunk_ref("chunk1").unwrap();
        assert_eq!(new_count, 5);

        // 删除
        db.remove_chunk_ref("chunk1").unwrap();
        assert!(db.get_chunk_ref("chunk1").unwrap().is_none());
    }

    #[tokio::test]
    async fn test_flush() {
        let (db, _temp) = create_test_db();
        let now = Local::now().naive_local();

        let entry = FileIndexEntry {
            file_id: "test".to_string(),
            latest_version_id: "v1".to_string(),
            version_count: 1,
            created_at: now,
            modified_at: now,
        };

        db.put_file_index("test", &entry).unwrap();
        db.flush().await.unwrap();
    }
}
