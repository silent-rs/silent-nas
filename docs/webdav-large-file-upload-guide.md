# WebDAV 大文件上传使用指南

本指南介绍如何使用 Silent-NAS 的 WebDAV 大文件上传功能，包括基本使用、高级特性和最佳实践。

## 目录

- [功能概述](#功能概述)
- [快速开始](#快速开始)
- [客户端使用](#客户端使用)
- [高级功能](#高级功能)
- [配置说明](#配置说明)
- [最佳实践](#最佳实践)
- [常见问题](#常见问题)

---

## 功能概述

Silent-NAS v0.7.1 的 WebDAV 服务支持高效的大文件上传，主要特性包括：

### 核心特性

- ✅ **大文件支持**: 支持 1GB+ 甚至更大文件的上传
- ✅ **内存控制**: 上传过程中内存占用 < 100MB，避免内存溢出
- ✅ **流式处理**: 采用流式读取和写入，无需完整加载文件到内存
- ✅ **高并发**: 支持多用户同时上传大文件

### 高级特性

- ✅ **断点续传**: 上传中断后可从断点处继续上传
- ✅ **秒传功能**: 相同文件无需重复上传，秒级完成
- ✅ **自动去重**: 基于内容哈希的自动去重，节省存储空间
- ✅ **压缩存储**: 自动压缩文件内容，提高存储效率
- ✅ **版本控制**: 保留文件历史版本，支持版本回退

### 性能指标

| 指标 | 数值 |
|------|------|
| 最大文件大小 | 无限制（受磁盘空间限制）|
| 单文件上传内存 | < 100MB |
| 并发连接数 | 1000+ |
| 上传吞吐量 | 100+ MB/s（受网络和磁盘限制）|
| 断点续传粒度 | 8MB 块 |

---

## 快速开始

### 1. 启动服务

```bash
# 启动 Silent-NAS
cargo run --release

# 或使用已编译的二进制
./target/release/silent-nas
```

默认 WebDAV 服务运行在 `http://localhost:8000`

### 2. 认证

WebDAV 使用 HTTP Basic 认证，默认账号：

- **用户名**: `admin`
- **密码**: `admin123`

> ⚠️ **安全提示**: 生产环境请务必修改默认密码！

### 3. 访问 WebDAV

浏览器访问：`http://localhost:8000/`

---

## 客户端使用

### Windows 文件资源管理器

#### 添加网络位置

1. 打开"此电脑"
2. 右键点击空白处 → "添加一个网络位置"
3. 输入地址：`http://localhost:8000`
4. 输入用户名和密码
5. 设置网络位置名称

#### 上传大文件

- **拖拽上传**: 直接拖拽文件到 WebDAV 目录
- **复制粘贴**: 复制文件，粘贴到 WebDAV 目录

**注意事项**:
- Windows 默认限制 WebDAV 文件大小为 50MB
- 需要修改注册表以支持大文件：

```
HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\WebClient\Parameters
FileSizeLimitInBytes = 0xffffffff (十进制 4294967295)
```

重启 WebClient 服务：

```cmd
net stop webclient
net start webclient
```

### macOS Finder

#### 连接服务器

1. Finder → "前往" → "连接服务器"（快捷键 Cmd+K）
2. 输入地址：`http://localhost:8000`
3. 点击"连接"
4. 输入用户名和密码
5. 选择"记住此密码"（可选）

#### 上传大文件

- **拖拽上传**: 拖拽文件到挂载的 WebDAV 卷
- **复制粘贴**: 使用 Cmd+C / Cmd+V

**提示**: macOS 对 WebDAV 大文件支持良好，无需额外配置

### Linux (Nautilus/Dolphin)

#### Nautilus (GNOME)

1. 文件管理器 → "其他位置"
2. 底部输入：`davs://localhost:8000` (HTTPS) 或 `dav://localhost:8000` (HTTP)
3. 输入用户名和密码

#### Dolphin (KDE)

1. 位置栏输入：`webdav://localhost:8000`
2. 输入认证信息

#### 命令行挂载

```bash
# 安装 davfs2
sudo apt-get install davfs2  # Ubuntu/Debian
sudo yum install davfs2       # CentOS/RHEL

# 创建挂载点
sudo mkdir -p /mnt/webdav

# 挂载
sudo mount -t davfs http://localhost:8000 /mnt/webdav

# 输入用户名和密码

# 使用
cp large-file.iso /mnt/webdav/

# 卸载
sudo umount /mnt/webdav
```

### cURL 命令行

#### 上传文件

```bash
# 上传小文件
curl -X PUT -u admin:admin123 \
  -H "Content-Type: application/octet-stream" \
  --data-binary @file.txt \
  http://localhost:8000/path/to/file.txt

# 上传大文件（显示进度）
curl -X PUT -u admin:admin123 \
  -H "Content-Type: application/octet-stream" \
  --data-binary @large-file.iso \
  --progress-bar \
  http://localhost:8000/uploads/large-file.iso \
  | cat
```

#### 下载文件

```bash
curl -X GET -u admin:admin123 \
  http://localhost:8000/path/to/file.txt \
  -o downloaded-file.txt
```

#### 创建目录

```bash
curl -X MKCOL -u admin:admin123 \
  http://localhost:8000/new-folder
```

#### 删除文件/目录

```bash
curl -X DELETE -u admin:admin123 \
  http://localhost:8000/path/to/file.txt
```

### Python (requests-webdav)

```python
from webdav3.client import Client

# 配置客户端
options = {
    'webdav_hostname': 'http://localhost:8000',
    'webdav_login': 'admin',
    'webdav_password': 'admin123'
}

client = Client(options)

# 上传文件
client.upload_sync(
    remote_path='/uploads/large-file.iso',
    local_path='./large-file.iso'
)

# 下载文件
client.download_sync(
    remote_path='/uploads/file.txt',
    local_path='./downloaded-file.txt'
)

# 列出目录
files = client.list('/uploads/')
for file in files:
    print(file)
```

---

## 高级功能

### 断点续传

Silent-NAS 支持断点续传，允许在上传中断后从断点处继续。

#### REST API 使用方式

##### 1. 创建上传会话

```bash
curl -X POST -u admin:admin123 \
  -H "Content-Type: application/json" \
  -d '{
    "file_path": "/uploads/large-file.iso",
    "total_size": 1073741824
  }' \
  http://localhost:8000/api/upload-sessions
```

响应：

```json
{
  "session_id": "01JDK8PQRS2EXAMPLE",
  "file_path": "/uploads/large-file.iso",
  "total_size": 1073741824,
  "uploaded_size": 0,
  "status": "Initializing",
  "progress_percent": 0.0,
  "created_at": "2025-11-28T10:30:00",
  "expires_at": "2025-11-30T10:30:00"
}
```

##### 2. 分块上传文件

```bash
# 第一块 (0-8MB)
curl -X PUT -u admin:admin123 \
  -H "Content-Type: application/octet-stream" \
  -H "X-Upload-Session-Id: 01JDK8PQRS2EXAMPLE" \
  -H "Content-Range: bytes 0-8388607/1073741824" \
  --data-binary @large-file.iso.part1 \
  http://localhost:8000/uploads/large-file.iso

# 第二块 (8MB-16MB)
curl -X PUT -u admin:admin123 \
  -H "Content-Type: application/octet-stream" \
  -H "X-Upload-Session-Id: 01JDK8PQRS2EXAMPLE" \
  -H "Content-Range: bytes 8388608-16777215/1073741824" \
  --data-binary @large-file.iso.part2 \
  http://localhost:8000/uploads/large-file.iso

# ... 继续上传其他块
```

##### 3. 查询上传进度

```bash
curl -X GET -u admin:admin123 \
  http://localhost:8000/api/upload-sessions/01JDK8PQRS2EXAMPLE
```

响应：

```json
{
  "session_id": "01JDK8PQRS2EXAMPLE",
  "file_path": "/uploads/large-file.iso",
  "total_size": 1073741824,
  "uploaded_size": 268435456,
  "status": "Uploading",
  "progress_percent": 25.0,
  "created_at": "2025-11-28T10:30:00",
  "updated_at": "2025-11-28T10:35:00",
  "expires_at": "2025-11-30T10:30:00"
}
```

##### 4. 恢复中断的上传

如果上传中断，可以查询会话获取已上传的大小，然后从该位置继续：

```bash
# 查询会话
SESSION_INFO=$(curl -X GET -u admin:admin123 \
  http://localhost:8000/api/upload-sessions/01JDK8PQRS2EXAMPLE)

# 提取已上传大小
UPLOADED_SIZE=$(echo $SESSION_INFO | jq -r '.uploaded_size')

# 从断点继续上传
curl -X PUT -u admin:admin123 \
  -H "Content-Type: application/octet-stream" \
  -H "X-Upload-Session-Id: 01JDK8PQRS2EXAMPLE" \
  -H "Content-Range: bytes ${UPLOADED_SIZE}-*/1073741824" \
  --data-binary @large-file.iso.remaining \
  http://localhost:8000/uploads/large-file.iso
```

##### 5. 列出所有上传会话

```bash
curl -X GET -u admin:admin123 \
  http://localhost:8000/api/upload-sessions
```

##### 6. 删除会话

```bash
curl -X DELETE -u admin:admin123 \
  http://localhost:8000/api/upload-sessions/01JDK8PQRS2EXAMPLE
```

### 秒传功能

秒传功能通过文件哈希检测重复文件，避免重复上传。

#### 使用方式

##### 1. 计算文件哈希

```bash
# 使用 SHA-256
FILE_HASH=$(shasum -a 256 large-file.iso | awk '{print $1}')
FILE_SIZE=$(stat -f%z large-file.iso)  # macOS
# 或
FILE_SIZE=$(stat -c%s large-file.iso)  # Linux
```

##### 2. 带哈希信息上传

```bash
curl -X PUT -u admin:admin123 \
  -H "Content-Type: application/octet-stream" \
  -H "X-File-Hash: $FILE_HASH" \
  -H "X-File-Size: $FILE_SIZE" \
  --data-binary @large-file.iso \
  http://localhost:8000/uploads/large-file-copy.iso
```

##### 3. 服务器响应

如果文件已存在（哈希匹配），服务器会立即返回成功，无需上传文件内容：

```
HTTP/1.1 201 Created
X-Instant-Upload: true
Content-Length: 0
```

如果文件不存在，正常上传。

#### 秒传的好处

- **节省带宽**: 相同文件无需重复传输
- **节省时间**: 大文件秒级完成
- **节省存储**: 自动去重，只存储一份副本

---

## 配置说明

### config.toml 配置

Silent-NAS 的 WebDAV 配置位于 `config.toml` 文件的 `[webdav]` 部分：

```toml
[webdav]
# WebDAV 服务器端口
port = 8000

# 内存限制（MB）
memory_limit_mb = 100

# 内存警告阈值（百分比）
memory_warning_threshold = 80

# 最大并发上传数
max_concurrent_uploads = 10

# 上传会话过期时间（小时）
session_ttl_hours = 24

# 最大活跃会话数
max_active_sessions = 100

# 分块大小（字节）
chunk_size = 8388608  # 8MB

# 是否启用秒传
enable_instant_upload = true

# 是否启用自动去重
enable_deduplication = true

# 是否启用压缩
enable_compression = true
```

### 调优建议

#### 高并发场景

```toml
[webdav]
memory_limit_mb = 200
max_concurrent_uploads = 20
max_active_sessions = 500
```

#### 大文件场景

```toml
[webdav]
memory_limit_mb = 200
chunk_size = 16777216  # 16MB
session_ttl_hours = 48  # 延长过期时间
```

#### 低内存环境

```toml
[webdav]
memory_limit_mb = 50
max_concurrent_uploads = 5
chunk_size = 4194304  # 4MB
```

---

## 最佳实践

### 1. 大文件上传建议

- **使用断点续传**: 对于 >100MB 的文件，建议使用断点续传 API
- **分块上传**: 将大文件分成 8MB 或 16MB 的块上传
- **计算哈希**: 上传前计算文件哈希，利用秒传功能
- **避免并发**: 单个大文件上传时，避免同时上传多个大文件

### 2. 网络优化

- **使用有线连接**: 避免使用不稳定的 WiFi
- **检查带宽**: 确保网络带宽足够
- **使用本地网络**: 尽量在同一局域网内上传

### 3. 安全建议

- **使用 HTTPS**: 生产环境务必使用 HTTPS
- **强密码**: 使用强密码，定期更换
- **访问控制**: 配置防火墙规则，限制访问来源
- **审计日志**: 启用审计日志，监控异常操作

### 4. 错误处理

- **重试机制**: 上传失败时自动重试
- **超时设置**: 设置合理的超时时间
- **错误日志**: 记录错误日志，便于排查问题

---

## 常见问题

### Q1: Windows 无法上传大于 50MB 的文件

**原因**: Windows WebDAV 客户端默认限制文件大小为 50MB

**解决方法**: 修改注册表

1. 打开注册表编辑器（regedit）
2. 定位到：`HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\WebClient\Parameters`
3. 修改 `FileSizeLimitInBytes` 为 `0xffffffff`（十进制 4294967295）
4. 重启 WebClient 服务

```cmd
net stop webclient
net start webclient
```

### Q2: 上传速度很慢

**可能原因**:
- 网络带宽限制
- 磁盘 I/O 性能瓶颈
- 内存限制过低
- 压缩/加密开销

**解决方法**:
- 检查网络连接
- 使用 SSD 提升 I/O 性能
- 增加内存限制配置
- 临时禁用压缩（修改配置 `enable_compression = false`）

### Q3: 上传中断后如何续传？

**方法1**: 使用支持断点续传的客户端（如 rclone）

**方法2**: 使用 REST API

1. 查询上传会话获取已上传大小
2. 使用 `Content-Range` 头从断点继续上传

详见[断点续传](#断点续传)章节

### Q4: 内存占用过高

**原因**: 并发上传过多或内存限制配置过高

**解决方法**:
- 降低 `max_concurrent_uploads`
- 降低 `memory_limit_mb`
- 检查是否有内存泄漏

### Q5: 秒传不工作

**可能原因**:
- 配置中禁用了秒传功能
- 文件哈希不匹配
- 文件大小不一致

**解决方法**:
- 检查配置 `enable_instant_upload = true`
- 使用 SHA-256 算法计算哈希
- 确保 `X-File-Hash` 和 `X-File-Size` 头正确

### Q6: 连接超时

**原因**: 网络不稳定或服务器负载高

**解决方法**:
- 增加客户端超时时间
- 检查服务器资源使用情况
- 使用更稳定的网络连接

### Q7: 认证失败

**原因**: 用户名或密码错误

**解决方法**:
- 检查认证信息是否正确
- 检查用户是否存在
- 检查用户权限

### Q8: 如何查看上传进度？

**方法1**: 客户端显示进度（如 curl 的 `--progress-bar`）

**方法2**: 查询上传会话

```bash
curl -X GET -u admin:admin123 \
  http://localhost:8000/api/upload-sessions/{session_id}
```

查看 `progress_percent` 字段

### Q9: 上传会话过期怎么办？

**原因**: 会话超过 `session_ttl_hours` 配置的时间未活动

**解决方法**:
- 增加 `session_ttl_hours` 配置
- 定期更新会话（发送 PUT 请求）
- 重新创建会话并从头上传

### Q10: 如何批量上传文件？

**方法1**: 使用脚本循环上传

```bash
for file in *.iso; do
  curl -X PUT -u admin:admin123 \
    --data-binary @"$file" \
    "http://localhost:8000/uploads/$file"
done
```

**方法2**: 使用 rclone

```bash
rclone copy /local/path webdav:/uploads \
  --progress \
  --transfers 4
```

---

## 相关文档

- [性能测试指南](./webdav-performance-testing.md)
- [性能调优最佳实践](./performance-tuning.md)
- [API 文档](./api-guide.md)
- [架构设计](./ARCHITECTURE.md)
- [故障排查](./troubleshooting.md)

---

## 技术支持

如有问题或建议，请通过以下方式联系：

- GitHub Issues: https://github.com/silent-rs/silent-nas/issues
- 讨论区: https://github.com/silent-rs/silent-nas/discussions

---

**最后更新**: 2025-11-28
**版本**: v0.7.1
