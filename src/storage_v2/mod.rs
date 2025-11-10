//! 增量存储与差异算法模块
//!
//! 该模块提供基于块差异的文件版本存储功能，包括：
//! - 滚动哈希算法（Rabin-Karp）
//! - 内容定义分块（Content-Defined Chunking）
//! - 版本链式存储
//! - 增量更新与读取
//! - 自动压缩与冷热分离
//! - 数据生命周期管理
//! - 跨文件块级去重

pub mod chunker;
pub mod compatibility;
pub mod compression;
pub mod dedup;
pub mod delta;
pub mod engine;
pub mod index;
pub mod lifecycle;
pub mod storage;
pub mod tiering;

#[allow(ambiguous_glob_reexports)]
pub use chunker::*;
#[allow(ambiguous_glob_reexports)]
pub use compatibility::*;
#[allow(ambiguous_glob_reexports)]
pub use compression::*;
#[allow(ambiguous_glob_reexports)]
pub use dedup::*;
#[allow(ambiguous_glob_reexports)]
pub use delta::*;
#[allow(ambiguous_glob_reexports)]
pub use engine::*;
#[allow(ambiguous_glob_reexports)]
pub use index::*;
#[allow(ambiguous_glob_reexports)]
pub use lifecycle::*;
#[allow(ambiguous_glob_reexports)]
pub use storage::*;
#[allow(ambiguous_glob_reexports)]
pub use tiering::*;

use serde::{Deserialize, Serialize};

/// 增量存储配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncrementalConfig {
    /// 分块算法类型
    pub chunker_type: ChunkerType,
    /// 平均分块大小（字节）
    pub avg_chunk_size: usize,
    /// 最小分块大小（字节）
    pub min_chunk_size: usize,
    /// 最大分块大小（字节）
    pub max_chunk_size: usize,
    /// 滚动哈希多项式（Rabin-Karp）
    pub rabin_poly: u64,
    /// 弱哈希模数
    pub weak_hash_mod: usize,
    /// 启用压缩
    pub enable_compression: bool,
    /// 压缩算法 (lz4, zstd)
    pub compression_algorithm: String,
    /// 启用去重
    pub enable_deduplication: bool,
}

impl Default for IncrementalConfig {
    fn default() -> Self {
        Self {
            chunker_type: ChunkerType::RabinKarp,
            avg_chunk_size: 8 * 1024,  // 8KB
            min_chunk_size: 4 * 1024,  // 4KB
            max_chunk_size: 16 * 1024, // 16KB
            rabin_poly: 0x3b9aca07,    // 常用质数
            weak_hash_mod: 2048,       // 2^11
            enable_compression: true,
            compression_algorithm: "lz4".to_string(),
            enable_deduplication: true,
        }
    }
}

/// 分块算法类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChunkerType {
    /// 固定大小分块
    FixedSize,
    /// Rabin-Karp 滚动哈希分块
    RabinKarp,
    /// 快速分块（简单哈希）
    Fast,
}

/// 块信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkInfo {
    /// 块ID（哈希值）
    pub chunk_id: String,
    /// 块在文件中的偏移量
    pub offset: usize,
    /// 块大小
    pub size: usize,
    /// 弱哈希值
    pub weak_hash: u32,
    /// 强哈希值（SHA-256）
    pub strong_hash: String,
}

/// 文件差异信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDelta {
    /// 文件ID
    pub file_id: String,
    /// 基础版本ID（空字符串表示从空文件开始）
    pub base_version_id: String,
    /// 新版本ID
    pub new_version_id: String,
    /// 使用的块列表
    pub chunks: Vec<ChunkInfo>,
    /// 创建时间
    pub created_at: chrono::NaiveDateTime,
}

/// 版本存储信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    /// 版本ID
    pub version_id: String,
    /// 文件ID
    pub file_id: String,
    /// 父版本ID（链式存储）
    pub parent_version_id: Option<String>,
    /// 文件大小
    pub file_size: u64,
    /// 块数量
    pub chunk_count: usize,
    /// 实际存储大小（压缩/去重后）
    pub storage_size: u64,
    /// 创建时间
    pub created_at: chrono::NaiveDateTime,
    /// 是否为当前版本
    pub is_current: bool,
}
