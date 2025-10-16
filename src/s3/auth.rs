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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s3_auth_new() {
        let auth = S3Auth::new("test_access_key".to_string(), "test_secret_key".to_string());
        assert_eq!(auth.access_key, "test_access_key");
    }

    #[test]
    fn test_s3_auth_clone() {
        let auth = S3Auth::new("key1".to_string(), "secret1".to_string());
        let cloned = auth.clone();
        assert_eq!(auth.access_key, cloned.access_key);
    }

    #[test]
    fn test_s3_auth_access_key_storage() {
        let key = "my_complex_access_key_123";
        let secret = "my_secret";
        let auth = S3Auth::new(key.to_string(), secret.to_string());

        assert_eq!(auth.access_key, key);
        assert!(!auth.access_key.is_empty());
    }

    #[test]
    fn test_multiple_auth_instances() {
        let auth1 = S3Auth::new("key1".to_string(), "secret1".to_string());
        let auth2 = S3Auth::new("key2".to_string(), "secret2".to_string());

        assert_eq!(auth1.access_key, "key1");
        assert_eq!(auth2.access_key, "key2");
        assert_ne!(auth1.access_key, auth2.access_key);
    }

    #[test]
    fn test_s3_auth_with_empty_strings() {
        let auth = S3Auth::new("".to_string(), "".to_string());
        assert_eq!(auth.access_key, "");
        assert!(auth.access_key.is_empty());
    }

    #[test]
    fn test_s3_auth_with_special_characters() {
        let key = "key!@#$%^&*()";
        let secret = "secret-_=+[]{}";
        let auth = S3Auth::new(key.to_string(), secret.to_string());

        assert_eq!(auth.access_key, key);
    }

    #[test]
    fn test_s3_auth_long_keys() {
        let long_key = "a".repeat(256);
        let long_secret = "b".repeat(512);
        let auth = S3Auth::new(long_key.clone(), long_secret);

        assert_eq!(auth.access_key.len(), 256);
        assert_eq!(auth.access_key, long_key);
    }

    #[test]
    fn test_s3_auth_unicode_keys() {
        let key = "访问密钥_アクセスキー_🔑";
        let secret = "秘密_シークレット";
        let auth = S3Auth::new(key.to_string(), secret.to_string());

        assert_eq!(auth.access_key, key);
    }
}
