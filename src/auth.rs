use crate::error::{NasError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// 用户信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub username: String,
    pub password_hash: String, // 实际应使用 bcrypt 或 argon2 加密
    pub role: UserRole,
}

/// 用户角色
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum UserRole {
    ReadOnly = 0,
    User = 1,
    Admin = 2,
}

/// 认证管理器
#[derive(Clone)]
pub struct AuthManager {
    users: Arc<RwLock<HashMap<String, User>>>,
}

impl AuthManager {
    pub fn new() -> Self {
        let users = HashMap::new();
        Self {
            users: Arc::new(RwLock::new(users)),
        }
    }

    /// 添加用户
    #[allow(dead_code)]
    pub fn add_user(&self, username: String, password: String, role: UserRole) -> Result<()> {
        let password_hash = Self::hash_password(&password);
        let user = User {
            username: username.clone(),
            password_hash,
            role,
        };

        let mut users = self
            .users
            .write()
            .map_err(|e| NasError::Storage(format!("锁定用户列表失败: {}", e)))?;

        if users.contains_key(&username) {
            return Err(NasError::Storage(format!("用户已存在: {}", username)));
        }

        users.insert(username, user);
        Ok(())
    }

    /// 验证用户
    #[allow(dead_code)]
    pub fn verify_user(&self, username: &str, password: &str) -> Result<User> {
        let users = self
            .users
            .read()
            .map_err(|e| NasError::Storage(format!("锁定用户列表失败: {}", e)))?;

        let user = users
            .get(username)
            .ok_or_else(|| NasError::Storage(format!("用户不存在: {}", username)))?;

        let password_hash = Self::hash_password(password);
        if user.password_hash != password_hash {
            return Err(NasError::Storage("密码错误".to_string()));
        }

        Ok(user.clone())
    }

    /// 检查用户权限
    #[allow(dead_code)]
    pub fn check_permission(&self, username: &str, required_role: UserRole) -> Result<bool> {
        let users = self
            .users
            .read()
            .map_err(|e| NasError::Storage(format!("锁定用户列表失败: {}", e)))?;

        let user = users
            .get(username)
            .ok_or_else(|| NasError::Storage(format!("用户不存在: {}", username)))?;

        Ok(matches!(
            (&user.role, &required_role),
            (UserRole::Admin, _)
                | (UserRole::User, UserRole::ReadOnly)
                | (UserRole::User, UserRole::User)
                | (UserRole::ReadOnly, UserRole::ReadOnly)
        ))
    }

    /// 简单的密码哈希（实际应使用 bcrypt 或 argon2）
    fn hash_password(password: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// 初始化默认管理员用户
    #[allow(dead_code)]
    pub fn init_default_admin(&self) -> Result<()> {
        self.add_user("admin".to_string(), "admin123".to_string(), UserRole::Admin)?;
        Ok(())
    }
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_manager() {
        let auth = AuthManager::new();

        // 添加用户
        auth.add_user("test".to_string(), "password".to_string(), UserRole::User)
            .unwrap();

        // 验证用户
        let user = auth.verify_user("test", "password").unwrap();
        assert_eq!(user.username, "test");
        assert_eq!(user.role, UserRole::User);

        // 错误密码
        assert!(auth.verify_user("test", "wrong").is_err());

        // 不存在的用户
        assert!(auth.verify_user("nonexistent", "password").is_err());
    }

    #[test]
    fn test_permissions() {
        let auth = AuthManager::new();

        auth.add_user("admin".to_string(), "admin".to_string(), UserRole::Admin)
            .unwrap();
        auth.add_user("user".to_string(), "user".to_string(), UserRole::User)
            .unwrap();
        auth.add_user(
            "readonly".to_string(),
            "readonly".to_string(),
            UserRole::ReadOnly,
        )
        .unwrap();

        // Admin 可以访问所有
        assert!(auth.check_permission("admin", UserRole::Admin).unwrap());
        assert!(auth.check_permission("admin", UserRole::User).unwrap());
        assert!(auth.check_permission("admin", UserRole::ReadOnly).unwrap());

        // User 可以访问 User 和 ReadOnly
        assert!(!auth.check_permission("user", UserRole::Admin).unwrap());
        assert!(auth.check_permission("user", UserRole::User).unwrap());
        assert!(auth.check_permission("user", UserRole::ReadOnly).unwrap());

        // ReadOnly 只能访问 ReadOnly
        assert!(!auth.check_permission("readonly", UserRole::Admin).unwrap());
        assert!(!auth.check_permission("readonly", UserRole::User).unwrap());
        assert!(
            auth.check_permission("readonly", UserRole::ReadOnly)
                .unwrap()
        );
    }

    #[test]
    fn test_user_role_ordering() {
        assert!(UserRole::Admin > UserRole::User);
        assert!(UserRole::User > UserRole::ReadOnly);
        assert!(UserRole::Admin > UserRole::ReadOnly);
    }

    #[test]
    fn test_user_role_equality() {
        assert_eq!(UserRole::Admin, UserRole::Admin);
        assert_eq!(UserRole::User, UserRole::User);
        assert_eq!(UserRole::ReadOnly, UserRole::ReadOnly);
        assert_ne!(UserRole::Admin, UserRole::User);
    }

    #[test]
    fn test_add_duplicate_user() {
        let auth = AuthManager::new();
        auth.add_user("test".to_string(), "password".to_string(), UserRole::User)
            .unwrap();

        // 尝试添加同名用户应该失败
        let result = auth.add_user("test".to_string(), "password2".to_string(), UserRole::Admin);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_with_wrong_password() {
        let auth = AuthManager::new();
        auth.add_user("test".to_string(), "correct".to_string(), UserRole::User)
            .unwrap();

        let result = auth.verify_user("test", "wrong");
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_nonexistent_user() {
        let auth = AuthManager::new();
        let result = auth.verify_user("nobody", "password");
        assert!(result.is_err());
    }

    #[test]
    fn test_check_permission_nonexistent_user() {
        let auth = AuthManager::new();
        let result = auth.check_permission("nobody", UserRole::User);
        assert!(result.is_err());
    }

    #[test]
    fn test_password_hashing() {
        let auth = AuthManager::new();
        auth.add_user(
            "test".to_string(),
            "password123".to_string(),
            UserRole::User,
        )
        .unwrap();

        // 密码应该被哈希存储，而不是明文
        let users = auth.users.read().unwrap();
        let user = users.get("test").unwrap();
        assert_ne!(user.password_hash, "password123");
        assert_eq!(user.password_hash.len(), 64); // SHA-256 = 64 hex chars
    }

    #[test]
    fn test_user_clone() {
        let user = User {
            username: "test".to_string(),
            password_hash: "hash123".to_string(),
            role: UserRole::User,
        };

        let cloned = user.clone();
        assert_eq!(user.username, cloned.username);
        assert_eq!(user.password_hash, cloned.password_hash);
        assert_eq!(user.role, cloned.role);
    }

    #[test]
    fn test_auth_manager_default() {
        let auth1 = AuthManager::new();
        let auth2 = AuthManager::default();

        // 两个实例都应该是空的
        assert_eq!(
            auth1.users.read().unwrap().len(),
            auth2.users.read().unwrap().len()
        );
    }

    #[test]
    fn test_init_default_admin() {
        let auth = AuthManager::new();
        auth.init_default_admin().unwrap();

        // 验证默认管理员
        let admin = auth.verify_user("admin", "admin123").unwrap();
        assert_eq!(admin.username, "admin");
        assert_eq!(admin.role, UserRole::Admin);
    }

    #[test]
    fn test_multiple_users() {
        let auth = AuthManager::new();

        // 添加多个用户
        for i in 0..10 {
            let username = format!("user{}", i);
            let password = format!("pass{}", i);
            auth.add_user(username, password, UserRole::User).unwrap();
        }

        // 验证所有用户
        for i in 0..10 {
            let username = format!("user{}", i);
            let password = format!("pass{}", i);
            let user = auth.verify_user(&username, &password).unwrap();
            assert_eq!(user.username, format!("user{}", i));
        }
    }

    #[test]
    fn test_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let auth = Arc::new(AuthManager::new());
        let mut handles = vec![];

        // 多线程添加用户
        for i in 0..5 {
            let auth_clone = Arc::clone(&auth);
            let handle = thread::spawn(move || {
                let username = format!("user{}", i);
                let password = format!("pass{}", i);
                auth_clone
                    .add_user(username, password, UserRole::User)
                    .unwrap();
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // 验证所有用户都已添加
        assert_eq!(auth.users.read().unwrap().len(), 5);
    }
}
