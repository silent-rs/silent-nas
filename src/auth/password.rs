//! 密码哈希处理

use crate::error::{NasError, Result};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

/// 密码处理器
pub struct PasswordHandler;

impl PasswordHandler {
    /// 哈希密码
    pub fn hash_password(password: &str) -> Result<String> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();

        let password_hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| NasError::Auth(format!("密码哈希失败: {}", e)))?
            .to_string();

        Ok(password_hash)
    }

    /// 验证密码
    pub fn verify_password(password: &str, password_hash: &str) -> Result<bool> {
        let parsed_hash = PasswordHash::new(password_hash)
            .map_err(|e| NasError::Auth(format!("解析密码哈希失败: {}", e)))?;

        let argon2 = Argon2::default();
        Ok(argon2
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok())
    }

    /// 检查密码强度
    pub fn check_password_strength(password: &str) -> Result<PasswordStrength> {
        let length = password.len();
        let has_lowercase = password.chars().any(|c| c.is_lowercase());
        let has_uppercase = password.chars().any(|c| c.is_uppercase());
        let has_digit = password.chars().any(|c| c.is_ascii_digit());
        let has_special = password.chars().any(|c| !c.is_alphanumeric());

        let score = [has_lowercase, has_uppercase, has_digit, has_special]
            .iter()
            .filter(|&&x| x)
            .count();

        let strength = if length < 8 {
            PasswordStrength::Weak
        } else if length >= 12 && score >= 3 {
            PasswordStrength::Strong
        } else if score >= 2 {
            PasswordStrength::Medium
        } else {
            PasswordStrength::Weak
        };

        Ok(strength)
    }
}

/// 密码强度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordStrength {
    Weak,
    Medium,
    Strong,
}

impl std::fmt::Display for PasswordStrength {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PasswordStrength::Weak => write!(f, "Weak"),
            PasswordStrength::Medium => write!(f, "Medium"),
            PasswordStrength::Strong => write!(f, "Strong"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_password() {
        let password = "SecurePassword123!";
        let hash = PasswordHandler::hash_password(password).unwrap();

        // 哈希不应该等于原密码
        assert_ne!(hash, password);
        // 哈希应该是 Argon2 格式
        assert!(hash.starts_with("$argon2"));
    }

    #[test]
    fn test_verify_password() {
        let password = "SecurePassword123!";
        let hash = PasswordHandler::hash_password(password).unwrap();

        // 正确密码应该验证通过
        assert!(PasswordHandler::verify_password(password, &hash).unwrap());

        // 错误密码应该验证失败
        assert!(!PasswordHandler::verify_password("WrongPassword", &hash).unwrap());
    }

    #[test]
    fn test_different_hashes() {
        let password = "SamePassword123!";
        let hash1 = PasswordHandler::hash_password(password).unwrap();
        let hash2 = PasswordHandler::hash_password(password).unwrap();

        // 即使密码相同，哈希也应该不同（因为盐不同）
        assert_ne!(hash1, hash2);

        // 但都应该能验证成功
        assert!(PasswordHandler::verify_password(password, &hash1).unwrap());
        assert!(PasswordHandler::verify_password(password, &hash2).unwrap());
    }

    #[test]
    fn test_password_strength_weak() {
        let weak_passwords = vec![
            "short",    // 太短
            "12345678", // 只有数字
            "abcdefgh", // 只有小写字母
            "ABCDEFGH", // 只有大写字母
        ];

        for password in weak_passwords {
            let strength = PasswordHandler::check_password_strength(password).unwrap();
            assert_eq!(strength, PasswordStrength::Weak);
        }
    }

    #[test]
    fn test_password_strength_medium() {
        let medium_passwords = vec![
            "Password1",   // 大小写+数字
            "password123", // 小写+数字（8字符）
            "MyPass123",   // 大小写+数字
        ];

        for password in medium_passwords {
            let strength = PasswordHandler::check_password_strength(password).unwrap();
            assert!(strength == PasswordStrength::Medium || strength == PasswordStrength::Strong);
        }
    }

    #[test]
    fn test_password_strength_strong() {
        let strong_passwords = vec![
            "StrongP@ssw0rd!",  // 12字符，所有类型
            "MySecure123!Pass", // 长度足够，复杂
            "Compl3x!Password", // 符合强密码要求
        ];

        for password in strong_passwords {
            let strength = PasswordHandler::check_password_strength(password).unwrap();
            assert_eq!(strength, PasswordStrength::Strong);
        }
    }

    #[test]
    fn test_invalid_hash_format() {
        let result = PasswordHandler::verify_password("password", "invalid-hash");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_password() {
        let result = PasswordHandler::hash_password("");
        assert!(result.is_ok()); // Argon2 可以哈希空字符串

        let strength = PasswordHandler::check_password_strength("").unwrap();
        assert_eq!(strength, PasswordStrength::Weak);
    }

    #[test]
    fn test_unicode_password() {
        let password = "密码123!@#";
        let hash = PasswordHandler::hash_password(password).unwrap();
        assert!(PasswordHandler::verify_password(password, &hash).unwrap());
    }

    #[test]
    fn test_very_long_password() {
        let password = "a".repeat(100);
        let hash = PasswordHandler::hash_password(&password).unwrap();
        assert!(PasswordHandler::verify_password(&password, &hash).unwrap());
    }

    #[test]
    fn test_special_characters() {
        let passwords_with_special = vec!["Pass!@#$%^&*()", "P@ssw0rd!", "Test#123"];

        for password in passwords_with_special {
            let hash = PasswordHandler::hash_password(password).unwrap();
            assert!(PasswordHandler::verify_password(password, &hash).unwrap());
        }
    }
}
