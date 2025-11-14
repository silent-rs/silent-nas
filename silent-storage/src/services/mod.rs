//! 有状态服务层模块
//!
//! 该模块包含需要维护状态的服务：
//! - 去重服务（块级去重、引用计数）
//! - 索引服务（块索引、文件索引）
//! - 分层存储（热数据、冷数据）
//! - 生命周期管理（数据清理、过期处理）

pub mod dedup;
pub mod index;
pub mod lifecycle;
pub mod tiering;

pub use dedup::*;
pub use index::*;
pub use lifecycle::*;
pub use tiering::*;
