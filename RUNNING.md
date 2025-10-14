# Silent-NAS 运行指南

## 系统要求

- Rust 1.83+ (edition 2024)
- NATS 服务器
- 足够的磁盘空间用于文件存储

## 已实现的功能

✅ **核心功能**
- HTTP REST API（文件上传/下载/删除/列表）
- gRPC 接口（高性能文件操作）
- QUIC 文件传输（高速传输协议）
- NATS 事件推送（文件变更通知）
- 基础用户认证（角色权限管理）

## 依赖安装

### 1. 安装 Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. 安装 NATS

#### macOS
```bash
brew install nats-server
```

#### Linux
```bash
# 下载最新版本
curl -L https://github.com/nats-io/nats-server/releases/download/v2.10.5/nats-server-v2.10.5-linux-amd64.tar.gz | tar xz
sudo mv nats-server-v2.10.5-linux-amd64/nats-server /usr/local/bin/
```

#### Docker
```bash
docker pull nats:latest
```

## 运行服务器

### 使用默认配置

```bash
cargo run
```

默认配置：
- HTTP 端口：8080
- gRPC 端口：50051
- QUIC 端口：4433
- 存储路径：./storage
- NATS 地址：nats://127.0.0.1:4222

### 使用自定义配置

编辑 `config.toml` 文件，然后运行：
```bash
cargo run
```

## 测试 API

### 1. 健康检查

```bash
curl http://127.0.0.1:8080/api/health
```

### 2. 上传文件

```bash
# 上传文本文件
echo "Hello, Silent-NAS!" > test.txt
curl -X POST http://127.0.0.1:8080/api/files \
  --data-binary @test.txt

# 响应示例：
# {"file_id":"01jexxxx","hash":"sha256...","size":19}
```

### 3. 列出文件

```bash
curl http://127.0.0.1:8080/api/files
```

### 4. 下载文件

```bash
# 使用上传时返回的 file_id
curl http://127.0.0.1:8080/api/files/<file_id> -o downloaded.txt
```

### 5. 删除文件

```bash
curl -X DELETE http://127.0.0.1:8080/api/files/<file_id>
```

## 测试 gRPC

使用 `grpcurl` 测试：

```bash
# 安装 grpcurl
brew install grpcurl

# 列出服务
grpcurl -plaintext localhost:50051 list

# 上传文件
grpcurl -plaintext -d '{"file_id":"test-001","data":"SGVsbG8gV29ybGQ="}' \
  localhost:50051 silent.nas.FileService/UploadFile

# 下载文件
grpcurl -plaintext -d '{"file_id":"test-001"}' \
  localhost:50051 silent.nas.FileService/DownloadFile
```

## 查看 NATS 事件

订阅文件事件：

```bash
# 安装 NATS CLI
brew install nats-io/nats-tools/nats

# 订阅所有文件事件
nats sub "silent.nas.files.>"

# 订阅特定事件
nats sub "silent.nas.files.created"
nats sub "silent.nas.files.modified"
nats sub "silent.nas.files.deleted"
```

## 日志级别

设置环境变量控制日志级别：

```bash
# 设置为 DEBUG 级别
RUST_LOG=debug cargo run

# 只显示 silent_nas 的日志
RUST_LOG=silent_nas=info cargo run
```

## 故障排查

### NATS 连接失败

错误：`连接 NATS 失败`

解决：
1. 确认 NATS 服务器正在运行：`ps aux | grep nats-server`
2. 检查端口是否被占用：`lsof -i :4222`
3. 修改 `config.toml` 中的 NATS URL

### 端口被占用

错误：`Address already in use`

解决：
1. 查找占用端口的进程：`lsof -i :8080`
2. 修改 `config.toml` 中的端口配置

### 存储目录权限问题

错误：`Permission denied`

解决：
```bash
mkdir -p storage
chmod 755 storage
```

## 性能测试

### 使用 Apache Bench 测试 HTTP 接口

```bash
# 安装 ab
brew install httpd

# 测试上传性能（1000 个请求，10 个并发）
ab -n 1000 -c 10 -p test.txt \
  http://127.0.0.1:8080/api/files
```

### 使用 ghz 测试 gRPC 性能

```bash
# 安装 ghz
brew install ghz

# 测试 gRPC 上传性能
ghz --insecure --proto proto/file_service.proto \
  --call silent.nas.FileService.UploadFile \
  -d '{"file_id":"bench-{{.RequestNumber}}","data":"SGVsbG8="}' \
  -n 1000 -c 10 \
  localhost:50051
```

## 开发模式

实时重新编译运行：

```bash
# 安装 cargo-watch
cargo install cargo-watch

# 自动重新编译
cargo watch -x run
```

## 清理

删除所有上传的文件和存储目录：

```bash
rm -rf storage
```
