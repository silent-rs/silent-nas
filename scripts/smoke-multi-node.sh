#!/usr/bin/env bash
set -euo pipefail

# 简单多节点冒烟测试：启动2个节点，经由WebDAV写入文件，验证另一节点在短时间内收敛

ROOT_DIR=$(cd "$(dirname "$0")/.." && pwd)
# 获取可执行文件路径
TARGET_DIR=$(cargo metadata --format-version 1 2>/dev/null | python3 -c "import sys, json; print(json.load(sys.stdin)['target_directory'])" 2>/dev/null || echo "$ROOT_DIR/target")
TARGET_BIN="$TARGET_DIR/debug/silent-nas"
WORK_DIR="$ROOT_DIR/scripts/.smoke"

get_free_tcp_port() {
  python3 - <<'PY'
import socket
s=socket.socket()
s.bind(('127.0.0.1',0))
port=s.getsockname()[1]
s.close()
print(port)
PY
}

get_free_udp_port() {
  python3 - <<'PY'
import socket
s=socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.bind(('127.0.0.1',0))
port=s.getsockname()[1]
s.close()
print(port)
PY
}

HTTP1=$(get_free_tcp_port)
GRPC1=$(get_free_tcp_port)
WEBDAV1=$(get_free_tcp_port)
S31=$(get_free_tcp_port)
QUIC1=$(get_free_udp_port)

HTTP2=$(get_free_tcp_port)
GRPC2=$(get_free_tcp_port)
WEBDAV2=$(get_free_tcp_port)
S32=$(get_free_tcp_port)
QUIC2=$(get_free_udp_port)

cleanup() {
  echo "[smoke] 清理进程与临时文件..."
  # 停止后台进程
  if [[ -n "${PID1:-}" ]]; then
    kill $PID1 2>/dev/null || true
    sleep 0.1 || true
    kill -9 $PID1 2>/dev/null || true
  fi
  if [[ -n "${PID2:-}" ]]; then
    kill $PID2 2>/dev/null || true
    sleep 0.1 || true
    kill -9 $PID2 2>/dev/null || true
  fi
  # 删除日志与临时目录
  rm -f /tmp/silent-nas-node1.log /tmp/silent-nas-node2.log 2>/dev/null || true
  rm -rf "$WORK_DIR" 2>/dev/null || true
  sleep 0.2 || true
}
trap cleanup EXIT

echo "[smoke] 构建二进制..."
cargo build -q

if [[ ! -x "$TARGET_BIN" ]]; then
  echo "[smoke] 可执行文件不存在: $TARGET_BIN"
  exit 1
fi
echo "[smoke] 使用可执行文件: $TARGET_BIN"

rm -rf "$WORK_DIR" && mkdir -p "$WORK_DIR/node1" "$WORK_DIR/node2"

cat > "$WORK_DIR/node1/config.toml" <<EOF
[server]
host = "127.0.0.1"
http_port = $HTTP1
grpc_port = $GRPC1
quic_port = $QUIC1
webdav_port = $WEBDAV1
s3_port = $S31

[storage]
root_path = "$WORK_DIR/node1/storage"
chunk_size = 1048576

[nats]
url = "nats://127.0.0.1:4222"
topic_prefix = "smoke.nas"

[s3]
access_key = "minioadmin"
secret_key = "minioadmin"
enable_auth = false

[auth]
enable = false
db_path = "$WORK_DIR/node1/auth.db"
jwt_secret = "smoke"
access_token_exp = 3600
refresh_token_exp = 604800

[node]
enable = true
seed_nodes = []
heartbeat_interval = 2
node_timeout = 120

[sync]
auto_sync = true
sync_interval = 2
max_files_per_sync = 100
max_retries = 3
EOF

cat > "$WORK_DIR/node2/config.toml" <<EOF
[server]
host = "127.0.0.1"
http_port = $HTTP2
grpc_port = $GRPC2
quic_port = $QUIC2
webdav_port = $WEBDAV2
s3_port = $S32

[storage]
root_path = "$WORK_DIR/node2/storage"
chunk_size = 1048576

[nats]
url = "nats://127.0.0.1:4222"
topic_prefix = "smoke.nas"

[s3]
access_key = "minioadmin"
secret_key = "minioadmin"
enable_auth = false

[auth]
enable = false
db_path = "$WORK_DIR/node2/auth.db"
jwt_secret = "smoke"
access_token_exp = 3600
refresh_token_exp = 604800

[node]
enable = true
seed_nodes = ["127.0.0.1:$GRPC1"]
heartbeat_interval = 2
node_timeout = 120

[sync]
auto_sync = true
sync_interval = 2
max_files_per_sync = 100
max_retries = 3
EOF

echo "[smoke] 启动节点1..."
(cd "$WORK_DIR/node1" && RUST_LOG=info ADVERTISE_HOST=127.0.0.1 exec "$TARGET_BIN") >/tmp/silent-nas-node1.log 2>&1 &
PID1=$!
disown

echo "[smoke] 等待节点1 HTTP就绪..."
for i in {1..100}; do
  if curl -fsS "http://127.0.0.1:$HTTP1/api/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.2
  if [[ $i -eq 100 ]]; then
    echo "[smoke] 等待节点1 HTTP就绪超时，节点1日志如下："
    tail -n 200 /tmp/silent-nas-node1.log || true
    exit 1
  fi
done

echo "[smoke] 启动节点2..."
(cd "$WORK_DIR/node2" && RUST_LOG=info ADVERTISE_HOST=127.0.0.1 exec "$TARGET_BIN") >/tmp/silent-nas-node2.log 2>&1 &
PID2=$!
disown

echo "[smoke] 等待节点2 HTTP就绪..."
for i in {1..100}; do
  if curl -fsS "http://127.0.0.1:$HTTP2/api/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.2
  if [[ $i -eq 100 ]]; then
    echo "[smoke] 等待节点2 HTTP就绪超时，节点2日志如下："
    tail -n 200 /tmp/silent-nas-node2.log || true
    exit 1
  fi
done

echo "[smoke] 通过WebDAV向节点1写入文件..."
echo "hello from smoke" > "$WORK_DIR/smoke.txt"
# WebDAV 实际挂载在根路径，直接写入根路径下的目标
curl -fsS -X PUT --data-binary @"$WORK_DIR/smoke.txt" "http://127.0.0.1:$WEBDAV1/smoke.txt"

echo "[smoke] 轮询节点2文件列表收敛(自动同步)..."
ok=false
for i in {1..50}; do
  COUNT=$(curl -fsS "http://127.0.0.1:$HTTP2/api/files" 2>/dev/null \
    | grep -o '"size":' \
    | wc -l | tr -d ' ')
  COUNT=${COUNT:-0}
  if [[ "$COUNT" -ge 1 ]]; then
    ok=true
    break
  fi
  sleep 0.3
done

if [[ "$ok" != "true" ]]; then
  echo "[smoke] 自动同步超时，尝试调用管理员API触发push..."
  curl -fsS -H 'Content-Type: application/json' \
    -d "{\"target\":\"127.0.0.1:$GRPC2\"}" \
    "http://127.0.0.1:$HTTP1/api/admin/sync/push" || true

  echo "[smoke] 触发push后再次轮询收敛..."
  for i in {1..50}; do
    COUNT=$(curl -fsS "http://127.0.0.1:$HTTP2/api/files" 2>/dev/null \
      | grep -o '"size":' \
      | wc -l | tr -d ' ')
    COUNT=${COUNT:-0}
    if [[ "$COUNT" -ge 1 ]]; then
      ok=true
      break
    fi
    sleep 0.3
  done
fi

if [[ "$ok" == "true" ]]; then
  echo "[smoke] ✅ 同步成功 (节点2检测到 >=1 个文件)"
else
  echo "[smoke] ❌ 同步失败"
  echo "--- 节点1日志 ---"; tail -n 120 /tmp/silent-nas-node1.log || true
  echo "--- 节点2日志 ---"; tail -n 120 /tmp/silent-nas-node2.log || true
  exit 1
fi
exit 0
