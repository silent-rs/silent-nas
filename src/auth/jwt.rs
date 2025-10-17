//! JWT Token 处理

use super::models::{Claims, User};
use crate::error::{NasError, Result};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use std::time::{SystemTime, UNIX_EPOCH};

/// JWT 配置
pub struct JwtConfig {
    /// JWT 签名密钥
    secret: String,
    /// 访问令牌过期时间（秒）
    access_token_exp: u64,
    /// 刷新令牌过期时间（秒）
    refresh_token_exp: u64,
}

impl JwtConfig {
    /// 创建 JWT 配置
    pub fn new(secret: String) -> Self {
        Self {
            secret,
            access_token_exp: 3600,    // 1小时
            refresh_token_exp: 604800, // 7天
        }
    }

    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let secret = std::env::var("JWT_SECRET")
            .unwrap_or_else(|_| "silent-nas-secret-key-change-in-production".to_string());

        let access_token_exp = std::env::var("JWT_ACCESS_EXP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600);

        let refresh_token_exp = std::env::var("JWT_REFRESH_EXP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(604800);

        Self {
            secret,
            access_token_exp,
            refresh_token_exp,
        }
    }

    /// 生成访问令牌
    pub fn generate_access_token(&self, user: &User) -> Result<String> {
        self.generate_token(user, self.access_token_exp)
    }

    /// 生成刷新令牌
    pub fn generate_refresh_token(&self, user: &User) -> Result<String> {
        self.generate_token(user, self.refresh_token_exp)
    }

    /// 生成 Token
    fn generate_token(&self, user: &User, exp_seconds: u64) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| NasError::Auth(format!("系统时间错误: {}", e)))?
            .as_secs();

        let claims = Claims {
            sub: user.id.clone(),
            username: user.username.clone(),
            role: user.role.to_string(),
            iat: now,
            exp: now + exp_seconds,
            jti: scru128::new_string(),
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )
        .map_err(|e| NasError::Auth(format!("生成Token失败: {}", e)))?;

        Ok(token)
    }

    /// 验证 Token
    pub fn verify_token(&self, token: &str) -> Result<Claims> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|e| NasError::Auth(format!("Token验证失败: {}", e)))?;

        Ok(token_data.claims)
    }

    /// 从 Token 提取用户ID
    pub fn extract_user_id(&self, token: &str) -> Result<String> {
        let claims = self.verify_token(token)?;
        Ok(claims.sub)
    }

    /// 检查 Token 是否过期
    pub fn is_token_expired(&self, token: &str) -> bool {
        match self.verify_token(token) {
            Ok(claims) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                claims.exp < now
            }
            Err(_) => true,
        }
    }

    /// 获取访问令牌过期时间
    pub fn get_access_token_exp(&self) -> u64 {
        self.access_token_exp
    }

    /// 获取刷新令牌过期时间
    pub fn get_refresh_token_exp(&self) -> u64 {
        self.refresh_token_exp
    }
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::models::UserRole;
    use chrono::Local;

    fn create_test_user() -> User {
        User {
            id: "test-id-123".to_string(),
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::User,
            status: crate::auth::models::UserStatus::Active,
            created_at: Local::now(),
            updated_at: Local::now(),
        }
    }

    #[test]
    fn test_jwt_config_creation() {
        let config = JwtConfig::new("test-secret".to_string());
        assert_eq!(config.secret, "test-secret");
        assert_eq!(config.access_token_exp, 3600);
        assert_eq!(config.refresh_token_exp, 604800);
    }

    #[test]
    fn test_generate_and_verify_token() {
        let config = JwtConfig::new("test-secret".to_string());
        let user = create_test_user();

        let token = config.generate_access_token(&user).unwrap();
        assert!(!token.is_empty());

        let claims = config.verify_token(&token).unwrap();
        assert_eq!(claims.sub, user.id);
        assert_eq!(claims.username, user.username);
        assert_eq!(claims.role, "User");
    }

    #[test]
    fn test_generate_refresh_token() {
        let config = JwtConfig::new("test-secret".to_string());
        let user = create_test_user();

        let token = config.generate_refresh_token(&user).unwrap();
        assert!(!token.is_empty());

        let claims = config.verify_token(&token).unwrap();
        assert_eq!(claims.sub, user.id);
    }

    #[test]
    fn test_extract_user_id() {
        let config = JwtConfig::new("test-secret".to_string());
        let user = create_test_user();

        let token = config.generate_access_token(&user).unwrap();
        let user_id = config.extract_user_id(&token).unwrap();
        assert_eq!(user_id, user.id);
    }

    #[test]
    fn test_invalid_token() {
        let config = JwtConfig::new("test-secret".to_string());
        let result = config.verify_token("invalid-token");
        assert!(result.is_err());
    }

    #[test]
    fn test_wrong_secret() {
        let config1 = JwtConfig::new("secret1".to_string());
        let config2 = JwtConfig::new("secret2".to_string());
        let user = create_test_user();

        let token = config1.generate_access_token(&user).unwrap();
        let result = config2.verify_token(&token);
        assert!(result.is_err());
    }

    #[test]
    fn test_token_not_expired() {
        let config = JwtConfig::new("test-secret".to_string());
        let user = create_test_user();

        let token = config.generate_access_token(&user).unwrap();
        assert!(!config.is_token_expired(&token));
    }

    #[test]
    fn test_different_roles() {
        let config = JwtConfig::new("test-secret".to_string());

        let mut admin = create_test_user();
        admin.role = UserRole::Admin;
        let token = config.generate_access_token(&admin).unwrap();
        let claims = config.verify_token(&token).unwrap();
        assert_eq!(claims.role, "Admin");

        let mut readonly = create_test_user();
        readonly.role = UserRole::ReadOnly;
        let token = config.generate_access_token(&readonly).unwrap();
        let claims = config.verify_token(&token).unwrap();
        assert_eq!(claims.role, "ReadOnly");
    }

    #[test]
    fn test_jwt_id_uniqueness() {
        let config = JwtConfig::new("test-secret".to_string());
        let user = create_test_user();

        let token1 = config.generate_access_token(&user).unwrap();
        let token2 = config.generate_access_token(&user).unwrap();

        let claims1 = config.verify_token(&token1).unwrap();
        let claims2 = config.verify_token(&token2).unwrap();

        // JTI 应该不同
        assert_ne!(claims1.jti, claims2.jti);
    }
}
