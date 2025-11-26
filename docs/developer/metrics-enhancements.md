# 指标增强说明（v0.6.0-dev）

本版本增强了分布式同步相关的监控指标，便于排障与容量观测。

- 新增指标
  - `sync_retries_total{stage}`：同步重试次数，stage=transfer|verify|other
  - `sync_fail_queue_length`：失败补偿队列当前长度
  - 已有指标继续保留：
    - `sync_operations_total{type,status}`
    - `sync_bytes_transferred_total{type}`
    - `sync_stage_duration_seconds_bucket`（直方图）

- 分位数（P50/P90/P95/Max）
  - 使用 Prometheus 的 `histogram_quantile()` 针对 `sync_stage_duration_seconds` 计算：
    - P50: `histogram_quantile(0.5, sum(rate(sync_stage_duration_seconds_bucket[5m])) by (le,stage,result))`
    - P90: `histogram_quantile(0.9, sum(rate(sync_stage_duration_seconds_bucket[5m])) by (le,stage,result))`
    - P95: `histogram_quantile(0.95, sum(rate(sync_stage_duration_seconds_bucket[5m])) by (le,stage,result))`
    - Max: 参考 `max_over_time(sync_stage_duration_seconds_sum[5m]) / max_over_time(sync_stage_duration_seconds_count[5m])` 或在日志中对齐

- /metrics 端点
  - 访问路径：`/api/metrics`
  - 在未启用认证时为公开端点；启用认证时需携带认证信息。

- 失败补偿队列持久化
  - 路径：`<root>/.sync/fail_queue.json`
  - 字段：`id`、`target_node_id`、`file_id`、`attempt`、`next_at`、`created_at`、`last_error`
  - 清理策略：基于 TTL（`[sync].fail_task_ttl_secs`）与容量（`[sync].fail_queue_max`）

以上指标可结合现有的 tracing span（`state_sync`、`sync_transfer`、`sync_verify`）共同定位问题。
