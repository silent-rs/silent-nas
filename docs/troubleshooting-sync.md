# 分布式同步排障与故障注入

本指南帮助你快速定位分布式同步问题，并在开发/测试环境中注入可控故障以验证回退与补偿链路。

## 观测与指标

- 指标端点：`/metrics`（Prometheus 格式）
- 关键指标：
  - `sync_operations_total{type,status}`：全量/增量同步成功/错误计数与字节数
  - `sync_stage_duration_seconds{stage,result}`：阶段时延（transfer/verify，success/error）
- 日志：
  - gRPC 重试日志带错误分级（unavailable/deadline_exceeded 等）
  - tracing span：`grpc_connect`、`state_sync`、`sync_transfer`、`sync_verify`

## 超时与退避

- HTTP 拉取：`[sync] http_connect_timeout/http_request_timeout`
- gRPC：`[sync] grpc_connect_timeout/grpc_request_timeout`
- 退避与重试：
  - gRPC 客户端带指数退避 + 抖动，最大退避 60s（默认）
  - 重试总预算：`ClientConfig.retry_budget_secs`（默认 120s），超出停止重试

## 失败补偿队列

- 持久化路径：`<root>/.sync/fail_queue.json`
- 清理策略：
  - TTL：`[sync] fail_task_ttl_secs`（默认 86400）
  - 容量：`[sync] fail_queue_max`（默认 1000）

## 故障注入（开发/测试）

- 配置项：
  - `fault_transfer_error_rate`：传输失败概率（0.0-1.0）
  - `fault_verify_error_rate`：校验失败概率（0.0-1.0）
  - `fault_delay_ms`：附加延迟（毫秒）
- 生效范围：NodeSyncCoordinator 中的传输/校验阶段
- 建议：仅在测试环境启用，避免影响生产

## 常见问题

1) “端到端校验失败”
- 原因：目标节点落盘不一致或读取失败
- 排查：
  - 查看 `sync_stage_duration_seconds{stage="verify",result="error"}`
  - 检查网络波动/磁盘状态
  - 观察失败补偿队列是否补偿成功

2) “重试仍失败/超时”
- 原因：网络不可达、目标节点异常
- 排查：
  - 核对 gRPC/HTTP 超时配置是否过小
  - 查看 gRPC 重试日志的错误分级与次数
  - 增加重试预算后重试

3) “队列增长过快/容量占满”
- 原因：长时间故障未恢复
- 处理：
  - 调整 `fail_task_ttl_secs`/`fail_queue_max`
  - 手工清理 `<root>/.sync/fail_queue.json` 中陈旧条目（停机维护）

## 验收回归建议

- 使用 `scripts/sync_3nodes_benchmark_docker.sh` 进行三节点端到端压测
- 验收目标：P95 < 8s，成功率 > 99.9%
- 建议在 CI 中周期性运行，捕获回归
