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
///
/// 注意：在测试环境中，如果全局存储已经初始化，此函数会忽略错误并继续使用现有存储。
/// 这是为了支持多个测试共享全局存储。
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

/// 测试辅助函数：异步初始化共享的测试存储
///
/// 在测试开始时调用一次即可初始化共享的测试存储。
/// 所有测试将共享同一个全局存储实例，避免重复初始化错误。
///
/// 使用 tokio::sync::OnceCell 确保并发安全的单次初始化。
#[cfg(test)]
pub async fn init_test_storage_async() -> &'static StorageManager {
    use std::sync::OnceLock;
    use tempfile::TempDir;
    use tokio::sync::OnceCell;

    // 使用 OnceCell 确保异步初始化只执行一次（并发安全）
    static INIT_CELL: OnceCell<()> = OnceCell::const_new();

    INIT_CELL
        .get_or_init(|| async {
            // 创建持久的临时目录（使用 OnceLock 确保只创建一次）
            static TEST_DIR: OnceLock<&'static TempDir> = OnceLock::new();
            let temp_dir = TEST_DIR.get_or_init(|| Box::leak(Box::new(TempDir::new().unwrap())));

            // 创建并初始化存储
            let mgr = StorageManager::new(
                temp_dir.path().to_path_buf(),
                64 * 1024,
                crate::storage::IncrementalConfig::default(),
            );

            // 初始化存储（这是唯一会初始化 Sled 数据库的地方）
            mgr.init().await.unwrap();

            // 初始化全局存储（忽略错误，因为可能已经初始化）
            init_global_storage(mgr).ok();
        })
        .await;

    // 返回已初始化的全局存储
    storage()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageManagerTrait;

    /// 此测试验证全局存储初始化API的正确性
    /// 注意：此测试使用 init_test_storage_async() 来避免与其他测试的竞争条件
    #[tokio::test]
    async fn test_global_storage_initialization() {
        // 使用共享的测试存储初始化（这会正确初始化存储并保持临时目录）
        let storage = init_test_storage_async().await;

        // 验证全局存储已正确初始化
        assert!(try_storage().is_some());

        // 验证存储可以正常工作
        let root = storage.root_dir();
        assert!(root.exists());
    }
}
