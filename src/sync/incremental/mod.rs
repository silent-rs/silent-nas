// 增量同步模块
// 实现基于块的文件差异检测和同步

pub mod api;
pub mod core;
pub mod handler;

// 重新导出核心类型
pub use core::{DeltaChunk, FileSignature, IncrementalSyncManager, SyncDelta};
pub use handler::IncrementalSyncHandler;
