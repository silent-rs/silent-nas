//! 认证模型定义

use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use validator::Validate;

// 自定义序列化器用于 DateTime<Local>
mod datetime_local_serde {
    use super::*;

    pub fn serialize<S>(dt: &DateTime<Local>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i64(dt.timestamp())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Local>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let timestamp = i64::deserialize(deserializer)?;
        Local
            .timestamp_opt(timestamp, 0)
            .single()
            .ok_or_else(|| serde::de::Error::custom("无效的时间戳"))
    }
}

/// 用户模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// 用户ID
    pub id: String,
    /// 用户名（唯一）
    pub username: String,
    /// 电子邮件（唯一）
    pub email: String,
    /// 密码哈希（Argon2）- 注意：在数据库中需要保存，只在API响应中隐藏
    pub password_hash: String,
    /// 用户角色
    pub role: UserRole,
    /// 用户状态
    pub status: UserStatus,
    /// 创建时间（存储为时间戳）
    #[serde(with = "datetime_local_serde")]
    pub created_at: DateTime<Local>,
    /// 更新时间（存储为时间戳）
    #[serde(with = "datetime_local_serde")]
    pub updated_at: DateTime<Local>,
}

/// 用户角色
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum UserRole {
    /// 只读权限
    ReadOnly = 0,
    /// 普通用户
    User = 1,
    /// 管理员
    Admin = 2,
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserRole::ReadOnly => write!(f, "ReadOnly"),
            UserRole::User => write!(f, "User"),
            UserRole::Admin => write!(f, "Admin"),
        }
    }
}

impl std::str::FromStr for UserRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "readonly" => Ok(UserRole::ReadOnly),
            "user" => Ok(UserRole::User),
            "admin" => Ok(UserRole::Admin),
            _ => Err(format!("无效的角色: {}", s)),
        }
    }
}

/// 用户状态
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum UserStatus {
    /// 活跃
    Active,
    /// 暂停
    Suspended,
    /// 已删除（软删除）
    Deleted,
}

impl std::fmt::Display for UserStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserStatus::Active => write!(f, "Active"),
            UserStatus::Suspended => write!(f, "Suspended"),
            UserStatus::Deleted => write!(f, "Deleted"),
        }
    }
}

/// 用户注册请求
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct RegisterRequest {
    /// 用户名（3-30个字符）
    #[validate(length(min = 3, max = 30, message = "用户名长度必须在3-30个字符之间"))]
    pub username: String,

    /// 电子邮件
    #[validate(email(message = "无效的电子邮件格式"))]
    pub email: String,

    /// 密码（8-72个字符）
    #[validate(length(min = 8, max = 72, message = "密码长度必须在8-72个字符之间"))]
    pub password: String,
}

/// 用户登录请求
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    /// 用户名或邮箱
    pub username: String,
    /// 密码
    pub password: String,
}

/// 修改密码请求
#[derive(Debug, Deserialize, Validate)]
pub struct ChangePasswordRequest {
    /// 旧密码
    pub old_password: String,

    /// 新密码（8-72个字符）
    #[validate(length(min = 8, max = 72, message = "密码长度必须在8-72个字符之间"))]
    pub new_password: String,
}

/// 登录响应
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    /// 访问令牌
    pub access_token: String,
    /// 刷新令牌
    pub refresh_token: String,
    /// 令牌类型
    pub token_type: String,
    /// 过期时间（秒）
    pub expires_in: u64,
    /// 用户信息
    pub user: UserInfo,
}

/// 用户信息（公开）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub username: String,
    pub email: String,
    pub role: UserRole,
    pub status: UserStatus,
    #[serde(with = "datetime_local_serde")]
    pub created_at: DateTime<Local>,
}

impl From<User> for UserInfo {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            username: user.username,
            email: user.email,
            role: user.role,
            status: user.status,
            created_at: user.created_at,
        }
    }
}

/// JWT Claims
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// 用户ID
    pub sub: String,
    /// 用户名
    pub username: String,
    /// 用户角色
    pub role: String,
    /// 签发时间
    pub iat: u64,
    /// 过期时间
    pub exp: u64,
    /// JWT ID（用于黑名单）
    pub jti: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_role_ordering() {
        assert!(UserRole::Admin > UserRole::User);
        assert!(UserRole::User > UserRole::ReadOnly);
    }

    #[test]
    fn test_user_role_display() {
        assert_eq!(UserRole::Admin.to_string(), "Admin");
        assert_eq!(UserRole::User.to_string(), "User");
        assert_eq!(UserRole::ReadOnly.to_string(), "ReadOnly");
    }

    #[test]
    fn test_user_role_from_str() {
        assert_eq!("admin".parse::<UserRole>().unwrap(), UserRole::Admin);
        assert_eq!("user".parse::<UserRole>().unwrap(), UserRole::User);
        assert_eq!("readonly".parse::<UserRole>().unwrap(), UserRole::ReadOnly);
    }

    #[test]
    fn test_register_request_validation() {
        use validator::Validate;

        // 有效请求
        let valid = RegisterRequest {
            username: "john_doe".to_string(),
            email: "john@example.com".to_string(),
            password: "SecureP@ss123".to_string(),
        };
        assert!(valid.validate().is_ok());

        // 用户名太短
        let short_username = RegisterRequest {
            username: "ab".to_string(),
            email: "john@example.com".to_string(),
            password: "SecureP@ss123".to_string(),
        };
        assert!(short_username.validate().is_err());

        // 无效邮箱
        let invalid_email = RegisterRequest {
            username: "john_doe".to_string(),
            email: "invalid-email".to_string(),
            password: "SecureP@ss123".to_string(),
        };
        assert!(invalid_email.validate().is_err());

        // 密码太短
        let short_password = RegisterRequest {
            username: "john_doe".to_string(),
            email: "john@example.com".to_string(),
            password: "short".to_string(),
        };
        assert!(short_password.validate().is_err());
    }

    #[test]
    fn test_user_info_from_user() {
        let user = User {
            id: "test-id".to_string(),
            username: "test".to_string(),
            email: "test@example.com".to_string(),
            password_hash: "hash".to_string(),
            role: UserRole::User,
            status: UserStatus::Active,
            created_at: Local::now(),
            updated_at: Local::now(),
        };

        let info: UserInfo = user.clone().into();
        assert_eq!(info.id, user.id);
        assert_eq!(info.username, user.username);
        assert_eq!(info.email, user.email);
    }
}
