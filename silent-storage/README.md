# Silent Storage

高性能、可靠的增量存储系统，基于内容定义分块（CDC）和块级去重技术。

## 特性

### 核心功能

- 🔄 **增量存储**: 基于内容定义分块（Content-Defined Chunking）的增量存储
- 🗜️ **高效去重**: 块级去重，跨文件共享相同的数据块
- 📦 **智能压缩**: 自适应压缩策略（LZ4 / Zstd），已压缩文件自动跳过
- 🔗 **版本链管理**: 自动检测和合并过长的版本链，保持读取性能
- 📊 **实时监控**: Prometheus 指标导出，完整的性能和健康状态监控
- 💾 **持久化存储**: 基于 Sled 的嵌入式数据库，高性能元数据管理

### 可靠性保障

- 📝 **WAL 日志**: Write-Ahead Log 确保操作可恢复
- ✅ **数据校验**: SHA256 哈希校验，防止数据损坏
- 🔍 **孤儿清理**: 自动检测和清理未被引用的数据块
- 🚀 **优雅关闭**: 确保所有数据安全落盘

### 性能优化

- ⚡ **三级缓存**: 文件元信息 + Chunk 索引 + 热数据缓存
- 🎯 **自适应分块**: 根据文件类型动态调整块大小（2KB-128KB）
- 🔥 **高吞吐量**: CDC 分块 102+ MiB/s，LZ4 压缩 19+ GiB/s
- 📈 **可扩展**: 支持大规模文件存储和高并发访问

## 架构设计

```text
silent-storage/
├── core/              # 核心存储引擎
│   ├── chunker        # 内容定义分块（CDC）
│   ├── compression    # 压缩算法（LZ4/Zstd）
│   ├── delta          # 增量计算
│   ├── engine         # 存储引擎
│   ├── file_type      # 文件类型检测
│   └── version_chain  # 版本链管理
├── services/          # 有状态服务
│   ├── dedup          # 去重服务
│   ├── index          # 索引服务
│   ├── lifecycle      # 生命周期管理
│   └── tiering        # 分层存储
├── cache.rs           # 三级缓存系统
├── metadata.rs        # 元数据管理（Sled）
├── metrics.rs         # Prometheus 指标
├── reliability.rs     # 可靠性保障（WAL/校验/清理）
└── storage.rs         # 顶层 API
```

## 快速开始

### 安装

在 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
silent-storage = { path = "../silent-storage" }
```

### 基本使用

```rust
use silent_storage::{StorageManager, IncrementalConfig};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 创建存储管理器
    let config = IncrementalConfig::default();
    let storage = StorageManager::new(
        PathBuf::from("./storage"),
        64 * 1024,  // 64KB 默认块大小
        config,
    );

    // 初始化存储
    storage.init().await?;

    // 保存文件版本
    let data = b"Hello, World!";
    let (delta, version) = storage.save_version(
        "my_file",
        data,
        None,  // 无父版本
    ).await?;

    println!("版本已保存: {}", version.version_id);
    println!("新增块数: {}", delta.chunks.len());

    // 读取文件数据
    let content = storage.read_version_data(&version.version_id).await?;
    assert_eq!(content, data);

    // 优雅关闭
    storage.shutdown().await?;

    Ok(())
}
```

### 增量更新

```rust
// 保存第一个版本
let data_v1 = b"Hello, World!";
let (_, version1) = storage.save_version("file", data_v1, None).await?;

// 保存增量版本（只存储变化的部分）
let data_v2 = b"Hello, Rust!";
let (delta, version2) = storage.save_version(
    "file",
    data_v2,
    Some(&version1.version_id),  // 指定父版本
).await?;

println!("增量块数: {}", delta.chunks.len());
```

### 去重统计

```rust
// 获取去重统计信息
let stats = storage.get_deduplication_stats().await?;

println!("去重率: {:.2}%", stats.dedup_ratio);
println!("节省空间: {} bytes", stats.space_saved());
println!("重复块: {}/{}", stats.duplicate_chunks, stats.total_chunks);
```

### 数据校验

```rust
// 验证所有 chunks 的完整性
let report = storage.verify_all_chunks().await?;

println!("总块数: {}", report.total);
println!("有效: {}, 损坏: {}, 缺失: {}",
    report.valid, report.invalid, report.missing);

// 检测孤儿 chunks
let orphans = storage.detect_orphan_chunks().await?;
println!("发现 {} 个孤儿块", orphans.len());

// 清理孤儿 chunks
if !orphans.is_empty() {
    let cleanup = storage.cleanup_orphan_chunks(&orphans).await?;
    println!("已清理: {}, 释放空间: {} bytes",
        cleanup.deleted, cleanup.freed_space);
}
```

### 缓存管理

```rust
// 获取缓存管理器
let cache = storage.get_cache_manager();

// 获取缓存统计
let stats = cache.get_stats().await;
println!("文件元信息缓存: {}/{} ({:.2}%)",
    stats.file_metadata_count,
    stats.file_metadata_capacity,
    stats.file_metadata_usage_percent
);
```

## 配置选项

### IncrementalConfig

```rust
use silent_storage::IncrementalConfig;

let config = IncrementalConfig {
    // 分块配置
    min_chunk_size: 2 * 1024,      // 最小块大小 2KB
    avg_chunk_size: 64 * 1024,     // 平均块大小 64KB
    max_chunk_size: 128 * 1024,    // 最大块大小 128KB

    // 压缩配置
    enable_compression: true,       // 启用压缩
    compression_algorithm: silent_storage::CompressionAlgorithm::Lz4,
    compression_level: 0,           // 压缩等级

    // 去重配置
    enable_deduplication: true,     // 启用去重

    // 版本链配置
    max_version_chain_depth: 5,     // 最大版本链深度
    keep_recent_versions: 2,        // 保留最近版本数
};
```

### 缓存配置

```rust
use silent_storage::{CacheManager, CacheConfig};
use std::time::Duration;

let cache_config = CacheConfig {
    // 文件元信息缓存
    file_metadata_capacity: 10_000,
    file_metadata_ttl: Duration::from_secs(3600),      // 1小时
    file_metadata_idle_time: Duration::from_secs(300), // 5分钟

    // Chunk 索引缓存
    chunk_index_capacity: 100_000,
    chunk_index_ttl: Duration::from_secs(3600),
    chunk_index_idle_time: Duration::from_secs(300),

    // 热数据缓存
    hot_data_max_weight: 100 * 1024 * 1024,  // 100MB
    hot_data_ttl: Duration::from_secs(3600),
    hot_data_idle_time: Duration::from_secs(300),
};

let cache = CacheManager::new(cache_config);
```

## 性能基准

### CDC 分块性能

| 文件大小 | 数据模式 | 吞吐量 |
|---------|---------|--------|
| 1KB     | Text    | 102 MiB/s |
| 10KB    | Binary  | 115 MiB/s |
| 100KB   | Random  | 108 MiB/s |
| 1MB     | Repetitive | 124 MiB/s |
| 10MB    | Mixed   | 118 MiB/s |

### 压缩性能

| 算法 | 吞吐量 (压缩) | 吞吐量 (解压) | 压缩比 (文本) |
|------|--------------|--------------|--------------|
| LZ4  | 19+ GiB/s    | 25+ GiB/s    | 3-4x        |
| Zstd | 7.8+ GiB/s   | 15+ GiB/s    | 5-8x        |

### 去重效果

- **相同文件**: 去重率 ~50%
- **相似文件**: 去重率 20-40%
- **不同文件**: 去重率 <5%

## 监控和运维

### Prometheus 指标

暴露在 `/metrics/storage` 端点：

```text
# 存储指标
storage_total_space_bytes
storage_used_space_bytes
storage_chunk_count

# 去重指标
storage_dedup_ratio
storage_dedup_space_saved_bytes

# 压缩指标
storage_compression_ratio
storage_compression_space_saved_bytes

# 性能指标
storage_read_latency_seconds
storage_write_latency_seconds
storage_throughput_bytes_per_second

# 操作计数
storage_operations_total{operation="create"}
storage_operations_total{operation="read"}
storage_operations_total{operation="delete"}
```

### 健康检查

```bash
curl http://localhost:8080/metrics/storage-v2/health
```

响应：
```json
{
  "status": "healthy",
  "version": "0.1.0",
  "uptime_seconds": 3600,
  "total_files": 1000,
  "total_chunks": 50000
}
```

## 故障排除

### 数据损坏

```rust
// 验证所有 chunks
let report = storage.verify_all_chunks().await?;

// 输出损坏的 chunks
for chunk in &report.corrupted_chunks {
    eprintln!("损坏的 chunk: {}", chunk);
}
```

### 性能问题

1. **检查缓存命中率**
```rust
let stats = cache.get_stats().await;
println!("缓存使用率: {:.2}%", stats.file_metadata_usage_percent);
```

2. **检查版本链深度**
```rust
let versions = storage.list_file_versions("file_id").await?;
println!("版本数: {}", versions.len());
```

3. **运行垃圾回收**
```rust
let result = storage.garbage_collect().await?;
println!("清理了 {} 个孤立块，回收 {} bytes",
    result.orphaned_chunks, result.reclaimed_space);
```

## 高级用法

### 自定义文件类型检测

```rust
use silent_storage::FileType;

let data = &[0x1f, 0x8b, 0x08]; // GZIP 魔数
let file_type = FileType::detect(data);

if file_type.is_compressed() {
    println!("文件已压缩，跳过二次压缩");
}

let (min_chunk, max_chunk) = file_type.recommended_chunk_size();
println!("推荐块大小: {}-{} bytes", min_chunk, max_chunk);
```

### 版本链合并

```rust
use silent_storage::VersionChainManager;

let manager = VersionChainManager::default();

// 检查是否需要合并
if manager.should_merge(version_chain_depth) {
    let plan = manager.generate_merge_plan(&versions, 2);
    println!("建议合并 {} 个版本", plan.versions_to_merge.len());
}
```

## 开发和测试

### 运行测试

```bash
# 运行所有测试
cargo test

# 运行特定模块测试
cargo test --lib storage

# 运行基准测试
cargo bench
```

### 性能分析

```bash
# CDC 分块性能
cargo bench --bench cdc_benchmark

# 压缩性能
cargo bench --bench compression_benchmark
```

## 许可证

MIT License

## 贡献

欢迎提交 Issue 和 Pull Request！

## 相关项目

- [Silent NAS](https://github.com/silent-rs/silent-nas) - 基于 Silent 的网络附加存储系统
- [Silent](https://github.com/silent-rs/silent) - 高性能 Rust Web 框架
