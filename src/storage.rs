//! 存储管理器类型定义和重新导出
//!
//! 这个模块定义了整个项目使用的存储实现。
//! 支持运行时通过配置文件选择不同的存储引擎。
//!
//! ## 全局存储
//!
//! 项目使用全局单例模式管理存储实例，避免在各个模块中传递 `Arc<StorageManager>`。
//! 使用 `init_global_storage()` 初始化，使用 `storage()` 访问。
//!
//! ## 配置说明
//!
//! 在 `config.toml` 中配置存储引擎版本：
//!
//! ```toml
//! [storage]
//! root_path = "./storage"
//! chunk_size = 4194304  # 4MB
//! version = "v1"  # 可选值: "v1" 或 "v2"
//! ```
//!
//! ### V1 存储引擎（默认）
//! - **特点**：简单可靠的文件存储
//! - **适用场景**：
//!   - 小规模部署（< 1TB）
//!   - 对性能要求不高的场景
//!   - 需要简单维护的环境
//! - **优势**：
//!   - 实现简单，易于理解和维护
//!   - 文件直接存储，方便备份和恢复
//!   - 无额外依赖
//! - **劣势**：
//!   - 无去重功能，存储空间利用率较低
//!   - 无增量同步支持
//!   - 无数据压缩
//!
//! ### V2 存储引擎（实验性）
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
//! - **劣势**：
//!   - 实现复杂，维护成本较高
//!   - 需要额外的索引存储
//!   - 恢复过程相对复杂
//! - **注意**：V2 当前处于实验阶段，建议在生产环境使用 V1
//!
//! ## 切换存储引擎
//!
//! 修改配置文件中的 `storage.version` 字段即可切换：
//!
//! ```toml
//! [storage]
//! version = "v2"  # 切换到 V2
//! ```
//!
//! **警告**：切换存储引擎需要数据迁移，请提前备份数据！

mod global;

pub use global::{init_global_storage, storage};

use crate::config::StorageConfig;
use crate::error::{NasError, Result};
use std::sync::Arc;

// 重新导出 StorageManager trait，让代码可以使用 trait 约束
pub use silent_nas_core::S3CompatibleStorage as S3CompatibleStorageTrait;
pub use silent_nas_core::StorageManager as StorageManagerTrait;

// 导出具体的存储实现
pub use silent_storage_v1::StorageManager as StorageV1;
// V2 适配器已完成，生产环境测试中
pub use silent_storage_v2::StorageV2Adapter;

// 导出错误类型
pub use silent_storage_v1::StorageError;

use async_trait::async_trait;
use silent_nas_core::FileMetadata;

/// 统一存储后端枚举
///
/// 支持运行时在 V1 和 V2 之间切换
#[derive(Clone)]
pub enum StorageBackend {
    /// V1 简单文件存储
    V1(StorageV1),
    /// V2 增量存储
    V2(StorageV2Adapter),
}

impl StorageBackend {
    /// 创建 V1 存储实例（用于测试）
    #[allow(dead_code)]
    pub fn new(root_path: std::path::PathBuf, chunk_size: usize) -> Self {
        StorageBackend::V1(StorageV1::new(root_path, chunk_size))
    }
}

// 为 StorageBackend 实现 StorageManagerTrait
#[async_trait]
impl StorageManagerTrait for StorageBackend {
    type Error = StorageError;

    async fn init(&self) -> std::result::Result<(), Self::Error> {
        match self {
            StorageBackend::V1(storage) => storage.init().await,
            StorageBackend::V2(storage) => storage.init().await,
        }
    }

    async fn save_file(
        &self,
        id: &str,
        data: &[u8],
    ) -> std::result::Result<FileMetadata, Self::Error> {
        match self {
            StorageBackend::V1(storage) => storage.save_file(id, data).await,
            StorageBackend::V2(storage) => storage.save_file(id, data).await,
        }
    }

    async fn save_at_path(
        &self,
        relative_path: &str,
        data: &[u8],
    ) -> std::result::Result<FileMetadata, Self::Error> {
        match self {
            StorageBackend::V1(storage) => storage.save_at_path(relative_path, data).await,
            StorageBackend::V2(storage) => storage.save_at_path(relative_path, data).await,
        }
    }

    async fn read_file(&self, id: &str) -> std::result::Result<Vec<u8>, Self::Error> {
        match self {
            StorageBackend::V1(storage) => storage.read_file(id).await,
            StorageBackend::V2(storage) => storage.read_file(id).await,
        }
    }

    async fn delete_file(&self, id: &str) -> std::result::Result<(), Self::Error> {
        match self {
            StorageBackend::V1(storage) => storage.delete_file(id).await,
            StorageBackend::V2(storage) => storage.delete_file(id).await,
        }
    }

    async fn file_exists(&self, id: &str) -> bool {
        match self {
            StorageBackend::V1(storage) => storage.file_exists(id).await,
            StorageBackend::V2(storage) => storage.file_exists(id).await,
        }
    }

    async fn get_metadata(&self, id: &str) -> std::result::Result<FileMetadata, Self::Error> {
        match self {
            StorageBackend::V1(storage) => storage.get_metadata(id).await,
            StorageBackend::V2(storage) => storage.get_metadata(id).await,
        }
    }

    async fn list_files(&self) -> std::result::Result<Vec<FileMetadata>, Self::Error> {
        match self {
            StorageBackend::V1(storage) => storage.list_files().await,
            StorageBackend::V2(storage) => storage.list_files().await,
        }
    }

    async fn verify_hash(
        &self,
        file_id: &str,
        expected_hash: &str,
    ) -> std::result::Result<bool, Self::Error> {
        match self {
            StorageBackend::V1(storage) => storage.verify_hash(file_id, expected_hash).await,
            StorageBackend::V2(storage) => storage.verify_hash(file_id, expected_hash).await,
        }
    }

    fn root_dir(&self) -> &std::path::Path {
        match self {
            StorageBackend::V1(storage) => storage.root_dir(),
            StorageBackend::V2(storage) => storage.root_dir(),
        }
    }

    fn get_full_path(&self, relative_path: &str) -> std::path::PathBuf {
        match self {
            StorageBackend::V1(storage) => storage.get_full_path(relative_path),
            StorageBackend::V2(storage) => storage.get_full_path(relative_path),
        }
    }
}

// 为 StorageBackend 实现 S3CompatibleStorageTrait
#[async_trait]
impl S3CompatibleStorageTrait for StorageBackend {
    type Error = StorageError;

    async fn create_bucket(&self, bucket_name: &str) -> std::result::Result<(), Self::Error> {
        match self {
            StorageBackend::V1(storage) => storage.create_bucket(bucket_name).await,
            StorageBackend::V2(storage) => storage.create_bucket(bucket_name).await,
        }
    }

    async fn delete_bucket(&self, bucket_name: &str) -> std::result::Result<(), Self::Error> {
        match self {
            StorageBackend::V1(storage) => storage.delete_bucket(bucket_name).await,
            StorageBackend::V2(storage) => storage.delete_bucket(bucket_name).await,
        }
    }

    async fn bucket_exists(&self, bucket_name: &str) -> bool {
        match self {
            StorageBackend::V1(storage) => storage.bucket_exists(bucket_name).await,
            StorageBackend::V2(storage) => storage.bucket_exists(bucket_name).await,
        }
    }

    async fn list_buckets(&self) -> std::result::Result<Vec<String>, Self::Error> {
        match self {
            StorageBackend::V1(storage) => storage.list_buckets().await,
            StorageBackend::V2(storage) => storage.list_buckets().await,
        }
    }

    async fn list_bucket_objects(
        &self,
        bucket_name: &str,
        prefix: &str,
    ) -> std::result::Result<Vec<String>, Self::Error> {
        match self {
            StorageBackend::V1(storage) => storage.list_bucket_objects(bucket_name, prefix).await,
            StorageBackend::V2(storage) => storage.list_bucket_objects(bucket_name, prefix).await,
        }
    }
}

/// 存储管理器（支持 V1 和 V2）
///
/// 这是主项目使用的存储管理器类型。
/// - V1: 简单文件存储，生产就绪（默认）
/// - V2: 高级增量存储，支持去重和增量同步
pub type StorageManager = StorageBackend;

/// 根据配置创建存储管理器
///
/// # 参数
/// * `config` - 存储配置
///
/// # 返回
/// 返回配置的存储管理器实例（支持 V1 和 V2）
///
/// # 错误
/// 如果配置的存储版本不受支持或初始化失败，返回错误
pub async fn create_storage(config: &StorageConfig) -> Result<Arc<StorageManager>> {
    match config.version.as_str() {
        "v1" => {
            tracing::info!("🔧 初始化 V1 存储引擎");
            let storage = StorageV1::new(config.root_path.clone(), config.chunk_size);
            storage
                .init()
                .await
                .map_err(|e| NasError::Config(format!("V1 存储初始化失败: {}", e)))?;
            tracing::info!("✅ V1 存储引擎初始化完成");
            Ok(Arc::new(StorageBackend::V1(storage)))
        }
        "v2" => {
            use silent_storage_v2::{IncrementalConfig, IncrementalStorage};

            tracing::info!("🔧 初始化 V2 增量存储引擎");

            // 创建 V1 作为底层存储
            let v1_storage = Arc::new(StorageV1::new(config.root_path.clone(), config.chunk_size));

            // 初始化 V1
            v1_storage
                .init()
                .await
                .map_err(|e| NasError::Config(format!("V1 底层存储初始化失败: {}", e)))?;

            // 创建 V2 配置
            let v2_config = IncrementalConfig::default();

            // 创建 V2 增量存储（包装 V1）
            let v2_root = config.root_path.join("v2").to_string_lossy().to_string();
            let v2_storage = Arc::new(IncrementalStorage::new(v1_storage, v2_config, &v2_root));

            // 初始化 V2
            v2_storage
                .init()
                .await
                .map_err(|e| NasError::Config(format!("V2 存储初始化失败: {}", e)))?;

            // 创建适配器
            let adapter = StorageV2Adapter::new(v2_storage);

            tracing::info!("✅ V2 增量存储引擎初始化完成");
            tracing::info!("💡 V2 特性：文件去重、增量同步、版本管理");
            Ok(Arc::new(StorageBackend::V2(adapter)))
        }
        version => Err(NasError::Config(format!(
            "不支持的存储版本: {}。当前支持: v1, v2",
            version
        ))),
    }
}

/// 创建 V2 存储引擎用于测试
///
/// # 参数
/// * `config` - 存储配置
///
/// # 返回
/// 返回 V2 存储适配器实例
///
/// # 错误
/// 如果初始化失败，返回错误
#[allow(dead_code)]
pub async fn create_storage_v2(config: &StorageConfig) -> Result<Arc<StorageV2Adapter>> {
    use silent_storage_v2::{IncrementalConfig, IncrementalStorage};

    tracing::info!("初始化 V2 存储引擎（测试模式）");

    // 创建 V1 作为底层存储
    let v1_storage = Arc::new(StorageV1::new(config.root_path.clone(), config.chunk_size));

    // 初始化 V1
    v1_storage
        .init()
        .await
        .map_err(|e| NasError::Config(format!("V1 底层存储初始化失败: {}", e)))?;

    // 创建 V2 配置
    let v2_config = IncrementalConfig::default();

    // 创建 V2 增量存储（包装 V1）
    let v2_root = config.root_path.join("v2").to_string_lossy().to_string();
    let v2_storage = Arc::new(IncrementalStorage::new(v1_storage, v2_config, &v2_root));

    // 初始化 V2
    v2_storage
        .init()
        .await
        .map_err(|e| NasError::Config(format!("V2 存储初始化失败: {}", e)))?;

    // 创建适配器
    let adapter = StorageV2Adapter::new(v2_storage);

    tracing::info!("✅ V2 存储引擎初始化完成");
    Ok(Arc::new(adapter))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_storage_implements_trait() {
        let temp_dir = TempDir::new().unwrap();
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 1024 * 1024);

        // 验证实现了 StorageManagerTrait
        let _trait_obj: &dyn StorageManagerTrait<Error = StorageError> = &storage;

        // 验证实现了 S3CompatibleStorageTrait
        let _s3_trait_obj: &dyn S3CompatibleStorageTrait<Error = StorageError> = &storage;
    }

    #[tokio::test]
    async fn test_storage_basic_operations() {
        let temp_dir = TempDir::new().unwrap();
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 1024 * 1024);

        // 初始化
        storage.init().await.unwrap();

        // 保存文件
        let data = b"test data";
        let metadata = storage.save_file("test_id", data).await.unwrap();
        assert_eq!(metadata.id, "test_id");
        assert_eq!(metadata.size, data.len() as u64);

        // 读取文件
        let read_data = storage.read_file("test_id").await.unwrap();
        assert_eq!(read_data, data);

        // 验证文件存在
        assert!(storage.file_exists("test_id").await);

        // 删除文件
        storage.delete_file("test_id").await.unwrap();
        assert!(!storage.file_exists("test_id").await);
    }
}
