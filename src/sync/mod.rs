// 同步功能模块
// 包含CRDT同步、增量同步、节点同步等功能

pub mod crdt;
pub mod incremental;
pub mod node;

// 重新导出常用类型，保持向后兼容性
// 这些在main.rs、webdav.rs等地方会被使用
#[allow(unused_imports)]
pub use crdt::{FileSync, SyncManager};
