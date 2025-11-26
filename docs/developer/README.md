# 开发者文档

本目录包含 Silent-NAS 的技术文档和需求文档，面向希望深入了解系统实现细节的开发者。

## 📖 文档列表

### 技术文档

1. **[WebDAV 技术文档](webdav-technical.md)** ⭐
   - 模块架构（constants, types, handler, files, locks, props, deltav, routes）
   - 核心能力（资源操作、版本控制、锁机制）
   - 锁与并发（锁冲突矩阵、条件请求、持久化）
   - 属性管理（PROPPATCH 实现、属性存储）
   - 版本报告（REPORT、sync-collection、version-tree、silent:filter）
   - 客户端互通（Cyberduck、Nextcloud、Finder）
   - 报告扩展示例（完整的 XML 请求/响应）
   - 测试验证（互通测试脚本）

2. **[搜索功能技术文档](search-technical.md)** ⭐
   - 功能概览（9 大核心功能）
   - 技术架构（核心技术栈、模块依赖、数据流）
   - 核心模块（搜索引擎、内容提取器、增量索引器、S3 Select、统一搜索）
   - API 端点（全文搜索、搜索统计、搜索建议、重建索引）
   - 使用示例（HTTP API、WebDAV SEARCH、S3 Select）
   - 配置说明（搜索引擎、增量索引、环境变量）
   - 性能指标（已实现目标、代码质量、索引性能）
   - 文件结构（新增 2,210 行代码）
   - 测试结果（10/10 单元测试通过）
   - 后续优化方向（权限控制、性能优化、中文分词）

### 需求文档

3. **[WebDAV 需求文档](requirements.md)**
   - 目标：macOS Finder 兼容
   - 协议与路由要求
   - PROPFIND 响应规范
   - 必需的文件属性

4. **[同步需求文档](requirements-sync.md)**
   - 分布式同步需求
   - 同步协议设计
   - CRDT 同步机制

### 历史文档

5. **[指标增强说明](metrics-enhancements.md)**
   - v0.6.0 指标增强
   - Prometheus 指标定义
   - 分位数计算方法

---

## 🎯 快速导航

### 我想了解 WebDAV 的实现细节
→ [WebDAV 技术文档](webdav-technical.md)

### 我想了解搜索功能的架构设计
→ [搜索功能技术文档](search-technical.md)

### 我想了解系统需求
→ [WebDAV 需求文档](requirements.md) + [同步需求文档](requirements-sync.md)

### 我想贡献代码
→ 先阅读 [WebDAV 技术文档](webdav-technical.md) 和 [搜索功能技术文档](search-technical.md)，了解现有架构

---

## 📐 技术栈概览

### 核心技术
- **语言**: Rust 2024 Edition
- **Web 框架**: Silent（本地路径依赖）
- **异步运行时**: Tokio
- **序列化**: Serde
- **数据库**: Sled（LSM-tree 嵌入式数据库）
- **搜索引擎**: Tantivy 0.22

### 存储系统
- **存储引擎**: silent-storage v2（CDC、去重、压缩）
- **内容寻址**: SHA-256 哈希
- **去重**: Bloom Filter + 引用计数
- **压缩**: LZ4/Zstd
- **WAL**: 写前日志
- **缓存**: 三级缓存系统

### 分布式同步
- **CRDT**: silent-crdt（自定义 CRDT 库）
- **消息队列**: NATS
- **节点管理**: gRPC 心跳与发现
- **增量同步**: 基于时间戳的增量同步

### 协议支持
- **HTTP REST API**: Silent 框架
- **WebDAV**: RFC 4918, RFC 3253, RFC 6578
- **S3 兼容 API**: AWS S3 子集
- **gRPC**: tonic-prost
- **QUIC**: quinn

---

## 🔍 代码风格指南

### Rust 编码规范
- 遵循 Rust 2024 Edition
- 格式化：`cargo fmt`（配置在 `rustfmt.toml`）
- Lint：`cargo clippy --all-targets --all-features`
- 遵循 [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)

### ID 生成规则
- **统一使用 scru128**，不使用 UUID
- 确保 ID 的高可用性和排序性

### 时间处理规则
- 统一使用本地时间：`chrono::Local::now().naive_local()`
- 除非特别指定 chrono 库，否则均使用本地时间

### 命名约定
- 文件 ID: `file_id`
- 版本 ID: `version_id`
- 块 ID: `chunk_id`
- 节点 ID: `node_id`

---

## 🧪 开发工作流

### 1. 代码检查
```bash
# 格式化代码
cargo fmt

# 优先使用 cargo check
cargo check --all-features

# Clippy 检查
cargo clippy --all-targets --all-features -- -D warnings
```

### 2. 运行测试
```bash
# 所有测试
cargo test --all-features

# 特定模块测试
cargo test storage:: --all-features
cargo test search:: --all-features
cargo test webdav:: --all-features
```

### 3. 构建和运行
```bash
# 开发模式运行
cargo run --release

# 详细日志
RUST_LOG=debug cargo run --all-features

# 构建 release
cargo build --release --all-features
```

### 4. 代码提交
- **不跳过 hook**：确保代码通过所有检查
- **提交格式**：遵循 Conventional Commits
  - `feat(module): 新功能描述`
  - `fix(module): 修复描述`
  - `perf(module): 性能优化描述`
  - `refactor(module): 重构描述`
  - `docs(module): 文档更新描述`
  - `test(module): 测试相关描述`

---

## 📚 相关资源

### 项目文档
- [用户文档](../README.md)
- [API 使用指南](../api-guide.md)
- [部署指南](../deployment.md)
- [项目路线图](../../ROADMAP.md)
- [开发任务清单](../../TODO.md)

### 外部资源
- [Rust 官方文档](https://doc.rust-lang.org/)
- [Tokio 文档](https://tokio.rs/)
- [Tantivy 文档](https://docs.rs/tantivy/)
- [WebDAV RFC](https://tools.ietf.org/html/rfc4918)
- [S3 API 文档](https://docs.aws.amazon.com/s3/)

---

## 🤝 贡献指南

发现技术文档问题或想要改进？欢迎提交 Pull Request！

1. Fork 项目
2. 创建特性分支
3. 提交更改
4. 推送到分支
5. 创建 Pull Request

---

**版本**: v0.7.0
**最后更新**: 2025-11-26
