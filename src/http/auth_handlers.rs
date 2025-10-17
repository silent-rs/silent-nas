//! 认证API处理器

use super::state::AppState;
use crate::auth::{ChangePasswordRequest, LoginRequest, RegisterRequest, UserInfo};
use crate::error::NasError;
use http::StatusCode;
use http_body_util::BodyExt;
use silent::extractor::Configs as CfgExtractor;
use silent::prelude::*;
use silent::SilentError;

/// 用户注册
///
/// POST /api/auth/register
/// Body: { "username": "...", "email": "...", "password": "..." }
pub async fn register_handler(
    mut req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    // 获取认证管理器
    let auth_manager = state
        .auth_manager
        .as_ref()
        .ok_or_else(|| SilentError::business_error(StatusCode::SERVICE_UNAVAILABLE, "认证功能未启用"))?;

    // 解析请求体
    let body = req.take_body();
    let bytes = match body {
        ReqBody::Incoming(body) => body.collect().await?.to_bytes().to_vec(),
        ReqBody::Once(bytes) => bytes.to_vec(),
        ReqBody::Empty => {
            return Err(SilentError::business_error(StatusCode::BAD_REQUEST, "请求体为空"));
        }
    };

    let register_req: RegisterRequest = serde_json::from_slice(&bytes)
        .map_err(|e| SilentError::business_error(StatusCode::BAD_REQUEST, &e.to_string()))?;

    // 注册用户
    let user_info = auth_manager.register(register_req).map_err(|e| match e {
        NasError::Auth(msg) => SilentError::business_error(StatusCode::BAD_REQUEST, &msg),
        _ => SilentError::business_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    })?;

    // 返回用户信息
    Ok(serde_json::to_value(&user_info).unwrap())
}

/// 用户登录
///
/// POST /api/auth/login
/// Body: { "username": "...", "password": "..." }
pub async fn login_handler(
    mut req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    // 获取认证管理器
    let auth_manager = state
        .auth_manager
        .as_ref()
        .ok_or_else(|| SilentError::business_error(StatusCode::SERVICE_UNAVAILABLE, "认证功能未启用"))?;

    // 解析请求体
    let body = req.take_body();
    let bytes = match body {
        ReqBody::Incoming(body) => body.collect().await?.to_bytes().to_vec(),
        ReqBody::Once(bytes) => bytes.to_vec(),
        ReqBody::Empty => {
            return Err(SilentError::business_error(StatusCode::BAD_REQUEST, "请求体为空"));
        }
    };

    let login_req: LoginRequest =
        serde_json::from_slice(&bytes).map_err(|e| SilentError::business_error(StatusCode::BAD_REQUEST, &e.to_string()))?;

    // 登录
    let login_resp = auth_manager.login(login_req).map_err(|e| match e {
        NasError::Auth(msg) => SilentError::business_error(StatusCode::BAD_REQUEST, &msg),
        _ => SilentError::business_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    })?;

    // 返回登录响应
    Ok(serde_json::to_value(&login_resp).unwrap())
}

/// 刷新Token
///
/// POST /api/auth/refresh
/// Body: { "refresh_token": "..." }
pub async fn refresh_handler(
    mut req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct RefreshRequest {
        refresh_token: String,
    }

    // 获取认证管理器
    let auth_manager = state
        .auth_manager
        .as_ref()
        .ok_or_else(|| SilentError::business_error(StatusCode::SERVICE_UNAVAILABLE, "认证功能未启用"))?;

    // 解析请求体
    let body = req.take_body();
    let bytes = match body {
        ReqBody::Incoming(body) => body.collect().await?.to_bytes().to_vec(),
        ReqBody::Once(bytes) => bytes.to_vec(),
        ReqBody::Empty => {
            return Err(SilentError::business_error(StatusCode::BAD_REQUEST, "请求体为空"));
        }
    };

    let refresh_req: RefreshRequest =
        serde_json::from_slice(&bytes).map_err(|e| SilentError::business_error(StatusCode::BAD_REQUEST, &e.to_string()))?;

    // 刷新Token
    let login_resp = auth_manager
        .refresh_token(&refresh_req.refresh_token)
        .map_err(|e| match e {
            NasError::Auth(msg) => SilentError::business_error(StatusCode::BAD_REQUEST, &msg),
            _ => SilentError::business_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        })?;

    // 返回新的Token
    Ok(serde_json::to_value(&login_resp).unwrap())
}

/// 获取当前用户信息
///
/// GET /api/auth/me
/// Header: Authorization: Bearer <token>
pub async fn me_handler(
    req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    // 获取认证管理器
    let auth_manager = state
        .auth_manager
        .as_ref()
        .ok_or_else(|| SilentError::business_error(StatusCode::SERVICE_UNAVAILABLE, "认证功能未启用"))?;

    // 从请求头获取Token
    let token = extract_token(&req)?;

    // 验证Token并获取用户
    let user = auth_manager.verify_token(&token).map_err(|e| match e {
        NasError::Auth(msg) => SilentError::business_error(StatusCode::BAD_REQUEST, &msg),
        _ => SilentError::business_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    })?;

    // 转换为UserInfo（隐藏密码）
    let user_info: UserInfo = user.into();

    // 返回用户信息
    Ok(serde_json::to_value(&user_info).unwrap())
}

/// 修改密码
///
/// PUT /api/auth/password
/// Header: Authorization: Bearer <token>
/// Body: { "old_password": "...", "new_password": "..." }
pub async fn change_password_handler(
    mut req: Request,
    CfgExtractor(state): CfgExtractor<AppState>,
) -> silent::Result<serde_json::Value> {
    // 获取认证管理器
    let auth_manager = state
        .auth_manager
        .as_ref()
        .ok_or_else(|| SilentError::business_error(StatusCode::SERVICE_UNAVAILABLE, "认证功能未启用"))?;

    // 从请求头获取Token
    let token = extract_token(&req)?;

    // 验证Token并获取用户ID
    let user = auth_manager.verify_token(&token).map_err(|e| match e {
        NasError::Auth(msg) => SilentError::business_error(StatusCode::BAD_REQUEST, &msg),
        _ => SilentError::business_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    })?;

    // 解析请求体
    let body = req.take_body();
    let bytes = match body {
        ReqBody::Incoming(body) => body.collect().await?.to_bytes().to_vec(),
        ReqBody::Once(bytes) => bytes.to_vec(),
        ReqBody::Empty => {
            return Err(SilentError::business_error(StatusCode::BAD_REQUEST, "请求体为空"));
        }
    };

    let change_req: ChangePasswordRequest =
        serde_json::from_slice(&bytes).map_err(|e| SilentError::business_error(StatusCode::BAD_REQUEST, &e.to_string()))?;

    // 修改密码
    auth_manager
        .change_password(&user.id, change_req)
        .map_err(|e| match e {
            NasError::Auth(msg) => SilentError::business_error(StatusCode::BAD_REQUEST, &msg),
            _ => SilentError::business_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        })?;

    // 返回成功
    Ok(serde_json::json!({
        "message": "密码修改成功"
    }))
}

/// 从请求头提取Bearer Token
fn extract_token(req: &Request) -> silent::Result<String> {
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| SilentError::business_error(StatusCode::UNAUTHORIZED, "缺少Authorization头"))?;

    if !auth_header.starts_with("Bearer ") {
        return Err(SilentError::business_error(StatusCode::UNAUTHORIZED, "无效的Authorization格式"));
    }

    Ok(auth_header[7..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_token() {
        let req = Request::builder()
            .header("Authorization", "Bearer test-token-123")
            .body(String::new())
            .unwrap();

        let token = extract_token(&req).unwrap();
        assert_eq!(token, "test-token-123");
    }

    #[test]
    fn test_extract_token_missing_header() {
        let req = Request::builder().body(String::new()).unwrap();

        let result = extract_token(&req);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_token_invalid_format() {
        let req = Request::builder()
            .header("Authorization", "InvalidFormat test-token")
            .body(String::new()).unwrap();

        let result = extract_token(&req);
        assert!(result.is_err());
    }
}
