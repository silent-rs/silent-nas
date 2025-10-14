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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum UserRole {
    Admin,
    User,
    ReadOnly,
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
}
