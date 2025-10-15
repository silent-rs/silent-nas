use silent::prelude::*;

/// S3认证信息
#[derive(Clone)]
pub struct S3Auth {
    pub(crate) access_key: String,
}

impl S3Auth {
    pub fn new(access_key: String, _secret_key: String) -> Self {
        Self { access_key }
    }

    /// 验证请求
    pub fn verify_request(&self, req: &Request) -> bool {
        // 简化版认证：检查Authorization头是否包含access_key
        let auth_header = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        match auth_header {
            Some(header) => header.contains(&self.access_key),
            None => false,
        }
    }
}
