//! 核心存储引擎模块
//!
//! 该模块包含无状态的核心存储算法：
//! - 分块算法（固定大小、Rabin-Karp 滚动哈希）
//! - 压缩算法（LZ4、Zstd）
//! - 差异计算（块级增量）
//! - 存储引擎（组合上述功能）

pub mod chunker;
pub mod compression;
pub mod delta;
pub mod engine;

pub use chunker::*;
pub use compression::*;
pub use delta::*;
pub use engine::*;
