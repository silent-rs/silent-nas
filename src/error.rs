use thiserror::Error;

#[derive(Error, Debug)]
pub enum NasError {
    #[error("文件未找到: {0}")]
    FileNotFound(String),

    #[allow(dead_code)]
    #[error("文件已存在: {0}")]
    FileAlreadyExists(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("序列化错误: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("NATS 错误: {0}")]
    Nats(String),

    #[error("配置错误: {0}")]
    Config(String),

    #[error("存储错误: {0}")]
    Storage(String),

    #[error("传输错误: {0}")]
    Transfer(String),

    #[allow(dead_code)]
    #[error("无效的文件路径: {0}")]
    InvalidPath(String),

    #[allow(dead_code)]
    #[error("哈希校验失败")]
    HashMismatch,

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, NasError>;
