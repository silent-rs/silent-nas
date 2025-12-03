//! 静态文件服务

use http::StatusCode;
use silent::SilentError;
use silent::prelude::*;
use std::path::{Path, PathBuf};
use tokio::fs;

/// 静态文件处理器
///
/// 服务管理端前端构建产物
pub async fn serve_static_file(req: Request) -> silent::Result<Response> {
    // 获取请求路径
    let path = req.uri().path();

    // 移除 /admin 前缀
    let file_path = path.strip_prefix("/admin").unwrap_or(path);

    // 如果是根路径或目录，返回 index.html
    let file_path = if file_path == "/" || file_path.is_empty() || !file_path.contains('.') {
        "/index.html"
    } else {
        file_path
    };

    // 构建完整文件路径
    let mut full_path = PathBuf::from("admin-dashboard/dist");
    full_path.push(file_path.trim_start_matches('/'));

    // 防止目录遍历攻击
    let canonical_base = fs::canonicalize("admin-dashboard/dist")
        .await
        .map_err(|e| {
            tracing::error!("无法获取静态文件目录: {}", e);
            SilentError::business_error(StatusCode::INTERNAL_SERVER_ERROR, "静态文件目录配置错误")
        })?;

    let canonical_path = match fs::canonicalize(&full_path).await {
        Ok(path) => path,
        Err(_) => {
            // 文件不存在，返回 404
            return Err(SilentError::business_error(
                StatusCode::NOT_FOUND,
                "文件不存在",
            ));
        }
    };

    // 确保请求的文件在允许的目录内
    if !canonical_path.starts_with(&canonical_base) {
        return Err(SilentError::business_error(
            StatusCode::FORBIDDEN,
            "禁止访问",
        ));
    }

    // 读取文件内容
    let content = fs::read(&canonical_path).await.map_err(|e| {
        tracing::error!("读取文件失败: {} - {}", canonical_path.display(), e);
        SilentError::business_error(StatusCode::INTERNAL_SERVER_ERROR, "读取文件失败")
    })?;

    // 根据文件扩展名设置 Content-Type
    let content_type = get_content_type(&canonical_path);

    // 构建响应
    let mut resp = Response::empty();
    resp.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static(content_type),
    );
    resp.headers_mut().insert(
        http::header::CACHE_CONTROL,
        http::HeaderValue::from_static("public, max-age=3600"),
    );
    resp.set_body(full(content));
    Ok(resp)
}

/// 根据文件扩展名获取 Content-Type
fn get_content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("eot") => "application/vnd.ms-fontobject",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_content_type() {
        assert_eq!(
            get_content_type(&PathBuf::from("test.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            get_content_type(&PathBuf::from("test.css")),
            "text/css; charset=utf-8"
        );
        assert_eq!(
            get_content_type(&PathBuf::from("test.js")),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(get_content_type(&PathBuf::from("test.png")), "image/png");
    }
}
