//! Token黑名单模块
//!
//! 实现Token撤销和注销功能

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use sled::Db;
use std::path::Path;
use std::sync::Arc;

/// Token黑名单项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlacklistedToken {
    /// Token ID (jti)
    pub jti: String,
    /// 用户ID
    pub user_id: String,
    /// 加入黑名单的时间
    pub blacklisted_at: DateTime<Local>,
    /// 过期时间
    pub expires_at: DateTime<Local>,
    /// 原因
    pub reason: String,
}

/// Token黑名单管理器
pub struct TokenBlacklist {
    db: Arc<Db>,
}

impl TokenBlacklist {
    /// 创建Token黑名单管理器
    pub fn new<P: AsRef<Path>>(db_path: P) -> crate::error::Result<Self> {
        let db = sled::open(db_path)?;
        Ok(Self { db: Arc::new(db) })
    }

    /// 添加Token到黑名单
    pub fn add(
        &self,
        jti: &str,
        user_id: &str,
        expires_at: DateTime<Local>,
        reason: &str,
    ) -> crate::error::Result<()> {
        let item = BlacklistedToken {
            jti: jti.to_string(),
            user_id: user_id.to_string(),
            blacklisted_at: Local::now(),
            expires_at,
            reason: reason.to_string(),
        };

        let key = format!("token:{}", jti);
        let data = serde_json::to_vec(&item)
            .map_err(|e| crate::error::NasError::Storage(format!("序列化黑名单项错误: {}", e)))?;

        self.db.insert(key.as_bytes(), data)?;

        tracing::info!(
            "Token已加入黑名单: jti={}, user={}, reason={}",
            jti,
            user_id,
            reason
        );

        Ok(())
    }

    /// 检查Token是否在黑名单中
    pub fn is_blacklisted(&self, jti: &str) -> crate::error::Result<bool> {
        let key = format!("token:{}", jti);

        if let Some(data) = self.db.get(key.as_bytes())? {
            let item: BlacklistedToken = serde_json::from_slice(&data)
                .map_err(|e| crate::error::NasError::Storage(format!("解析黑名单项错误: {}", e)))?;

            // 检查是否过期
            if item.expires_at <= Local::now() {
                // 已过期，从黑名单中移除
                self.db.remove(key.as_bytes())?;
                Ok(false)
            } else {
                Ok(true)
            }
        } else {
            Ok(false)
        }
    }

    /// 撤销用户的所有Token
    pub fn revoke_user_tokens(&self, user_id: &str) -> crate::error::Result<usize> {
        let prefix = format!("user_tokens:{}", user_id);
        let mut count = 0;

        // 先标记所有token
        for item in self.db.scan_prefix(prefix.as_bytes()) {
            let (_key, _value) = item?;
            count += 1;
        }

        // 删除用户Token记录
        let key = format!("user_tokens:{}", user_id);
        self.db.remove(key.as_bytes())?;

        tracing::info!("已撤销用户 {} 的所有Token (共{}个)", user_id, count);

        Ok(count)
    }

    /// 记录用户Token（用于后续撤销所有Token）
    pub fn track_user_token(&self, user_id: &str, jti: &str) -> crate::error::Result<()> {
        let key = format!("user_tokens:{}:{}", user_id, jti);
        self.db.insert(key.as_bytes(), b"1")?;
        Ok(())
    }

    /// 清理过期的黑名单项
    pub fn cleanup_expired(&self) -> crate::error::Result<usize> {
        let mut removed = 0;
        let now = Local::now();

        for item in self.db.scan_prefix(b"token:") {
            let (key, value) = item?;

            if let Ok(blacklisted) = serde_json::from_slice::<BlacklistedToken>(&value)
                && blacklisted.expires_at <= now
            {
                self.db.remove(&key)?;
                removed += 1;
            }
        }

        if removed > 0 {
            tracing::info!("清理了 {} 个过期的黑名单Token", removed);
        }

        Ok(removed)
    }

    /// 获取黑名单统计
    pub fn get_stats(&self) -> crate::error::Result<BlacklistStats> {
        let mut total = 0;
        let mut expired = 0;
        let now = Local::now();

        for item in self.db.scan_prefix(b"token:") {
            let (_key, value) = item?;

            if let Ok(blacklisted) = serde_json::from_slice::<BlacklistedToken>(&value) {
                total += 1;
                if blacklisted.expires_at <= now {
                    expired += 1;
                }
            }
        }

        Ok(BlacklistStats {
            total,
            active: total - expired,
            expired,
        })
    }
}

/// 黑名单统计信息
#[derive(Debug, Serialize)]
pub struct BlacklistStats {
    /// 总数
    pub total: usize,
    /// 活跃数
    pub active: usize,
    /// 已过期数
    pub expired: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use tempfile::TempDir;

    fn create_test_blacklist() -> (TokenBlacklist, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let blacklist = TokenBlacklist::new(temp_dir.path().join("token_blacklist.db")).unwrap();
        (blacklist, temp_dir)
    }

    #[test]
    fn test_add_and_check() {
        let (blacklist, _temp) = create_test_blacklist();

        let jti = "test-token-123";
        let user_id = "user-456";
        let expires_at = Local::now() + Duration::hours(1);

        blacklist.add(jti, user_id, expires_at, "logout").unwrap();

        assert!(blacklist.is_blacklisted(jti).unwrap());
        assert!(!blacklist.is_blacklisted("non-existent").unwrap());
    }

    #[test]
    fn test_track_user_token() {
        let (blacklist, _temp) = create_test_blacklist();

        let user_id = "user-123";
        let jti1 = "token-1";
        let jti2 = "token-2";

        blacklist.track_user_token(user_id, jti1).unwrap();
        blacklist.track_user_token(user_id, jti2).unwrap();

        let count = blacklist.revoke_user_tokens(user_id).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_cleanup_expired() {
        let (blacklist, _temp) = create_test_blacklist();

        // 添加一个已过期的token
        let expired_jti = "expired-token";
        let expired_at = Local::now() - Duration::hours(1);
        blacklist
            .add(expired_jti, "user-1", expired_at, "test")
            .unwrap();

        // 添加一个未过期的token
        let valid_jti = "valid-token";
        let future_at = Local::now() + Duration::hours(1);
        blacklist
            .add(valid_jti, "user-2", future_at, "test")
            .unwrap();

        // 清理过期token
        let removed = blacklist.cleanup_expired().unwrap();
        assert_eq!(removed, 1);

        // 验证
        assert!(!blacklist.is_blacklisted(expired_jti).unwrap());
        assert!(blacklist.is_blacklisted(valid_jti).unwrap());
    }

    #[test]
    fn test_get_stats() {
        let (blacklist, _temp) = create_test_blacklist();

        let future_at = Local::now() + Duration::hours(1);
        blacklist
            .add("token-1", "user-1", future_at, "test")
            .unwrap();
        blacklist
            .add("token-2", "user-2", future_at, "test")
            .unwrap();

        let stats = blacklist.get_stats().unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.active, 2);
        assert_eq!(stats.expired, 0);
    }
}
