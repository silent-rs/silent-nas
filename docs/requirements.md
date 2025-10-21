# Silent-NAS 需求整理（v0.6.0 分布式同步 Phase 1）

最后更新：2025-10-21

## 背景
- 依据 ROADMAP（v0.6.0）与 TODO（Week 1-2），当前最高优先级为“跨节点文件同步 gRPC 实现”。
- 目标是打通节点间的文件状态与文件内容同步闭环，满足 3 节点以内的端到端同步，延迟 < 5s，并具备并发冲突的正确处理能力与基本故障恢复。

## 范围（本次迭代）
- gRPC 通道：节点注册/心跳/列表、文件状态同步、文件请求、文件内容传输。
- 同步方向：
  - Push：本地节点向目标节点发送文件内容（统一走流式接口，规避小文件直传语义不一致问题）。
  - Pull：本地节点向目标节点请求文件（使用单次传输接口拉取小文件，或按需扩展为流式）。
- 冲突处理：
  - 基于 VectorClock 判定并发（concurrent）、happens-before 关系。
  - 使用 LWW（modified_at）在并发场景做自动化优先级决策；保留冲突列表供上层审计。
- 状态应用：
  - 将远端 FileSyncState 转换为内部 CRDT 结构，调用 `SyncManager::handle_remote_sync` 生效。

## 非目标（本次不做）
- 多副本一致性、分片与一致性哈希（属 v0.8.0）。
- 完整 ACL/RBAC（进入下一 Sprint）。

## 接口与模块
- proto：`proto/file_service.proto`（已具备 NodeSyncService 定义）。
- 服务端：`src/sync/node/service.rs`
  - `sync_file_state`：接收并合并远端状态，产出冲突清单。
  - `request_file_sync`：触发对端执行 push。
  - `transfer_file`：当前为拉取小文件（返回内容）。
  - `stream_file_content`：接收对端流式传输并保存文件。
- 客户端：`src/sync/node/client.rs`
  - `sync_file_states` / `request_file_sync` / `get_sync_status` / `transfer_file` / `stream_file_content`。
- 协调器：`src/sync/node/manager.rs`（NodeSyncCoordinator）
  - `sync_to_node`：执行 push（建议统一走流式接口）。
  - `request_files_from_node`：执行 pull。

## 待办与修正点
- 统一 push 语义：`sync_to_node` 小文件路径不再调用 `transfer_file`，改为统一 `stream_file_content` 发送，避免与服务端 `transfer_file`（拉取）语义相悖。
- 校验 `apply_remote_state` 调用链：确保所有 happens-before 与并发场景均覆盖。

## 验收标准（DoD）
- 3 节点拓扑下：
  - Push：任一节点新增/更新/删除文件，其他节点在 < 5s 内收敛。
  - Pull：任一节点可请求另一节点的 N 个文件并正确落盘。
  - 并发冲突：出现并发写入时，自动根据 LWW 选择较新版本，并返回冲突列表（可审计）。
- 构建与检查：`cargo check` / `cargo clippy` 通过。

## 风险与缓解
- 小文件直传与服务端实现语义不一致 → 统一走流式接口规避。
- 大文件分块校验一致性 → 保留 md5 分块校验字段，必要时追加端到端校验。

## 时间与排期（预计）
- Week 1-2：完成 gRPC push/pull 与冲突处理打通、基础验证。

## 里程碑产物
- gRPC 同步在 3 节点端到端联调通过，满足 DoD。
