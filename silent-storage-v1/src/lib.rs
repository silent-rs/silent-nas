//! Silent Storage V1
//!
//! 简单的文件存储管理器，提供基础的文件存储功能。

mod error;
mod storage;

pub use error::{Result, StorageError};
pub use silent_nas_core::FileMetadata;
pub use storage::StorageManager;
