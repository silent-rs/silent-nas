//! WebDAV 秒传功能
//!
//! 通过文件哈希快速判断文件是否已存在，实现秒传

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 秒传索引条目
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct InstantUploadEntry {
    /// 文件哈希 (SHA-256)
    pub file_hash: String,
    /// 文件大小
    pub file_size: u64,
    /// 文件路径列表 (同一文件可能有多个路径)
    pub file_paths: Vec<String>,
    /// 创建时间
    pub created_at: chrono::NaiveDateTime,
    /// 最后访问时间
    pub last_accessed: chrono::NaiveDateTime,
}

/// 秒传管理器
#[allow(dead_code)]
pub struct InstantUploadManager {
    /// 哈希索引 (file_hash -> InstantUploadEntry)
    index: Arc<RwLock<HashMap<String, InstantUploadEntry>>>,
}

impl InstantUploadManager {
    /// 创建新的秒传管理器
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 检查文件是否可以秒传
    ///
    /// # 返回
    /// - `Some(file_path)`: 文件已存在，返回已有文件路径
    /// - `None`: 文件不存在，需要正常上传
    #[allow(dead_code)]
    pub async fn check_instant_upload(&self, file_hash: &str, file_size: u64) -> Option<String> {
        let index = self.index.read().await;

        if let Some(entry) = index.get(file_hash) {
            // 验证文件大小匹配
            if entry.file_size == file_size && !entry.file_paths.is_empty() {
                tracing::info!(
                    "秒传命中: hash={}, size={}, existing_paths={:?}",
                    file_hash,
                    file_size,
                    entry.file_paths
                );
                return Some(entry.file_paths[0].clone());
            }
        }

        None
    }

    /// 添加秒传索引
    #[allow(dead_code)]
    pub async fn add_entry(&self, file_hash: String, file_size: u64, file_path: String) {
        let mut index = self.index.write().await;
        let now = chrono::Local::now().naive_local();

        if let Some(entry) = index.get_mut(&file_hash) {
            // 更新现有条目
            if !entry.file_paths.contains(&file_path) {
                entry.file_paths.push(file_path);
            }
            entry.last_accessed = now;
        } else {
            // 创建新条目
            let entry = InstantUploadEntry {
                file_hash: file_hash.clone(),
                file_size,
                file_paths: vec![file_path],
                created_at: now,
                last_accessed: now,
            };
            index.insert(file_hash, entry);
        }
    }

    /// 删除秒传索引
    #[allow(dead_code)]
    pub async fn remove_entry(&self, file_hash: &str) {
        let mut index = self.index.write().await;
        index.remove(file_hash);
    }

    /// 从存储管理器重建索引
    ///
    /// 注意: 当前实现为预留接口，实际使用时需要遍历存储中的所有文件
    /// 由于 StorageManagerTrait 没有提供遍历所有文件的方法，
    /// 实际使用时需要在上传时主动调用 add_entry 来建立索引
    #[allow(dead_code)]
    pub async fn rebuild_index(&self) -> Result<usize, String> {
        // 预留接口，暂不实现
        // 实际使用时应在文件上传成功后调用 add_entry 建立索引
        tracing::warn!("rebuild_index 是预留接口，当前不执行实际操作");
        Ok(0)
    }

    /// 获取索引统计信息
    #[allow(dead_code)]
    pub async fn get_stats(&self) -> (usize, u64) {
        let index = self.index.read().await;
        let entry_count = index.len();
        let total_size: u64 = index.values().map(|e| e.file_size).sum();
        (entry_count, total_size)
    }

    /// 清理未使用的索引条目
    ///
    /// 移除指定时间内未访问的条目
    #[allow(dead_code)]
    pub async fn cleanup_unused(&self, days: i64) -> usize {
        let mut index = self.index.write().await;
        let now = chrono::Local::now().naive_local();
        let threshold = now - chrono::Duration::days(days);

        let to_remove: Vec<String> = index
            .iter()
            .filter(|(_, entry)| entry.last_accessed < threshold)
            .map(|(hash, _)| hash.clone())
            .collect();

        let count = to_remove.len();
        for hash in to_remove {
            index.remove(&hash);
        }

        if count > 0 {
            tracing::info!("清理未使用秒传索引: {} 个条目", count);
        }

        count
    }
}

impl Default for InstantUploadManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_instant_upload_manager_new() {
        let manager = InstantUploadManager::new();
        let (count, size) = manager.get_stats().await;
        assert_eq!(count, 0);
        assert_eq!(size, 0);
    }

    #[tokio::test]
    async fn test_add_and_check() {
        let manager = InstantUploadManager::new();
        let hash = "abc123".to_string();
        let size = 1000;
        let path = "/test/file.txt".to_string();

        // 添加条目
        manager.add_entry(hash.clone(), size, path.clone()).await;

        // 检查秒传
        let result = manager.check_instant_upload(&hash, size).await;
        assert!(result.is_some());
        assert_eq!(result.unwrap(), path);

        // 检查不匹配的大小
        let result2 = manager.check_instant_upload(&hash, size + 1).await;
        assert!(result2.is_none());

        // 检查不存在的哈希
        let result3 = manager.check_instant_upload("xyz789", size).await;
        assert!(result3.is_none());
    }

    #[tokio::test]
    async fn test_add_multiple_paths() {
        let manager = InstantUploadManager::new();
        let hash = "abc123".to_string();
        let size = 1000;

        // 添加同一文件的多个路径
        manager
            .add_entry(hash.clone(), size, "/path1/file.txt".to_string())
            .await;
        manager
            .add_entry(hash.clone(), size, "/path2/file.txt".to_string())
            .await;

        // 验证两个路径都被记录
        let index = manager.index.read().await;
        let entry = index.get(&hash).unwrap();
        assert_eq!(entry.file_paths.len(), 2);
        assert!(entry.file_paths.contains(&"/path1/file.txt".to_string()));
        assert!(entry.file_paths.contains(&"/path2/file.txt".to_string()));
    }

    #[tokio::test]
    async fn test_remove_entry() {
        let manager = InstantUploadManager::new();
        let hash = "abc123".to_string();
        let size = 1000;
        let path = "/test/file.txt".to_string();

        // 添加并验证
        manager.add_entry(hash.clone(), size, path).await;
        assert!(manager.check_instant_upload(&hash, size).await.is_some());

        // 删除并验证
        manager.remove_entry(&hash).await;
        assert!(manager.check_instant_upload(&hash, size).await.is_none());
    }

    #[tokio::test]
    async fn test_get_stats() {
        let manager = InstantUploadManager::new();

        manager
            .add_entry("hash1".to_string(), 1000, "/file1.txt".to_string())
            .await;
        manager
            .add_entry("hash2".to_string(), 2000, "/file2.txt".to_string())
            .await;

        let (count, total_size) = manager.get_stats().await;
        assert_eq!(count, 2);
        assert_eq!(total_size, 3000);
    }

    #[tokio::test]
    async fn test_cleanup_unused() {
        let manager = InstantUploadManager::new();
        let hash = "old_hash".to_string();
        let size = 1000;
        let path = "/old/file.txt".to_string();

        // 添加一个条目并手动设置旧的访问时间
        manager.add_entry(hash.clone(), size, path).await;

        {
            let mut index = manager.index.write().await;
            if let Some(entry) = index.get_mut(&hash) {
                let old_time = chrono::Local::now().naive_local() - chrono::Duration::days(100);
                entry.last_accessed = old_time;
            }
        }

        // 清理 30 天未访问的条目
        let count = manager.cleanup_unused(30).await;
        assert_eq!(count, 1);

        // 验证条目已被删除
        let (total, _) = manager.get_stats().await;
        assert_eq!(total, 0);
    }
}
