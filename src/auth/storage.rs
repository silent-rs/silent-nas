//! 用户存储层

use super::models::{User, UserStatus};
#[cfg(test)]
use super::models::UserRole;
use crate::error::{NasError, Result};
use chrono::Local;
use std::path::Path;

/// 用户存储
pub struct UserStorage {
    db: sled::Db,
    users_tree: sled::Tree,
    username_index: sled::Tree,
    email_index: sled::Tree,
}

impl UserStorage {
    /// 创建用户存储
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = sled::open(path).map_err(|e| NasError::Storage(format!("打开数据库失败: {}", e)))?;

        let users_tree = db
            .open_tree("users")
            .map_err(|e| NasError::Storage(format!("打开用户表失败: {}", e)))?;

        let username_index = db
            .open_tree("username_index")
            .map_err(|e| NasError::Storage(format!("打开用户名索引失败: {}", e)))?;

        let email_index = db
            .open_tree("email_index")
            .map_err(|e| NasError::Storage(format!("打开邮箱索引失败: {}", e)))?;

        Ok(Self {
            db,
            users_tree,
            username_index,
            email_index,
        })
    }

    /// 创建用户
    pub fn create_user(&self, user: User) -> Result<User> {
        // 检查用户名是否已存在
        if self.username_index.contains_key(&user.username)? {
            return Err(NasError::Auth(format!("用户名已存在: {}", user.username)));
        }

        // 检查邮箱是否已存在
        if self.email_index.contains_key(&user.email)? {
            return Err(NasError::Auth(format!("邮箱已存在: {}", user.email)));
        }

        // 序列化用户
        let user_json = serde_json::to_string(&user)
            .map_err(|e| NasError::Storage(format!("序列化用户失败: {}", e)))?;
        let user_bytes = user_json.as_bytes();

        // 存储用户
        self.users_tree.insert(&user.id, user_bytes)?;

        // 建立索引
        self.username_index.insert(&user.username, user.id.as_bytes())?;
        self.email_index.insert(&user.email, user.id.as_bytes())?;

        // 刷新到磁盘
        self.db.flush()?;

        Ok(user)
    }

    /// 根据ID获取用户
    pub fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>> {
        let Some(bytes) = self.users_tree.get(user_id)? else {
            return Ok(None);
        };

        let user_json = std::str::from_utf8(&bytes)
            .map_err(|e| NasError::Storage(format!("解析JSON失败: {}", e)))?;
        let user: User = serde_json::from_str(user_json)
            .map_err(|e| NasError::Storage(format!("反序列化用户失败: {}", e)))?;

        Ok(Some(user))
    }

    /// 根据用户名获取用户
    pub fn get_user_by_username(&self, username: &str) -> Result<Option<User>> {
        let Some(user_id_bytes) = self.username_index.get(username)? else {
            return Ok(None);
        };

        let user_id = String::from_utf8(user_id_bytes.to_vec())
            .map_err(|e| NasError::Storage(format!("解析用户ID失败: {}", e)))?;

        self.get_user_by_id(&user_id)
    }

    /// 根据邮箱获取用户
    pub fn get_user_by_email(&self, email: &str) -> Result<Option<User>> {
        let Some(user_id_bytes) = self.email_index.get(email)? else {
            return Ok(None);
        };

        let user_id = String::from_utf8(user_id_bytes.to_vec())
            .map_err(|e| NasError::Storage(format!("解析用户ID失败: {}", e)))?;

        self.get_user_by_id(&user_id)
    }

    /// 更新用户
    pub fn update_user(&self, user: User) -> Result<User> {
        // 获取旧用户信息
        let old_user = self
            .get_user_by_id(&user.id)?
            .ok_or_else(|| NasError::Auth(format!("用户不存在: {}", user.id)))?;

        // 如果用户名变更，更新索引
        if old_user.username != user.username {
            if self.username_index.contains_key(&user.username)? {
                return Err(NasError::Auth(format!("用户名已存在: {}", user.username)));
            }
            self.username_index.remove(&old_user.username)?;
            self.username_index.insert(&user.username, user.id.as_bytes())?;
        }

        // 如果邮箱变更，更新索引
        if old_user.email != user.email {
            if self.email_index.contains_key(&user.email)? {
                return Err(NasError::Auth(format!("邮箱已存在: {}", user.email)));
            }
            self.email_index.remove(&old_user.email)?;
            self.email_index.insert(&user.email, user.id.as_bytes())?;
        }

        // 更新用户
        let user_json = serde_json::to_string(&user)
            .map_err(|e| NasError::Storage(format!("序列化用户失败: {}", e)))?;
        self.users_tree.insert(&user.id, user_json.as_bytes())?;

        self.db.flush()?;

        Ok(user)
    }

    /// 删除用户（软删除）
    pub fn delete_user(&self, user_id: &str) -> Result<()> {
        let mut user = self
            .get_user_by_id(user_id)?
            .ok_or_else(|| NasError::Auth(format!("用户不存在: {}", user_id)))?;

        user.status = UserStatus::Deleted;
        user.updated_at = Local::now();

        self.update_user(user)?;

        Ok(())
    }

    /// 列出所有用户
    pub fn list_users(&self) -> Result<Vec<User>> {
        let mut users = Vec::new();

        for item in self.users_tree.iter() {
            let (_key, value) = item?;
            let user_json = std::str::from_utf8(&value)
                .map_err(|e| NasError::Storage(format!("解析JSON失败: {}", e)))?;
            let user: User = serde_json::from_str(user_json)
                .map_err(|e| NasError::Storage(format!("反序列化用户失败: {}", e)))?;

            // 不返回已删除的用户
            if user.status != UserStatus::Deleted {
                users.push(user);
            }
        }

        Ok(users)
    }

    /// 计数用户
    pub fn count_users(&self) -> Result<usize> {
        let mut count = 0;
        for item in self.users_tree.iter() {
            let (_key, value) = item?;
            let user_json = std::str::from_utf8(&value)
                .map_err(|e| NasError::Storage(format!("解析JSON失败: {}", e)))?;
            let user: User = serde_json::from_str(user_json)
                .map_err(|e| NasError::Storage(format!("反序列化用户失败: {}", e)))?;
            if user.status != UserStatus::Deleted {
                count += 1;
            }
        }
        Ok(count)
    }

    /// 用户名是否存在
    pub fn username_exists(&self, username: &str) -> Result<bool> {
        Ok(self.username_index.contains_key(username)?)
    }

    /// 邮箱是否存在
    pub fn email_exists(&self, email: &str) -> Result<bool> {
        Ok(self.email_index.contains_key(email)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_storage() -> (UserStorage, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = UserStorage::new(temp_dir.path()).unwrap();
        (storage, temp_dir)
    }

    fn create_test_user(username: &str, email: &str) -> User {
        User {
            id: scru128::new_string(),
            username: username.to_string(),
            email: email.to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::User,
            status: UserStatus::Active,
            created_at: Local::now(),
            updated_at: Local::now(),
        }
    }

    #[test]
    fn test_create_and_get_user() {
        let (storage, _temp) = create_test_storage();
        let user = create_test_user("test", "test@example.com");

        // 创建用户
        let created = storage.create_user(user.clone()).unwrap();
        assert_eq!(created.username, user.username);

        // 根据ID获取
        let found = storage.get_user_by_id(&user.id).unwrap().unwrap();
        assert_eq!(found.username, user.username);

        // 根据用户名获取
        let found = storage.get_user_by_username("test").unwrap().unwrap();
        assert_eq!(found.id, user.id);

        // 根据邮箱获取
        let found = storage.get_user_by_email("test@example.com").unwrap().unwrap();
        assert_eq!(found.id, user.id);
    }

    #[test]
    fn test_duplicate_username() {
        let (storage, _temp) = create_test_storage();
        let user1 = create_test_user("test", "test1@example.com");
        let user2 = create_test_user("test", "test2@example.com");

        storage.create_user(user1).unwrap();
        let result = storage.create_user(user2);
        assert!(result.is_err());
    }

    #[test]
    fn test_duplicate_email() {
        let (storage, _temp) = create_test_storage();
        let user1 = create_test_user("test1", "test@example.com");
        let user2 = create_test_user("test2", "test@example.com");

        storage.create_user(user1).unwrap();
        let result = storage.create_user(user2);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_user() {
        let (storage, _temp) = create_test_storage();
        let mut user = create_test_user("test", "test@example.com");

        storage.create_user(user.clone()).unwrap();

        // 更新用户角色
        user.role = UserRole::Admin;
        storage.update_user(user.clone()).unwrap();

        let found = storage.get_user_by_id(&user.id).unwrap().unwrap();
        assert_eq!(found.role, UserRole::Admin);
    }

    #[test]
    fn test_delete_user() {
        let (storage, _temp) = create_test_storage();
        let user = create_test_user("test", "test@example.com");

        storage.create_user(user.clone()).unwrap();
        storage.delete_user(&user.id).unwrap();

        let found = storage.get_user_by_id(&user.id).unwrap().unwrap();
        assert_eq!(found.status, UserStatus::Deleted);
    }

    #[test]
    fn test_list_users() {
        let (storage, _temp) = create_test_storage();

        for i in 0..5 {
            let user = create_test_user(&format!("user{}", i), &format!("user{}@example.com", i));
            storage.create_user(user).unwrap();
        }

        let users = storage.list_users().unwrap();
        assert_eq!(users.len(), 5);
    }

    #[test]
    fn test_count_users() {
        let (storage, _temp) = create_test_storage();

        for i in 0..3 {
            let user = create_test_user(&format!("user{}", i), &format!("user{}@example.com", i));
            storage.create_user(user).unwrap();
        }

        assert_eq!(storage.count_users().unwrap(), 3);
    }

    #[test]
    fn test_username_exists() {
        let (storage, _temp) = create_test_storage();
        let user = create_test_user("test", "test@example.com");

        assert!(!storage.username_exists("test").unwrap());
        storage.create_user(user).unwrap();
        assert!(storage.username_exists("test").unwrap());
    }

    #[test]
    fn test_email_exists() {
        let (storage, _temp) = create_test_storage();
        let user = create_test_user("test", "test@example.com");

        assert!(!storage.email_exists("test@example.com").unwrap());
        storage.create_user(user).unwrap();
        assert!(storage.email_exists("test@example.com").unwrap());
    }
}
