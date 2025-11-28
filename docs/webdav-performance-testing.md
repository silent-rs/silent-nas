# WebDAV 性能测试指南

本文档介绍如何对 Silent-NAS 的 WebDAV 大文件上传功能进行性能测试和基准测试。

## 目录

- [概述](#概述)
- [测试工具](#测试工具)
- [压力测试](#压力测试)
- [基准测试](#基准测试)
- [性能指标](#性能指标)
- [测试场景](#测试场景)
- [结果解读](#结果解读)
- [故障排查](#故障排查)

---

## 概述

Silent-NAS v0.7.1 引入了 WebDAV 大文件流式上传优化，支持：

- **大文件支持**: 1GB+ 文件上传
- **内存控制**: 峰值内存 < 100MB
- **并发优化**: 支持 1000+ 并发连接
- **断点续传**: 上传会话管理和恢复
- **秒传功能**: 基于哈希的重复文件检测

性能测试工具包括：

1. **压力测试** (`webdav_stress_test.sh`): 验证高并发场景下的稳定性和性能
2. **基准测试** (`webdav_benchmark.sh`): 建立性能基线，对比不同版本

---

## 测试工具

### 依赖安装

#### macOS

```bash
# 安装 wrk (HTTP 压力测试工具)
brew install wrk

# 安装 bc (基准测试需要)
brew install bc
```

#### Ubuntu/Debian

```bash
sudo apt-get update
sudo apt-get install wrk bc curl
```

#### 手动编译 wrk

```bash
git clone https://github.com/wg/wrk
cd wrk
make
sudo cp wrk /usr/local/bin/
```

### 环境变量配置

```bash
# WebDAV 服务器地址
export WEBDAV_HOST=http://localhost:8000

# 认证信息
export WEBDAV_USER=admin
export WEBDAV_PASS=admin123

# 测试目录
export TEST_DIR=/stress-test
```

---

## 压力测试

### 快速开始

```bash
cd scripts
./webdav_stress_test.sh
```

### 测试场景

压力测试脚本 (`webdav_stress_test.sh`) 包含以下测试场景：

#### 测试1: 小文件高并发上传

- **场景**: 1KB 文件，1000 并发连接，持续 30 秒
- **目的**: 验证高并发连接处理能力
- **预期**: 并发连接 ≥ 1000，无连接失败

```bash
# 单独运行此测试
wrk -t8 -c1000 -d30s -s /tmp/wrk_upload_small.lua http://localhost:8000
```

#### 测试2: 中等文件并发上传

- **场景**: 100KB 文件，500 并发连接，持续 30 秒
- **目的**: 测试中等负载下的吞吐量
- **预期**: 高吞吐量，低延迟

#### 测试3: 大文件上传吞吐量

- **场景**: 10MB 文件，并发上传 10 次
- **目的**: 测试大文件上传的聚合吞吐量
- **预期**: 吞吐量接近网络带宽限制

#### 测试4: 逐步增加并发数

- **场景**: 从 100 到 2000 并发，梯度增加
- **目的**: 找到系统的并发临界点
- **测试并发数**: 100, 250, 500, 750, 1000, 1500, 2000

#### 测试5: 长时间稳定性测试

- **场景**: 200 并发连接，持续 5 分钟
- **目的**: 验证长时间运行稳定性
- **预期**: 无内存泄漏，无性能衰减

### 结果文件

测试结果保存在 `./performance-results/` 目录：

```
performance-results/
├── test1_small_file_1000conn.txt           # 测试1结果
├── test2_medium_file_500conn.txt           # 测试2结果
├── test3_throughput_mbs.txt                # 测试3吞吐量
├── test4_concurrency_100.txt               # 测试4各并发级别结果
├── test4_concurrency_summary.csv           # 测试4汇总
├── test5_stability_5min.txt                # 测试5结果
└── summary_report.txt                      # 总体测试报告
```

### 自定义测试

```bash
# 使用自定义服务器地址
WEBDAV_HOST=http://192.168.1.100:8000 ./webdav_stress_test.sh

# 使用自定义测试目录
TEST_DIR=/my-test ./webdav_stress_test.sh

# 仅运行特定测试（编辑脚本注释掉不需要的测试）
```

---

## 基准测试

### 快速开始

```bash
cd scripts
./webdav_benchmark.sh run
```

### 测试套件

基准测试脚本 (`webdav_benchmark.sh`) 包含 8 个基准测试：

| 测试 | 描述 | 指标 |
|------|------|------|
| 1 | 1MB 文件上传 | MB/s |
| 2 | 10MB 文件上传 | MB/s |
| 3 | 100MB 文件上传 | MB/s |
| 4 | 1GB 文件上传 | MB/s |
| 5 | 并发上传 (10个 10MB) | MB/s |
| 6 | 并发上传 (5个 100MB) | MB/s |
| 7 | 100MB 文件下载 | MB/s |
| 8 | 100 个小文件操作 | 文件/秒 |

### 设置性能基线

```bash
# 运行基准测试并设置为基线
./webdav_benchmark.sh run
./webdav_benchmark.sh set-baseline
```

### 性能对比

```bash
# 运行新的基准测试
./webdav_benchmark.sh run

# 与基线对比（自动）
# 脚本会自动显示与基线的对比结果
```

输出示例：

```
========================================
性能对比分析
========================================

1MB上传: 45.23 MB/s (基线: 42.10 MB/s, +7.43%)
10MB上传: 98.56 MB/s (基线: 95.20 MB/s, +3.53%)
100MB上传: 112.34 MB/s (基线: 110.50 MB/s, +1.67%)
1GB上传: 108.90 MB/s (基线: 105.30 MB/s, +3.42%)
并发10x10MB: 250.45 MB/s (基线: 245.00 MB/s, +2.22%)
并发5x100MB: 280.12 MB/s (基线: 275.80 MB/s, +1.57%)
100MB下载: 125.67 MB/s (基线: 120.40 MB/s, +4.38%)
100小文件: 156.78 文件/秒 (基线: 150.20 文件/秒, +4.38%)
```

### 结果文件

基准测试结果保存在 `./benchmark-results/` 目录：

```
benchmark-results/
├── benchmark_20251128_103045.json          # 带时间戳的测试结果
├── benchmark_20251128_105230.json
└── ...

benchmark_baseline.json                     # 性能基线文件
```

结果文件格式（JSON）：

```json
{
  "timestamp": "20251128_103045",
  "version": "v0.7.1",
  "host": "http://localhost:8000",
  "results": {
    "upload_1mb_mbs": 45.23,
    "upload_10mb_mbs": 98.56,
    "upload_100mb_mbs": 112.34,
    "upload_1gb_mbs": 108.90,
    "concurrent_10x10mb_mbs": 250.45,
    "concurrent_5x100mb_mbs": 280.12,
    "download_100mb_mbs": 125.67,
    "small_files_100_ops": 156.78
  }
}
```

---

## 性能指标

### v0.7.1 目标指标

| 指标 | 目标值 | 当前状态 |
|------|--------|----------|
| 大文件支持 | 1GB+ | ✅ 已验证 |
| 内存控制 | < 100MB | ✅ 已实现 |
| 并发连接 | ≥ 1000 | 🔄 需压力测试验证 |
| 系统吞吐量提升 | ≥ 50% vs v0.7.0 | 🔄 需基准对比 |
| 响应时间 | < 100ms | 🔄 需实测 |

### 关键性能指标 (KPI)

1. **上传吞吐量**
   - 小文件 (1-10MB): > 80 MB/s
   - 中等文件 (10-100MB): > 100 MB/s
   - 大文件 (100MB-1GB): > 100 MB/s
   - 超大文件 (1GB+): > 90 MB/s

2. **并发性能**
   - 最大并发连接: ≥ 1000
   - 并发聚合吞吐量: > 200 MB/s
   - 平均响应时间: < 100ms
   - P99 响应时间: < 500ms

3. **资源使用**
   - 峰值内存: < 100MB (单个上传)
   - 总内存使用: < 2GB (1000 并发)
   - CPU 使用率: < 80% (峰值)

4. **稳定性**
   - 连接成功率: > 99.9%
   - 错误率: < 0.1%
   - 长时间运行无性能衰减

---

## 测试场景

### 场景1: 单用户大文件上传

**场景描述**: 单个用户上传 1GB 文件

```bash
# 生成测试文件
dd if=/dev/urandom of=/tmp/test_1gb.bin bs=1M count=1024

# 上传测试
time curl -X PUT -u admin:admin123 \
  -H "Content-Type: application/octet-stream" \
  --data-binary @/tmp/test_1gb.bin \
  http://localhost:8000/test/large_file.bin
```

**预期结果**:
- 上传成功
- 内存峰值 < 100MB
- 吞吐量 > 90 MB/s

### 场景2: 多用户并发上传

**场景描述**: 10 个用户同时上传 100MB 文件

```bash
for i in {1..10}; do
  curl -X PUT -u admin:admin123 \
    -H "Content-Type: application/octet-stream" \
    --data-binary @/tmp/test_100mb.bin \
    http://localhost:8000/test/file_$i.bin &
done
wait
```

**预期结果**:
- 所有上传成功
- 聚合吞吐量 > 200 MB/s
- 无内存溢出

### 场景3: 断点续传测试

**场景描述**: 中断上传后续传

```bash
# 1. 创建会话
curl -X POST -u admin:admin123 \
  -H "Content-Type: application/json" \
  -d '{"file_path":"/test/resume.bin","total_size":104857600}' \
  http://localhost:8000/api/upload-sessions

# 2. 部分上传后中断...

# 3. 查询会话状态
curl -X GET -u admin:admin123 \
  http://localhost:8000/api/upload-sessions/{session_id}

# 4. 续传
curl -X PUT -u admin:admin123 \
  -H "Content-Range: bytes 52428800-104857599/104857600" \
  --data-binary @/tmp/test_100mb_part2.bin \
  http://localhost:8000/test/resume.bin
```

### 场景4: 秒传测试

**场景描述**: 上传相同文件，验证秒传

```bash
# 第一次上传
curl -X PUT -u admin:admin123 \
  -H "Content-Type: application/octet-stream" \
  --data-binary @/tmp/test_file.bin \
  http://localhost:8000/test/file1.bin

# 第二次上传相同文件（应该秒传）
curl -X PUT -u admin:admin123 \
  -H "Content-Type: application/octet-stream" \
  -H "X-File-Hash: <file-hash>" \
  -H "X-File-Size: <file-size>" \
  --data-binary @/tmp/test_file.bin \
  http://localhost:8000/test/file2.bin
```

---

## 结果解读

### wrk 输出解读

```
Running 30s test @ http://localhost:8000
  8 threads and 1000 connections
  Thread Stats   Avg      Stdev     Max   +/- Stdev
    Latency    45.23ms   12.34ms  150.00ms   85.67%
    Req/Sec     2.80k    456.12     4.20k    78.90%
  672340 requests in 30.10s, 658.50MB read
Requests/sec:  22341.53
Transfer/sec:    21.88MB
```

**关键指标说明**:

- **Latency**: 平均延迟 45.23ms（目标 < 100ms）✅
- **Req/Sec**: 每秒请求数 2800（线程级）
- **Requests/sec**: 总请求数 22341/秒
- **Transfer/sec**: 吞吐量 21.88 MB/s

### 性能等级判定

| 等级 | 延迟 | 吞吐量 | 并发 | 错误率 |
|------|------|--------|------|--------|
| 优秀 | < 50ms | > 150 MB/s | > 1500 | < 0.01% |
| 良好 | 50-100ms | 100-150 MB/s | 1000-1500 | 0.01-0.1% |
| 及格 | 100-200ms | 50-100 MB/s | 500-1000 | 0.1-1% |
| 较差 | > 200ms | < 50 MB/s | < 500 | > 1% |

### 性能瓶颈识别

1. **低吞吐量**
   - 检查网络带宽限制
   - 检查磁盘 I/O 性能
   - 检查 CPU 使用率

2. **高延迟**
   - 检查内存监控器配置
   - 检查并发限制设置
   - 检查数据库连接池

3. **并发受限**
   - 调整 `max_concurrent_uploads` 配置
   - 调整内存限制配置
   - 检查系统 ulimit 设置

---

## 故障排查

### 常见问题

#### 1. wrk: command not found

**解决方法**:
```bash
# macOS
brew install wrk

# Ubuntu
sudo apt-get install wrk
```

#### 2. 连接被拒绝

**原因**: 服务器未启动或端口配置错误

**解决方法**:
```bash
# 检查服务器状态
curl http://localhost:8000

# 检查端口
netstat -an | grep 8000

# 启动服务器
cargo run --release
```

#### 3. 认证失败

**原因**: 用户名或密码错误

**解决方法**:
```bash
# 检查认证信息
export WEBDAV_USER=admin
export WEBDAV_PASS=admin123

# 或在脚本中修改
```

#### 4. 内存不足错误

**原因**: 内存监控器限制过低

**解决方法**:

编辑配置文件，调整内存限制：

```toml
[webdav]
memory_limit_mb = 200  # 增加到 200MB
```

#### 5. 测试结果不稳定

**原因**: 系统负载高或缓存影响

**解决方法**:
```bash
# 清理缓存
./webdav_benchmark.sh clean

# 关闭其他应用
# 多次运行取平均值
```

### 性能调优建议

1. **调整内存限制**
   ```toml
   [webdav]
   memory_limit_mb = 200
   memory_warning_threshold = 80
   ```

2. **调整并发限制**
   ```toml
   [webdav]
   max_concurrent_uploads = 10
   ```

3. **调整会话过期时间**
   ```toml
   [webdav]
   session_ttl_hours = 48
   ```

4. **系统级优化**
   ```bash
   # 增加文件描述符限制
   ulimit -n 10000

   # 调整 TCP 参数
   sysctl -w net.core.somaxconn=4096
   sysctl -w net.ipv4.tcp_max_syn_backlog=4096
   ```

---

## 持续性能监控

### 集成到 CI/CD

在 CI 流程中运行基准测试：

```yaml
# .github/workflows/benchmark.yml
name: Performance Benchmark

on:
  push:
    branches: [main]
  pull_request:

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2
      - name: Install dependencies
        run: |
          sudo apt-get install wrk bc
      - name: Build
        run: cargo build --release
      - name: Run benchmark
        run: |
          cargo run --release &
          sleep 5
          ./scripts/webdav_benchmark.sh run
      - name: Compare with baseline
        run: ./scripts/webdav_benchmark.sh compare
```

### Prometheus 监控

WebDAV 性能指标已集成到 Prometheus：

```bash
# 查看指标
curl http://localhost:8000/metrics | grep webdav
```

关键指标：
- `webdav_upload_total`: 上传总数
- `webdav_upload_bytes_total`: 上传字节数
- `webdav_upload_duration_seconds`: 上传耗时
- `webdav_memory_usage_bytes`: 内存使用量
- `webdav_active_sessions`: 活跃会话数

---

## 参考资料

- [wrk GitHub](https://github.com/wg/wrk)
- [WebDAV RFC 4918](https://tools.ietf.org/html/rfc4918)
- [Silent-NAS 架构文档](./ARCHITECTURE.md)
- [WebDAV 使用指南](./webdav-usage-guide.md)
- [性能调优最佳实践](./performance-tuning.md)

---

**最后更新**: 2025-11-28
**版本**: v0.7.1
