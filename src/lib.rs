// Silent-NAS 库接口
// 用于测试和外部集成

pub mod audit;
pub mod cache;
pub mod config;
pub mod error;
pub mod metrics;
pub mod models;
pub mod notify;
pub mod s3;
pub mod s3_search;
pub mod search;
pub mod storage;
pub mod storage_v2;
pub mod unified_search;
pub mod version;

// 注意：sync、transfer、webdav、event_listener 模块包含复杂的依赖，
// 暂不在lib中导出，避免编译问题
