// 节点同步模块
// 实现跨节点的文件同步功能

pub mod client;
pub mod manager;
pub mod service;

// 重新导出核心类型
pub use manager::{NodeInfo, NodeManager, NodeSyncCoordinator};
