use thiserror::Error;

#[derive(Error, Debug)]
pub enum NasError {
    #[allow(dead_code)]
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

    #[error("认证错误: {0}")]
    Auth(String),

    #[allow(dead_code)]
    #[error("无效的文件路径: {0}")]
    InvalidPath(String),

    #[allow(dead_code)]
    #[error("哈希校验失败")]
    HashMismatch,

    #[error("{0}")]
    Other(String),
}

// 为 sled::Error 实现 From trait
impl From<sled::Error> for NasError {
    fn from(err: sled::Error) -> Self {
        NasError::Storage(format!("数据库错误: {}", err))
    }
}

// 为 silent_storage::StorageError 实现 From trait
impl From<silent_storage::StorageError> for NasError {
    fn from(err: silent_storage::StorageError) -> Self {
        NasError::Storage(format!("存储错误: {}", err))
    }
}

pub type Result<T> = std::result::Result<T, NasError>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_file_not_found_error() {
        let err = NasError::FileNotFound("test.txt".to_string());
        assert_eq!(err.to_string(), "文件未找到: test.txt");
    }

    #[test]
    fn test_file_already_exists_error() {
        let err = NasError::FileAlreadyExists("existing.txt".to_string());
        assert_eq!(err.to_string(), "文件已存在: existing.txt");
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let nas_err = NasError::from(io_err);
        assert!(nas_err.to_string().contains("IO 错误"));
    }

    #[test]
    fn test_serialization_error_conversion() {
        let json_err = serde_json::from_str::<i32>("invalid").unwrap_err();
        let nas_err = NasError::from(json_err);
        assert!(nas_err.to_string().contains("序列化错误"));
    }

    #[test]
    fn test_nats_error() {
        let err = NasError::Nats("连接失败".to_string());
        assert_eq!(err.to_string(), "NATS 错误: 连接失败");
    }

    #[test]
    fn test_config_error() {
        let err = NasError::Config("配置无效".to_string());
        assert_eq!(err.to_string(), "配置错误: 配置无效");
    }

    #[test]
    fn test_storage_error() {
        let err = NasError::Storage("磁盘已满".to_string());
        assert_eq!(err.to_string(), "存储错误: 磁盘已满");
    }

    #[test]
    fn test_transfer_error() {
        let err = NasError::Transfer("传输中断".to_string());
        assert_eq!(err.to_string(), "传输错误: 传输中断");
    }

    #[test]
    fn test_invalid_path_error() {
        let err = NasError::InvalidPath("/invalid/../path".to_string());
        assert_eq!(err.to_string(), "无效的文件路径: /invalid/../path");
    }

    #[test]
    fn test_hash_mismatch_error() {
        let err = NasError::HashMismatch;
        assert_eq!(err.to_string(), "哈希校验失败");
    }

    #[test]
    fn test_other_error() {
        let err = NasError::Other("其他错误".to_string());
        assert_eq!(err.to_string(), "其他错误");
    }

    #[test]
    fn test_result_ok() {
        let result: Result<i32> = Ok(42);
        assert!(result.is_ok());
        if let Ok(value) = result {
            assert_eq!(value, 42);
        }
    }

    #[test]
    fn test_result_err() {
        let result: Result<i32> = Err(NasError::Other("错误".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_error_debug() {
        let err = NasError::FileNotFound("debug.txt".to_string());
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("FileNotFound"));
    }

    #[test]
    fn test_error_chain() {
        // 测试错误可以作为其他错误的源
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "权限不足");
        let nas_err: NasError = io_err.into();
        assert!(nas_err.to_string().contains("权限不足"));
    }
}
