# 变更日志

本文档记录 Silent-NAS 的所有重要变更。

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

---

## [未发布]

### 计划中
- 实时性能监控面板 (Grafana dashboard)
- SIMD 加速分块算法
- Redis 缓存会话支持
- 分布式部署支持

---

## [0.7.1] - 2025-11-28

### ✨ 新增

#### WebDAV 大文件上传优化

- **上传会话管理**: 新增 REST API 用于创建、查询、更新和删除上传会话
  - `POST /api/upload-sessions` - 创建会话
  - `GET /api/upload-sessions/{id}` - 查询会话
  - `PUT /api/upload-sessions/{id}` - 更新会话状态
  - `DELETE /api/upload-sessions/{id}` - 删除会话
  - `GET /api/upload-sessions` - 列出所有会话

- **断点续传功能**: 支持大文件上传中断后从断点继续
  - 会话状态管理 (Initializing, Uploading, Paused, Completed, Failed, Cancelled)
  - 上传进度跟踪（百分比、已上传大小）
  - 会话自动过期机制（默认24小时）
  - 支持暂停和恢复上传

- **秒传功能**: 基于文件哈希的重复文件检测
  - SHA-256 哈希匹配
  - 自动去重，节省存储空间
  - 大文件秒级完成上传

- **内存监控器**: 精确控制上传过程内存使用
  - 内存使用限制（默认100MB）
  - 内存警告阈值（默认80%）
  - RAII 模式自动释放内存
  - 并发上传内存控制

- **流式处理**: HTTP 层流式读取和写入
  - 8MB 分块处理（可配置）
  - 无需完整加载文件到内存
  - 支持 1GB+ 大文件上传

#### 测试和性能工具

- **集成测试**: 11个集成测试用例
  - 完整上传工作流测试
  - 并发上传内存管理测试
  - 秒传去重测试
  - 会话生命周期测试
  - 会话清理测试

- **性能测试**: 6个性能测试用例
  - 1GB 文件上传内存控制测试
  - 并发512MB文件上传测试
  - 2GB 超大文件上传测试
  - 1000会话管理性能基准
  - 10000条秒传索引性能基准
  - 100000次内存监控器操作性能基准

- **压力测试工具** (`scripts/webdav_stress_test.sh`)
  - 5个压力测试场景
  - 1000+并发连接测试
  - 自动生成测试报告

- **基准测试工具** (`scripts/webdav_benchmark.sh`)
  - 8个基准测试用例
  - 性能基线设置和对比
  - JSON 格式结果输出

#### 文档

- **使用指南** (`docs/webdav-large-file-upload-guide.md`)
  - 功能概述和快速开始
  - 多平台客户端使用说明
  - 断点续传详细教程
  - 秒传功能使用说明
  - 10个常见问题解答

- **性能测试指南** (`docs/webdav-performance-testing.md`)
  - 压力测试工具使用说明
  - 基准测试工具使用说明
  - 性能指标解读
  - 故障排查指南

- **性能调优指南** (`docs/webdav-performance-tuning.md`)
  - 配置优化建议
  - 系统调优 (Linux/macOS/Windows)
  - 场景化优化方案
  - Prometheus 监控指标

- **API 文档更新** (`docs/api-guide.md`)
  - 上传会话管理 API 完整文档
  - 断点续传示例代码
  - 秒传功能使用示例

### 🔧 改进

- **性能优化**
  - 并发上传处理优化
  - I/O 批量优化
  - 缓存策略调整

- **配置增强**
  - 新增 WebDAV 配置项
  - 内存限制可配置
  - 并发数可配置
  - 会话过期时间可配置

- **监控指标**
  - 新增 Prometheus 指标
  - `webdav_upload_total` - 上传总数
  - `webdav_upload_bytes_total` - 上传字节数
  - `webdav_upload_duration_seconds` - 上传耗时
  - `webdav_memory_usage_bytes` - 内存使用量
  - `webdav_active_sessions` - 活跃会话数
  - `webdav_instant_upload_hits` - 秒传命中数

### 📊 性能指标

- **大文件支持**: 1GB+ 文件上传 ✅
- **内存控制**: 峰值内存 < 100MB ✅
- **测试覆盖率**: > 90% ✅
- **上传吞吐量**: 100+ MB/s (受网络和磁盘限制)
- **并发连接**: 支持 1000+ 并发

### 🐛 修复

- 修复大文件上传内存溢出问题
- 修复并发上传时的竞争条件
- 修复会话过期清理逻辑

### 🔒 安全

- 增强会话管理安全性
- 上传会话自动过期机制
- 文件哈希验证

### 📝 配置示例

新增 WebDAV 配置项：

```toml
[webdav]
# 内存配置
memory_limit_mb = 100
memory_warning_threshold = 80

# 并发配置
max_concurrent_uploads = 10
max_active_sessions = 100

# 分块配置
chunk_size = 8388608  # 8MB

# 会话配置
session_ttl_hours = 24

# 功能开关
enable_instant_upload = true
enable_deduplication = true
enable_compression = true
```

### 🔗 相关链接

- [使用指南](docs/webdav-large-file-upload-guide.md)
- [性能测试指南](docs/webdav-performance-testing.md)
- [性能调优指南](docs/webdav-performance-tuning.md)
- [API 文档](docs/api-guide.md)

---

## [0.7.0] - 2025-11-26

### ✨ 新增

- **存储架构 V2 重构**
  - 内容寻址存储 (Content-Addressed Storage)
  - 增量存储和去重
  - 多种压缩算法支持 (LZ4, Zstd)
  - 写前日志 (WAL) 确保数据安全

- **三级缓存系统**
  - L1: 热数据缓存
  - L2: 元数据缓存
  - L3: 内容块缓存

- **性能优化**
  - Bloom Filter 加速查询
  - 批量写入优化
  - 流式存储处理

### 🔧 改进

- 测试覆盖率提升至 85%
- 290个测试通过
- 性能提升 50%+

### 📊 性能指标

- 读取吞吐量: 150+ MB/s
- 写入吞吐量: 100+ MB/s
- 去重率: 平均 30%

---

## [0.6.0] - 2025-11-01

### ✨ 新增

- **分布式功能完善**
  - CRDT 同步
  - 节点管理
  - 增量同步

- **多协议支持增强**
  - WebDAV 协议完善
  - S3 API 兼容性提升
  - gRPC 服务优化

- **基础存储功能**
  - 版本控制
  - 文件元数据管理
  - 搜索引擎集成

### 🔧 改进

- 系统稳定性提升
- 性能优化
- 文档完善

---

## 版本对比

| 版本 | 发布日期 | 主要特性 | 性能提升 |
|------|---------|---------|---------|
| 0.7.1 | 2025-11-28 | WebDAV 大文件上传优化 | 内存优化 |
| 0.7.0 | 2025-11-26 | 存储架构 V2 | 50%+ |
| 0.6.0 | 2025-11-01 | 分布式功能 | - |

---

## 贡献指南

欢迎贡献代码和文档！请查看 [贡献指南](CONTRIBUTING.md)。

## 许可证

本项目采用 MIT 许可证。详见 [LICENSE](LICENSE) 文件。

---

**最后更新**: 2025-11-28
