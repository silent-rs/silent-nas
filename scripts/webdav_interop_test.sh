#!/usr/bin/env bash
set -euo pipefail

# WebDAV 互通验证脚本（单脚本、零配置）
# 依赖：curl；若服务未就绪，尝试自动启动（cargo run）并等待

ROOT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")"/.. && pwd)
CONFIG_FILE="$ROOT_DIR/config.toml"

detect_base_url() {
  local host="127.0.0.1" port="8081"
  if [[ -f "$CONFIG_FILE" ]]; then
    local cfg_host cfg_port
    cfg_host=$(awk -F'=' '/^host[[:space:]]*=/{gsub(/["[:space:]]/, "", $2); print $2}' "$CONFIG_FILE" | tail -n1 || true)
    cfg_port=$(awk -F'=' '/webdav_port[[:space:]]*=/{gsub(/["[:space:]]/, "", $2); print $2}' "$CONFIG_FILE" | tail -n1 || true)
    [[ -n "$cfg_host" ]] && host="$cfg_host"
    [[ -n "$cfg_port" ]] && port="$cfg_port"
  fi
  echo "http://$host:$port/webdav"
}

BASE_URL=$(detect_base_url)
RUN_ID=$(date +%s)
SRC_PATH="/interop/a_${RUN_ID}.txt"
DST_PATH="/interop/a_${RUN_ID}_renamed.txt"
TMP_DIR=$(mktemp -d)
HEADERS_FILE="$TMP_DIR/headers.txt"
BODY_FILE="$TMP_DIR/body.bin"
SERVER_PID=""

cleanup() {
  rm -rf "$TMP_DIR" || true
  if [[ -n "$SERVER_PID" ]] && ps -p "$SERVER_PID" >/dev/null 2>&1; then
    echo "停止本地 WebDAV 服务 (pid=$SERVER_PID)"
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

wait_ready() {
  local url="$1" max=60 i=0
  while (( i < max )); do
    if curl -s -o /dev/null -X OPTIONS "$url/"; then
      return 0
    fi
    sleep 0.5; i=$((i+1))
  done
  return 1
}

supports_method() {
  local url="$1" method="$2"
  code=$(curl -s -o /dev/null -w "%{http_code}" -X "$method" "$url/") || code=000
  # 405 表示方法不被当前服务支持
  [[ "$code" != "405" && "$code" != "000" ]]
}

echo "检测 WebDAV 服务: $BASE_URL"
# 总是以可控实例运行，避免与外部旧进程冲突
if pgrep -f "silent-nas" >/dev/null 2>&1; then
  echo "检测到已有 silent-nas 进程，先行停止..."
  pkill -f "silent-nas" || true
  sleep 1
fi
echo "启动本地 WebDAV 服务..."
(cd "$ROOT_DIR" && RUST_LOG=info cargo run >/dev/null 2>&1 & echo $! > "$TMP_DIR/pid") || true
SERVER_PID=$(cat "$TMP_DIR/pid" 2>/dev/null || true)
if [[ -z "$SERVER_PID" ]]; then
  echo "无法启动服务，请手动运行：cargo run" >&2
  exit 1
fi
echo "已启动本地服务 (pid=$SERVER_PID)，等待就绪..."
if ! wait_ready "$BASE_URL"; then
  echo "服务未在预期时间内就绪：$BASE_URL" >&2
  exit 1
fi

echo "[1/9] PUT 文件: $SRC_PATH"
echo "hello-webdav" > "$BODY_FILE"
curl -sS -X PUT --fail \
  --data-binary @"$BODY_FILE" \
  "$BASE_URL$SRC_PATH" >/dev/null

echo "[2/9] PROPFIND Depth:1"
curl -sS -X PROPFIND --fail \
  -H "Depth: 1" \
  "$BASE_URL/interop" >/dev/null

echo "[3/9] LOCK 独占锁"
LOCK_BODY='<?xml version="1.0" encoding="utf-8"?>
<D:lockinfo xmlns:D="DAV:">
  <D:lockscope><D:exclusive/></D:lockscope>
  <D:locktype><D:write/></D:locktype>
  <D:owner>interop-test</D:owner>
</D:lockinfo>'
curl -sS -D "$HEADERS_FILE" -o /dev/null --fail \
  -X LOCK -H "Content-Type: application/xml" \
  --data "$LOCK_BODY" "$BASE_URL$SRC_PATH"
LOCK_TOKEN=$(grep -i "^lock-token:" "$HEADERS_FILE" | awk '{print $2}' | tr -d '\r\n<>')
if [[ -z "$LOCK_TOKEN" ]]; then
  echo "未获取到 Lock-Token" >&2; exit 1
fi
echo "Lock-Token: $LOCK_TOKEN"

echo "[4/9] PROPPATCH 设置自定义属性"
PROP_BODY='<?xml version="1.0" encoding="utf-8"?>
<D:propertyupdate xmlns:D="DAV:">
  <D:set>
    <D:prop>
      <Z:category xmlns:Z="urn:x-example">interop</Z:category>
    </D:prop>
  </D:set>
</D:propertyupdate>'
curl -sS --fail -X PROPPATCH \
  -H "Content-Type: application/xml" \
  -H "If: (<$LOCK_TOKEN>)" \
  --data "$PROP_BODY" "$BASE_URL$SRC_PATH" >/dev/null

echo "[5/9] GET 下载校验"
curl -sS --fail "$BASE_URL$SRC_PATH" >/dev/null

echo "[6/9] REPORT 版本列表"
curl -sS --fail -X REPORT "$BASE_URL$SRC_PATH" >/dev/null

echo "[7/9] MOVE 重命名"
curl -sS --fail -X MOVE \
  -H "Destination: $BASE_URL$DST_PATH" \
  -H "If: (<$LOCK_TOKEN>)" \
  "$BASE_URL$SRC_PATH" >/dev/null

echo "[8/9] UNLOCK 解锁"
curl -sS --fail -X UNLOCK \
  -H "Lock-Token: <$LOCK_TOKEN>" \
  "$BASE_URL$DST_PATH" >/dev/null

echo "[9/9] 清理"
curl -sS --fail -X DELETE "$BASE_URL$DST_PATH" >/dev/null || true

echo "OK: WebDAV 互通基础流程通过"
