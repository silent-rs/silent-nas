//! 登录限流模块
//!
//! 提供基于IP和用户的登录失败次数限制功能

use chrono::{DateTime, Duration, Local};
use serde::{Deserialize, Serialize};
use sled::Db;
use std::path::Path;
use std::sync::Arc;

/// 登录尝试记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginAttempt {
    /// 用户标识（用户名或IP）
    pub identifier: String,
    /// 失败次数
    pub failed_count: u32,
    /// 首次失败时间
    pub first_failed_at: DateTime<Local>,
    /// 最后失败时间
    pub last_failed_at: DateTime<Local>,
    /// 锁定到期时间（如果被锁定）
    pub locked_until: Option<DateTime<Local>>,
}

impl LoginAttempt {
    /// 创建新的尝试记录
    pub fn new(identifier: String) -> Self {
        Self {
            identifier,
            failed_count: 1,
            first_failed_at: Local::now(),
            last_failed_at: Local::now(),
            locked_until: None,
        }
    }

    /// 增加失败次数
    pub fn increment(&mut self) {
        self.failed_count += 1;
        self.last_failed_at = Local::now();
    }

    /// 检查是否被锁定
    pub fn is_locked(&self) -> bool {
        if let Some(locked_until) = self.locked_until {
            locked_until > Local::now()
        } else {
            false
        }
    }

    /// 设置锁定
    pub fn lock_for(&mut self, duration_minutes: i64) {
        self.locked_until = Some(Local::now() + Duration::minutes(duration_minutes));
    }

    /// 检查是否应该重置（超过时间窗口）
    pub fn should_reset(&self, window_minutes: i64) -> bool {
        let window = Duration::minutes(window_minutes);
        Local::now() - self.first_failed_at > window
    }
}

/// 限流配置
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// 最大失败次数
    pub max_attempts: u32,
    /// 时间窗口（分钟）
    pub window_minutes: i64,
    /// 锁定时长（分钟）
    pub lock_duration_minutes: i64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            window_minutes: 15,
            lock_duration_minutes: 30,
        }
    }
}

/// 登录限流管理器
pub struct RateLimiter {
    db: Arc<Db>,
    config: RateLimitConfig,
}

impl RateLimiter {
    /// 创建限流管理器
    pub fn new<P: AsRef<Path>>(db_path: P, config: RateLimitConfig) -> crate::error::Result<Self> {
        let db = sled::open(db_path)?;
        Ok(Self {
            db: Arc::new(db),
            config,
        })
    }

    /// 记录登录失败
    pub fn record_failure(&self, identifier: &str) -> crate::error::Result<()> {
        let key = format!("attempt:{}", identifier);

        let attempt = if let Some(data) = self.db.get(key.as_bytes())? {
            let mut attempt: LoginAttempt = serde_json::from_slice(&data)
                .map_err(|e| crate::error::NasError::Storage(format!("解析失败记录错误: {}", e)))?;

            // 如果超过时间窗口，重置
            if attempt.should_reset(self.config.window_minutes) {
                LoginAttempt::new(identifier.to_string())
            } else {
                attempt.increment();

                // 如果达到最大尝试次数，锁定账户
                if attempt.failed_count >= self.config.max_attempts
                    && attempt.locked_until.is_none()
                {
                    attempt.lock_for(self.config.lock_duration_minutes);
                    tracing::warn!(
                        "用户/IP {} 因失败次数过多被锁定 {} 分钟",
                        identifier,
                        self.config.lock_duration_minutes
                    );
                }

                attempt
            }
        } else {
            LoginAttempt::new(identifier.to_string())
        };

        // 保存到数据库
        let data = serde_json::to_vec(&attempt)
            .map_err(|e| crate::error::NasError::Storage(format!("序列化失败记录错误: {}", e)))?;
        self.db.insert(key.as_bytes(), data)?;

        Ok(())
    }

    /// 检查是否被锁定
    pub fn is_locked(&self, identifier: &str) -> crate::error::Result<bool> {
        let key = format!("attempt:{}", identifier);

        if let Some(data) = self.db.get(key.as_bytes())? {
            let attempt: LoginAttempt = serde_json::from_slice(&data)
                .map_err(|e| crate::error::NasError::Storage(format!("解析失败记录错误: {}", e)))?;

            // 如果锁定已过期，清除记录
            if attempt.is_locked() {
                Ok(true)
            } else if attempt.locked_until.is_some() {
                // 锁定已过期，清除记录
                self.db.remove(key.as_bytes())?;
                Ok(false)
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }

    /// 获取剩余锁定时间（秒）
    pub fn get_lock_remaining(&self, identifier: &str) -> crate::error::Result<Option<i64>> {
        let key = format!("attempt:{}", identifier);

        if let Some(data) = self.db.get(key.as_bytes())? {
            let attempt: LoginAttempt = serde_json::from_slice(&data)
                .map_err(|e| crate::error::NasError::Storage(format!("解析失败记录错误: {}", e)))?;

            if let Some(locked_until) = attempt.locked_until {
                let remaining = locked_until - Local::now();
                if remaining.num_seconds() > 0 {
                    return Ok(Some(remaining.num_seconds()));
                }
            }
        }

        Ok(None)
    }

    /// 清除失败记录（登录成功后调用）
    pub fn clear(&self, identifier: &str) -> crate::error::Result<()> {
        let key = format!("attempt:{}", identifier);
        self.db.remove(key.as_bytes())?;
        Ok(())
    }

    /// 获取失败次数
    pub fn get_failed_count(&self, identifier: &str) -> crate::error::Result<u32> {
        let key = format!("attempt:{}", identifier);

        if let Some(data) = self.db.get(key.as_bytes())? {
            let attempt: LoginAttempt = serde_json::from_slice(&data)
                .map_err(|e| crate::error::NasError::Storage(format!("解析失败记录错误: {}", e)))?;

            if attempt.should_reset(self.config.window_minutes) {
                // 过期了，返回0
                Ok(0)
            } else {
                Ok(attempt.failed_count)
            }
        } else {
            Ok(0)
        }
    }

    /// 手动锁定用户/IP
    pub fn manual_lock(&self, identifier: &str, duration_minutes: i64) -> crate::error::Result<()> {
        let key = format!("attempt:{}", identifier);

        let mut attempt = if let Some(data) = self.db.get(key.as_bytes())? {
            serde_json::from_slice(&data)
                .map_err(|e| crate::error::NasError::Storage(format!("解析失败记录错误: {}", e)))?
        } else {
            LoginAttempt::new(identifier.to_string())
        };

        attempt.lock_for(duration_minutes);

        let data = serde_json::to_vec(&attempt)
            .map_err(|e| crate::error::NasError::Storage(format!("序列化失败记录错误: {}", e)))?;
        self.db.insert(key.as_bytes(), data)?;

        tracing::info!("手动锁定用户/IP: {} ({}分钟)", identifier, duration_minutes);

        Ok(())
    }

    /// 手动解锁用户/IP
    pub fn manual_unlock(&self, identifier: &str) -> crate::error::Result<()> {
        self.clear(identifier)?;
        tracing::info!("手动解锁用户/IP: {}", identifier);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_limiter() -> (RateLimiter, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = RateLimitConfig {
            max_attempts: 3,
            window_minutes: 15,
            lock_duration_minutes: 30,
        };
        let limiter = RateLimiter::new(temp_dir.path().join("rate_limit.db"), config).unwrap();
        (limiter, temp_dir)
    }

    #[test]
    fn test_record_and_check() {
        let (limiter, _temp) = create_test_limiter();

        // 第一次失败
        limiter.record_failure("test@example.com").unwrap();
        assert!(!limiter.is_locked("test@example.com").unwrap());
        assert_eq!(limiter.get_failed_count("test@example.com").unwrap(), 1);

        // 第二次失败
        limiter.record_failure("test@example.com").unwrap();
        assert!(!limiter.is_locked("test@example.com").unwrap());
        assert_eq!(limiter.get_failed_count("test@example.com").unwrap(), 2);

        // 第三次失败 - 应该被锁定
        limiter.record_failure("test@example.com").unwrap();
        assert!(limiter.is_locked("test@example.com").unwrap());
        assert_eq!(limiter.get_failed_count("test@example.com").unwrap(), 3);
    }

    #[test]
    fn test_clear_after_success() {
        let (limiter, _temp) = create_test_limiter();

        limiter.record_failure("test@example.com").unwrap();
        limiter.record_failure("test@example.com").unwrap();

        // 登录成功，清除记录
        limiter.clear("test@example.com").unwrap();

        assert!(!limiter.is_locked("test@example.com").unwrap());
        assert_eq!(limiter.get_failed_count("test@example.com").unwrap(), 0);
    }

    #[test]
    fn test_manual_lock_unlock() {
        let (limiter, _temp) = create_test_limiter();

        // 手动锁定
        limiter.manual_lock("test@example.com", 10).unwrap();
        assert!(limiter.is_locked("test@example.com").unwrap());

        // 手动解锁
        limiter.manual_unlock("test@example.com").unwrap();
        assert!(!limiter.is_locked("test@example.com").unwrap());
    }

    #[test]
    fn test_get_lock_remaining() {
        let (limiter, _temp) = create_test_limiter();

        limiter.manual_lock("test@example.com", 10).unwrap();

        let remaining = limiter.get_lock_remaining("test@example.com").unwrap();
        assert!(remaining.is_some());
        assert!(remaining.unwrap() > 0);
        assert!(remaining.unwrap() <= 600); // 10 minutes = 600 seconds
    }
}
