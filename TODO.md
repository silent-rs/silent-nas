# Silent-NAS 开发任务清单

当前版本: v0.6.0-dev

产品路线图: [ROADMAP.md](ROADMAP.md) | 架构文档: [README.md](README.md)

---

## 当前状态

- 已完成: 多协议支持、文件管理、CRDT 同步、监控审计、认证 Phase 1
- 技术指标: 176 测试(100%通过) | 覆盖率 86.38% | API 60+

---

## P0 当前迭代目标（v0.6.0 核心）

1) 分布式文件同步（gRPC 跨节点）
- 已完成：
  - gRPC 服务端 NodeSyncService（register_node/heartbeat/list/sync_file_state/request_file_sync/transfer/stream）
  - gRPC 客户端 NodeSyncClient（注册、心跳、请求同步、状态同步、传输/流式）
  - 文件状态同步与冲突检测（VectorClock + LWW 兜底）
  - 同步协调器 NodeSyncCoordinator（sync_to_node/request_files_from_node/auto_sync）
  - 管理路由与查询：`/admin/sync/*`、`/sync/*`
- 待完善：
 - 重试与回退：请求/传输的超时、指数退避、错误分级
  - 一致性与校验：端到端哈希校验、失败补偿重拉
  - 自动同步稳定性：参数可配（并发/批量/间隔）、观测指标补全
  - 端到端演练：3 节点拓扑压测与延迟/冲突指标采集（< 5s）
  - 文档同步：运行参数与调试指引

  - 进展：
    - 已为 gRPC 客户端接入连接/请求超时与重试
    - 服务端流式上传增加分块 MD5 校验，失败立即返回

2) WebDAV 协议完善
- 已有：OPTIONS / PROPFIND / HEAD / GET / PUT / DELETE / MKCOL / MOVE / COPY
- 待做：
  - 锁管理：LOCK / UNLOCK（独占/共享、超时与续约、锁令牌）
  - 自定义属性：PROPPATCH（扩展属性持久化与校验）
  - 版本控制：DeltaV 最小闭环（创建/查询/回滚）
  - 互通验证：Cyberduck / Nextcloud 用例通过

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

---

## 开发规范

- Git 分支: `main` | `feat/*` | `fix/*`
- 提交信息: `feat:` `fix:` `docs:` `refactor:` `test:`（遵循项目提交规范）
- 测试标准: 单元 > 80% | 关键路径 > 90%
- DoD: 代码 + 测试 + 审查 + CI + 文档

---

## 备注

- 本清单已与最新 ROADMAP 同步，已移出企业级相关内容至“后续拓展（Enterprise）”。
