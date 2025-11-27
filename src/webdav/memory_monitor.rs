//! WebDAV 内存使用监控
//!
//! 用于监控大文件上传过程中的内存占用，确保不超过限制

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// 内存使用监控器
#[derive(Clone)]
#[allow(dead_code)]
pub struct MemoryMonitor {
    /// 当前内存使用量 (字节)
    current_usage: Arc<AtomicU64>,
    /// 内存使用限制 (字节)
    limit: u64,
    /// 警告阈值 (字节)
    warning_threshold: u64,
}

impl MemoryMonitor {
    /// 创建新的内存监控器
    ///
    /// # 参数
    /// - `limit_mb`: 内存限制 (MB)
    /// - `warning_percent`: 警告阈值百分比 (0-100)
    #[allow(dead_code)]
    pub fn new(limit_mb: u64, warning_percent: u8) -> Self {
        let limit = limit_mb * 1024 * 1024; // 转换为字节
        let warning_threshold = limit * (warning_percent as u64) / 100;

        Self {
            current_usage: Arc::new(AtomicU64::new(0)),
            limit,
            warning_threshold,
        }
    }

    /// 默认配置: 100MB 限制, 80% 警告阈值
    #[allow(dead_code)]
    pub fn default() -> Self {
        Self::new(100, 80)
    }

    /// 分配内存
    ///
    /// # 返回
    /// - `Ok(())`: 分配成功
    /// - `Err(String)`: 超过限制
    #[allow(dead_code)]
    pub fn allocate(&self, size: u64) -> Result<(), String> {
        let current = self.current_usage.load(Ordering::Relaxed);
        let new_usage = current + size;

        if new_usage > self.limit {
            return Err(format!(
                "内存分配失败: 请求 {}MB, 当前使用 {}MB, 限制 {}MB",
                size / 1024 / 1024,
                current / 1024 / 1024,
                self.limit / 1024 / 1024
            ));
        }

        self.current_usage.store(new_usage, Ordering::Relaxed);

        // 检查警告阈值
        if new_usage >= self.warning_threshold {
            tracing::warn!(
                "内存使用接近限制: {:.1}% ({}/{}MB)",
                (new_usage as f64 / self.limit as f64) * 100.0,
                new_usage / 1024 / 1024,
                self.limit / 1024 / 1024
            );
        }

        Ok(())
    }

    /// 释放内存
    #[allow(dead_code)]
    pub fn release(&self, size: u64) {
        let current = self.current_usage.load(Ordering::Relaxed);
        let new_usage = current.saturating_sub(size);
        self.current_usage.store(new_usage, Ordering::Relaxed);
    }

    /// 获取当前内存使用量 (字节)
    #[allow(dead_code)]
    pub fn current_usage(&self) -> u64 {
        self.current_usage.load(Ordering::Relaxed)
    }

    /// 获取当前内存使用量 (MB)
    #[allow(dead_code)]
    pub fn current_usage_mb(&self) -> f64 {
        self.current_usage() as f64 / 1024.0 / 1024.0
    }

    /// 获取内存限制 (字节)
    #[allow(dead_code)]
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// 获取内存限制 (MB)
    #[allow(dead_code)]
    pub fn limit_mb(&self) -> u64 {
        self.limit / 1024 / 1024
    }

    /// 获取剩余可用内存 (字节)
    #[allow(dead_code)]
    pub fn available(&self) -> u64 {
        self.limit.saturating_sub(self.current_usage())
    }

    /// 获取内存使用百分比
    #[allow(dead_code)]
    pub fn usage_percent(&self) -> f64 {
        if self.limit == 0 {
            0.0
        } else {
            (self.current_usage() as f64 / self.limit as f64) * 100.0
        }
    }

    /// 检查是否超过警告阈值
    #[allow(dead_code)]
    pub fn is_warning(&self) -> bool {
        self.current_usage() >= self.warning_threshold
    }

    /// 检查是否可以分配指定大小的内存
    #[allow(dead_code)]
    pub fn can_allocate(&self, size: u64) -> bool {
        let current = self.current_usage();
        current + size <= self.limit
    }

    /// 重置内存使用量
    #[allow(dead_code)]
    pub fn reset(&self) {
        self.current_usage.store(0, Ordering::Relaxed);
    }
}

/// RAII 内存分配守卫
///
/// 自动管理内存分配和释放
#[allow(dead_code)]
pub struct MemoryGuard {
    monitor: MemoryMonitor,
    size: u64,
}

impl MemoryGuard {
    /// 创建新的内存守卫
    #[allow(dead_code)]
    pub fn new(monitor: MemoryMonitor, size: u64) -> Result<Self, String> {
        monitor.allocate(size)?;
        Ok(Self { monitor, size })
    }

    /// 获取监控器
    #[allow(dead_code)]
    pub fn monitor(&self) -> &MemoryMonitor {
        &self.monitor
    }
}

impl Drop for MemoryGuard {
    fn drop(&mut self) {
        self.monitor.release(self.size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_monitor_new() {
        let monitor = MemoryMonitor::new(100, 80); // 100MB, 80% 警告
        assert_eq!(monitor.limit_mb(), 100);
        assert_eq!(monitor.current_usage(), 0);
        assert_eq!(monitor.usage_percent(), 0.0);
        assert!(!monitor.is_warning());
    }

    #[test]
    fn test_memory_monitor_allocate_release() {
        let monitor = MemoryMonitor::new(100, 80);

        // 分配 50MB
        let size = 50 * 1024 * 1024;
        assert!(monitor.allocate(size).is_ok());
        assert_eq!(monitor.current_usage(), size);
        assert_eq!(monitor.usage_percent(), 50.0);

        // 释放 30MB
        let release_size = 30 * 1024 * 1024;
        monitor.release(release_size);
        assert_eq!(monitor.current_usage(), size - release_size);
        assert_eq!(monitor.usage_percent(), 20.0);
    }

    #[test]
    fn test_memory_monitor_limit() {
        let monitor = MemoryMonitor::new(100, 80);

        // 分配 90MB - 成功
        let size1 = 90 * 1024 * 1024;
        assert!(monitor.allocate(size1).is_ok());
        assert!(monitor.is_warning()); // 超过 80% 阈值

        // 再分配 20MB - 失败 (总计 110MB > 100MB)
        let size2 = 20 * 1024 * 1024;
        assert!(monitor.allocate(size2).is_err());
    }

    #[test]
    fn test_memory_monitor_can_allocate() {
        let monitor = MemoryMonitor::new(100, 80);

        assert!(monitor.can_allocate(50 * 1024 * 1024));
        assert!(monitor.can_allocate(100 * 1024 * 1024));
        assert!(!monitor.can_allocate(101 * 1024 * 1024));

        monitor.allocate(50 * 1024 * 1024).unwrap();
        assert!(monitor.can_allocate(50 * 1024 * 1024));
        assert!(!monitor.can_allocate(51 * 1024 * 1024));
    }

    #[test]
    fn test_memory_guard() {
        let monitor = MemoryMonitor::new(100, 80);
        let size = 50 * 1024 * 1024;

        {
            let _guard = MemoryGuard::new(monitor.clone(), size).unwrap();
            assert_eq!(monitor.current_usage(), size);
        }

        // 守卫析构后自动释放
        assert_eq!(monitor.current_usage(), 0);
    }

    #[test]
    fn test_memory_guard_limit() {
        let monitor = MemoryMonitor::new(100, 80);
        let size = 110 * 1024 * 1024; // 超过限制

        let result = MemoryGuard::new(monitor.clone(), size);
        assert!(result.is_err());
        assert_eq!(monitor.current_usage(), 0); // 分配失败，没有占用内存
    }

    #[test]
    fn test_memory_monitor_reset() {
        let monitor = MemoryMonitor::new(100, 80);

        monitor.allocate(50 * 1024 * 1024).unwrap();
        assert_eq!(monitor.current_usage(), 50 * 1024 * 1024);

        monitor.reset();
        assert_eq!(monitor.current_usage(), 0);
    }

    #[test]
    fn test_memory_monitor_available() {
        let monitor = MemoryMonitor::new(100, 80);

        assert_eq!(monitor.available(), 100 * 1024 * 1024);

        monitor.allocate(30 * 1024 * 1024).unwrap();
        assert_eq!(monitor.available(), 70 * 1024 * 1024);
    }
}
