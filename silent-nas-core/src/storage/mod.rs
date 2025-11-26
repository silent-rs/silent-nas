//! 存储管理器 trait 定义
//!
//! 提供统一的存储接口，支持不同的存储实现

mod s3;

pub use s3::S3CompatibleStorageTrait;

use crate::FileMetadata;
use async_trait::async_trait;
use std::path::Path;

/// 存储管理器 trait
///
/// 定义了文件存储的基本操作接口，所有存储实现都应该实现此 trait
#[async_trait]
pub trait StorageManagerTrait: Send + Sync {
    /// 错误类型
    type Error: std::error::Error + Send + Sync + 'static;

    /// 初始化存储目录
    async fn init(&self) -> Result<(), Self::Error>;

    /// 保存文件
    ///
    /// # 参数
    /// * `file_id` - 文件ID
    /// * `data` - 文件数据
    ///
    /// # 返回
    /// 返回文件元数据
    async fn save_file(&self, file_id: &str, data: &[u8]) -> Result<FileMetadata, Self::Error>;

    /// 按相对路径保存文件（用于 WebDAV/S3 路径语义）
    ///
    /// # 参数
    /// * `relative_path` - 相对路径
    /// * `data` - 文件数据
    ///
    /// # 返回
    /// 返回文件元数据
    async fn save_at_path(
        &self,
        relative_path: &str,
        data: &[u8],
    ) -> Result<FileMetadata, Self::Error>;

    /// 读取文件
    ///
    /// # 参数
    /// * `file_id` - 文件ID
    ///
    /// # 返回
    /// 返回文件数据
    async fn read_file(&self, file_id: &str) -> Result<Vec<u8>, Self::Error>;

    /// 删除文件
    ///
    /// # 参数
    /// * `file_id` - 文件ID
    async fn delete_file(&self, file_id: &str) -> Result<(), Self::Error>;

    /// 检查文件是否存在
    ///
    /// # 参数
    /// * `file_id` - 文件ID
    async fn file_exists(&self, file_id: &str) -> bool;

    /// 获取文件元数据
    ///
    /// # 参数
    /// * `file_id` - 文件ID
    ///
    /// # 返回
    /// 返回文件元数据
    async fn get_metadata(&self, file_id: &str) -> Result<FileMetadata, Self::Error>;

    /// 列出所有文件
    ///
    /// # 返回
    /// 返回文件元数据列表
    async fn list_files(&self) -> Result<Vec<FileMetadata>, Self::Error>;

    /// 验证文件哈希
    ///
    /// # 参数
    /// * `file_id` - 文件ID
    /// * `expected_hash` - 期望的哈希值
    ///
    /// # 返回
    /// 如果哈希匹配返回 true，否则返回 false
    async fn verify_hash(&self, file_id: &str, expected_hash: &str) -> Result<bool, Self::Error>;

    /// 获取根目录路径
    fn root_dir(&self) -> &Path;

    /// 获取文件的完整路径（基于相对路径，用于 WebDAV）
    fn get_full_path(&self, relative_path: &str) -> std::path::PathBuf;
}
