# Silent-NAS

## 项目概述
Silent-NAS 是 Silent Odyssey 第六阶段的实验项目，旨在构建一个基于 Silent 框架的轻量级 NAS（网络附加存储）服务器。该项目聚焦于高性能文件传输、事件推送、一致性管理和多协议集成，全面验证 Silent 在现代分布式存储场景下的能力。
本阶段是连接 QUIC/WebTransport（高效传输层）与 CRDT（数据一致性层）的综合实验平台。

## 功能与协议范围

> **注意**：本项目仅实现服务端功能，客户端使用现有成熟产品（如 Nextcloud、FolderSync、rclone 等），不在当前项目范围内开发。

### 核心传输与控制
- ✅ 基于 QUIC 的高速文件传输
- ✅ 基于 gRPC 的文件控制与元数据接口
- ✅ 使用 NATS 进行文件变更事件推送
- ✅ 用户鉴权与访问控制（基础实现）
- ✅ 基于 CRDT 的多客户端文件同步与冲突合并

### 服务端协议兼容层
- ✅ HTTP/HTTPS 文件访问接口（REST API）
- ✅ WebDAV 服务端实现（完整支持）
- ✅ S3 兼容 API 实现（核心对象存储 + 高级特性）
  - ✅ 对象CRUD操作
  - ✅ Bucket管理
  - ✅ Range请求（断点续传）
  - ✅ CopyObject
  - ✅ 用户元数据
  - ✅ HTTP条件请求（If-None-Match、If-Modified-Since、If-Match）
  - ✅ 批量删除（DeleteObjects）
  - ✅ Bucket查询（Location、Versioning）
  - ✅ Multipart Upload（分片上传，支持大文件）
- ❌ NFS/SMB 协议支持（后续阶段）
- ❌ 多协议统一访问网关（后续阶段）

### 服务端自动化与同步能力
- ✅ 文件变更自动检测与事件推送
- ✅ 断点续传支持（S3 Range请求）
- ✅ HTTP条件请求支持（ETag、Last-Modified）
- ✅ 缓存优化（304 Not Modified）
- ✅ 并发更新保护（412 Precondition Failed）
- ✅ 分片上传支持（Multipart Upload，大文件>5GB）
- ✅ 跨节点文件同步（事件驱动 + 内容拉取）
- ✅ 文件版本管理（已完成，包含 HTTP API）
- ❌ 元数据索引与全文检索（后续阶段）

### 分布式与高级特性
- ❌ 分布式分片存储与副本恢复（后续阶段实现）
- ❌ 多节点文件一致性与版本控制（后续阶段）
- ❌ 全局命名空间与分布式锁（后续阶段）

## 架构设计

### 项目目录结构
```
src/
├── main.rs          # 启动入口
├── config.rs        # 配置管理
├── error.rs         # 错误定义
├── models.rs        # 数据模型
├── storage.rs       # 文件存储管理
├── transfer.rs      # QUIC 文件传输
├── rpc.rs           # gRPC 接口
├── notify.rs        # NATS 事件推送
└── auth.rs          # 用户认证（基础版本）

proto/
└── file_service.proto  # gRPC 协议定义

docs/
└── 需求整理.md      # 需求文档
```

### 已实现的模块
- **storage.rs**: 文件上传/下载/删除、元数据管理、SHA-256 校验、Bucket管理
- **transfer.rs**: QUIC 服务端、自签名证书、双向流通信
- **rpc.rs**: gRPC 文件服务（GetFile/ListFiles/DeleteFile）
- **notify.rs**: NATS 事件发布（created/modified/deleted）
- **auth.rs**: 基于角色的访问控制（Admin/User/ReadOnly）
- **webdav.rs**: WebDAV 协议服务器（PROPFIND/GET/PUT/DELETE/MKCOL/MOVE/COPY）
- **s3.rs**: S3兼容API服务器
  - 对象操作: PutObject/GetObject/HeadObject/DeleteObject/CopyObject
  - Bucket管理: ListBuckets/PutBucket/DeleteBucket/HeadBucket
  - 列表操作: ListObjectsV2/ListObjects
  - 高级特性: Range请求、用户元数据
- **sync.rs**: CRDT 文件同步管理器
  - 文件状态追踪: 基于 LWW-Register 和向量时钟
  - 冲突检测: 自动识别并发修改
  - 自动合并: Last-Write-Wins 策略
  - 同步 API: 查询状态、冲突列表
- **version.rs**: 文件版本管理器
  - 版本创建与存储: 全量版本备份
  - 版本历史查询: 按文件ID查询所有版本
  - 版本恢复: 回退到指定历史版本
  - 版本清理: 自动清理过期和超量版本
  - 版本统计: 文件数、版本数、存储占用
  - HTTP API: GET/DELETE /api/files/{id}/versions, POST /api/files/{id}/versions/{version_id}/restore, GET /api/versions/stats
- **sync/incremental/**: 增量同步模块
  - 文件块签名计算: SHA256强哈希 + Adler-32弱哈希
  - 块级差异检测: 识别文件变化的块
  - 增量传输: 仅传输变化的块
  - 自动回退: 失败时自动使用全量下载
  - HTTP API: GET /api/sync/signature/{id}, POST /api/sync/delta/{id}

## 快速开始

### 1. 启动 NATS 服务器
```bash
# 使用 Docker
docker run -d --name nats -p 4222:4222 nats:latest

# 或使用本地安装
nats-server
```

### 2. 配置服务
编辑 `config.toml`：
```toml
[server]
host = "127.0.0.1"
http_port = 8080
grpc_port = 50051
quic_port = 4433
webdav_port = 8081
s3_port = 9000

[storage]
root_path = "./storage"
chunk_size = 4194304  # 4MB

[nats]
url = "nats://127.0.0.1:4222"
topic_prefix = "silent.nas.files"

[s3]
access_key = "minioadmin"
secret_key = "minioadmin"
enable_auth = false
```

### 3. 运行服务
```bash
cargo run
```

### 4. 测试 HTTP API
```bash
# 上传文件
curl -X POST -d @example.txt http://127.0.0.1:8080/api/files

# 下载文件
curl http://127.0.0.1:8080/api/files/<file_id> -o downloaded.txt

# 列出文件
curl http://127.0.0.1:8080/api/files

# 删除文件
curl -X DELETE http://127.0.0.1:8080/api/files/<file_id>

# 健康检查
curl http://127.0.0.1:8080/api/health
```

### 5. 测试 WebDAV
使用任意 WebDAV 客户端连接：
```
WebDAV URL: http://127.0.0.1:8081/
```

**推荐的客户端：**
- **macOS**: Finder → 前往 → 连接服务器
- **Windows**: 网络位置 → 添加一个网络位置
- **Linux**: Nautilus/Dolphin 文件管理器
- **跨平台**: Cyberduck, WinSCP, rclone

**命令行测试：**
```bash
# 上传文件
curl -X PUT -T example.txt http://127.0.0.1:8081/example.txt

# 列出文件
curl -X PROPFIND http://127.0.0.1:8081/ -H "Depth: 1"

# 下载文件
curl http://127.0.0.1:8081/example.txt -o downloaded.txt

# 删除文件
curl -X DELETE http://127.0.0.1:8081/example.txt
```

### 6. 测试 S3 API
使用 MinIO Client (mc) 或 AWS CLI 进行测试：

**使用 MinIO Client (mc)：**
```bash
# 安装 mc (macOS)
brew install minio/stable/mc

# 配置别名
mc alias set silent-nas http://127.0.0.1:9000 minioadmin minioadmin

# 创建 bucket
mc mb silent-nas/test-bucket

# 上传文件
echo "Hello S3" > test.txt
mc cp test.txt silent-nas/test-bucket/

# 列出文件
mc ls silent-nas/test-bucket/

# 下载文件
mc cp silent-nas/test-bucket/test.txt downloaded.txt

# 查看文件信息
mc stat silent-nas/test-bucket/test.txt

# 删除文件
mc rm silent-nas/test-bucket/test.txt

# 删除 bucket
mc rb silent-nas/test-bucket
```

**使用 AWS CLI：**
```bash
# 配置 AWS CLI
aws configure set aws_access_key_id minioadmin
aws configure set aws_secret_access_key minioadmin
aws configure set region us-east-1

# 使用 S3 命令（指定 endpoint）
export S3_ENDPOINT=http://127.0.0.1:9000

# 列出 buckets
aws s3 ls --endpoint-url $S3_ENDPOINT

# 上传文件
aws s3 cp test.txt s3://test-bucket/ --endpoint-url $S3_ENDPOINT

# 列出文件
aws s3 ls s3://test-bucket/ --endpoint-url $S3_ENDPOINT

# 下载文件
aws s3 cp s3://test-bucket/test.txt downloaded.txt --endpoint-url $S3_ENDPOINT

# 删除文件
aws s3 rm s3://test-bucket/test.txt --endpoint-url $S3_ENDPOINT
```

**使用 curl 直接测试：**
```bash
# 上传对象
curl -X PUT -T test.txt http://127.0.0.1:9000/test-bucket/test.txt

# 下载对象
curl http://127.0.0.1:9000/test-bucket/test.txt

# 获取对象元数据
curl -I http://127.0.0.1:9000/test-bucket/test.txt

# 列出对象
curl "http://127.0.0.1:9000/test-bucket?list-type=2"

# 删除对象
curl -X DELETE http://127.0.0.1:9000/test-bucket/test.txt
```


## 测试与验证
- 启动多个 Silent-NAS 实例，验证跨节点文件同步：
  ```bash
  cargo run -- --port 8080
  cargo run -- --port 8081
  ```
  通过 gRPC 接口或 NATS 消息触发文件更新并观察状态收敛。
- 性能测试：使用 QUIC/WebTransport 测量文件传输吞吐量与延迟。
- 一致性验证：结合 Silent-CRDT 进行多节点文件版本冲突与自动合并测试。

## 部署模式与演进路线

Silent-NAS 既可以作为单节点服务运行，也可扩展为分布式多节点集群。根据验证目标与应用场景，可分为两个阶段：

### 单节点模式（当前阶段）
- **特点**：简单易部署，单机文件系统存储。
- **用途**：验证文件传输性能（QUIC/WebTransport）、CRDT 合并逻辑正确性、NATS 事件推送可靠性。
- **架构示意：**
  ```mermaid
  graph TD
      Client[客户端] -->|gRPC / HTTP| SilentNAS[Silent-NAS Server]
      SilentNAS -->|文件访问| LocalFS[(本地文件系统)]
      SilentNAS -->|事件推送| NATS[NATS Broker]
  ```

### 分布式模式（后续阶段）
- **特点**：多节点共享命名空间，文件副本自动同步，节点间基于 CRDT / NATS 保持最终一致性。
- **用途**：验证 Silent 框架在分布式文件一致性、版本控制、冲突合并与副本恢复方面的能力。
- **架构示意：**
  ```mermaid
  graph TD
      subgraph Cluster[Silent-NAS 集群]
          N1[Node 1]:::node
          N2[Node 2]:::node
          N3[Node 3]:::node
      end
      Client[客户端]
      Client -->|gRPC / QUIC| N1
      N1 <-->|CRDT Merge / NATS| N2
      N2 <-->|CRDT Merge / NATS| N3
      N3 -->|文件副本同步| N1
      classDef node fill:#9cf,stroke:#333,stroke-width:1px;
  ```

### 推荐演进路径
| 阶段 | 模式 | 验证重点 | 依赖项目 |
|------|------|-----------|-----------|
| Phase 1 | 单节点 | 传输性能、接口稳定性 | silent-quic |
| Phase 2 | 多节点（事件同步） | 文件变更事件推送 | silent-nats |
| Phase 3 | 多节点（文件同步） | 文件元数据一致性 | silent-crdt |
| Phase 4 | 分布式副本 | 副本同步与节点恢复 | silent-nas |
| Phase 5 | 集群协作 | 全局命名空间与分布式锁 | silent-odyssey-core |

## 服务端协议与生态兼容性

Silent-NAS 服务端通过实现标准协议接口，兼容现有成熟的 NAS 客户端生态。本项目专注于服务端协议实现，客户端直接使用开源或商业产品。

### 服务端协议支持模式
| 模式 | 服务端实现内容 | 兼容客户端 |
|------|----------------|-----------|
| **HTTP/gRPC 模式（当前阶段）** | 基于 Silent 框架的 HTTP API 与 gRPC 接口 | curl、Postman、gRPC 测试工具 |
| **WebDAV 协议模式** | 实现 WebDAV 服务端标准（RFC 4918） | Cyberduck、Nextcloud 客户端、rclone、系统原生挂载 |
| **S3 兼容模式** | 实现 S3 API 子集（基本对象存储操作） | AWS CLI、s3cmd、MinIO Client、rclone |
| **混合协议模式（成熟阶段）** | 同时支持多种协议的统一网关 | 所有上述客户端 |

### 可兼容的客户端工具
服务端实现相应协议后，可直接使用以下现有客户端：

| 协议类型 | 可用客户端 / 工具 | 说明 |
|-----------|-------------------|------|
| **WebDAV** | Cyberduck、rclone、WinSCP、Nextcloud、macOS / Windows 原生挂载 | 跨平台文件管理 |
| **S3 兼容** | AWS CLI、s3cmd、MinIO Client (mc)、rclone | 对象存储操作 |
| **HTTP/HTTPS** | curl、wget、浏览器、Postman | 基础文件访问 |
| **gRPC** | grpcurl、BloomRPC、Postman | API 测试与验证 |
| **NFS / SMB**（后续） | Linux / macOS / Windows 系统原生支持 | 网络文件系统 |

### 服务端协议支持路线图
| 阶段 | 服务端实现 | 验证方式 |
|------|-----------|----------|
| Phase 1 | HTTP API + gRPC 接口 | curl、grpcurl 测试 |
| Phase 2 | WebDAV 协议实现 | 使用 Cyberduck、rclone 验证 |
| Phase 3 | S3 兼容 API 实现 | 使用 AWS CLI、MinIO Client 验证 |
| Phase 4 | 多协议统一网关 | 多客户端并发测试 |
| Phase 5 | NFS/SMB 协议支持 | 系统原生客户端验证 |

## 未来规划
- 支持多节点分布式文件系统（DFS）
- 引入版本控制与文件锁机制
- 提供管理 API 接口（供独立的 Web 控制台项目使用）
- 实现 ACL 与多用户隔离策略
- 增强服务端监控与指标收集能力

## 关联项目
- [silent-quic](https://github.com/silent-rs/silent-quic) — 传输层支持
- [silent-crdt](https://github.com/silent-rs/silent-crdt) — 一致性与冲突合并
- [silent-nats](https://github.com/silent-rs/silent-nats) — 事件推送与订阅机制

## 移动端自动上传与同步支持

Silent-NAS 服务端通过实现标准协议（如 WebDAV / S3），直接兼容现有的移动端 NAS 客户端，验证跨设备备份、实时同步和 CRDT 文件合并能力。无需开发专用移动客户端，直接使用成熟的开源或商业产品。

### 支持自动上传的移动客户端

| 应用 | 平台 | 协议 | 自动上传能力 | 是否开源 | Silent-NAS 兼容性 |
|------|--------|-----------|----------------|---------------|----------------|
| **FolderSync** | Android | WebDAV / S3 / SMB / FTP | ✅ 支持相册和文件夹自动同步、条件同步 | ✅ 部分开源 | ✅ 推荐 |
| **Nextcloud App** | iOS / Android | WebDAV | ✅ 自动上传相册、文档，支持版本控制 | ✅ 开源 | ✅ 直接兼容 |
| **PhotoSync** | iOS / Android | WebDAV / S3 | ✅ 自动检测新照片并上传 | ❌ 商业应用 | ✅ 可用 |
| **FE File Explorer** | iOS / Android | WebDAV / SMB | 🚧 半自动（需手动触发同步） | ❌ 商业应用 | ✅ 部分兼容 |
| **Syncthing App** | Android / iOS | 自研 P2P 协议 | ✅ 自动同步指定目录，端到端加密 | ✅ 开源 | 🚧 可用于验证模型 |
| **ownCloud App** | iOS / Android | WebDAV | ✅ 自动上传相册、视频与文档 | ✅ 开源 | ✅ 高兼容性 |

### 服务端实现要求

为兼容移动端客户端，Silent-NAS 服务端需实现以下接口与机制：
- **WebDAV 服务端接口**：支持 `PUT` 上传、`PROPFIND` 列表查询、`PATCH` 分块续传等标准操作。
- **元数据管理**：记录文件修改时间、版本号和哈希值。
- **上传事件通知**：通过 NATS 推送文件变更事件。
- **CRDT 合并策略**：在多客户端写入冲突时自动合并元数据和内容。
- ✅ **HTTP 条件请求支持**：支持 `If-None-Match`、`ETag`、`If-Modified-Since`、`If-Match` 等标准头实现版本控制和缓存优化。

### 验证场景与测试建议

| 场景 | 验证内容 | 依赖模块 |
|------|------------|------------|
| 相册自动备份 | 上传性能与传输稳定性 | `transfer.rs` + `silent-quic` |
| 文档同步 | CRDT 文件合并与冲突检测 | `sync.rs` + `silent-crdt` |
| 多设备共享 | 跨节点文件一致性验证 | `silent-nas` + `silent-nats` |
| 离线重传 | 文件索引与断点续传验证 | `storage.rs` |

### 服务端协议实现路线
1. **阶段 1**：实现 WebDAV 服务端协议，使用 FolderSync / Nextcloud 客户端验证；
2. **阶段 2**：集成上传事件推送与同步日志；
3. **阶段 3**：结合 Silent-CRDT 验证冲突合并；
4. **阶段 4**：实现 S3 兼容 API，扩展客户端支持范围。

通过这些阶段，Silent-NAS 服务端将完全兼容主流移动端 NAS 客户端，实现跨端一致性验证。
