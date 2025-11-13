use thiserror::Error;

/// Storage V2 错误类型
#[derive(Error, Debug)]
pub enum StorageError {
    #[error("文件未找到: {0}")]
    FileNotFound(String),

    #[error("存储错误: {0}")]
    Storage(String),

    #[error("去重错误: {0}")]
    Dedup(String),

    #[error("压缩错误: {0}")]
    Compression(String),

    #[error("索引错误: {0}")]
    Index(String),

    #[error("分层存储错误: {0}")]
    Tiering(String),

    #[error("生命周期管理错误: {0}")]
    Lifecycle(String),

    #[error("Delta生成错误: {0}")]
    Delta(String),

    #[error("IO错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("序列化错误: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Result 类型别名
pub type Result<T> = std::result::Result<T, StorageError>;

/// 转换为 V1 错误类型（用于适配器）
impl From<StorageError> for silent_storage_v1::StorageError {
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::FileNotFound(msg) => silent_storage_v1::StorageError::FileNotFound(msg),
            StorageError::Storage(msg) => silent_storage_v1::StorageError::Storage(msg),
            StorageError::Dedup(msg) => {
                silent_storage_v1::StorageError::Storage(format!("去重错误: {}", msg))
            }
            StorageError::Compression(msg) => {
                silent_storage_v1::StorageError::Storage(format!("压缩错误: {}", msg))
            }
            StorageError::Index(msg) => {
                silent_storage_v1::StorageError::Storage(format!("索引错误: {}", msg))
            }
            StorageError::Tiering(msg) => {
                silent_storage_v1::StorageError::Storage(format!("分层存储错误: {}", msg))
            }
            StorageError::Lifecycle(msg) => {
                silent_storage_v1::StorageError::Storage(format!("生命周期管理错误: {}", msg))
            }
            StorageError::Delta(msg) => {
                silent_storage_v1::StorageError::Storage(format!("Delta生成错误: {}", msg))
            }
            StorageError::Io(e) => silent_storage_v1::StorageError::Io(e),
            StorageError::Serialization(e) => {
                silent_storage_v1::StorageError::Storage(format!("序列化错误: {}", e))
            }
        }
    }
}
