# 多节点部署与联调（gRPC 节点同步）

最后更新：2025-10-21

## 目标
- 在 2-3 个节点间实现文件与状态的自动同步
- 通过 gRPC 节点服务进行节点注册、心跳与文件内容传输

## 前置条件
- 各节点网络可直连（gRPC 端口，默认 `50051`）
- 时钟大致一致（建议开启 NTP）
- 配置文件可分别设置端口与种子节点

## 关键配置

在 `config.toml` 中新增的节点与同步配置：

```
[node]
# 是否启用节点功能
enable = true
# 种子节点列表（host:grpc_port）
seed_nodes = ["192.168.1.10:50051", "192.168.1.11:50051"]
# 心跳与超时（秒）
heartbeat_interval = 10
node_timeout = 30

[sync]
# 自动同步与节奏（秒）
auto_sync = true
sync_interval = 60
# 每次同步的文件数量与重试
max_files_per_sync = 100
max_retries = 3
```

环境变量覆盖（可选）：
- `NODE_ENABLE=true|false`
- `NODE_SEEDS=host1:50051,host2:50051`
- `NODE_HEARTBEAT=10`，`NODE_TIMEOUT=30`
- `SYNC_AUTO=true|false`，`SYNC_INTERVAL=60`
- `SYNC_MAX_FILES=100`，`SYNC_MAX_RETRIES=3`

## 步骤

1) 准备节点 A（种子）
- `server.grpc_port = 50051`
- `[node] enable=true, seed_nodes=[]`

2) 准备节点 B（加入 A）
- `server.grpc_port = 50052`（避免端口冲突）
- `[node] enable=true, seed_nodes=["<A的IP或主机>:50051"]`

3) 启动各节点
- 观察日志中 “gRPC 服务器启动” 与 “成功注册/连接到种子节点”
- 若失败，检查端口、防火墙与地址可达性

4) 验证同步
- 在节点 A 上传或修改一个文件
- 在 5s 内节点 B 应收到变更并持久化到存储
- 并发修改时，采用 LWW（较新 modified_at 获胜），冲突会记录在服务端日志

## 管理 API 触发同步

当自动同步未在期望时间内收敛时，可通过管理员 API 显式触发：

- 触发从本节点推送到目标节点（push）
  - `POST /api/admin/sync/push`
  - 请求体：
    ```json
    { "target": "<目标grpc_host:port>", "file_ids": ["可选: 指定文件ID数组"] }
    ```
  - 示例：
    ```bash
    curl -H 'Content-Type: application/json' \
      -d '{"target":"127.0.0.1:50052"}' \
      http://127.0.0.1:8080/api/admin/sync/push
    ```

- 触发从源节点推送到本节点（request，对端执行 push）
  - `POST /api/admin/sync/request`
  - 请求体：
    ```json
    { "source": "<源grpc_host:port>", "file_ids": ["必填: 文件ID数组"] }
    ```
  - 示例：
    ```bash
    curl -H 'Content-Type: application/json' \
      -d '{"source":"127.0.0.1:50051","file_ids":["01JE..."]}' \
      http://127.0.0.1:8080/api/admin/sync/request
    ```

注意：若开启认证，这两个接口需要管理员权限；未开启认证时默认开放用于内网联调。

## 脚本冒烟测试

提供一个简易的多节点冒烟脚本：

```bash
./scripts/smoke-multi-node.sh
```

- 启动两个节点并通过 WebDAV 写入文件到节点1

提示：若为单节点部署或未连接 NATS，本脚本与多节点同步配置不生效。启用多节点需：
- NATS 连接可用（`nats://...`）
- `[node].enable = true`
- 按需配置 `[sync]`（超时、退避、重试等）
- 第一阶段等待自动同步；若超时自动回退调用 `admin/sync/push`
- 结束时自动清理后台进程与临时文件

## 故障排查
- 种子不可达：确认 `seed_nodes` 是否为 `host:grpc_port` 形式，确认目标端口开放
- 不收敛：检查 `[sync]` 的 `auto_sync`、`sync_interval` 与日志中是否出现错误
- 大文件慢：可调大 `storage.chunk_size`（如 8MB+），或提升网络带宽
 - 节点被误判为离线：确认时间同步（NTP）正常；日志中可见对外心跳发送与入站心跳校验

## 安全建议
- 生产中建议放置在可信网络或内网，gRPC 放行仅限集群内地址
- 后续版本将引入节点认证与传输加密增强
