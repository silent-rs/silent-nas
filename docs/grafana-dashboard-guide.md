# Grafana Dashboard 使用指南

本文档介绍如何安装和使用 Silent-NAS WebDAV 大文件上传监控 Dashboard。

---

## 前置条件

1. **Prometheus** 已安装并运行
2. **Grafana** 已安装并运行（推荐版本 9.0+）
3. **Silent-NAS** 服务已启动，且 `/metrics` 端点可访问

---

## 快速开始

### 1. 配置 Prometheus

在 Prometheus 配置文件（`prometheus.yml`）中添加 Silent-NAS 作为抓取目标：

```yaml
scrape_configs:
  - job_name: 'silent-nas'
    static_configs:
      - targets: ['localhost:8080']  # 替换为您的 Silent-NAS HTTP 端口
    metrics_path: '/metrics'
    scrape_interval: 5s
```

重启 Prometheus 使配置生效：

```bash
# Docker
docker restart prometheus

# 系统服务
sudo systemctl restart prometheus
```

### 2. 验证 Prometheus 数据采集

访问 Prometheus Web UI（默认 `http://localhost:9090`），在查询框中输入：

```promql
upload_sessions_active
```

如果能看到数据，说明 Prometheus 已成功抓取 Silent-NAS 的指标。

### 3. 导入 Grafana Dashboard

#### 方法 1：通过 JSON 文件导入

1. 登录 Grafana（默认 `http://localhost:3000`，默认账号 `admin/admin`）
2. 点击左侧菜单 **"+"** → **Import**
3. 点击 **Upload JSON file**
4. 选择项目根目录的 `grafana-dashboard-webdav.json` 文件
5. 在 **Prometheus** 下拉框中选择您的 Prometheus 数据源
6. 点击 **Import**

#### 方法 2：通过 UI 手动配置

如果文件导入失败，可以手动复制 JSON 内容：

1. 登录 Grafana
2. 点击左侧菜单 **"+"** → **Import**
3. 在 **Import via panel json** 文本框中，粘贴 `grafana-dashboard-webdav.json` 的完整内容
4. 点击 **Load**
5. 选择 Prometheus 数据源
6. 点击 **Import**

---

## Dashboard 面板说明

Dashboard 包含 12 个核心监控面板：

### 上传会话监控

| 面板 | 说明 | 指标 |
|------|------|------|
| **当前活跃上传会话数** | 实时显示正在进行的上传会话数量 | `upload_sessions_active` |
| **上传会话速率** | 按状态（创建/完成/失败/取消）显示会话创建和结束速率 | `rate(upload_sessions_total[5m])` |
| **上传会话时延** | P50/P90/P99 时延分位数，监控上传完成时间 | `histogram_quantile(..., upload_session_duration_seconds_bucket)` |

### 内存与资源监控

| 面板 | 说明 | 阈值 |
|------|------|------|
| **上传会话内存使用** | 显示当前内存占用，黄色警告 80MB，红色警告 100MB | 80MB (黄) / 100MB (红) |
| **上传会话总大小** | 所有活跃会话的文件总大小 | - |

### 吞吐量监控

| 面板 | 说明 |
|------|------|
| **上传吞吐量** | 显示已完成和失败的上传字节速率（MB/s） |
| **文件传输吞吐量** | 总体上传/下载带宽使用情况 |

### 秒传功能监控

| 面板 | 说明 |
|------|------|
| **秒传与会话清理速率** | 秒传成功次数和过期会话清理次数 |
| **秒传节省带宽** | 通过文件去重节省的带宽（MB/s） |

### WebDAV 性能监控

| 面板 | 说明 |
|------|------|
| **WebDAV PUT 请求延迟** | P50/P90/P99 请求延迟分位数 |
| **WebDAV PUT 请求速率** | 按状态码（2xx/4xx/5xx）分类的请求速率 |
| **当前活跃请求与连接数** | 实时 HTTP 请求和连接数 |

---

## 性能指标解读

### 健康状态指标

| 指标 | 良好范围 | 警告范围 | 异常范围 |
|------|----------|----------|----------|
| 上传会话内存 | < 80MB | 80-100MB | > 100MB |
| PUT 请求延迟 (P99) | < 5s | 5-10s | > 10s |
| 失败率 | < 1% | 1-5% | > 5% |
| 活跃连接数 | < 500 | 500-1000 | > 1000 |

### 性能优化建议

#### 内存使用过高（> 80MB）

**问题**: `upload_sessions_memory_bytes` 接近或超过 100MB

**解决方案**:
```toml
# config.toml
[webdav]
memory_limit_mb = 150  # 增加内存限制
max_concurrent_uploads = 5  # 减少并发数
chunk_size = 4194304  # 减小块大小到 4MB
```

#### 上传会话时延过高（P99 > 10s）

**问题**: 大文件上传完成时间过长

**解决方案**:
```toml
[webdav]
chunk_size = 16777216  # 增大块大小到 16MB
enable_compression = false  # 禁用压缩减少 CPU 开销
```

#### 秒传命中率低

**观察指标**: `rate(upload_instant_success_total[5m])` 很低

**解决方案**:
- 检查 InstantUploadManager 是否正常工作
- 确认文件哈希计算正确
- 查看日志确认没有秒传错误

#### 高并发场景下失败率高

**问题**: `rate(http_requests_total{status=~"5.."}[5m])` 持续升高

**解决方案**:
```toml
[webdav]
max_concurrent_uploads = 20  # 增加并发限制
max_active_sessions = 2000  # 增加最大会话数
```

---

## 告警规则推荐

在 Prometheus 中配置告警规则（`alert.rules.yml`）：

```yaml
groups:
  - name: silent_nas_webdav
    interval: 30s
    rules:
      # 内存使用告警
      - alert: UploadSessionMemoryHigh
        expr: upload_sessions_memory_bytes > 83886080  # 80MB
        for: 2m
        labels:
          severity: warning
        annotations:
          summary: "上传会话内存使用过高"
          description: "当前内存使用 {{ $value | humanize1024 }}B，接近 100MB 限制"

      # 上传失败率告警
      - alert: UploadFailureRateHigh
        expr: |
          rate(upload_sessions_total{status="failed"}[5m])
          /
          rate(upload_sessions_total{status="created"}[5m])
          > 0.05
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "上传失败率过高"
          description: "过去 5 分钟失败率为 {{ $value | humanizePercentage }}"

      # 活跃会话数告警
      - alert: UploadSessionsActiveHigh
        expr: upload_sessions_active > 50
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "活跃上传会话数过多"
          description: "当前有 {{ $value }} 个活跃会话，可能影响性能"

      # PUT 请求延迟告警
      - alert: WebDAVPutLatencyHigh
        expr: |
          histogram_quantile(0.99,
            rate(http_request_duration_seconds_bucket{method="PUT"}[5m])
          ) > 10
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "WebDAV PUT 请求延迟过高"
          description: "P99 延迟为 {{ $value }}s，超过 10s 阈值"
```

添加告警规则后重启 Prometheus：

```bash
# 验证配置
promtool check rules alert.rules.yml

# 重启 Prometheus
sudo systemctl restart prometheus
```

---

## 高级配置

### 自定义刷新间隔

Dashboard 默认 5 秒刷新，可在右上角修改：

1. 点击右上角时钟图标
2. 选择刷新间隔：5s / 10s / 30s / 1m

### 调整时间范围

默认显示最近 15 分钟数据，可调整为：

- **Last 5 minutes**: 实时监控
- **Last 1 hour**: 短期趋势分析
- **Last 24 hours**: 日常性能回顾
- **Last 7 days**: 长期性能对比

### 配置数据保留策略

在 Prometheus 配置中设置数据保留期：

```yaml
# prometheus.yml
global:
  scrape_interval: 5s
  evaluation_interval: 5s

# 启动参数
# --storage.tsdb.retention.time=30d  # 保留 30 天
# --storage.tsdb.retention.size=50GB  # 最大 50GB
```

---

## 故障排查

### Dashboard 无数据显示

**问题**: 所有面板显示 "No data"

**检查步骤**:

1. **验证 Prometheus 数据源**:
   ```bash
   # 访问 Prometheus UI
   curl http://localhost:9090/api/v1/targets
   ```
   确认 `silent-nas` target 状态为 `UP`

2. **检查 Silent-NAS metrics 端点**:
   ```bash
   curl http://localhost:8080/metrics | grep upload_sessions
   ```
   应该能看到 `upload_sessions_*` 相关指标

3. **验证 Grafana 数据源配置**:
   - 进入 Grafana → Configuration → Data Sources
   - 点击 Prometheus 数据源
   - 点击 **Test** 按钮，确认连接成功

### 部分面板无数据

**问题**: 个别面板显示 "No data"，如上传会话相关面板

**原因**: 可能还没有上传活动，指标值为 0

**验证**:
```bash
# 创建一个测试上传
curl -X PUT -u admin:admin123 \
  --data-binary @test.txt \
  http://localhost:8081/test.txt

# 检查指标
curl http://localhost:8080/metrics | grep upload_sessions_total
```

### 指标查询错误

**问题**: 面板显示 "Error: invalid expression"

**原因**: Prometheus 版本不支持某些查询语法

**解决**:
- 确保 Prometheus 版本 >= 2.30（支持 `histogram_quantile`）
- 检查 Prometheus 日志：`journalctl -u prometheus -f`

---

## 最佳实践

### 1. 监控仪表板使用

- **日常监控**: 保持 Dashboard 在后台打开，关注异常峰值
- **性能调优**: 对比调整配置前后的指标变化
- **容量规划**: 观察长期趋势，预测资源需求

### 2. 数据分析技巧

- **时间对比**: 使用 Grafana 的 "Compare to yesterday/last week" 功能
- **分位数分析**: 关注 P99 而非平均值，避免长尾问题被掩盖
- **相关性分析**: 同时观察内存、吞吐量、延迟的变化关系

### 3. 告警策略

- **分级响应**: Warning 级别可邮件通知，Critical 级别需立即处理
- **避免告警风暴**: 设置合理的 `for` 持续时间（如 5m）
- **定期复查**: 每月检查告警规则的有效性，调整阈值

---

## 扩展资源

### 相关文档

- [性能调优指南](webdav-performance-tuning.md)
- [性能测试指南](webdav-performance-testing.md)
- [WebDAV 大文件上传指南](webdav-large-file-upload-guide.md)
- [API 文档](api-guide.md)

### 官方资源

- **Prometheus 文档**: https://prometheus.io/docs/
- **Grafana 文档**: https://grafana.com/docs/
- **PromQL 教程**: https://prometheus.io/docs/prometheus/latest/querying/basics/

### 社区支持

如遇到问题，可以：
- 查看 [故障排查文档](troubleshooting.md)
- 提交 GitHub Issue
- 参考 [部署指南](deployment.md)

---

**最后更新**: 2025-11-28
**适用版本**: Silent-NAS v0.7.1+
