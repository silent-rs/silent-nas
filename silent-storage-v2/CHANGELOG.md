# Changelog

All notable changes to Silent Storage V2 will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2025-11-14

### Added - Phase 5 (v0.7.0 完整实现)

#### Step 1: Prometheus 指标收集
- 新增 `StorageMetrics` 结构体，包含 6 大类监控指标
  - 存储指标：总空间、已用空间、chunk 数量
  - 去重指标：去重率、节省空间
  - 压缩指标：压缩比、节省空间
  - 增量指标：Delta 率、节省空间
  - 性能指标：读写延迟、吞吐量
  - 操作计数：create/read/delete/copy
- Prometheus 格式指标导出
- 健康状态检查 API
- 5 个单元测试覆盖核心功能
- 总测试：77 passed
- Commit: 8286f4b

#### Step 2: 独立 HTTP 端点
- 新增 `/metrics/storage-v2` HTTP 端点
- 新增 `/metrics/storage-v2/health` 健康检查端点
- 新增 `/metrics/storage-v2/json` JSON 格式指标
- `StorageV2MetricsState` 状态管理器
- 实时统计更新支持
- 集成到 AppState
- 4 个单元测试
- 总测试：77 passed
- Commit: 393b7d2

#### Step 3: 性能优化和缓存策略
- 新增 `CacheManager` 高性能缓存管理器
- 使用 moka 库实现 LRU 缓存策略
- 三级缓存架构：
  - 文件元信息缓存（10,000 条目）
  - Chunk 索引缓存（100,000 条目）
  - 热数据缓存（100MB 权重限制）
- 缓存特性：
  - TTL：1 小时自动过期
  - Idle：5 分钟空闲淘汰
  - 异步并发安全（RwLock）
  - 批量操作支持
- `CacheConfig` 灵活配置
- `CacheStats` 统计信息
- 集成到 StorageManager
- 7 个单元测试
- 总测试：84 passed
- Commit: 793b631

#### Step 4: 可靠性增强
- 新增 `reliability.rs` 模块（525 行）
- **WAL (Write-Ahead Log) 支持**：
  - `WalManager`：WAL 日志管理器
  - `WalEntry`：支持校验和验证
  - 操作类型：CreateVersion, DeleteVersion, DeleteFile, GarbageCollect
  - 自动序列号和时间戳记录
- **Chunk 数据校验**：
  - `ChunkVerifier`：SHA256 校验
  - 批量验证支持
  - 扫描并验证所有 chunks
  - `ChunkVerifyReport`：详细验证报告
- **孤儿 Chunk 清理**：
  - `OrphanChunkCleaner`：孤儿检测和清理
  - `CleanupReport`：清理统计报告
  - 支持分层存储路径
- 集成到 StorageManager：
  - WAL 自动初始化
  - save_version 自动记录 WAL
  - 5 个新 API 方法：
    - `verify_all_chunks()`
    - `verify_chunks()`
    - `detect_orphan_chunks()`
    - `cleanup_orphan_chunks()`
    - `shutdown()`
- 7 个单元测试
- 总测试：91 passed
- Commit: f6008d7

#### Step 5: 完整测试和文档
- 新增完整的 README.md 文档
  - 快速开始指南
  - 架构说明
  - API 使用示例
  - 配置选项
  - 性能基准
  - 监控和运维
  - 故障排除
- 完善 Rustdoc 文档
  - 模块级文档
  - 完整的 API 文档
  - 代码示例
- 新增 CHANGELOG.md
- 文档生成无警告
- 91 个测试全部通过
- Clippy 检查通过（0 warnings）

### Added - Phase 4 (压缩和版本链优化)

#### Step 1: 版本链深度管理
- 新增 `VersionChainManager` 版本链管理器
- 支持版本链深度检测和自动合并
- 默认最大深度 5 层，保留最近 2 个版本
- 防止版本链过长导致恢复性能退化
- 实现合并计划生成和块数据合并
- 4 个单元测试覆盖所有核心功能
- 总测试：72 passed
- Commit: df375cf

#### Step 2: 压缩性能基准测试
- 创建压缩性能基准测试套件
- 测试 LZ4 vs Zstd vs 无压缩性能
- 测试不同数据模式的压缩比
- 测试不同压缩等级的性能权衡
- 性能结果：
  - LZ4 吞吐量：19+ GiB/s
  - Zstd 吞吐量：7.8+ GiB/s
  - 文本压缩比：3-8x
- 新增 `PHASE4_COMPLETION_REPORT.md`
- Commit: a80fc52

### Added - Phase 3 (CDC 分块和去重增强)

#### Step 1: 分析当前实现
- 检查 chunker.rs 的滚动哈希实现
- 检查 delta.rs 的去重逻辑
- 分析块大小策略
- 评估跨文件去重的可行性
- 编写性能分析报告

#### Step 2: 优化 RabinKarpChunker
- 实现环形缓冲区 `CircularBuffer`
- 替换低效的 `Vec::remove(0)`
- 修复滚动哈希算法
- 优化边界检测逻辑
- 新增 `circular_buffer.rs`
- 总测试：56 passed

#### Step 3: 增强块去重策略
- 新增 `DeduplicationStats` 统计结构
- 在 save_version 中实现块去重检查
- 先检查块是否已存在再决定是否写入
- 新增 `get_deduplication_stats` API
- 编写去重测试
- 总测试：58 passed

#### Step 4: 自适应块大小策略
- 实现智能文件类型检测（`FileType` 枚举）
- 支持 7 种文件类型
- 20+ 魔数格式检测
- 根据文件类型动态调整块大小（2KB-128KB）
- 已压缩文件跳过二次压缩
- 新增 `file_type.rs`
- 总测试：72 passed
- Commit: 1d15c44

#### Step 5: 性能测试和优化
- 创建 CDC 分块性能基准测试套件
- 测试不同文件大小（1KB-10MB）和数据模式
- 验证自适应块大小策略效果
- 评估去重率和文件类型检测开销
- 性能结果：
  - 102+ MiB/s 稳定吞吐量
  - 自适应策略提升 0.44-0.01%
- 新增 `PERFORMANCE_REPORT.md`
- Commit: bcfe65d

### Added - Phase 2 (Sled 数据库集成)

- `SledMetadataDb` 封装完成
- 移除所有内存缓存
- 所有元数据操作直接使用 Sled
- 数据迁移逻辑完善
- 总测试：50 passed

### Added - Phase 1 (现状评估与架构调整)

- 审查现有 StorageManager 实现
- 确认已实现 trait
- 明确渐进式改进方案

## Performance Benchmarks

### CDC Chunking Performance
- 1KB files: 102 MiB/s (Text)
- 10KB files: 115 MiB/s (Binary)
- 100KB files: 108 MiB/s (Random)
- 1MB files: 124 MiB/s (Repetitive)
- 10MB files: 118 MiB/s (Mixed)

### Compression Performance
- LZ4: 19+ GiB/s (compression), 25+ GiB/s (decompression)
- Zstd: 7.8+ GiB/s (compression), 15+ GiB/s (decompression)
- Text compression ratio: 3-8x

### Deduplication Effectiveness
- Identical files: ~50% dedup ratio
- Similar files: 20-40% dedup ratio
- Different files: <5% dedup ratio

## Test Coverage

- Total tests: 91
- All tests passing
- Code coverage: High (core modules fully tested)
- Clippy warnings: 0

## Documentation

- Complete README.md with examples
- Rustdoc API documentation
- Architecture documentation
- Performance benchmarks
- Troubleshooting guide

[0.1.0]: https://github.com/silent-rs/silent-nas/tree/v0.1.0
