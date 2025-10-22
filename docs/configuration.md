# 配置指南

本文档详细说明 Silent-NAS 的配置选项。

## 配置文件

Silent-NAS 使用 TOML 格式的配置文件，默认路径为 `config.toml`。

可以通过命令行参数指定配置文件：
```bash
silent-nas --config /path/to/config.toml
```

## 完整配置示例

```toml
[server]
host = "0.0.0.0"
http_port = 8080
grpc_port = 50051
quic_port = 4433
webdav_port = 8081
s3_port = 9000

[storage]
root_path = "./storage"
chunk_size = 4194304  # 4MB
max_file_size = 10737418240  # 10GB
enable_compression = false
enable_deduplication = false

[nats]
url = "nats://127.0.0.1:4222"
topic_prefix = "silent.nas.files"
enable = true
reconnect_delay = 5
max_reconnects = 10

[auth]
enable = true
admin_user = "admin"
admin_password = "changeme"
jwt_secret = "your-secret-key-change-me"
token_expiry = 86400  # 24小时

[s3]
access_key = "minioadmin"
secret_key = "minioadmin"
enable_auth = false
region = "us-east-1"

[sync]
auto_sync = true
sync_interval = 60  # 秒
max_files_per_sync = 100
enable_incremental = true

[versioning]
enable = true
max_versions = 10
retention_days = 30
auto_cleanup = true

[cache]
enable = true
metadata_ttl = 3600  # 1小时
content_ttl = 600    # 10分钟
max_cache_size = 1073741824  # 1GB

[metrics]
enable = true
prometheus_port = 9090

[log]
level = "info"  # trace, debug, info, warn, error
format = "json"  # json, text
output = "stdout"  # stdout, stderr, file
file_path = "./logs/silent-nas.log"
max_size = 104857600  # 100MB
max_backups = 5
```

## 配置项详解

### [server] - 服务器配置

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `host` | string | "0.0.0.0" | 监听地址，0.0.0.0 表示所有网卡 |
| `http_port` | integer | 8080 | HTTP API 端口 |
| `grpc_port` | integer | 50051 | gRPC 端口 |
| `quic_port` | integer | 4433 | QUIC 端口 |
| `webdav_port` | integer | 8081 | WebDAV 端口 |
| `s3_port` | integer | 9000 | S3 API 端口 |

**示例**:
```toml
[server]
host = "127.0.0.1"  # 仅本地访问
http_port = 8888    # 修改 HTTP 端口
```

### [storage] - 存储配置

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `root_path` | string | "./storage" | 存储根目录 |
| `chunk_size` | integer | 4194304 | 文件块大小（字节），4MB |
| `max_file_size` | integer | 10737418240 | 最大文件大小，10GB |
| `enable_compression` | boolean | false | 启用压缩（未来支持） |
| `enable_deduplication` | boolean | false | 启用去重（未来支持） |

**示例**:
```toml
[storage]
root_path = "/mnt/nas/storage"  # 使用独立磁盘
chunk_size = 8388608            # 8MB 块大小（大文件优化）
max_file_size = 53687091200     # 50GB 最大文件
```

### [nats] - 消息服务配置

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `url` | string | "nats://127.0.0.1:4222" | NATS 服务器地址 |
| `topic_prefix` | string | "silent.nas.files" | 主题前缀 |
| `enable` | boolean | true | 启用 NATS（单节点可关闭） |
| `reconnect_delay` | integer | 5 | 重连延迟（秒） |
| `max_reconnects` | integer | 10 | 最大重连次数 |

**集群配置示例**:
```toml
[nats]
url = "nats://nats1:4222,nats2:4222,nats3:4222"  # 多节点
enable = true
reconnect_delay = 2
max_reconnects = -1  # 无限重连
```

**单节点禁用示例**:
```toml
[nats]
enable = false  # 不需要事件推送
```

### [auth] - 认证配置

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `enable` | boolean | true | 启用认证 |
| `admin_user` | string | "admin" | 管理员用户名 |
| `admin_password` | string | "changeme" | 管理员密码（生产环境必须修改） |
| `jwt_secret` | string | "your-secret-key" | JWT 密钥（生产环境必须修改） |
| `token_expiry` | integer | 86400 | Token 过期时间（秒），24小时 |

**生产环境配置**:
```toml
[auth]
enable = true
admin_user = "admin"
admin_password = "ComplexP@ssw0rd!123"  # 强密码
jwt_secret = "random-generated-secret-key-32-chars-min"  # 随机生成
token_expiry = 28800  # 8小时
```

**开发环境配置**:
```toml
[auth]
enable = false  # 关闭认证，方便测试
```

### [s3] - S3 API 配置

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `access_key` | string | "minioadmin" | 访问密钥 |
| `secret_key` | string | "minioadmin" | 密钥 |
| `enable_auth` | boolean | false | 启用 S3 认证 |
| `region` | string | "us-east-1" | 区域名称 |

**启用 S3 认证**:
```toml
[s3]
access_key = "AKIAIOSFODNN7EXAMPLE"
secret_key = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
enable_auth = true
region = "us-west-2"
```

### [node] - 节点发现与心跳（gRPC 节点同步）

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `enable` | boolean | true | 启用节点功能（节点注册/心跳/跨节点同步）|
| `seed_nodes` | array(string) | [] | 种子节点地址列表，`host:grpc_port` |
| `heartbeat_interval` | integer | 10 | 心跳间隔（秒） |
| `node_timeout` | integer | 30 | 判定离线的超时时间（秒） |

**示例**:
```toml
[node]
enable = true
seed_nodes = ["192.168.1.10:50051", "192.168.1.11:50051"]
heartbeat_interval = 10
node_timeout = 30
```

### [sync] - 跨节点同步行为

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `auto_sync` | boolean | true | 启用自动同步 |
| `sync_interval` | integer | 60 | 同步间隔（秒） |
| `max_files_per_sync` | integer | 100 | 每次最大同步文件数 |
| `max_retries` | integer | 3 | 失败重试次数 |

**集群配置**:
```toml
[sync]
auto_sync = true
sync_interval = 30  # 30秒同步一次
max_files_per_sync = 200
max_retries = 3
```

### [versioning] - 版本控制配置

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `enable` | boolean | true | 启用版本控制 |
| `max_versions` | integer | 10 | 每个文件最大版本数 |
| `retention_days` | integer | 30 | 版本保留天数 |
| `auto_cleanup` | boolean | true | 自动清理过期版本 |

**长期归档配置**:
```toml
[versioning]
enable = true
max_versions = 50    # 保留更多版本
retention_days = 365 # 保留一年
auto_cleanup = false # 手动清理
```

### [cache] - 缓存配置

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `enable` | boolean | true | 启用缓存 |
| `metadata_ttl` | integer | 3600 | 元数据缓存TTL（秒） |
| `content_ttl` | integer | 600 | 内容缓存TTL（秒） |
| `max_cache_size` | integer | 1073741824 | 最大缓存大小（字节），1GB |

**高性能配置**:
```toml
[cache]
enable = true
metadata_ttl = 7200     # 2小时
content_ttl = 1800      # 30分钟
max_cache_size = 5368709120  # 5GB
```

### [metrics] - 监控配置

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `enable` | boolean | true | 启用 Prometheus metrics |
| `prometheus_port` | integer | 9090 | Prometheus 端口 |

**集成 Prometheus**:
```toml
[metrics]
enable = true
prometheus_port = 9090
```

### [log] - 日志配置

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `level` | string | "info" | 日志级别：trace, debug, info, warn, error |
| `format` | string | "json" | 日志格式：json, text |
| `output` | string | "stdout" | 输出：stdout, stderr, file |
| `file_path` | string | "./logs/silent-nas.log" | 日志文件路径 |
| `max_size` | integer | 104857600 | 最大文件大小（字节），100MB |
| `max_backups` | integer | 5 | 最大备份数 |

**开发环境**:
```toml
[log]
level = "debug"
format = "text"
output = "stdout"
```

**生产环境**:
```toml
[log]
level = "info"
format = "json"
output = "file"
file_path = "/var/log/silent-nas/app.log"
max_size = 524288000  # 500MB
max_backups = 10
```

## 环境变量

除全局环境变量外，以下关键选项可直接覆盖：

```bash
export NODE_ENABLE=true                         # 启用节点
export NODE_SEEDS=host1:50051,host2:50051      # 逗号分隔的种子列表
export NODE_HEARTBEAT=10                       # 心跳间隔
export NODE_TIMEOUT=30                         # 节点超时

export SYNC_AUTO=true                          # 自动同步
export SYNC_INTERVAL=60                        # 同步间隔
export SYNC_MAX_FILES=200                      # 每次最大同步文件数
export SYNC_MAX_RETRIES=3                      # 重试次数

export ENABLE_AUTH=false                       # 其它覆盖示例
```

## 配置模板

### 单节点开发环境

```toml
[server]
host = "127.0.0.1"
http_port = 8080

[storage]
root_path = "./storage"

[nats]
enable = false

[auth]
enable = false

[log]
level = "debug"
format = "text"
```

### 生产环境单节点

```toml
[server]
host = "0.0.0.0"
http_port = 8080

[storage]
root_path = "/mnt/storage"
max_file_size = 53687091200  # 50GB

[auth]
enable = true
admin_user = "admin"
admin_password = "StrongPassword123!"
jwt_secret = "your-random-secret-key-min-32-chars"

[versioning]
enable = true
max_versions = 20
retention_days = 90

[cache]
enable = true
max_cache_size = 5368709120  # 5GB

[metrics]
enable = true

[log]
level = "info"
format = "json"
output = "file"
file_path = "/var/log/silent-nas/app.log"
```

### 集群模式

```toml
[server]
host = "0.0.0.0"
http_port = 8080

[storage]
root_path = "/mnt/storage"

[nats]
url = "nats://nats1:4222,nats2:4222,nats3:4222"
enable = true
max_reconnects = -1

[sync]
auto_sync = true
sync_interval = 30
enable_incremental = true

[auth]
enable = true
admin_user = "admin"
admin_password = "ClusterPassword123!"

[metrics]
enable = true

[log]
level = "info"
format = "json"
```

## 下一步

- [API 使用指南](api-guide.md) - 各协议 API 使用方法
- [部署指南](deployment.md) - 生产环境部署建议
