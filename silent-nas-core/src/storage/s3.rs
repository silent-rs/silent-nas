//! S3 兼容存储 trait
//!
//! 提供 S3 风格的 bucket 操作接口

use async_trait::async_trait;

/// S3 兼容存储 trait
///
/// 提供 S3 风格的 bucket 操作接口，实现此 trait 可以支持 S3 API
#[async_trait]
pub trait S3CompatibleStorageTrait: Send + Sync {
    /// 错误类型
    type Error: std::error::Error + Send + Sync + 'static;

    /// 创建 bucket 目录
    ///
    /// # 参数
    /// * `bucket_name` - Bucket 名称
    async fn create_bucket(&self, bucket_name: &str) -> Result<(), Self::Error>;

    /// 删除 bucket 目录
    ///
    /// # 参数
    /// * `bucket_name` - Bucket 名称
    async fn delete_bucket(&self, bucket_name: &str) -> Result<(), Self::Error>;

    /// 检查 bucket 是否存在
    ///
    /// # 参数
    /// * `bucket_name` - Bucket 名称
    async fn bucket_exists(&self, bucket_name: &str) -> bool;

    /// 列出所有 buckets
    ///
    /// # 返回
    /// 返回 bucket 名称列表
    async fn list_buckets(&self) -> Result<Vec<String>, Self::Error>;

    /// 列出 bucket 中的所有对象
    ///
    /// # 参数
    /// * `bucket_name` - Bucket 名称
    /// * `prefix` - 对象键前缀过滤
    ///
    /// # 返回
    /// 返回对象键列表
    async fn list_bucket_objects(
        &self,
        bucket_name: &str,
        prefix: &str,
    ) -> Result<Vec<String>, Self::Error>;
}
