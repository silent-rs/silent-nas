//! 存储管理器类型定义和重新导出
//!
//! 这个模块定义了整个项目使用的存储实现。
//! 支持运行时通过配置文件选择不同的存储引擎。
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

use crate::config::StorageConfig;
use crate::error::{NasError, Result};
use std::sync::Arc;

// 重新导出 StorageManager trait，让代码可以使用 trait 约束
#[allow(unused_imports)]
pub use silent_nas_core::S3CompatibleStorage as S3CompatibleStorageTrait;
#[allow(unused_imports)]
pub use silent_nas_core::StorageManager as StorageManagerTrait;

// 导出具体的存储实现
pub use silent_storage_v1::StorageManager as StorageV1;

// 导出错误类型
#[allow(unused_imports)]
pub use silent_storage_v1::StorageError;

/// 存储管理器（当前仅支持 V1）
///
/// 这是主项目使用的存储管理器类型。
/// 当前实现仅支持 V1，V2 支持将在未来版本中添加。
pub type StorageManager = StorageV1;

/// 根据配置创建存储管理器
///
/// # 参数
/// * `config` - 存储配置
///
/// # 返回
/// 返回配置的存储管理器实例
///
/// # 错误
/// 如果配置的存储版本不受支持，返回错误
pub fn create_storage(config: &StorageConfig) -> Result<Arc<StorageManager>> {
    match config.version.as_str() {
        "v1" => {
            let storage = StorageV1::new(config.root_path.clone(), config.chunk_size);
            Ok(Arc::new(storage))
        }
        "v2" => {
            // V2 当前不支持，因为它的 API 与 V1 不兼容
            Err(NasError::Config(
                "V2 存储引擎当前不支持。V2 的 API 需要 &mut self，与当前的 Arc<Storage> 模式不兼容。\
                 如需使用 V2，请参考 silent-storage-v2 crate 的文档单独集成。"
                    .to_string(),
            ))
        }
        version => Err(NasError::Config(format!(
            "不支持的存储版本: {}。当前支持: v1",
            version
        ))),
    }
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
