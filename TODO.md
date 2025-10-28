# Silent-NAS 开发任务清单（仅待完成项）

当前版本: v0.6.0-dev

产品路线图: [ROADMAP.md](ROADMAP.md)

---

## P0 当前迭代目标（v0.6.0 核心）

1) 分布式文件同步（gRPC 跨节点）
- 待完成：
  - 重试与回退：
    - [已完成] 统一连接/请求/流式传输超时（HTTP/gRPC 可配）
    - [已完成] 指数退避 + 抖动策略 + 重试总预算（gRPC 客户端）
    - [已完成] 错误分级：重试日志分类（Unavailable/DeadlineExceeded 等）
  - 一致性与校验：
    - [已完成] 端到端 SHA‑256 校验（增量/全量均校验）
    - [已完成] 失败补偿重拉：失败队列 + 后台 worker（持久化/TTL/容量/退避）
    - [已完成] 指标：成功率/阶段时延（transfer/verify）、字节数上报
  - 自动同步稳定性：
    - [已完成] 批量/间隔/重试参数可配与热更新（定时重载）
    - [已完成] 同步阶段埋点：连接/状态同步/内容传输（tracing span）
  - 端到端演练：
    - [已完成] 验收阈值（调整后）：N=200，P95 < 5s，成功率 > 99.9%
  - 已完成：
    - 三节点拓扑压测脚本（Docker 版本统一）：`scripts/sync_3nodes_benchmark_docker.sh`
    - 端到端 SHA‑256 校验落地：增量/全量拉取均校验，校验通过才保存（事件监听与巡检补拉）
    - 拉取超时/重试/退避配置化：`[sync] http_connect_timeout/http_request_timeout/fetch_max_retries/fetch_base_backoff/fetch_max_backoff`
    - gRPC 重试与上限统一：NodeSyncClient.max_retries 由 `[sync].max_retries` 驱动
    - 指标埋点（初版）：全量/增量同步成功与失败计数与字节数上报
    - 单节点优化：未连接 NATS 时不启用巡检补拉任务；单节点可省略 `[sync]` 配置
  - 文档同步：
    - [已完成] 运行参数与调试指引、故障注入与排障（docs/troubleshooting-sync.md）

  - 下一步计划（按优先级推进）：
    - 已全部完成，相关能力已落地并文档化（详见 docs/metrics-enhancements.md 与 docs/troubleshooting-sync.md）

2) WebDAV 协议完善
- 状态：基本完成（详见 docs/webdav-guide.md、docs/webdav-interop.md、docs/webdav-report-extensions.md）
- 已完成：
  - 锁与并发：共享/独占锁、owner/depth/timeout；If 条件（Lock‑Token/ETag，AND/OR/Not）
  - 属性模型：xmlns 解析、结构化键 ns:{URI}#{local}；DAV: 只读；值长度限制；命名空间冲突检测；类型校验（.bool/.int）
  - 报告：sync-collection（limit/sync-token/删除差异404）、version-tree、silent:filter（mime/时间/limit/标签）、属性选择（<D:prop>）
- 保留优化项：
  - 差异记录增强：MOVE 以 from→to 标记表达（当前按删除+创建）
  - PROPFIND 支持 <D:prop> 选择（目前在 REPORT 中已支持）
  - 扩展属性前缀回显映射（根据客户端偏好回退原前缀）

---

## P1 后续（v0.7.0 候选）

- 版本存储优化：增量存储、差异算法、冷热分离、块级去重
- 搜索功能增强：全文搜索、过滤与建议、协议层搜索集成
- 性能监控完善：指标补充、基准测试套件、缓存策略优化

---

## 后续拓展（Enterprise）Backlog

- 权限与安全：路径级 ACL、用户组与组权限、用户存储配额、完整 RBAC、审计与合规、传输与静态加密
- 多租户与隔离：多租户隔离、共享链接与外部协作
- S3 企业增强：对象标签、对象 ACL、生命周期管理、预签名 URL、CORS 配置

---

## 技术债务

- 搜索索引大小控制与维护策略（与搜索增强关联）
- 错误码与日志规范统一（跨服务一致性）
- 事件监听回退链路复查（HTTP 拉取失败 -> WebDAV 回退的健壮性）
- WebDAV 路径与存储路径的边界与转义规则对齐
