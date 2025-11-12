//! 全局存储管理器
//!
//! 提供全局单例的 StorageManager 访问，避免在各个模块中传递 Arc<StorageManager>

use super::StorageManager;
use crate::error::{NasError, Result};
use std::sync::OnceLock;

/// 全局存储管理器实例
static STORAGE: OnceLock<StorageManager> = OnceLock::new();

/// 初始化全局存储管理器
///
/// 该函数应该在程序启动时调用一次，通常在 main.rs 中
pub fn init_global_storage(storage: StorageManager) -> Result<()> {
    STORAGE
        .set(storage)
        .map_err(|_| NasError::Other("全局存储已经初始化".to_string()))
}

/// 获取全局存储管理器的引用
///
/// # Panics
/// 如果存储未初始化则会 panic
pub fn storage() -> &'static StorageManager {
    STORAGE
        .get()
        .expect("全局存储未初始化，请先调用 init_global_storage")
}

/// 尝试获取全局存储管理器的引用
///
/// 如果存储未初始化则返回 None
#[allow(dead_code)]
pub fn try_storage() -> Option<&'static StorageManager> {
    STORAGE.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_global_storage_initialization() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 64 * 1024);

        // 注意：这个测试只能运行一次，因为全局变量只能初始化一次
        // 在实际测试中，应该使用独立的测试进程或者避免依赖全局状态
        if try_storage().is_none() {
            assert!(init_global_storage(storage).is_ok());
            assert!(try_storage().is_some());
        }
    }
}
