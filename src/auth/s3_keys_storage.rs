//! S3 访问密钥存储层

use super::models::S3AccessKey;
#[cfg(test)]
use super::models::S3KeyStatus;
use crate::error::{NasError, Result};
use chrono::Local;
use std::path::Path;

/// S3 访问密钥存储
pub struct S3KeyStorage {
    db: sled::Db,
    keys_tree: sled::Tree,
    user_keys_index: sled::Tree,
    access_key_index: sled::Tree,
}

impl S3KeyStorage {
    /// 创建 S3 密钥存储
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db =
            sled::open(path).map_err(|e| NasError::Storage(format!("打开数据库失败: {}", e)))?;

        let keys_tree = db
            .open_tree("s3_keys")
            .map_err(|e| NasError::Storage(format!("打开 S3 密钥表失败: {}", e)))?;

        let user_keys_index = db
            .open_tree("s3_user_keys_index")
            .map_err(|e| NasError::Storage(format!("打开用户密钥索引失败: {}", e)))?;

        let access_key_index = db
            .open_tree("s3_access_key_index")
            .map_err(|e| NasError::Storage(format!("打开访问密钥索引失败: {}", e)))?;

        Ok(Self {
            db,
            keys_tree,
            user_keys_index,
            access_key_index,
        })
    }

    /// 创建 S3 密钥
    pub fn create_key(&self, key: S3AccessKey) -> Result<S3AccessKey> {
        // 检查访问密钥是否已存在
        if self.access_key_index.contains_key(&key.access_key)? {
            return Err(NasError::Auth(format!(
                "访问密钥已存在: {}",
                key.access_key
            )));
        }

        // 序列化密钥
        let key_json = serde_json::to_string(&key)
            .map_err(|e| NasError::Storage(format!("序列化 S3 密钥失败: {}", e)))?;

        // 存储密钥
        self.keys_tree.insert(&key.id, key_json.as_bytes())?;

        // 建立访问密钥索引
        self.access_key_index
            .insert(&key.access_key, key.id.as_bytes())?;

        // 建立用户密钥索引 (user_id:key_id -> "")
        let user_key_id = format!("{}:{}", key.user_id, key.id);
        self.user_keys_index.insert(user_key_id.as_bytes(), b"")?;

        // 刷新到磁盘
        self.db.flush()?;

        Ok(key)
    }

    /// 根据 ID 获取密钥
    pub fn get_key_by_id(&self, key_id: &str) -> Result<Option<S3AccessKey>> {
        let Some(bytes) = self.keys_tree.get(key_id)? else {
            return Ok(None);
        };

        let key_json = std::str::from_utf8(&bytes)
            .map_err(|e| NasError::Storage(format!("解析 JSON 失败: {}", e)))?;
        let key: S3AccessKey = serde_json::from_str(key_json)
            .map_err(|e| NasError::Storage(format!("反序列化 S3 密钥失败: {}", e)))?;

        Ok(Some(key))
    }

    /// 根据访问密钥获取密钥
    pub fn get_key_by_access_key(&self, access_key: &str) -> Result<Option<S3AccessKey>> {
        let Some(key_id_bytes) = self.access_key_index.get(access_key)? else {
            return Ok(None);
        };

        let key_id = String::from_utf8(key_id_bytes.to_vec())
            .map_err(|e| NasError::Storage(format!("解析密钥 ID 失败: {}", e)))?;

        self.get_key_by_id(&key_id)
    }

    /// 根据用户 ID 获取所有密钥
    pub fn get_keys_by_user_id(&self, user_id: &str) -> Result<Vec<S3AccessKey>> {
        let mut keys = Vec::new();
        let prefix = format!("{}:", user_id);

        for item in self.user_keys_index.scan_prefix(prefix.as_bytes()) {
            let (key_bytes, _) = item?;
            let user_key_id = String::from_utf8(key_bytes.to_vec())
                .map_err(|e| NasError::Storage(format!("解析用户密钥 ID 失败: {}", e)))?;

            // 从 "user_id:key_id" 中提取 key_id
            let parts: Vec<&str> = user_key_id.split(':').collect();
            if parts.len() == 2
                && let Some(key) = self.get_key_by_id(parts[1])?
            {
                keys.push(key);
            }
        }

        Ok(keys)
    }

    /// 更新密钥
    pub fn update_key(&self, key: S3AccessKey) -> Result<S3AccessKey> {
        // 检查密钥是否存在
        let _old_key = self
            .get_key_by_id(&key.id)?
            .ok_or_else(|| NasError::Auth(format!("S3 密钥不存在: {}", key.id)))?;

        // 更新密钥
        let key_json = serde_json::to_string(&key)
            .map_err(|e| NasError::Storage(format!("序列化 S3 密钥失败: {}", e)))?;
        self.keys_tree.insert(&key.id, key_json.as_bytes())?;

        self.db.flush()?;

        Ok(key)
    }

    /// 删除密钥
    pub fn delete_key(&self, key_id: &str) -> Result<()> {
        let key = self
            .get_key_by_id(key_id)?
            .ok_or_else(|| NasError::Auth(format!("S3 密钥不存在: {}", key_id)))?;

        // 删除密钥
        self.keys_tree.remove(key_id)?;

        // 删除访问密钥索引
        self.access_key_index.remove(&key.access_key)?;

        // 删除用户密钥索引
        let user_key_id = format!("{}:{}", key.user_id, key.id);
        self.user_keys_index.remove(user_key_id.as_bytes())?;

        self.db.flush()?;

        Ok(())
    }

    /// 更新最后使用时间
    pub fn update_last_used(&self, key_id: &str) -> Result<()> {
        let mut key = self
            .get_key_by_id(key_id)?
            .ok_or_else(|| NasError::Auth(format!("S3 密钥不存在: {}", key_id)))?;

        key.last_used_at = Some(Local::now());
        self.update_key(key)?;

        Ok(())
    }

    /// 列出所有密钥
    pub fn list_all_keys(&self) -> Result<Vec<S3AccessKey>> {
        let mut keys = Vec::new();

        for item in self.keys_tree.iter() {
            let (_key, value) = item?;
            let key_json = std::str::from_utf8(&value)
                .map_err(|e| NasError::Storage(format!("解析 JSON 失败: {}", e)))?;
            let key: S3AccessKey = serde_json::from_str(key_json)
                .map_err(|e| NasError::Storage(format!("反序列化 S3 密钥失败: {}", e)))?;

            keys.push(key);
        }

        Ok(keys)
    }

    /// 计数用户的密钥数量
    pub fn count_user_keys(&self, user_id: &str) -> Result<usize> {
        let keys = self.get_keys_by_user_id(user_id)?;
        Ok(keys.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_storage() -> (S3KeyStorage, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = S3KeyStorage::new(temp_dir.path()).unwrap();
        (storage, temp_dir)
    }

    fn create_test_key(user_id: &str, access_key: &str) -> S3AccessKey {
        S3AccessKey {
            id: scru128::new_string(),
            user_id: user_id.to_string(),
            access_key: access_key.to_string(),
            secret_key: "test_secret_key".to_string(),
            description: "Test Key".to_string(),
            status: S3KeyStatus::Active,
            created_at: Local::now(),
            last_used_at: None,
        }
    }

    #[test]
    fn test_create_and_get_key() {
        let (storage, _temp) = create_test_storage();
        let key = create_test_key("user123", "AKIAIOSFODNN7EXAMPLE");

        // 创建密钥
        let created = storage.create_key(key.clone()).unwrap();
        assert_eq!(created.access_key, key.access_key);

        // 根据 ID 获取
        let found = storage.get_key_by_id(&key.id).unwrap().unwrap();
        assert_eq!(found.access_key, key.access_key);

        // 根据访问密钥获取
        let found = storage
            .get_key_by_access_key("AKIAIOSFODNN7EXAMPLE")
            .unwrap()
            .unwrap();
        assert_eq!(found.id, key.id);
    }

    #[test]
    fn test_duplicate_access_key() {
        let (storage, _temp) = create_test_storage();
        let key1 = create_test_key("user1", "AKIAIOSFODNN7EXAMPLE");
        let key2 = create_test_key("user2", "AKIAIOSFODNN7EXAMPLE");

        storage.create_key(key1).unwrap();
        let result = storage.create_key(key2);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_keys_by_user_id() {
        let (storage, _temp) = create_test_storage();

        // 为同一用户创建多个密钥
        for i in 0..3 {
            let key = create_test_key("user123", &format!("AKIA{}", i));
            storage.create_key(key).unwrap();
        }

        // 为另一用户创建密钥
        let key = create_test_key("user456", "AKIA999");
        storage.create_key(key).unwrap();

        // 获取用户的所有密钥
        let keys = storage.get_keys_by_user_id("user123").unwrap();
        assert_eq!(keys.len(), 3);

        let keys = storage.get_keys_by_user_id("user456").unwrap();
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn test_update_key() {
        let (storage, _temp) = create_test_storage();
        let mut key = create_test_key("user123", "AKIAIOSFODNN7EXAMPLE");

        storage.create_key(key.clone()).unwrap();

        // 更新描述
        key.description = "Updated Description".to_string();
        storage.update_key(key.clone()).unwrap();

        let found = storage.get_key_by_id(&key.id).unwrap().unwrap();
        assert_eq!(found.description, "Updated Description");
    }

    #[test]
    fn test_delete_key() {
        let (storage, _temp) = create_test_storage();
        let key = create_test_key("user123", "AKIAIOSFODNN7EXAMPLE");

        storage.create_key(key.clone()).unwrap();
        storage.delete_key(&key.id).unwrap();

        let found = storage.get_key_by_id(&key.id).unwrap();
        assert!(found.is_none());

        // 确保索引也被删除
        let found = storage
            .get_key_by_access_key("AKIAIOSFODNN7EXAMPLE")
            .unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn test_update_last_used() {
        let (storage, _temp) = create_test_storage();
        let key = create_test_key("user123", "AKIAIOSFODNN7EXAMPLE");

        storage.create_key(key.clone()).unwrap();

        // 初始时没有最后使用时间
        let found = storage.get_key_by_id(&key.id).unwrap().unwrap();
        assert!(found.last_used_at.is_none());

        // 更新最后使用时间
        storage.update_last_used(&key.id).unwrap();

        let found = storage.get_key_by_id(&key.id).unwrap().unwrap();
        assert!(found.last_used_at.is_some());
    }

    #[test]
    fn test_list_all_keys() {
        let (storage, _temp) = create_test_storage();

        for i in 0..5 {
            let key = create_test_key(&format!("user{}", i), &format!("AKIA{}", i));
            storage.create_key(key).unwrap();
        }

        let keys = storage.list_all_keys().unwrap();
        assert_eq!(keys.len(), 5);
    }

    #[test]
    fn test_count_user_keys() {
        let (storage, _temp) = create_test_storage();

        for i in 0..3 {
            let key = create_test_key("user123", &format!("AKIA{}", i));
            storage.create_key(key).unwrap();
        }

        assert_eq!(storage.count_user_keys("user123").unwrap(), 3);
        assert_eq!(storage.count_user_keys("user456").unwrap(), 0);
    }
}
