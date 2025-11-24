//! 存储管理器类型定义和重新导出
//!
//! 这个模块定义了整个项目使用的存储实现。
//!
//! ## 全局存储
//!
//! 项目使用全局单例模式管理存储实例，避免在各个模块中传递 `Arc<StorageManager>`。
//! 使用 `init_global_storage()` 初始化，使用 `storage()` 访问。
//!
//! ## 配置说明
//!
//! 在 `config.toml` 中配置存储引擎：
//!
//! ```toml
//! [storage]
//! root_path = "./storage"
//! chunk_size = 4194304  # 4MB
//! ```
//!
//! ## 存储引擎特性
//! - **特点**：高级增量存储，支持去重和压缩
//! - **适用场景**：
//!   - 大规模部署（> 1TB）
//!   - 需要高存储效率的场景
//!   - 多节点同步场景
//! - **优势**：
//!   - 文件级去重，节省存储空间
//!   - 增量存储和同步，减少网络传输
//!   - 支持数据压缩
//!   - 版本管理更高效
//!   - WAL 日志保障数据可靠性
//!   - 三级缓存提升性能

mod global;

#[cfg(test)]
pub use global::init_test_storage_async;
pub use global::{init_global_storage, storage};

use crate::config::StorageConfig;
use crate::error::{NasError, Result};

// 重新导出 StorageManager trait
pub use silent_nas_core::StorageManagerTrait;

// 导出存储实现
pub use silent_storage::IncrementalConfig;
pub use silent_storage::StorageManager;

/// 从配置创建存储管理器
///
/// # 示例
/// ```no_run
/// use silent_nas::config::StorageConfig;
/// use silent_nas::storage::create_storage;
/// use std::path::PathBuf;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = StorageConfig {
///     root_path: PathBuf::from("./storage"),
///     chunk_size: 4 * 1024 * 1024,
///     enable_compression: true,
///     compression_algorithm: "lz4".to_string(),
///     enable_auto_gc: true,
///     gc_interval_secs: 3600,
/// };
///
/// let storage = create_storage(&config).await?;
/// # Ok(())
/// # }
/// ```
pub async fn create_storage(config: &StorageConfig) -> Result<StorageManager> {
    // 创建增量配置（去重功能已内置于存储策略，无需配置）
    let incremental_config = IncrementalConfig {
        enable_compression: config.enable_compression,
        compression_algorithm: config.compression_algorithm.clone(),
        enable_auto_gc: config.enable_auto_gc,
        gc_interval_secs: config.gc_interval_secs,
        ..IncrementalConfig::default()
    };

    // 创建存储管理器
    let storage = StorageManager::new(
        config.root_path.clone(),
        config.chunk_size,
        incremental_config,
    );

    // 初始化存储
    storage
        .init()
        .await
        .map_err(|e| NasError::Storage(e.to_string()))?;

    tracing::info!(
        "存储管理器初始化成功: root={:?}, chunk_size={}, compression={}, auto_gc={}, gc_interval={}s",
        config.root_path,
        config.chunk_size,
        config.enable_compression,
        config.enable_auto_gc,
        config.gc_interval_secs
    );

    Ok(storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_storage() {
        let temp_dir = TempDir::new().unwrap();
        let config = StorageConfig {
            root_path: temp_dir.path().to_path_buf(),
            chunk_size: 64 * 1024,
            enable_compression: false, // 禁用压缩以加快测试速度
            compression_algorithm: "lz4".to_string(),
            enable_auto_gc: false, // 禁用自动GC以加快测试速度
            gc_interval_secs: 3600,
        };

        let storage = create_storage(&config).await.unwrap();

        // 测试基本操作
        let test_data = b"test data";
        let metadata = storage.save_file("test_id", test_data).await.unwrap();
        assert_eq!(metadata.id, "test_id");
        assert_eq!(metadata.size, test_data.len() as u64);

        let read_data = storage.read_file("test_id").await.unwrap();
        assert_eq!(read_data, test_data);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_global_storage() {
        use silent_nas_core::StorageManagerTrait;
        use tempfile::TempDir;

        // 创建独立的测试存储，避免全局状态竞态条件
        let temp_dir = TempDir::new().unwrap();
        let config = IncrementalConfig {
            enable_compression: false,
            ..IncrementalConfig::default()
        };

        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 64 * 1024, config);
        storage.init().await.unwrap();

        // 测试基本操作
        let test_data = b"global storage test";
        let metadata = storage
            .save_file("global_test_id", test_data)
            .await
            .unwrap();
        assert_eq!(metadata.id, "global_test_id");

        let read_data = storage.read_file("global_test_id").await.unwrap();
        assert_eq!(read_data, test_data);
    }
}
