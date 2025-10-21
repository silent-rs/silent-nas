# Silent-NAS

Silent-NAS 是一个基于 Rust 的高性能分布式网络存储服务器（NAS），支持多种访问协议和高级存储特性。

## ✨ 主要特性

### 多协议支持
- 🌐 **HTTP REST API** - 简单易用的文件操作接口
- 🔌 **WebDAV** - 兼容标准 WebDAV 客户端
- 🪣 **S3 兼容** - 兼容 AWS S3 API 的对象存储
- ⚡ **gRPC** - 高性能文件传输接口
- 🚀 **QUIC** - 基于 QUIC 的高速文件传输

### 核心功能
- 📁 **文件存储** - 可靠的文件上传、下载、管理
- 🔄 **文件同步** - 基于 CRDT 的多节点自动同步
- 📝 **版本控制** - 完整的文件版本管理和恢复
- 🔐 **用户认证** - 基于角色的访问控制（Admin/User/ReadOnly）
- 📊 **性能监控** - Prometheus metrics 支持
- 🔍 **文件搜索** - 快速的文件检索功能

### 高级特性
- ⚡ **断点续传** - Range 请求支持
- 📦 **分片上传** - Multipart Upload，支持大文件（>5GB）
- 🎯 **HTTP 条件请求** - ETag、Last-Modified 缓存优化
- 📢 **事件推送** - NATS 消息总线，实时文件变更通知
- 🌍 **分布式存储** - 多节点集群部署

## 系统要求

- **操作系统**: Linux / macOS / Windows
- **Rust**: 1.83+ (edition 2024)
- **NATS**: 消息服务器（可选，用于事件推送和集群模式）
- **磁盘空间**: 根据存储需求

## 🚀 快速开始

### 方式一：Docker（推荐）

```bash
docker run -d \
  -p 8080:8080 \
  -p 8081:8081 \
  -p 9000:9000 \
  -v ./storage:/data \
  silent-rs/silent-nas:latest
```

访问 http://localhost:8080/api/health 验证服务运行。

### 方式二：从源码运行

```bash
# 克隆项目
git clone https://github.com/silent-rs/silent-nas.git
cd silent-nas

# 配置
cp config.example.toml config.toml

# 运行
cargo run --release
```

### 方式三：预编译二进制

从 [Releases](https://github.com/silent-rs/silent-nas/releases) 下载对应平台的二进制文件，配置后直接运行。

详细安装说明请参考 **[安装指南](docs/installation.md)**

## 📖 文档

- **[安装指南](docs/installation.md)** - 详细的安装步骤和系统要求
- **[配置指南](docs/configuration.md)** - 完整的配置选项说明
- **[API 使用指南](docs/api-guide.md)** - HTTP/WebDAV/S3/gRPC API 使用方法
- **[部署指南](docs/deployment.md)** - 生产环境部署和高可用配置
- **[运行指南](RUNNING.md)** - 日常运维和故障排查

查看完整文档索引：**[docs/README.md](docs/README.md)**

## 🏗️ 架构

### 单节点模式
```
┌─────────────┐
│   Client    │
└──────┬──────┘
       │
┌──────▼──────┐
│ Silent-NAS  │
│ ┌─────────┐ │
│ │ HTTP    │ │
│ │ WebDAV  │ │
│ │ S3 API  │ │
│ └─────────┘ │
│ ┌─────────┐ │
│ │ Storage │ │
│ └─────────┘ │
└─────────────┘
```

### 集群模式
```
       ┌─────────────┐
       │Load Balancer│
       └──────┬──────┘
    ┌─────────┼─────────┐
    │         │         │
┌───▼──┐  ┌───▼──┐  ┌───▼──┐
│Node 1│  │Node 2│  │Node 3│
└───┬──┘  └───┬──┘  └───┬──┘
    └─────────┼─────────┘
              │
         ┌────▼────┐
         │  NATS   │
         └─────────┘
```

详细架构说明见 **[部署指南](docs/deployment.md)**

## 许可证

本项目采用 MIT 许可证 - 详见 [LICENSE](LICENSE) 文件

## 相关项目

- [Silent Framework](https://github.com/silent-rs/silent) - Web 框架
- [Silent CRDT](https://github.com/silent-rs/silent-crdt) - 分布式数据同步
- [Silent QUIC](https://github.com/silent-rs/silent-quic) - QUIC 协议实现

## 贡献

欢迎提交 Issue 和 Pull Request！

## 联系方式

- GitHub: https://github.com/silent-rs/silent-nas
- Issues: https://github.com/silent-rs/silent-nas/issues

### HTTP REST API

**上传文件**
```bash
curl -X POST -F "file=@example.txt" http://localhost:8080/api/files/upload
```

**列出文件**
```bash
curl http://localhost:8080/api/files/list
```

**下载文件**
```bash
curl http://localhost:8080/api/files/<file_id> -o downloaded.txt
```

**删除文件**
```bash
curl -X DELETE http://localhost:8080/api/files/<file_id>
```

**健康检查**
```bash
curl http://localhost:8080/api/health
```

### WebDAV 访问

**连接地址**: `http://localhost:8081/`

**支持的客户端**:
- **macOS**: Finder → 前往 → 连接服务器
- **Windows**: 网络位置 → 添加一个网络位置
- **Linux**: Nautilus/Dolphin 文件管理器
- **跨平台**: Cyberduck, WinSCP, rclone

**命令行操作**:
```bash
# 上传文件
curl -X PUT -T example.txt http://localhost:8081/example.txt

# 列出文件
curl -X PROPFIND http://localhost:8081/ -H "Depth: 1"

# 下载文件
curl http://localhost:8081/example.txt -o downloaded.txt
```

### S3 兼容 API

**使用 MinIO Client**:
```bash
# 安装
brew install minio/stable/mc

# 配置
mc alias set nas http://localhost:9000 minioadmin minioadmin

# 创建 bucket
mc mb nas/my-bucket

# 上传文件
mc cp file.txt nas/my-bucket/

# 列出文件
mc ls nas/my-bucket/
```

**使用 AWS CLI**:
```bash
# 配置
aws configure set aws_access_key_id minioadmin
aws configure set aws_secret_access_key minioadmin

# 使用 S3 命令
aws s3 ls --endpoint-url http://localhost:9000
aws s3 cp file.txt s3://my-bucket/ --endpoint-url http://localhost:9000
```

详细使用说明见 [运行指南](RUNNING.md)
