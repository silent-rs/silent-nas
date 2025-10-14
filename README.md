# Silent-NAS

## 项目概述
Silent-NAS 是 Silent Odyssey 第六阶段的实验项目，旨在构建一个基于 Silent 框架的轻量级 NAS（网络附加存储）服务器。该项目聚焦于高性能文件传输、事件推送、一致性管理和多协议集成，全面验证 Silent 在现代分布式存储场景下的能力。
本阶段是连接 QUIC/WebTransport（高效传输层）与 CRDT（数据一致性层）的综合实验平台。

## 功能与协议范围
- ✅ 基于 QUIC/WebTransport 的高速文件传输
- ✅ 基于 gRPC 的文件控制与元数据接口
- ✅ 使用 NATS 进行文件变更事件推送
- 🚧 基于 CRDT 的多客户端文件同步与冲突合并
- 🚧 用户鉴权与访问控制
- ❌ 分布式分片存储与副本恢复（后续阶段实现）

## 架构设计
建议的项目目录结构如下：
```
src/
├── main.rs          # 启动入口
├── storage.rs       # 文件系统与分块管理
├── transfer.rs      # QUIC/WebTransport 文件传输逻辑
├── rpc.rs           # gRPC 控制接口
├── notify.rs        # NATS 文件事件推送
├── sync.rs          # CRDT 文件同步逻辑
└── auth.rs          # 用户鉴权与访问控制
```

## 使用示例
运行服务器：
```bash
cargo run
```
上传或下载文件（示例）：
```bash
curl -F "file=@example.txt" http://127.0.0.1:8080/upload
curl -O http://127.0.0.1:8080/download/example.txt
```

## 测试与验证
- 启动多个 Silent-NAS 实例，验证跨节点文件同步：
  ```bash
  cargo run -- --port 8080
  cargo run -- --port 8081
  ```
  通过 gRPC 接口或 NATS 消息触发文件更新并观察状态收敛。
- 性能测试：使用 QUIC/WebTransport 测量文件传输吞吐量与延迟。
- 一致性验证：结合 Silent-CRDT 进行多节点文件版本冲突与自动合并测试。

## 未来规划
- 支持多节点分布式文件系统（DFS）
- 引入版本控制与文件锁机制
- 增加 Web 前端控制台与可视化监控界面
- 实现 ACL 与多用户隔离策略

## 关联项目
- [silent-quic](https://github.com/silent-rs/silent-quic) — 传输层支持
- [silent-crdt](https://github.com/silent-rs/silent-crdt) — 一致性与冲突合并
- [silent-nats](https://github.com/silent-rs/silent-nats) — 事件推送与订阅机制
