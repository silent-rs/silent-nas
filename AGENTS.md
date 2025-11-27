# Silent-NAS 开发指南

## 构建和测试命令

```bash
# 构建
cargo build --release --all-features

# 代码检查（优先使用）
cargo check --all-features
cargo clippy --all-targets --all-features -- -D warnings

# 格式化
cargo fmt

# 运行测试
cargo test --all-features                    # 所有测试
cargo test storage:: --all-features          # 存储模块
cargo test s3:: --all-features               # S3 模块
cargo test webdav:: --all-features           # WebDAV 模块
cargo test <test_name> --all-features        # 单个测试

# 使用 Makefile
make check     # 运行所有检查
make test      # 所有测试
make fmt       # 格式化
make clippy    # Clippy 检查
```

## 代码风格指南

### 导入和格式
- 使用 `cargo fmt` 格式化，配置在 `rustfmt.toml`
- 重排序导入：`reorder_imports = true`
- 最大行宽：100 字符

### 命名约定
- ID 使用 `scru128` 库，不使用 UUID
- 时间使用 `chrono::Local::now().naive_local()`（本地时间）
- 错误处理优先使用 `SResult<T>`（Silent 框架类型）

### 架构原则
- 存储操作通过 `storage::get_global_storage()` 访问
- 认证使用 JWT + Argon2 密码加密
- 多协议支持：HTTP、gRPC、WebDAV、S3、QUIC
- 分布式同步：CRDT + NATS 事件通知

### 测试要求
- 使用 `tempfile` 创建临时文件和目录
- 异步测试使用 `tokio::test`
- 集成测试放在 `tests/` 目录

### 依赖管理
- Silent 框架：本地路径依赖 `./silent/silent`
- silent-crdt：本地路径依赖 `./silent-crdt`
- 修改 proto 文件后需重新构建（build.rs 自动处理）
