// Silent-NAS 库接口
// 用于测试和外部集成

pub mod audit;
pub mod cache;
pub mod config;
pub mod error;
pub mod metrics;
pub mod notify;
pub mod s3;
pub mod s3_search;
pub mod search;
pub mod storage; // 导出 storage 模块以支持 V2 测试
pub mod unified_search;

// Re-export core types and storage
pub use silent_nas_core as models;
pub use silent_storage;

// Re-export storage traits for easier access
pub use silent_nas_core::{S3CompatibleStorageTrait, StorageManagerTrait};

// 注意：sync、transfer、webdav、event_listener 模块包含复杂的依赖，
// 暂不在lib中导出，避免编译问题
