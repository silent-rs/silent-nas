//! 认证与授权模块
//!
//! 提供完整的JWT认证、用户管理和权限控制功能

#![allow(dead_code)] // 功能尚未完全集成，后续会使用

pub mod jwt;
pub mod models;
pub mod password;
pub mod rate_limit;
pub mod s3_keys_storage;
pub mod storage;
pub mod token_blacklist;

pub use jwt::JwtConfig;
pub use models::{
    ChangePasswordRequest, CreateS3KeyRequest, CreateS3KeyResponse, LoginRequest, LoginResponse,
    RegisterRequest, S3AccessKey, S3AccessKeyInfo, S3KeyStatus, UpdateS3KeyRequest, User, UserInfo,
    UserRole, UserStatus,
};
pub use s3_keys_storage::S3KeyStorage;

use crate::error::{NasError, Result};
use chrono::{Local, TimeZone};
use password::PasswordHandler;
use rate_limit::{RateLimitConfig, RateLimiter};
use std::path::Path;
use std::sync::{Arc, RwLock};
use storage::UserStorage;
use token_blacklist::TokenBlacklist;
use validator::Validate;

/// 认证管理器
#[derive(Clone)]
pub struct AuthManager {
    pub(crate) storage: Arc<UserStorage>,
    pub(crate) s3_keys_storage: Arc<S3KeyStorage>,
    jwt_config: Arc<RwLock<JwtConfig>>,
    rate_limiter: Option<Arc<RateLimiter>>,
    token_blacklist: Option<Arc<TokenBlacklist>>,
}

impl AuthManager {
    /// 创建认证管理器
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let storage = UserStorage::new(&db_path)?;
        let jwt_config = JwtConfig::from_env();

        let db_dir = db_path
            .as_ref()
            .parent()
            .ok_or_else(|| NasError::Config("无效的数据库路径".to_string()))?;

        // 创建 S3 密钥存储
        let s3_keys_path = db_dir.join("s3_keys.db");
        let s3_keys_storage = S3KeyStorage::new(s3_keys_path)?;

        // 创建限流器
        let rate_limiter = {
            let rate_limit_path = db_dir.join("rate_limit.db");
            match RateLimiter::new(rate_limit_path, RateLimitConfig::default()) {
                Ok(limiter) => Some(Arc::new(limiter)),
                Err(e) => {
                    tracing::warn!("创建限流器失败: {}, 限流功能将被禁用", e);
                    None
                }
            }
        };

        // 创建Token黑名单
        let token_blacklist = {
            let blacklist_path = db_dir.join("token_blacklist.db");
            match TokenBlacklist::new(blacklist_path) {
                Ok(blacklist) => Some(Arc::new(blacklist)),
                Err(e) => {
                    tracing::warn!("创建Token黑名单失败: {}, 注销功能将被禁用", e);
                    None
                }
            }
        };

        Ok(Self {
            storage: Arc::new(storage),
            s3_keys_storage: Arc::new(s3_keys_storage),
            jwt_config: Arc::new(RwLock::new(jwt_config)),
            rate_limiter,
            token_blacklist,
        })
    }

    /// 设置JWT配置
    pub fn set_jwt_config(&self, config: JwtConfig) {
        *self.jwt_config.write().unwrap() = config;
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
        // 检查限流
        if let Some(ref limiter) = self.rate_limiter
            && limiter.is_locked(&req.username)?
        {
            let remaining = limiter.get_lock_remaining(&req.username)?;
            if let Some(seconds) = remaining {
                return Err(NasError::Auth(format!(
                    "账户已被锁定，请在 {} 秒后重试",
                    seconds
                )));
            }
        }

        // 尝试通过用户名或邮箱查找用户
        let user = self
            .storage
            .get_user_by_username(&req.username)?
            .or_else(|| self.storage.get_user_by_email(&req.username).ok().flatten());

        if user.is_none() {
            // 记录失败
            if let Some(ref limiter) = self.rate_limiter {
                let _ = limiter.record_failure(&req.username);
            }
            return Err(NasError::Auth("用户名或密码错误".to_string()));
        }

        let user = user.unwrap();

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
            // 记录失败
            if let Some(ref limiter) = self.rate_limiter {
                let _ = limiter.record_failure(&req.username);
            }
            return Err(NasError::Auth("用户名或密码错误".to_string()));
        }

        // 登录成功，清除失败记录
        if let Some(ref limiter) = self.rate_limiter {
            let _ = limiter.clear(&req.username);
        }

        // 生成 Token
        let jwt_config = self.jwt_config.read().unwrap();
        let access_token = jwt_config.generate_access_token(&user)?;
        let refresh_token = jwt_config.generate_refresh_token(&user)?;

        Ok(LoginResponse {
            access_token,
            refresh_token,
            token_type: "Bearer".to_string(),
            expires_in: jwt_config.get_access_token_exp(),
            user: user.into(),
        })
    }

    /// 刷新 Token
    pub fn refresh_token(&self, refresh_token: &str) -> Result<LoginResponse> {
        // 验证刷新令牌
        let claims = self
            .jwt_config
            .read()
            .unwrap()
            .verify_token(refresh_token)?;

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
        let jwt_config = self.jwt_config.read().unwrap();
        let access_token = jwt_config.generate_access_token(&user)?;
        let new_refresh_token = jwt_config.generate_refresh_token(&user)?;

        Ok(LoginResponse {
            access_token,
            refresh_token: new_refresh_token,
            token_type: "Bearer".to_string(),
            expires_in: jwt_config.get_access_token_exp(),
            user: user.into(),
        })
    }

    /// 验证 Token 并获取用户信息
    pub fn verify_token(&self, token: &str) -> Result<User> {
        let claims = self.jwt_config.read().unwrap().verify_token(token)?;

        // 检查Token是否在黑名单中
        if let Some(ref blacklist) = self.token_blacklist
            && blacklist.is_blacklisted(&claims.jti)?
        {
            return Err(NasError::Auth("Token已被撤销".to_string()));
        }

        let user = self
            .storage
            .get_user_by_id(&claims.sub)?
            .ok_or_else(|| NasError::Auth("用户不存在".to_string()))?;

        if user.status != UserStatus::Active {
            return Err(NasError::Auth("账户不可用".to_string()));
        }

        Ok(user)
    }

    /// 注销（将Token加入黑名单）
    pub fn logout(&self, token: &str) -> Result<()> {
        let blacklist = self
            .token_blacklist
            .as_ref()
            .ok_or_else(|| NasError::Auth("注销功能未启用".to_string()))?;

        let claims = self.jwt_config.read().unwrap().verify_token(token)?;

        // 计算过期时间
        let expires_at = chrono::Local
            .timestamp_opt(claims.exp as i64, 0)
            .single()
            .unwrap();

        blacklist.add(&claims.jti, &claims.sub, expires_at, "user_logout")?;

        Ok(())
    }

    /// 撤销用户的所有Token
    pub fn revoke_all_tokens(&self, user_id: &str) -> Result<usize> {
        let blacklist = self
            .token_blacklist
            .as_ref()
            .ok_or_else(|| NasError::Auth("注销功能未启用".to_string()))?;

        blacklist.revoke_user_tokens(user_id)
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
    pub async fn list_users(&self) -> Result<Vec<User>> {
        self.storage.list_users()
    }

    /// 根据ID获取用户（仅管理员）
    pub async fn get_user_by_id(&self, user_id: &str) -> Result<Option<User>> {
        self.storage.get_user_by_id(user_id)
    }

    /// 更新用户信息（仅管理员）
    pub async fn update_user(&self, user: &User) -> Result<()> {
        let mut updated_user = user.clone();
        updated_user.updated_at = Local::now();
        self.storage.update_user(updated_user)?;
        Ok(())
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

    /// 重置用户密码（仅管理员）
    pub async fn reset_password(&self, user_id: &str, new_password: &str) -> Result<()> {
        let mut user = self
            .storage
            .get_user_by_id(user_id)?
            .ok_or_else(|| NasError::Auth("用户不存在".to_string()))?;

        // 哈希新密码
        user.password_hash = PasswordHandler::hash_password(new_password)?;
        user.updated_at = Local::now();

        // 更新用户
        self.storage.update_user(user)?;
        Ok(())
    }

    /// 删除用户（仅管理员）
    pub async fn delete_user(&self, user_id: &str) -> Result<()> {
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

    // ==================== S3 密钥管理方法 ====================

    /// 生成随机的 S3 访问密钥
    fn generate_access_key() -> String {
        use rand::Rng;
        const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut rng = rand::thread_rng();

        let key: String = (0..20)
            .map(|_| {
                let idx = rng.gen_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect();

        format!("AKIA{}", key)
    }

    /// 生成随机的 S3 密钥密钥
    fn generate_secret_key() -> String {
        use rand::Rng;
        const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut rng = rand::thread_rng();

        (0..40)
            .map(|_| {
                let idx = rng.gen_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect()
    }

    /// 创建 S3 访问密钥
    pub async fn create_s3_key(
        &self,
        user_id: &str,
        req: CreateS3KeyRequest,
    ) -> Result<CreateS3KeyResponse> {
        // 验证请求
        req.validate()
            .map_err(|e| NasError::Auth(format!("验证失败: {}", e)))?;

        // 验证用户是否存在
        let _user = self
            .storage
            .get_user_by_id(user_id)?
            .ok_or_else(|| NasError::Auth("用户不存在".to_string()))?;

        // 生成密钥
        let access_key = Self::generate_access_key();
        let secret_key = Self::generate_secret_key();

        // 创建密钥记录
        let key_id = scru128::new_string();
        let key = S3AccessKey {
            id: key_id.clone(),
            user_id: user_id.to_string(),
            access_key: access_key.clone(),
            secret_key: secret_key.clone(),
            description: req.description.clone(),
            status: S3KeyStatus::Active,
            created_at: Local::now(),
            last_used_at: None,
        };

        let created_at = key.created_at;
        self.s3_keys_storage.create_key(key)?;

        Ok(CreateS3KeyResponse {
            id: key_id,
            access_key,
            secret_key,
            description: req.description,
            status: S3KeyStatus::Active,
            created_at,
        })
    }

    /// 获取用户的所有 S3 密钥
    pub async fn list_s3_keys(&self, user_id: &str) -> Result<Vec<S3AccessKeyInfo>> {
        let keys = self.s3_keys_storage.get_keys_by_user_id(user_id)?;
        Ok(keys.into_iter().map(|k| k.into()).collect())
    }

    /// 获取所有 S3 密钥（仅管理员）
    pub async fn list_all_s3_keys(&self) -> Result<Vec<S3AccessKeyInfo>> {
        let keys = self.s3_keys_storage.list_all_keys()?;
        Ok(keys.into_iter().map(|k| k.into()).collect())
    }

    /// 根据 ID 获取 S3 密钥
    pub async fn get_s3_key(&self, key_id: &str) -> Result<Option<S3AccessKeyInfo>> {
        Ok(self
            .s3_keys_storage
            .get_key_by_id(key_id)?
            .map(|k| k.into()))
    }

    /// 更新 S3 密钥
    pub async fn update_s3_key(
        &self,
        user_id: &str,
        key_id: &str,
        req: UpdateS3KeyRequest,
    ) -> Result<S3AccessKeyInfo> {
        // 验证请求
        req.validate()
            .map_err(|e| NasError::Auth(format!("验证失败: {}", e)))?;

        // 获取密钥
        let mut key = self
            .s3_keys_storage
            .get_key_by_id(key_id)?
            .ok_or_else(|| NasError::Auth("S3 密钥不存在".to_string()))?;

        // 验证所有权
        if key.user_id != user_id {
            return Err(NasError::Auth("无权操作此密钥".to_string()));
        }

        // 更新字段
        if let Some(description) = req.description {
            key.description = description;
        }
        if let Some(status) = req.status {
            key.status = status;
        }

        let updated = self.s3_keys_storage.update_key(key)?;
        Ok(updated.into())
    }

    /// 删除 S3 密钥
    pub async fn delete_s3_key(&self, user_id: &str, key_id: &str) -> Result<()> {
        // 获取密钥
        let key = self
            .s3_keys_storage
            .get_key_by_id(key_id)?
            .ok_or_else(|| NasError::Auth("S3 密钥不存在".to_string()))?;

        // 验证所有权
        if key.user_id != user_id {
            return Err(NasError::Auth("无权操作此密钥".to_string()));
        }

        self.s3_keys_storage.delete_key(key_id)
    }

    /// 管理员删除任意 S3 密钥
    pub async fn admin_delete_s3_key(&self, key_id: &str) -> Result<()> {
        self.s3_keys_storage.delete_key(key_id)
    }

    /// 验证 S3 访问密钥
    pub async fn verify_s3_key(&self, access_key: &str, secret_key: &str) -> Result<S3AccessKey> {
        let key = self
            .s3_keys_storage
            .get_key_by_access_key(access_key)?
            .ok_or_else(|| NasError::Auth("无效的访问密钥".to_string()))?;

        // 验证密钥状态
        if key.status != S3KeyStatus::Active {
            return Err(NasError::Auth("密钥已被禁用".to_string()));
        }

        // 验证密钥密钥
        if key.secret_key != secret_key {
            return Err(NasError::Auth("无效的密钥密钥".to_string()));
        }

        // 更新最后使用时间（异步执行，不阻塞验证）
        let storage = self.s3_keys_storage.clone();
        let key_id = key.id.clone();
        tokio::spawn(async move {
            let _ = storage.update_last_used(&key_id);
        });

        Ok(key)
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
