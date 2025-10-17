//! 认证中间件
//!
//! 提供Token验证和权限检查功能

use crate::auth::{AuthManager, UserRole};
use crate::error::NasError;
use http::StatusCode;
use silent::SilentError;
use silent::middleware::MiddleWareHandler;
use silent::prelude::*;
use std::sync::Arc;

/// 从请求头提取Bearer Token
fn extract_token(req: &Request) -> silent::Result<String> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            SilentError::business_error(StatusCode::UNAUTHORIZED, "缺少Authorization头")
        })?;

    if !auth_header.starts_with("Bearer ") {
        return Err(SilentError::business_error(
            StatusCode::UNAUTHORIZED,
            "无效的Authorization格式",
        ));
    }

    Ok(auth_header[7..].to_string())
}

/// 认证中间件 - 验证Token并将用户信息注入到请求配置中
#[derive(Clone)]
pub struct AuthHook {
    auth_manager: Arc<AuthManager>,
    required_role: Option<UserRole>,
}

impl AuthHook {
    /// 创建认证中间件（任何已登录用户）
    pub fn new(auth_manager: Arc<AuthManager>) -> Self {
        Self {
            auth_manager,
            required_role: None,
        }
    }

    /// 创建需要特定角色的认证中间件
    #[allow(dead_code)]
    pub fn with_role(auth_manager: Arc<AuthManager>, role: UserRole) -> Self {
        Self {
            auth_manager,
            required_role: Some(role),
        }
    }

    /// 创建需要管理员权限的认证中间件
    #[allow(dead_code)]
    pub fn admin_only(auth_manager: Arc<AuthManager>) -> Self {
        Self::with_role(auth_manager, UserRole::Admin)
    }
}

#[async_trait::async_trait]
impl MiddleWareHandler for AuthHook {
    async fn handle(&self, mut req: Request, next: &Next) -> silent::Result<Response> {
        // 提取Token
        let token = extract_token(&req)?;

        // 验证Token并获取用户
        let user = self
            .auth_manager
            .verify_token(&token)
            .map_err(|e| match e {
                NasError::Auth(msg) => SilentError::business_error(StatusCode::UNAUTHORIZED, msg),
                _ => SilentError::business_error(StatusCode::UNAUTHORIZED, "Token验证失败"),
            })?;

        // 检查用户状态
        if user.status != crate::auth::UserStatus::Active {
            return Err(SilentError::business_error(
                StatusCode::FORBIDDEN,
                "用户账户已被暂停",
            ));
        }

        // 检查角色权限
        if let Some(required_role) = &self.required_role
            && &user.role != required_role
            && user.role != UserRole::Admin
        {
            return Err(SilentError::business_error(
                StatusCode::FORBIDDEN,
                format!("需要 {:?} 权限", required_role),
            ));
        }

        // 将用户对象注入到请求配置中（后续处理器可以提取）
        req.configs_mut().insert(user);

        // 继续处理请求
        next.call(req).await
    }
}

/// 可选认证中间件 - Token验证通过则注入用户信息，否则继续处理
#[derive(Clone)]
pub struct OptionalAuthHook {
    auth_manager: Arc<AuthManager>,
}

impl OptionalAuthHook {
    pub fn new(auth_manager: Arc<AuthManager>) -> Self {
        Self { auth_manager }
    }
}

#[async_trait::async_trait]
impl MiddleWareHandler for OptionalAuthHook {
    async fn handle(&self, mut req: Request, next: &Next) -> silent::Result<Response> {
        // 尝试提取Token
        if let Ok(token) = extract_token(&req)
            && let Ok(user) = self.auth_manager.verify_token(&token)
            && user.status == crate::auth::UserStatus::Active
        {
            // 注入用户对象
            req.configs_mut().insert(user);
        }

        // 无论Token是否有效都继续处理
        next.call(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::RegisterRequest;

    fn create_test_auth_manager() -> Arc<AuthManager> {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test_auth.db");
        Arc::new(AuthManager::new(db_path.to_str().unwrap()).unwrap())
    }

    #[test]
    fn test_extract_token_success() {
        let http_req = http::Request::builder()
            .header("Authorization", "Bearer test-token-123")
            .body(())
            .unwrap();
        let (parts, _) = http_req.into_parts();
        let req = Request::from_parts(parts, ReqBody::Empty);

        let token = extract_token(&req).unwrap();
        assert_eq!(token, "test-token-123");
    }

    #[test]
    fn test_extract_token_missing() {
        let http_req = http::Request::builder().body(()).unwrap();
        let (parts, _) = http_req.into_parts();
        let req = Request::from_parts(parts, ReqBody::Empty);

        let result = extract_token(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_token_invalid_format() {
        let http_req = http::Request::builder()
            .header("Authorization", "InvalidFormat test-token")
            .body(())
            .unwrap();
        let (parts, _) = http_req.into_parts();
        let req = Request::from_parts(parts, ReqBody::Empty);

        let result = extract_token(&req);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_auth_manager_integration() {
        let auth_manager = create_test_auth_manager();

        // 注册用户
        let req = RegisterRequest {
            username: "testuser".to_string(),
            email: "test@example.com".to_string(),
            password: "Test123!@#".to_string(),
        };
        let user_info = auth_manager.register(req).unwrap();
        assert_eq!(user_info.username, "testuser");

        // 登录
        let login_req = crate::auth::LoginRequest {
            username: "testuser".to_string(),
            password: "Test123!@#".to_string(),
        };
        let login_resp = auth_manager.login(login_req).unwrap();
        assert!(!login_resp.access_token.is_empty());

        // 验证Token
        let user = auth_manager.verify_token(&login_resp.access_token).unwrap();
        assert_eq!(user.username, "testuser");
    }
}
