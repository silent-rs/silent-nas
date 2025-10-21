# 安装指南

本文档介绍如何在不同平台上安装 Silent-NAS。

## 系统要求

### 最低要求
- **CPU**: 2 核心
- **内存**: 2 GB RAM
- **磁盘**: 10 GB 可用空间（不包括存储数据）
- **操作系统**: Linux / macOS / Windows

### 推荐配置
- **CPU**: 4 核心或更多
- **内存**: 4 GB RAM 或更多
- **磁盘**: SSD，根据存储需求配置
- **网络**: 千兆网络

### 软件依赖
- **Rust**: 1.83+ (edition 2024) - 仅源码编译需要
- **NATS**: 消息服务器（集群模式必需）
- **Docker**: 容器化部署可选

## 安装方式

### 方式一：使用预编译二进制（推荐）

适合快速部署和生产环境使用。

#### 1. 下载

访问 [GitHub Releases](https://github.com/silent-rs/silent-nas/releases) 页面，下载适合您系统的预编译二进制文件：

```bash
# Linux (x86_64)
wget https://github.com/silent-rs/silent-nas/releases/download/v0.6.0/silent-nas-linux-x86_64.tar.gz
tar xzf silent-nas-linux-x86_64.tar.gz

# macOS (Apple Silicon)
wget https://github.com/silent-rs/silent-nas/releases/download/v0.6.0/silent-nas-macos-aarch64.tar.gz
tar xzf silent-nas-macos-aarch64.tar.gz

# macOS (Intel)
wget https://github.com/silent-rs/silent-nas/releases/download/v0.6.0/silent-nas-macos-x86_64.tar.gz
tar xzf silent-nas-macos-x86_64.tar.gz
```

#### 2. 安装到系统路径（可选）

```bash
# Linux / macOS
sudo mv silent-nas /usr/local/bin/
sudo chmod +x /usr/local/bin/silent-nas

# 验证安装
silent-nas --version
```

#### 3. 配置文件

```bash
# 下载配置文件模板
wget https://raw.githubusercontent.com/silent-rs/silent-nas/main/config.example.toml -O config.toml

# 编辑配置
vim config.toml
```

#### 4. 启动服务

```bash
silent-nas
```

### 方式二：从源码编译

适合开发者和需要自定义构建的场景。

#### 1. 安装 Rust

```bash
# 安装 Rust 工具链
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 配置环境变量
source $HOME/.cargo/env

# 验证安装
rustc --version
cargo --version
```

#### 2. 克隆项目

```bash
git clone https://github.com/silent-rs/silent-nas.git
cd silent-nas
```

#### 3. 编译

```bash
# 开发模式（快速编译，包含调试信息）
cargo build

# 生产模式（优化编译，性能最佳）
cargo build --release
```

#### 4. 配置和运行

```bash
# 复制配置文件
cp config.example.toml config.toml

# 编辑配置
vim config.toml

# 开发模式运行
cargo run

# 生产模式运行
./target/release/silent-nas
```

### 方式三：使用 Docker

适合容器化部署和快速测试。

#### 单节点部署

```bash
# 拉取镜像
docker pull silent-rs/silent-nas:latest

# 创建数据目录
mkdir -p ./storage

# 运行容器
docker run -d \
  --name silent-nas \
  -p 8080:8080 \
  -p 8081:8081 \
  -p 9000:9000 \
  -p 50051:50051 \
  -v ./storage:/data \
  -v ./config.toml:/config.toml \
  silent-rs/silent-nas:latest
```

#### 集群部署

```bash
# 克隆项目（获取 docker-compose.yml）
git clone https://github.com/silent-rs/silent-nas.git
cd silent-nas/docker

# 启动集群
docker-compose up -d

# 查看状态
docker-compose ps
```

详细的 Docker 部署说明见 [docker/README.md](../docker/README.md)

### 方式四：使用包管理器（未来支持）

计划支持的包管理器：
- [ ] Homebrew (macOS/Linux)
- [ ] APT (Debian/Ubuntu)
- [ ] YUM (RHEL/CentOS)
- [ ] Chocolatey (Windows)

## 依赖服务安装

### NATS 消息服务器

NATS 用于事件推送和集群节点通信。单节点模式可选，集群模式必需。

#### 使用 Docker（推荐）

```bash
docker run -d \
  --name nats \
  -p 4222:4222 \
  -p 8222:8222 \
  nats:latest
```

#### 使用包管理器

**macOS**:
```bash
brew install nats-server
nats-server
```

**Linux (Ubuntu/Debian)**:
```bash
curl -L https://github.com/nats-io/nats-server/releases/download/v2.10.7/nats-server-v2.10.7-linux-amd64.tar.gz | tar xz
sudo mv nats-server-v2.10.7-linux-amd64/nats-server /usr/local/bin/
nats-server
```

**验证 NATS 运行**:
```bash
# 检查端口
lsof -i :4222

# 查看信息
curl http://localhost:8222/varz
```

## 验证安装

安装完成后，运行以下命令验证：

### 1. 检查版本

```bash
silent-nas --version
# 输出: Silent-NAS v0.6.0
```

### 2. 启动服务

```bash
silent-nas
```

### 3. 健康检查

在另一个终端执行：

```bash
curl http://localhost:8080/api/health
# 输出: {"status":"ok","version":"0.6.0"}
```

### 4. 测试文件上传

```bash
# 创建测试文件
echo "Hello, Silent-NAS!" > test.txt

# 上传文件
curl -X POST -F "file=@test.txt" http://localhost:8080/api/files/upload

# 列出文件
curl http://localhost:8080/api/files/list
```

## 故障排查

### 编译错误

**问题**: `error: package requires rustc 1.83 or newer`

**解决**:
```bash
rustup update
```

### 端口被占用

**问题**: `Address already in use (os error 48)`

**解决**:
```bash
# 查找占用进程
lsof -i :8080

# 杀掉进程或修改配置文件中的端口
vim config.toml
```

### 权限问题

**问题**: `Permission denied` 创建存储目录

**解决**:
```bash
# 创建目录并设置权限
mkdir -p storage
chmod 755 storage

# 或使用 sudo 运行（不推荐）
sudo silent-nas
```

### NATS 连接失败

**问题**: `Failed to connect to NATS`

**解决**:
```bash
# 检查 NATS 是否运行
docker ps | grep nats
# 或
ps aux | grep nats-server

# 启动 NATS
docker run -d -p 4222:4222 nats:latest

# 检查配置文件中的 NATS URL
vim config.toml
```

## 下一步

- [配置指南](configuration.md) - 详细配置说明
- [API 使用指南](api-guide.md) - 各协议 API 使用方法
- [部署指南](deployment.md) - 生产环境部署建议

## 获取帮助

- [GitHub Issues](https://github.com/silent-rs/silent-nas/issues)
- [讨论区](https://github.com/silent-rs/silent-nas/discussions)
