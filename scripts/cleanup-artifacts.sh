#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
DOCKER_DIR="$ROOT_DIR/docker"

echo "[cleanup] 开始清理临时产物与容器..."

# 1) 本地进程冒烟日志与临时目录
rm -f /tmp/silent-nas-node1.log /tmp/silent-nas-node2.log 2>/dev/null || true
rm -rf "$ROOT_DIR/scripts/.smoke" 2>/dev/null || true

# 2) 三节点压测工作目录（脚本已清理，这里兜底）
rm -rf "$ROOT_DIR/scripts/.bench3nodes_docker" 2>/dev/null || true

# 3) 外层保存的摘要目录（如存在且无需保留，可删除）
if [[ -d "$ROOT_DIR/bench_runs" ]]; then
  echo "[cleanup] 检测到 bench_runs，如无需保留可删除。"
  read -r -p "是否删除 bench_runs? [y/N] " ans || true
  if [[ "${ans:-}" =~ ^[Yy]$ ]]; then
    rm -rf "$ROOT_DIR/bench_runs" || true
    echo "[cleanup] 已删除 bench_runs"
  else
    echo "[cleanup] 保留 bench_runs"
  fi
fi

# 4) 停止残留的临时 NATS 容器
for name in nas-nats-test smoke-nats; do
  if docker ps -a --format '{{.Names}}' | grep -q "^${name}$"; then
    docker rm -f "$name" >/dev/null 2>&1 || true
    echo "[cleanup] 已移除容器: $name"
  fi
done

# 5) 关闭 docker-compose 集群并移除卷/网络
if command -v docker >/dev/null 2>&1; then
  if command -v docker-compose >/dev/null 2>&1; then
    (cd "$DOCKER_DIR" && docker-compose down -v --remove-orphans) || true
  else
    (cd "$DOCKER_DIR" && docker compose down -v --remove-orphans) || true
  fi
fi

echo "[cleanup] 清理完成"
