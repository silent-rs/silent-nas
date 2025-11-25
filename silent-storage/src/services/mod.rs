//! 有状态服务层模块
//!
//! 该模块包含需要维护状态的服务：
//! - 分层存储（热数据、冷数据）
//! - 生命周期管理（数据清理、过期处理）

pub mod lifecycle;
pub mod tiering;

pub use lifecycle::*;
pub use tiering::*;
