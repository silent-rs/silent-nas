use thiserror::Error;

/// Storage 错误类型
#[derive(Error, Debug)]
pub enum StorageError {
    #[error("文件未找到: {0}")]
    FileNotFound(String),

    #[error("存储错误: {0}")]
    Storage(String),

    #[error("元数据错误: {0}")]
    Metadata(String),

    #[error("Chunk错误: {0}")]
    Chunk(String),

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

    #[error("配置错误: {0}")]
    Config(String),

    #[error("数据库错误: {0}")]
    Database(String),

    #[error("IO错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("序列化错误: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Result 类型别名
pub type Result<T> = std::result::Result<T, StorageError>;
