use thiserror::Error;

/// Storage V1 错误类型
#[derive(Error, Debug)]
pub enum StorageError {
    #[error("文件未找到: {0}")]
    FileNotFound(String),

    #[error("存储错误: {0}")]
    Storage(String),

    #[error("IO错误: {0}")]
    Io(#[from] std::io::Error),
}

/// Result 类型别名
pub type Result<T> = std::result::Result<T, StorageError>;
