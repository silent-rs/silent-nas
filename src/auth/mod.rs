//! 认证与授权模块
//!
//! 提供完整的JWT认证、用户管理和权限控制功能

#![allow(dead_code)] // 功能尚未完全集成，后续会使用

pub mod jwt;
pub mod models;
pub mod password;
pub mod storage;

pub use models::{
    ChangePasswordRequest, LoginRequest, LoginResponse, RegisterRequest, User, UserInfo,
    UserRole, UserStatus,
};

use crate::error::{NasError, Result};
use chrono::Local;
use jwt::JwtConfig;
use password::PasswordHandler;
use std::path::Path;
use std::sync::Arc;
use storage::UserStorage;
use validator::Validate;

/// 认证管理器
#[derive(Clone)]
pub struct AuthManager {
    storage: Arc<UserStorage>,
    jwt_config: Arc<JwtConfig>,
}

impl AuthManager {
    /// 创建认证管理器
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let storage = UserStorage::new(db_path)?;
        let jwt_config = JwtConfig::from_env();

        Ok(Self {
            storage: Arc::new(storage),
            jwt_config: Arc::new(jwt_config),
        })
    }

    /// 注册用户
    pub fn register(&self, req: RegisterRequest) -> Result<UserInfo> {
        // 验证请求
        req.validate()
            .map_err(|e| NasError::Auth(format!("验证失败: {}", e)))?;

        // 检查用户名是否存在
        if self.storage.username_exists(&req.username)? {
            return Err(NasError::Auth(format!("用户名已存在: {}", req.username)));
        }

        // 检查邮箱是否存在
        if self.storage.email_exists(&req.email)? {
            return Err(NasError::Auth(format!("邮箱已存在: {}", req.email)));
        }

        // 哈希密码
        let password_hash = PasswordHandler::hash_password(&req.password)?;

        // 创建用户
        let user = User {
            id: scru128::new_string(),
            username: req.username,
            email: req.email,
            password_hash,
            role: UserRole::User, // 默认角色
            status: UserStatus::Active,
            created_at: Local::now(),
            updated_at: Local::now(),
        };

        let created_user = self.storage.create_user(user)?;

        Ok(created_user.into())
    }

    /// 登录
    pub fn login(&self, req: LoginRequest) -> Result<LoginResponse> {
        // 尝试通过用户名或邮箱查找用户
        let user = self
            .storage
            .get_user_by_username(&req.username)?
            .or_else(|| self.storage.get_user_by_email(&req.username).ok().flatten())
            .ok_or_else(|| NasError::Auth("用户名或密码错误".to_string()))?;

        // 检查用户状态
        match user.status {
            UserStatus::Suspended => {
                return Err(NasError::Auth("账户已被暂停".to_string()));
            }
            UserStatus::Deleted => {
                return Err(NasError::Auth("账户已被删除".to_string()));
            }
            UserStatus::Active => {}
        }

        // 验证密码
        if !PasswordHandler::verify_password(&req.password, &user.password_hash)? {
            return Err(NasError::Auth("用户名或密码错误".to_string()));
        }

        // 生成 Token
        let access_token = self.jwt_config.generate_access_token(&user)?;
        let refresh_token = self.jwt_config.generate_refresh_token(&user)?;

        Ok(LoginResponse {
            access_token,
            refresh_token,
            token_type: "Bearer".to_string(),
            expires_in: self.jwt_config.get_access_token_exp(),
            user: user.into(),
        })
    }

    /// 刷新 Token
    pub fn refresh_token(&self, refresh_token: &str) -> Result<LoginResponse> {
        // 验证刷新令牌
        let claims = self.jwt_config.verify_token(refresh_token)?;

        // 获取用户
        let user = self
            .storage
            .get_user_by_id(&claims.sub)?
            .ok_or_else(|| NasError::Auth("用户不存在".to_string()))?;

        // 检查用户状态
        if user.status != UserStatus::Active {
            return Err(NasError::Auth("账户不可用".to_string()));
        }

        // 生成新的 Token
        let access_token = self.jwt_config.generate_access_token(&user)?;
        let new_refresh_token = self.jwt_config.generate_refresh_token(&user)?;

        Ok(LoginResponse {
            access_token,
            refresh_token: new_refresh_token,
            token_type: "Bearer".to_string(),
            expires_in: self.jwt_config.get_access_token_exp(),
            user: user.into(),
        })
    }

    /// 验证 Token 并获取用户信息
    pub fn verify_token(&self, token: &str) -> Result<User> {
        let claims = self.jwt_config.verify_token(token)?;

        let user = self
            .storage
            .get_user_by_id(&claims.sub)?
            .ok_or_else(|| NasError::Auth("用户不存在".to_string()))?;

        if user.status != UserStatus::Active {
            return Err(NasError::Auth("账户不可用".to_string()));
        }

        Ok(user)
    }

    /// 修改密码
    pub fn change_password(&self, user_id: &str, req: ChangePasswordRequest) -> Result<()> {
        // 验证请求
        req.validate()
            .map_err(|e| NasError::Auth(format!("验证失败: {}", e)))?;

        // 获取用户
        let mut user = self
            .storage
            .get_user_by_id(user_id)?
            .ok_or_else(|| NasError::Auth("用户不存在".to_string()))?;

        // 验证旧密码
        if !PasswordHandler::verify_password(&req.old_password, &user.password_hash)? {
            return Err(NasError::Auth("旧密码错误".to_string()));
        }

        // 哈希新密码
        user.password_hash = PasswordHandler::hash_password(&req.new_password)?;
        user.updated_at = Local::now();

        // 更新用户
        self.storage.update_user(user)?;

        Ok(())
    }

    /// 获取用户信息
    pub fn get_user(&self, user_id: &str) -> Result<Option<UserInfo>> {
        Ok(self.storage.get_user_by_id(user_id)?.map(|u| u.into()))
    }

    /// 列出所有用户（仅管理员）
    pub fn list_users(&self) -> Result<Vec<UserInfo>> {
        let users = self.storage.list_users()?;
        Ok(users.into_iter().map(|u| u.into()).collect())
    }

    /// 更新用户角色（仅管理员）
    pub fn update_user_role(&self, user_id: &str, role: UserRole) -> Result<UserInfo> {
        let mut user = self
            .storage
            .get_user_by_id(user_id)?
            .ok_or_else(|| NasError::Auth("用户不存在".to_string()))?;

        user.role = role;
        user.updated_at = Local::now();

        let updated = self.storage.update_user(user)?;
        Ok(updated.into())
    }

    /// 更新用户状态（仅管理员）
    pub fn update_user_status(&self, user_id: &str, status: UserStatus) -> Result<UserInfo> {
        let mut user = self
            .storage
            .get_user_by_id(user_id)?
            .ok_or_else(|| NasError::Auth("用户不存在".to_string()))?;

        user.status = status;
        user.updated_at = Local::now();

        let updated = self.storage.update_user(user)?;
        Ok(updated.into())
    }

    /// 删除用户（仅管理员）
    pub fn delete_user(&self, user_id: &str) -> Result<()> {
        self.storage.delete_user(user_id)
    }

    /// 初始化默认管理员（如果不存在）
    pub fn init_default_admin(&self) -> Result<()> {
        // 检查是否已有用户
        if self.storage.count_users()? > 0 {
            return Ok(());
        }

        // 创建默认管理员
        let password_hash = PasswordHandler::hash_password("admin123")?;

        let admin = User {
            id: scru128::new_string(),
            username: "admin".to_string(),
            email: "admin@silent-nas.local".to_string(),
            password_hash,
            role: UserRole::Admin,
            status: UserStatus::Active,
            created_at: Local::now(),
            updated_at: Local::now(),
        };

        self.storage.create_user(admin)?;

        tracing::info!("默认管理员账户已创建: admin / admin123");

        Ok(())
    }

    /// 检查权限
    pub fn check_permission(&self, user: &User, required_role: UserRole) -> bool {
        user.role >= required_role
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_auth_manager() -> (AuthManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let auth = AuthManager::new(temp_dir.path()).unwrap();
        (auth, temp_dir)
    }

    #[test]
    fn test_register_and_login() {
        let (auth, _temp) = create_test_auth_manager();

        // 注册
        let register_req = RegisterRequest {
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            password: "SecureP@ss123".to_string(),
        };

        let user_info = auth.register(register_req).unwrap();
        assert_eq!(user_info.username, "testuser");

        // 登录
        let login_req = LoginRequest {
            username: "testuser".to_string(),
            password: "SecureP@ss123".to_string(),
        };

        let login_resp = auth.login(login_req).unwrap();
        assert!(!login_resp.access_token.is_empty());
        assert_eq!(login_resp.user.username, "testuser");
    }

    #[test]
    fn test_duplicate_registration() {
        let (auth, _temp) = create_test_auth_manager();

        let register_req = RegisterRequest {
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            password: "SecureP@ss123".to_string(),
        };

        auth.register(register_req.clone()).unwrap();
        let result = auth.register(register_req);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_login() {
        let (auth, _temp) = create_test_auth_manager();

        let login_req = LoginRequest {
            username: "nonexistent".to_string(),
            password: "password".to_string(),
        };

        let result = auth.login(login_req);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_token() {
        let (auth, _temp) = create_test_auth_manager();

        // 注册并登录
        let register_req = RegisterRequest {
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            password: "SecureP@ss123".to_string(),
        };
        auth.register(register_req).unwrap();

        let login_req = LoginRequest {
            username: "testuser".to_string(),
            password: "SecureP@ss123".to_string(),
        };
        let login_resp = auth.login(login_req).unwrap();

        // 验证 Token
        let user = auth.verify_token(&login_resp.access_token).unwrap();
        assert_eq!(user.username, "testuser");
    }

    #[test]
    fn test_change_password() {
        let (auth, _temp) = create_test_auth_manager();

        // 注册
        let register_req = RegisterRequest {
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            password: "OldPass123!".to_string(),
        };
        let user_info = auth.register(register_req).unwrap();

        // 修改密码
        let change_req = ChangePasswordRequest {
            old_password: "OldPass123!".to_string(),
            new_password: "NewPass456!".to_string(),
        };
        auth.change_password(&user_info.id, change_req).unwrap();

        // 使用新密码登录
        let login_req = LoginRequest {
            username: "testuser".to_string(),
            password: "NewPass456!".to_string(),
        };
        assert!(auth.login(login_req).is_ok());
    }

    #[test]
    fn test_init_default_admin() {
        let (auth, _temp) = create_test_auth_manager();

        auth.init_default_admin().unwrap();

        // 使用默认管理员登录
        let login_req = LoginRequest {
            username: "admin".to_string(),
            password: "admin123".to_string(),
        };
        let login_resp = auth.login(login_req).unwrap();
        assert_eq!(login_resp.user.role, UserRole::Admin);
    }

    #[test]
    fn test_permission_check() {
        let (auth, _temp) = create_test_auth_manager();

        let admin = User {
            id: "admin-id".to_string(),
            username: "admin".to_string(),
            email: "admin@example.com".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::Admin,
            status: UserStatus::Active,
            created_at: Local::now(),
            updated_at: Local::now(),
        };

        let user = User {
            id: "user-id".to_string(),
            username: "user".to_string(),
            email: "user@example.com".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::User,
            status: UserStatus::Active,
            created_at: Local::now(),
            updated_at: Local::now(),
        };

        // Admin 可以访问所有权限
        assert!(auth.check_permission(&admin, UserRole::Admin));
        assert!(auth.check_permission(&admin, UserRole::User));
        assert!(auth.check_permission(&admin, UserRole::ReadOnly));

        // User 可以访问 User 和 ReadOnly
        assert!(!auth.check_permission(&user, UserRole::Admin));
        assert!(auth.check_permission(&user, UserRole::User));
        assert!(auth.check_permission(&user, UserRole::ReadOnly));
    }
}
