# WebDAV 性能调优最佳实践

本文档提供 Silent-NAS WebDAV 服务性能调优的详细指南，帮助您在不同场景下获得最佳性能。

## 目录

- [概述](#概述)
- [配置优化](#配置优化)
- [系统调优](#系统调优)
- [场景优化](#场景优化)
- [监控与诊断](#监控与诊断)
- [故障排查](#故障排查)
- [性能基准](#性能基准)

---

## 概述

### 性能影响因素

WebDAV 性能受以下因素影响：

1. **网络带宽**: 限制传输速率的上限
2. **磁盘 I/O**: 影响读写吞吐量
3. **内存配置**: 影响并发处理能力
4. **CPU 性能**: 影响压缩/加密等计算密集型操作
5. **并发设置**: 影响多用户场景性能

### 性能目标

| 场景 | 目标吞吐量 | 目标延迟 | 并发数 |
|------|-----------|----------|--------|
| 单用户大文件 | > 100 MB/s | < 100ms | 1 |
| 多用户中等文件 | > 200 MB/s | < 200ms | 10-50 |
| 高并发小文件 | > 150 MB/s | < 50ms | 100-1000 |

---

## 配置优化

### WebDAV 核心配置

#### config.toml 优化配置

```toml
[webdav]
# 网络配置
port = 8000
host = "0.0.0.0"  # 监听所有接口

# 内存配置
memory_limit_mb = 200          # 增加内存限制提升并发能力
memory_warning_threshold = 80  # 内存警告阈值

# 并发配置
max_concurrent_uploads = 20    # 根据服务器性能调整
max_active_sessions = 500      # 最大活跃会话数

# 分块配置
chunk_size = 8388608           # 8MB（可调整为 16MB 以提升大文件性能）

# 会话配置
session_ttl_hours = 48         # 延长过期时间支持长时间上传

# 功能开关
enable_instant_upload = true   # 启用秒传
enable_deduplication = true    # 启用去重
enable_compression = true      # 启用压缩（CPU 密集型）
```

### 不同场景的推荐配置

#### 场景1: 高吞吐量大文件上传

**特点**: 少量并发，单文件很大（1GB+）

```toml
[webdav]
memory_limit_mb = 200
max_concurrent_uploads = 5     # 限制并发数
chunk_size = 16777216          # 16MB 大块提升效率
enable_compression = false     # 禁用压缩减少 CPU 开销
session_ttl_hours = 72         # 延长过期时间
```

#### 场景2: 高并发小文件上传

**特点**: 大量并发，单文件较小（< 10MB）

```toml
[webdav]
memory_limit_mb = 100
max_concurrent_uploads = 50    # 提高并发数
max_active_sessions = 1000
chunk_size = 4194304           # 4MB 小块
enable_instant_upload = true   # 启用秒传减少重复上传
enable_compression = true      # 压缩节省带宽
```

#### 场景3: 平衡配置

**特点**: 混合负载

```toml
[webdav]
memory_limit_mb = 150
max_concurrent_uploads = 20
max_active_sessions = 500
chunk_size = 8388608           # 8MB
enable_instant_upload = true
enable_deduplication = true
enable_compression = true
```

---

## 系统调优

### Linux 系统优化

#### 1. 文件描述符限制

```bash
# 查看当前限制
ulimit -n

# 临时增加（当前会话）
ulimit -n 65536

# 永久修改 /etc/security/limits.conf
* soft nofile 65536
* hard nofile 65536
```

#### 2. TCP 参数优化

```bash
# /etc/sysctl.conf 添加以下配置

# 增加最大连接队列
net.core.somaxconn = 65536

# TCP 连接队列
net.ipv4.tcp_max_syn_backlog = 8192

# TIME_WAIT 重用
net.ipv4.tcp_tw_reuse = 1

# TCP keepalive 参数
net.ipv4.tcp_keepalive_time = 600
net.ipv4.tcp_keepalive_intvl = 30
net.ipv4.tcp_keepalive_probes = 3

# TCP 缓冲区
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216
net.ipv4.tcp_rmem = 4096 87380 16777216
net.ipv4.tcp_wmem = 4096 65536 16777216

# 应用配置
sudo sysctl -p
```

#### 3. 磁盘 I/O 优化

```bash
# 使用 SSD
# 检查磁盘调度器
cat /sys/block/sda/queue/scheduler

# SSD 推荐使用 noop 或 none
echo noop > /sys/block/sda/queue/scheduler

# 或 deadline（机械硬盘）
echo deadline > /sys/block/sda/queue/scheduler

# 增加 I/O 队列深度
echo 512 > /sys/block/sda/queue/nr_requests

# 禁用访问时间更新（减少写入）
# /etc/fstab 添加 noatime 选项
/dev/sda1  /data  ext4  defaults,noatime  0  2
```

#### 4. 内存优化

```bash
# 调整 swappiness（减少使用 swap）
echo 10 > /proc/sys/vm/swappiness

# 永久修改 /etc/sysctl.conf
vm.swappiness = 10

# 增加脏页刷新阈值
vm.dirty_ratio = 15
vm.dirty_background_ratio = 5
```

### macOS 系统优化

#### 1. 增加文件描述符限制

```bash
# 临时增加
ulimit -n 65536

# 永久修改（创建 /Library/LaunchDaemons/limit.maxfiles.plist）
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key>
    <string>limit.maxfiles</string>
    <key>ProgramArguments</key>
    <array>
      <string>launchctl</string>
      <string>limit</string>
      <string>maxfiles</string>
      <string>65536</string>
      <string>200000</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
  </dict>
</plist>
```

### Windows 系统优化

#### 1. WebClient 服务优化

```
# 注册表路径：
HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\WebClient\Parameters

# 调整以下值：
FileSizeLimitInBytes = 0xffffffff     # 文件大小限制
FileAttributesLimitInBytes = 0x100000 # 文件属性限制
```

#### 2. 重启 WebClient 服务

```cmd
net stop webclient
net start webclient
```

---

## 场景优化

### 场景1: 视频文件上传

**特点**: 文件大（几GB），顺序读写

**优化建议**:

1. **配置调整**:
```toml
chunk_size = 16777216          # 16MB 大块
enable_compression = false     # 视频已压缩，禁用再压缩
max_concurrent_uploads = 3     # 限制并发
```

2. **网络优化**:
   - 使用有线连接
   - 避免 VPN 或代理

3. **客户端建议**:
   - 使用支持断点续传的客户端（如 rclone）
   - 避免同时上传多个大文件

### 场景2: 文档备份

**特点**: 大量小文件（< 1MB），随机读写

**优化建议**:

1. **配置调整**:
```toml
chunk_size = 4194304           # 4MB
enable_instant_upload = true   # 启用秒传
enable_deduplication = true    # 启用去重
max_concurrent_uploads = 30    # 提高并发
```

2. **磁盘优化**:
   - 使用 SSD
   - 禁用访问时间更新（noatime）

3. **批量上传建议**:
   - 使用脚本批量上传
   - 控制并发数（10-20 个文件同时上传）

### 场景3: 代码仓库同步

**特点**: 中等数量文件，增量更新

**优化建议**:

1. **配置调整**:
```toml
enable_instant_upload = true   # 秒传加速重复文件
enable_deduplication = true    # 去重节省空间
```

2. **使用 Git 而非 WebDAV**:
   - Git 更适合代码管理
   - 如必须使用 WebDAV，建议打包成压缩包上传

### 场景4: 照片库同步

**特点**: 大量中等文件（1-10MB），偶尔大文件（RAW）

**优化建议**:

1. **配置调整**:
```toml
chunk_size = 8388608           # 8MB
enable_instant_upload = true   # 秒传避免重复上传相同照片
enable_compression = false     # JPEG 已压缩
max_concurrent_uploads = 10
```

2. **分类上传**:
   - 先上传 JPEG 预览，后上传 RAW 原文件
   - 按日期分目录组织

---

## 监控与诊断

### Prometheus 指标

Silent-NAS 暴露以下 WebDAV 性能指标：

```bash
# 查看所有 WebDAV 指标
curl http://localhost:8000/metrics | grep webdav
```

#### 关键指标

| 指标名称 | 描述 | 类型 |
|---------|------|------|
| `webdav_upload_total` | 上传总数 | Counter |
| `webdav_upload_bytes_total` | 上传字节总数 | Counter |
| `webdav_upload_duration_seconds` | 上传耗时分布 | Histogram |
| `webdav_memory_usage_bytes` | 内存使用量 | Gauge |
| `webdav_active_sessions` | 活跃会话数 | Gauge |
| `webdav_concurrent_uploads` | 当前并发上传数 | Gauge |
| `webdav_session_created_total` | 创建会话总数 | Counter |
| `webdav_session_completed_total` | 完成会话总数 | Counter |
| `webdav_session_failed_total` | 失败会话总数 | Counter |
| `webdav_instant_upload_hits` | 秒传命中数 | Counter |

### Grafana Dashboard

#### 推荐面板

1. **上传吞吐量**:
```promql
rate(webdav_upload_bytes_total[5m])
```

2. **平均上传延迟**:
```promql
rate(webdav_upload_duration_seconds_sum[5m]) /
rate(webdav_upload_duration_seconds_count[5m])
```

3. **并发数**:
```promql
webdav_concurrent_uploads
```

4. **秒传命中率**:
```promql
rate(webdav_instant_upload_hits[5m]) /
rate(webdav_upload_total[5m]) * 100
```

5. **内存使用**:
```promql
webdav_memory_usage_bytes / 1024 / 1024
```

### 日志分析

#### 启用详细日志

```bash
# 运行时启用 DEBUG 日志
RUST_LOG=debug cargo run --release

# 或设置环境变量
export RUST_LOG=silent_nas=debug
```

#### 关键日志模式

```bash
# 查看上传相关日志
tail -f logs/silent-nas.log | grep -i upload

# 查看内存相关日志
tail -f logs/silent-nas.log | grep -i memory

# 查看错误日志
tail -f logs/silent-nas.log | grep -i error

# 统计请求分布
cat logs/silent-nas.log | grep "PUT" | wc -l
```

---

## 故障排查

### 问题1: 上传速度慢

#### 诊断步骤

1. **检查网络带宽**:
```bash
# 使用 iperf 测试带宽
iperf3 -s  # 服务器端
iperf3 -c <server-ip>  # 客户端
```

2. **检查磁盘 I/O**:
```bash
# Linux
iostat -x 1

# 或使用 dd 测试写入速度
dd if=/dev/zero of=/data/test.img bs=1M count=1000 oflag=direct
```

3. **检查 CPU 使用率**:
```bash
top -p $(pgrep silent-nas)
```

4. **检查内存使用**:
```bash
free -h
```

#### 解决方案

- 网络瓶颈 → 升级网络或使用有线连接
- 磁盘瓶颈 → 使用 SSD 或 RAID 0
- CPU 瓶颈 → 禁用压缩（`enable_compression = false`）
- 内存瓶颈 → 增加内存限制或减少并发数

### 问题2: 高并发下性能下降

#### 诊断步骤

1. **检查并发数**:
```bash
curl http://localhost:8000/metrics | grep webdav_concurrent_uploads
```

2. **检查连接数**:
```bash
netstat -an | grep 8000 | wc -l
```

3. **检查文件描述符使用**:
```bash
lsof -p $(pgrep silent-nas) | wc -l
```

#### 解决方案

- 超过 `max_concurrent_uploads` → 增加配置值
- 文件描述符不足 → 增加 `ulimit -n`
- 连接队列满 → 调整 `net.core.somaxconn`

### 问题3: 内存占用过高

#### 诊断步骤

1. **检查活跃会话数**:
```bash
curl http://localhost:8000/metrics | grep webdav_active_sessions
```

2. **检查内存使用**:
```bash
curl http://localhost:8000/metrics | grep webdav_memory_usage_bytes
```

#### 解决方案

- 降低 `memory_limit_mb`
- 降低 `max_concurrent_uploads`
- 清理过期会话（会话自动过期机制）

### 问题4: 频繁超时

#### 诊断步骤

1. **检查日志中的超时错误**:
```bash
grep -i timeout logs/silent-nas.log
```

2. **检查网络延迟**:
```bash
ping <server-ip>
```

#### 解决方案

- 增加客户端超时设置
- 检查防火墙规则
- 优化网络路由

---

## 性能基准

### 基准测试方法

#### 1. 使用 webdav_benchmark.sh

```bash
cd scripts
./webdav_benchmark.sh run
```

#### 2. 建立性能基线

```bash
# 首次运行后设置基线
./webdav_benchmark.sh set-baseline
```

#### 3. 定期对比

```bash
# 每次调优后运行
./webdav_benchmark.sh run

# 自动对比基线
```

### 参考基准数据

#### 硬件环境

- **CPU**: 4 核 @ 2.5GHz
- **内存**: 16GB
- **磁盘**: NVMe SSD
- **网络**: 千兆以太网

#### 性能数据

| 测试项 | 吞吐量 | 延迟 | 备注 |
|--------|--------|------|------|
| 1MB 上传 | 85 MB/s | 12ms | - |
| 10MB 上传 | 110 MB/s | 90ms | - |
| 100MB 上传 | 120 MB/s | 830ms | - |
| 1GB 上传 | 115 MB/s | 8.9s | - |
| 并发 10x10MB | 280 MB/s | 350ms | 聚合吞吐量 |
| 并发 5x100MB | 300 MB/s | 1.6s | 聚合吞吐量 |
| 100MB 下载 | 135 MB/s | 740ms | - |
| 100 小文件 | 180 文件/s | 5ms | 每个 1KB |

### 性能优化检查清单

- [ ] 系统文件描述符限制 ≥ 65536
- [ ] TCP 连接队列 ≥ 4096
- [ ] 使用 SSD 存储
- [ ] 磁盘挂载使用 noatime
- [ ] WebDAV 内存限制合理（100-200MB）
- [ ] 并发数配置合理（根据硬件）
- [ ] 块大小适配文件类型（4-16MB）
- [ ] 压缩配置适配场景
- [ ] Prometheus 监控已启用
- [ ] 日志级别合理（生产环境 info）

---

## 高级调优技巧

### 1. 使用 Nginx 反向代理

#### 优势

- 连接复用
- 静态文件缓存
- SSL 卸载
- 负载均衡

#### 配置示例

```nginx
upstream webdav_backend {
    server localhost:8000;
    keepalive 64;
}

server {
    listen 443 ssl http2;
    server_name nas.example.com;

    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    client_max_body_size 0;  # 无限制
    client_body_timeout 3600s;

    location / {
        proxy_pass http://webdav_backend;
        proxy_http_version 1.1;

        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        proxy_set_header Connection "";
        proxy_buffering off;
        proxy_request_buffering off;
    }
}
```

### 2. 使用 Redis 缓存会话

（预留功能，未来版本支持）

### 3. 分布式部署

（预留功能，未来版本支持）

---

## 总结

### 快速调优步骤

1. **评估负载类型**: 大文件 vs 小文件，单用户 vs 多用户
2. **调整配置**: 根据负载类型选择推荐配置
3. **系统优化**: 调整文件描述符、TCP 参数、磁盘 I/O
4. **运行基准测试**: 建立性能基线
5. **监控指标**: 使用 Prometheus + Grafana
6. **迭代优化**: 根据监控数据持续调优

### 常用配置模板

#### 个人用户（低负载）

```toml
[webdav]
memory_limit_mb = 100
max_concurrent_uploads = 5
chunk_size = 8388608
enable_compression = true
```

#### 小团队（中等负载）

```toml
[webdav]
memory_limit_mb = 150
max_concurrent_uploads = 20
chunk_size = 8388608
max_active_sessions = 200
```

#### 企业用户（高负载）

```toml
[webdav]
memory_limit_mb = 200
max_concurrent_uploads = 50
chunk_size = 16777216
max_active_sessions = 1000
enable_instant_upload = true
enable_deduplication = true
```

---

## 相关文档

- [使用指南](./webdav-large-file-upload-guide.md)
- [性能测试指南](./webdav-performance-testing.md)
- [架构设计](./ARCHITECTURE.md)
- [API 文档](./api-guide.md)

---

**最后更新**: 2025-11-28
**版本**: v0.7.1
