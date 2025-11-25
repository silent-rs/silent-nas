//! # Silent Storage
//!
//! 高性能、可靠的增量存储系统，基于内容定义分块（CDC）和块级去重技术。
//!
//! ## 特性
//!
//! - **增量存储**: 基于 CDC 的增量存储，只保存变化的数据块
//! - **高效去重**: 跨文件块级去重，显著节省存储空间
//! - **智能压缩**: 自适应压缩策略（LZ4/Zstd），已压缩文件自动跳过
//! - **版本管理**: 完整的版本链管理，支持版本回溯
//! - **可靠性**: WAL 日志、数据校验、孤儿清理
//! - **性能**: 三级缓存、自适应分块、高吞吐量（CDC 102+ MiB/s）
//! - **监控**: Prometheus 指标导出，完整的性能监控
//!
//! ## 快速开始
//!
//! ```rust,no_run
//! use silent_storage::{StorageManager, IncrementalConfig};
//! use std::path::PathBuf;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // 创建存储管理器
//!     let config = IncrementalConfig::default();
//!     let storage = StorageManager::new(
//!         PathBuf::from("./storage"),
//!         64 * 1024,
//!         config,
//!     );
//!
//!     // 初始化
//!     storage.init().await?;
//!
//!     // 保存文件版本
//!     let data = b"Hello, World!";
//!     let (delta, version) = storage.save_version("file", data, None).await?;
//!
//!     // 读取数据
//!     let content = storage.read_version_data(&version.version_id).await?;
//!
//!     // 优雅关闭
//!     storage.shutdown().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## 架构
//!
//! ```text
//! silent-storage/
//! ├── core/           # 核心存储引擎
//! │   ├── chunker     # 内容定义分块（CDC）
//! │   ├── compression # 压缩算法（LZ4/Zstd）
//! │   ├── delta       # 增量计算
//! │   ├── engine      # 存储引擎
//! │   ├── file_type   # 文件类型检测
//! │   └── version_chain # 版本链管理
//! ├── services/       # 有状态服务
//! │   ├── dedup       # 去重服务
//! │   ├── index       # 索引服务
//! │   ├── lifecycle   # 生命周期管理
//! │   └── tiering     # 分层存储
//! ├── cache.rs        # 三级缓存系统
//! ├── metadata.rs     # 元数据管理（Sled）
//! ├── metrics.rs      # Prometheus 指标
//! ├── reliability.rs  # 可靠性保障
//! └── storage.rs      # 顶层 API
//! ```
//!
//! ## 主要组件
//!
//! - [`StorageManager`] - 顶层存储管理器
//! - [`CacheManager`] - 三级缓存管理
//! - [`WalManager`] - WAL 日志管理
//! - [`ChunkVerifier`] - Chunk 数据校验
//! - [`StorageMetrics`] - Prometheus 指标

mod error;

// ============================================================================
// 公共模块
// ============================================================================

pub mod bench;
pub mod bloom;
pub mod cache;
pub mod core;
pub mod metadata;
pub mod metrics;
pub mod optimization;
pub mod reliability;
pub mod services;
pub mod storage;

// ============================================================================
// 核心 API（最常用）
// ============================================================================

/// 存储管理器 - 主要入口点
pub use storage::StorageManager;

/// 错误处理
pub use error::{Result, StorageError};

// ============================================================================
// 存储类型和统计
// ============================================================================

pub use storage::{ChunkRefCount, FileIndexEntry, GarbageCollectResult, StorageStats};

// ============================================================================
// 缓存系统
// ============================================================================

pub use cache::{CacheConfig, CacheManager, CacheStats};

// ============================================================================
// 监控和指标
// ============================================================================

pub use metrics::{HealthStatus, StorageMetrics};

// ============================================================================
// 后台优化
// ============================================================================

pub use optimization::{
    OptimizationScheduler, OptimizationStats, OptimizationStrategy, OptimizationTask,
};

// ============================================================================
// 可靠性组件
// ============================================================================

pub use reliability::{
    ChunkVerifier, ChunkVerifyReport, CleanupReport, OrphanChunkCleaner, WalEntry, WalManager,
    WalOperation,
};

// ============================================================================
// 核心算法（CDC、压缩、增量）
// ============================================================================

pub use core::chunker::*;
pub use core::compression::*;
pub use core::delta::*;

// ============================================================================
// 服务模块（生命周期、分层）
// ============================================================================

pub use services::lifecycle::*;
pub use services::tiering::*;

// ============================================================================
// Prelude - 便捷导入
// ============================================================================

/// 预加载模块，包含最常用的类型
///
/// 使用方式:
/// ```rust
/// use silent_storage::prelude::*;
/// ```
pub mod prelude {
    pub use crate::error::{Result, StorageError};
    pub use crate::storage::{FileIndexEntry, StorageManager, StorageStats};
    pub use crate::{
        ChunkInfo, ChunkerType, DeduplicationStats, FileDelta, IncrementalConfig,
        OptimizationStatus, StorageMode, VersionInfo,
    };
}

use serde::{Deserialize, Serialize};

/// 增量存储配置
///
/// 注意：分块大小（chunk_size）在 StorageManager::new() 中单独传入，
/// 最小/最大分块大小根据 chunk_size 自动计算：
/// - min_chunk_size = chunk_size / 2
/// - max_chunk_size = chunk_size * 2
///
/// 去重功能已内置于存储策略中，无需单独配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncrementalConfig {
    /// 分块算法类型
    pub chunker_type: ChunkerType,
    /// 滚动哈希多项式（Rabin-Karp）
    pub rabin_poly: u64,
    /// 弱哈希模数
    pub weak_hash_mod: usize,
    /// 启用压缩
    pub enable_compression: bool,
    /// 压缩算法 (lz4, zstd)
    pub compression_algorithm: String,
    /// 启用自动GC
    pub enable_auto_gc: bool,
    /// GC触发间隔（秒）
    pub gc_interval_secs: u64,
}

impl Default for IncrementalConfig {
    fn default() -> Self {
        Self {
            chunker_type: ChunkerType::RabinKarp,
            rabin_poly: 0x3b9aca07,    // 常用质数
            weak_hash_mod: 2048,       // 2^11
            enable_compression: true,
            compression_algorithm: "lz4".to_string(),
            enable_auto_gc: true,
            gc_interval_secs: 3600, // 默认每小时执行一次GC
        }
    }
}

/// 分块算法类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChunkerType {
    /// 固定大小分块
    Fixed,
    /// Rabin-Karp滚动哈希
    RabinKarp,
}

/// 存储模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum StorageMode {
    /// 分块存储 - 分块+去重+压缩（默认模式）
    #[default]
    Chunked,
    /// 压缩存储 - 仅压缩，不分块（小文件优化）
    Compressed,
    /// 旧版热存储兼容（已弃用，仅用于读取旧数据）
    #[serde(alias = "Hot")]
    #[deprecated(note = "热存储已移除，使用 Chunked 模式")]
    Hot,
    /// 旧版冷存储兼容（已弃用）
    #[serde(alias = "Cold")]
    #[deprecated(note = "冷存储已重命名为 Chunked")]
    Cold,
}

/// 优化状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OptimizationStatus {
    /// 待优化 - 刚上传，等待后台处理
    #[default]
    Pending,
    /// 优化中 - 正在执行优化任务
    Optimizing,
    /// 已完成 - 优化完成或跳过
    Completed,
    /// 失败 - 优化失败
    Failed,
    /// 已跳过 - 不需要优化
    Skipped,
}

/// 块信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkInfo {
    /// 块ID（哈希值）
    pub chunk_id: String,
    /// 块在文件中的偏移量
    pub offset: usize,
    /// 块大小（原始大小）
    pub size: usize,
    /// 弱哈希值
    pub weak_hash: u32,
    /// 强哈希值（SHA-256）
    pub strong_hash: String,
    /// 压缩算法（用于读取时解压）
    #[serde(default)]
    pub compression: crate::core::compression::CompressionAlgorithm,
}

/// 文件差异信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDelta {
    /// 文件ID
    pub file_id: String,
    /// 基础版本ID
    pub base_version_id: String,
    /// 新版本ID
    pub new_version_id: String,
    /// 块列表
    pub chunks: Vec<ChunkInfo>,
    /// 创建时间
    pub created_at: chrono::NaiveDateTime,
}

/// 版本信息
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

/// 去重统计信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeduplicationStats {
    /// 总块数
    pub total_chunks: usize,
    /// 新增块数（实际写入磁盘）
    pub new_chunks: usize,
    /// 重复块数（已存在，跳过写入）
    pub duplicate_chunks: usize,
    /// 原始数据大小（字节）
    pub original_size: u64,
    /// 实际存储大小（字节）
    pub stored_size: u64,
    /// 节省空间（字节）
    pub space_saved: u64,
    /// 去重率（百分比，0-100）
    pub dedup_ratio: f64,
}

impl DeduplicationStats {
    /// 计算去重率
    pub fn calculate_dedup_ratio(&mut self) {
        if self.original_size > 0 {
            self.space_saved = self.original_size.saturating_sub(self.stored_size);
            self.dedup_ratio = (self.space_saved as f64 / self.original_size as f64) * 100.0;
        }
    }
}
