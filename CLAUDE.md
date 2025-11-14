# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概述

Silent-NAS 是一个基于 Rust 的高性能分布式网络存储服务器（NAS），支持多种访问协议（HTTP REST API、WebDAV、S3、gRPC、QUIC）和高级存储特性。

## 常用命令

### 构建和运行
```bash
# 开发模式运行（需要先复制配置文件）
cp config.example.toml config.toml
cargo run --release

# 开发模式运行（详细日志）
RUST_LOG=debug cargo run --all-features

# 构建 release 版本
cargo build --release --all-features

# 使用 Makefile
make run           # 开发模式运行
make dev           # 详细日志运行
make build-release # 构建 release
```

### 代码检查和格式化
```bash
# 格式化代码
cargo fmt

# 代码检查（优先使用）
cargo check --all-features

# Clippy 检查
cargo clippy --all-targets --all-features -- -D warnings

# 使用 Makefile
make fmt     # 格式化
make clippy  # Clippy 检查
make check   # 运行所有检查
```

### 测试
```bash
# 运行所有测试
cargo test --all-features

# 运行特定模块测试
cargo test storage:: --all-features
cargo test s3:: --all-features
cargo test webdav:: --all-features

# 运行单个测试
cargo test <test_name> --all-features

# 生成覆盖率报告
make coverage
make coverage-html  # HTML 报告

# 使用 Makefile
make test           # 所有测试
make test-storage   # 存储模块测试
make test-s3        # S3 模块测试
make test-webdav    # WebDAV 模块测试
```

### gRPC 构建
```bash
# gRPC 代码由 build.rs 自动生成
# Proto 文件位置: proto/file_service.proto
# 构建时会自动编译 proto 文件
```

## 项目架构

### 核心模块结构

```
silent-nas/
├── src/                      # 主应用代码
│   ├── main.rs              # 入口点，启动所有服务器
│   ├── lib.rs               # 库接口，用于测试和集成
│   ├── auth/                # 认证授权（JWT、密码、角色管理）
│   ├── cache.rs             # 缓存管理
│   ├── config.rs            # 配置管理（从 config.toml 和环境变量加载）
│   ├── error.rs             # 错误类型定义
│   ├── http/                # HTTP REST API 处理器
│   ├── webdav/              # WebDAV 协议实现
│   ├── s3/                  # S3 兼容 API 实现
│   ├── rpc.rs               # gRPC 服务实现
│   ├── storage/             # 存储管理（使用 silent-storage）
│   ├── sync/                # 分布式同步（CRDT、节点管理、增量同步）
│   ├── version.rs           # 版本控制管理
│   ├── search/              # 文件搜索引擎（基于 tantivy）
│   ├── notify.rs            # NATS 事件通知
│   ├── transfer.rs          # QUIC 传输
│   └── metrics.rs           # Prometheus 指标
├── silent-storage/          # 高级存储引擎（V2）
│   └── src/
│       ├── storage.rs       # 主存储实现（CDC、去重、压缩）
│       ├── cache.rs         # 三级缓存系统
│       ├── wal.rs           # 写前日志
│       └── types.rs         # 存储类型定义
├── silent-nas-core/         # 核心类型和 trait
│   └── src/
│       ├── models.rs        # 数据模型（FileMetadata 等）
│       └── traits.rs        # 存储 trait 定义
├── silent-crdt/             # CRDT 同步库
├── silent/                  # Silent Web 框架（子模块）
└── proto/                   # gRPC proto 定义
```

### 关键架构设计

1. **多协议支持**：main.rs 中并行启动 5 个服务器（HTTP、gRPC、WebDAV、S3、QUIC），每个协议有独立的路由和处理器。

2. **存储抽象**：
   - `silent-nas-core` 定义统一的 `StorageManagerTrait` 和 `S3CompatibleStorageTrait`
   - `silent-storage` 提供 V2 存储引擎实现（CDC、去重、压缩、WAL、缓存）
   - 通过 `storage::create_storage()` 创建存储实例，`storage::init_global_storage()` 设置全局单例

3. **分布式同步**：
   - **CRDT 层**（`sync/crdt.rs`）：使用 silent-crdt 进行文件元数据的 CRDT 同步
   - **NATS 事件层**（`notify.rs`、`event_listener.rs`）：跨节点文件变更事件通知
   - **节点管理层**（`sync/node/`）：gRPC 节点发现、心跳、自动同步协调
   - **增量同步**（`sync/incremental/`）：基于时间戳的增量同步 API

4. **版本控制**：
   - `version.rs` 提供文件版本管理
   - 支持版本列表、恢复、删除
   - WebDAV DeltaV 和 S3 版本控制都使用此模块

5. **认证与授权**：
   - JWT Token 认证（`auth/jwt.rs`）
   - 角色管理：Admin、User、ReadOnly（`auth/models.rs`）
   - 密码 Argon2 加密（`auth/password.rs`）
   - 速率限制（`auth/rate_limit.rs`）

6. **搜索引擎**：
   - 基于 tantivy 的全文搜索（`search/`）
   - 增量索引更新（`search/incremental_indexer.rs`）
   - 支持文本文件内容提取（`search/content_extractor.rs`）

## 开发注意事项

### 配置管理
- 配置文件：`config.toml`（从 `config.example.toml` 复制）
- 配置结构在 `src/config.rs` 中定义
- 配置通过 `Config::load()` 加载，支持环境变量覆盖

### 存储引擎
- 默认使用 V2 存储引擎（silent-storage）
- 支持内容寻址、去重、压缩（LZ4/Zstd）、WAL
- 全局存储通过 `storage::get_global_storage()` 访问
- 测试时使用 `tempfile` 创建临时存储目录

### gRPC 开发
- Proto 文件：`proto/file_service.proto`
- 修改 proto 后需要重新构建（build.rs 自动处理）
- 使用 tonic-prost 生成代码
- gRPC 服务实现在 `src/rpc.rs`

### 节点部署模式
- **单节点模式**：NATS 连接失败时自动降级，无需 NATS
- **多节点模式**：所有节点连接同一 NATS 服务器，启用心跳、发现、自动同步
- 配置通过 `[node]` 和 `[sync]` 控制

### Docker 部署
- Dockerfile 和 docker-compose.yaml 位于 `docker/` 目录
- 启动脚本：`docker/start.sh`
- 环境变量 `ADVERTISE_HOST` 用于容器间通信

### 依赖项
- **Silent 框架**：本地路径依赖 `./silent/silent`（Git 子模块）
- **silent-crdt**：本地路径依赖 `./silent-crdt`（Git 子模块）
- **ID 生成**：使用 `scru128`（不使用 UUID）
- **时间处理**：使用 `chrono::Local::now().naive_local()`（本地时间）
- **日志**：使用 `tracing` 和 `tracing-subscriber`

### 代码风格
- 使用 Rust 2024 edition
- 格式化：`cargo fmt`（配置在 `rustfmt.toml`）
- Lint：`cargo clippy`
- 遵循 Rust API Guidelines

### 测试
- 单元测试：每个模块的测试在同一文件或 `tests/` 子目录
- 集成测试：项目根目录 `tests/` 文件夹
- S3 集成测试：`s3_test/scripts/`
- 使用 `tempfile` 创建临时文件和目录
- 使用 `tokio::test` 进行异步测试

### 常见开发任务

#### 添加新的 HTTP 端点
1. 在 `src/http/` 下添加处理器函数
2. 在 `src/http/mod.rs` 中注册路由
3. 使用 Silent 框架的 `SResult<T>` 作为返回类型

#### 添加新的 S3 操作
1. 在 `src/s3/handlers/` 下添加处理器
2. 在 `src/s3/handlers/routes.rs` 中注册路由
3. 实现 S3 XML 响应格式（使用 `quick-xml`）

#### 修改存储引擎
1. 编辑 `silent-storage/src/storage.rs`
2. 确保实现 `StorageManagerTrait` 和 `S3CompatibleStorageTrait`
3. 运行 `make test-storage` 验证

#### 添加 CRDT 同步字段
1. 修改 `silent-crdt` 中的 CRDT 定义
2. 更新 `src/sync/crdt.rs` 中的同步逻辑
3. 测试多节点同步行为

## 性能和监控

- **Prometheus 指标**：通过 `/metrics` 端点暴露
- **缓存**：使用 `moka` 实现多级缓存
- **日志级别**：通过 `RUST_LOG` 环境变量控制
- **性能测试**：使用 `criterion`（benchmarks 在 `silent-storage/benches/`）

## 故障排查

- **端口占用**：检查 `config.toml` 中的端口配置
- **NATS 连接失败**：系统会自动降级为单节点模式，检查 NATS 服务器状态
- **gRPC 构建失败**：确保 `proto/` 文件存在且格式正确
- **存储错误**：检查 `storage/` 目录权限和磁盘空间
- **同步问题**：查看日志中的 "sync" 和 "crdt" 相关消息

## 相关资源

- Silent 框架文档：`silent/readme.md`
- API 使用指南：`docs/api-guide.md`
- 部署指南：`docs/deployment.md`
- 多节点部署：`docs/deployment-multi-node.md`
- 项目路线图：`ROADMAP.md`
- 开发任务：`TODO.md`
