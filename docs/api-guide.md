# API 使用指南

本文档详细说明如何使用 Silent-NAS 的各种 API 协议。

## HTTP REST API

Silent-NAS 提供标准的 RESTful API 用于文件管理。

### 基础配置

- **基础URL**: `http://localhost:8080`
- **认证**: Bearer Token（如启用）
- **Content-Type**: `application/json` 或 `multipart/form-data`

### 文件操作 API

#### 上传文件

```bash
POST /api/files/upload

# 示例
curl -X POST \
  -F "file=@example.txt" \
  http://localhost:8080/api/files/upload

# 响应
{
  "file_id": "01JE7XXXXXXXXXXXXXXXXXXXXXC",
  "filename": "example.txt",
  "size": 1024,
  "hash": "sha256:abc123...",
  "created_at": "2025-10-21T10:00:00Z"
}
```

#### 列出文件

```bash
GET /api/files/list

# 示例
curl http://localhost:8080/api/files/list

# 带分页
curl "http://localhost:8080/api/files/list?page=1&limit=20"

# 响应
{
  "files": [
    {
      "file_id": "01JE7X...",
      "filename": "example.txt",
      "size": 1024,
      "created_at": "2025-10-21T10:00:00Z"
    }
  ],
  "total": 100,
  "page": 1,
  "limit": 20
}
```

#### 下载文件

```bash
GET /api/files/{file_id}

# 示例
curl http://localhost:8080/api/files/01JE7X... -o downloaded.txt

# Range 请求（断点续传）
curl -H "Range: bytes=0-1023" \
  http://localhost:8080/api/files/01JE7X... -o part1.txt
```

#### 获取文件元数据

```bash
HEAD /api/files/{file_id}

# 示例
curl -I http://localhost:8080/api/files/01JE7X...

# 响应头
HTTP/1.1 200 OK
Content-Length: 1024
Content-Type: text/plain
ETag: "abc123..."
Last-Modified: Mon, 21 Oct 2025 10:00:00 GMT
```

#### 删除文件

```bash
DELETE /api/files/{file_id}

# 示例
curl -X DELETE http://localhost:8080/api/files/01JE7X...

# 响应
{
  "status": "deleted",
  "file_id": "01JE7X..."
}
```

### 版本控制 API

#### 查看文件版本历史

```bash
GET /api/files/{file_id}/versions

# 示例
curl http://localhost:8080/api/files/01JE7X.../versions

# 响应
{
  "file_id": "01JE7X...",
  "versions": [
    {
      "version_id": "v1",
      "size": 1024,
      "created_at": "2025-10-21T10:00:00Z"
    }
  ]
}
```

#### 恢复文件版本

```bash
POST /api/files/{file_id}/versions/{version_id}/restore

# 示例
curl -X POST \
  http://localhost:8080/api/files/01JE7X.../versions/v1/restore
```

### 认证 API

#### 登录

```bash
POST /api/auth/login

# 示例
curl -X POST \
  -H "Content-Type: application/json" \
  -d '{"username":"admin","password":"changeme"}' \
  http://localhost:8080/api/auth/login

# 响应
{
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "expires_in": 86400
}
```

#### 使用 Token

```bash
# 在请求头中添加 Authorization
curl -H "Authorization: Bearer eyJhbGc..." \
  http://localhost:8080/api/files/list
```

### 健康检查 API

```bash
# 简单健康检查
curl http://localhost:8080/api/health

# 就绪检查
curl http://localhost:8080/api/health/readiness

# 详细状态
curl http://localhost:8080/api/health/status
```

## WebDAV 协议

WebDAV 让您可以像访问本地文件系统一样访问 Silent-NAS。

### 连接信息

- **URL**: `http://localhost:8081/`
- **用户名**: `admin`（如启用认证）
- **密码**: `changeme`

### 客户端配置

#### macOS Finder

1. 打开 Finder
2. 菜单：前往 → 连接服务器（⌘K）
3. 输入：`http://localhost:8081`
4. 输入用户名和密码

#### Windows 文件资源管理器

1. 右键"此电脑" → 映射网络驱动器
2. 选择驱动器号
3. 文件夹：`http://localhost:8081`
4. 勾选"使用其他凭据连接"
5. 输入用户名和密码

#### Linux Nautilus

1. 打开文件管理器
2. 菜单：文件 → 连接到服务器
3. 服务器地址：`dav://localhost:8081`
4. 输入用户名和密码

### 命令行操作

#### 上传文件

```bash
curl -X PUT -T example.txt \
  http://localhost:8081/example.txt
```

#### 创建目录

```bash
curl -X MKCOL http://localhost:8081/mydir/
```

#### 列出文件

```bash
curl -X PROPFIND \
  -H "Depth: 1" \
  http://localhost:8081/
```

#### 下载文件

```bash
curl http://localhost:8081/example.txt -o downloaded.txt
```

#### 移动文件

```bash
curl -X MOVE \
  -H "Destination: http://localhost:8081/newpath/example.txt" \
  http://localhost:8081/example.txt
```

#### 复制文件

```bash
curl -X COPY \
  -H "Destination: http://localhost:8081/copy.txt" \
  http://localhost:8081/example.txt
```

#### 删除文件

```bash
curl -X DELETE http://localhost:8081/example.txt
```

### 使用 rclone

rclone 是强大的云存储同步工具。

#### 配置

```bash
# 交互式配置
rclone config

# 或直接编辑配置文件 ~/.config/rclone/rclone.conf
[silent-nas]
type = webdav
url = http://localhost:8081
vendor = other
user = admin
pass = obscured_password
```

#### 常用命令

```bash
# 列出文件
rclone ls silent-nas:

# 上传文件
rclone copy local_file.txt silent-nas:/

# 下载文件
rclone copy silent-nas:/file.txt ./

# 同步目录
rclone sync ./local_dir/ silent-nas:/remote_dir/

# 挂载为本地文件系统
rclone mount silent-nas:/ /mnt/nas
```

## S3 兼容 API

Silent-NAS 提供 S3 兼容的对象存储 API。

### 连接信息

- **Endpoint**: `http://localhost:9000`
- **Access Key**: `minioadmin`
- **Secret Key**: `minioadmin`
- **Region**: `us-east-1`

### 使用 MinIO Client (mc)

#### 安装

```bash
# macOS
brew install minio/stable/mc

# Linux
wget https://dl.min.io/client/mc/release/linux-amd64/mc
chmod +x mc
sudo mv mc /usr/local/bin/
```

#### 配置

```bash
mc alias set nas http://localhost:9000 minioadmin minioadmin
```

#### 基本操作

```bash
# 创建 bucket
mc mb nas/my-bucket

# 上传文件
mc cp file.txt nas/my-bucket/

# 上传目录
mc cp --recursive ./mydir/ nas/my-bucket/

# 列出 bucket
mc ls nas/

# 列出文件
mc ls nas/my-bucket/

# 下载文件
mc cp nas/my-bucket/file.txt ./

# 删除文件
mc rm nas/my-bucket/file.txt

# 删除 bucket
mc rb nas/my-bucket

# 同步目录
mc mirror ./local_dir/ nas/my-bucket/remote_dir/
```

### 使用 AWS CLI

#### 安装

```bash
# macOS
brew install awscli

# Linux
pip install awscli
```

#### 配置

```bash
aws configure set aws_access_key_id minioadmin
aws configure set aws_secret_access_key minioadmin
aws configure set region us-east-1

# 设置 endpoint
export S3_ENDPOINT=http://localhost:9000
```

#### 基本操作

```bash
# 列出 bucket
aws s3 ls --endpoint-url $S3_ENDPOINT

# 创建 bucket
aws s3 mb s3://my-bucket --endpoint-url $S3_ENDPOINT

# 上传文件
aws s3 cp file.txt s3://my-bucket/ --endpoint-url $S3_ENDPOINT

# 列出文件
aws s3 ls s3://my-bucket/ --endpoint-url $S3_ENDPOINT

# 下载文件
aws s3 cp s3://my-bucket/file.txt ./ --endpoint-url $S3_ENDPOINT

# 删除文件
aws s3 rm s3://my-bucket/file.txt --endpoint-url $S3_ENDPOINT

# 同步目录
aws s3 sync ./local_dir/ s3://my-bucket/remote_dir/ --endpoint-url $S3_ENDPOINT
```

### 使用 s3cmd

#### 安装和配置

```bash
# 安装
pip install s3cmd

# 配置
cat > ~/.s3cfg << EOF
[default]
access_key = minioadmin
secret_key = minioadmin
host_base = localhost:9000
host_bucket = localhost:9000/%(bucket)
use_https = False
EOF
```

#### 基本操作

```bash
# 创建 bucket
s3cmd mb s3://my-bucket

# 上传文件
s3cmd put file.txt s3://my-bucket/

# 列出文件
s3cmd ls s3://my-bucket/

# 下载文件
s3cmd get s3://my-bucket/file.txt

# 删除文件
s3cmd del s3://my-bucket/file.txt
```

## gRPC API

gRPC 提供高性能的二进制协议。

### 连接信息

- **Endpoint**: `localhost:50051`
- **Protocol**: gRPC (HTTP/2)

### 使用 grpcurl

#### 安装

```bash
# macOS
brew install grpcurl

# Linux
go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest
```

#### 基本操作

```bash
# 列出服务
grpcurl -plaintext localhost:50051 list

# 列出方法
grpcurl -plaintext localhost:50051 list silent.nas.FileService

# 上传文件
grpcurl -plaintext -d '{
  "file_id": "test-001",
  "data": "SGVsbG8gV29ybGQ="
}' localhost:50051 silent.nas.FileService/UploadFile

# 下载文件
grpcurl -plaintext -d '{"file_id": "test-001"}' \
  localhost:50051 silent.nas.FileService/DownloadFile
```

### Python 客户端示例

```python
import grpc
from proto import file_service_pb2, file_service_pb2_grpc

# 创建连接
channel = grpc.insecure_channel('localhost:50051')
stub = file_service_pb2_grpc.FileServiceStub(channel)

# 上传文件
response = stub.UploadFile(file_service_pb2.UploadFileRequest(
    file_id='test-001',
    data=b'Hello World'
))
print(f"Upload result: {response.success}")

# 下载文件
response = stub.DownloadFile(file_service_pb2.DownloadFileRequest(
    file_id='test-001'
))
print(f"Downloaded: {response.data}")
```

## 节点同步（管理员 API）

在自动同步之外，提供两个管理接口便于联调与运维：

### 触发推送（push）

- `POST /api/admin/sync/push`
- 请求体：
  ```json
  { "target": "<grpc_host:port>", "file_ids": ["可选: 指定文件ID数组"] }
  ```
- 示例：
  ```bash
  curl -H 'Content-Type: application/json' \
    -d '{"target":"127.0.0.1:50052"}' \
    http://127.0.0.1:8080/api/admin/sync/push
  ```

### 触发请求（request，对端执行 push）

- `POST /api/admin/sync/request`
- 请求体：
  ```json
  { "source": "<grpc_host:port>", "file_ids": ["必填: 文件ID数组"] }
  ```
- 示例：
  ```bash
  curl -H 'Content-Type: application/json' \
    -d '{"source":"127.0.0.1:50051","file_ids":["01JE..."]}' \
    http://127.0.0.1:8080/api/admin/sync/request
  ```

说明：若开启认证，以上接口需要管理员权限；未开启认证时默认开放用于内网联调。

## 性能监控

### Prometheus Metrics

```bash
# 查看所有 metrics
curl http://localhost:8080/api/metrics

# 主要指标
# - http_requests_total: HTTP 请求总数
# - http_request_duration_seconds: 请求耗时
# - file_operations_total: 文件操作总数
# - file_bytes_transferred: 传输字节数
# - cache_hit_rate: 缓存命中率
```

### Grafana 集成

1. 添加 Prometheus 数据源
2. 导入 Silent-NAS 仪表盘模板（未来提供）
3. 配置告警规则

## 错误处理

### HTTP 状态码

| 状态码 | 说明 |
|--------|------|
| 200 | 成功 |
| 201 | 创建成功 |
| 204 | 删除成功 |
| 206 | 部分内容（Range 请求） |
| 304 | 未修改（缓存有效） |
| 400 | 请求错误 |
| 401 | 未认证 |
| 403 | 无权限 |
| 404 | 文件不存在 |
| 409 | 冲突 |
| 412 | 前置条件失败 |
| 413 | 文件过大 |
| 500 | 服务器错误 |

### 错误响应格式

```json
{
  "error": {
    "code": "FILE_NOT_FOUND",
    "message": "File not found: 01JE7X...",
    "details": {}
  }
}
```

## 最佳实践

### 1. 使用 Range 请求实现断点续传

```bash
# 下载前 1MB
curl -H "Range: bytes=0-1048575" \
  http://localhost:8080/api/files/01JE7X... -o part1.bin

# 继续下载剩余部分
curl -H "Range: bytes=1048576-" \
  http://localhost:8080/api/files/01JE7X... -o part2.bin

# 合并
cat part1.bin part2.bin > complete.bin
```

### 2. 使用 ETag 进行条件请求

```bash
# 获取 ETag
ETAG=$(curl -sI http://localhost:8080/api/files/01JE7X... | grep -i etag | cut -d' ' -f2)

# 条件下载（仅在文件变化时下载）
curl -H "If-None-Match: $ETAG" \
  http://localhost:8080/api/files/01JE7X... -o file.txt
```

### 3. 批量操作

```bash
# 批量上传
for file in *.txt; do
  curl -X POST -F "file=@$file" http://localhost:8080/api/files/upload
done

# 使用 rclone 同步整个目录
rclone sync ./local_dir/ silent-nas:/remote_dir/
```

### 4. 并发上传（使用 GNU parallel）

```bash
ls *.jpg | parallel -j 10 \
  'curl -X POST -F "file=@{}" http://localhost:8080/api/files/upload'
```

## 下一步

- [部署指南](deployment.md) - 生产环境部署
- [故障排查](../RUNNING.md) - 常见问题解决
