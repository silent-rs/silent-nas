# 测试脚本总览

本文档汇总 `scripts/` 目录内的测试与验证脚本用途、使用方法与注意事项，便于快速选用合适的脚本完成联调与验证。

## 三节点同步压测（Docker）

- 脚本：`scripts/sync_3nodes_benchmark_docker.sh`
- 用途：在 3 节点拓扑下进行端到端文件同步性能压测，统计 P50/P90/P95/Max 延迟与丢失率。
- 场景：CI 压测、性能回归、环境一致性验证。
- 依赖：Docker、docker-compose、curl、python3。
- 快速开始：
  ```bash
  ./scripts/sync_3nodes_benchmark_docker.sh
  # 自定义：
  N_FILES=20 CONCURRENCY=5 ./scripts/sync_3nodes_benchmark_docker.sh
  ```
- 重要说明：已统一为容器化版本，非容器版本已移除。

## 多节点冒烟测试（本地进程）

- 脚本：`scripts/smoke-multi-node.sh`
- 用途：快速起本地 2 节点进行基础连通性与同步冒烟验证（启动/写入/读取/收敛）。
- 场景：开发联调、功能回归的最小验证。
- 依赖：已构建的可执行文件（`cargo build`）、python3。
- 快速开始：
  ```bash
  ./scripts/smoke-multi-node.sh
  ```

## WebDAV 协议互通测试（本地进程）

- 脚本：`scripts/webdav_interop_test.sh`
- 用途：验证 WebDAV 基本操作（MKCOL、PUT、GET、HEAD、DELETE、LOCK/UNLOCK、REPORT 等）与条件请求行为。
- 场景：协议层互通与回归测试。
- 依赖：已构建的可执行文件（`cargo build`）、curl、python3。
- 快速开始：
  ```bash
  ./scripts/webdav_interop_test.sh
  ```

## 常见问题

- 端口占用：本地进程脚本会动态选择空闲端口；如频繁测试，请确保上次进程已退出。
- NATS 依赖：容器化压测脚本内置 NATS 服务；本地进程脚本需自行准备或通过 Docker 启动。
- 基准阈值：压测脚本默认 P95 阈值为 8000ms，可通过环境变量调整（`BENCH_TARGET_P95_MS`）。

## 相关文档

- `scripts/README_BENCHMARK.md`
- `docs/deployment-multi-node.md`
- `docker/README.md`、`docker/QUICK_START.md`
