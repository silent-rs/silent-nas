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

echo "[0/15] OPTIONS 能力探测 (DAV/ALLOW)"
curl -sS -D "$HEADERS_FILE" -o /dev/null --fail -X OPTIONS "$BASE_URL/" || {
  echo "OPTIONS 失败" >&2; exit 1;
}
DAV_CAP=$(grep -i "^dav:" "$HEADERS_FILE" | awk -F': ' '{print tolower($2)}' | tr -d '\r\n ')
echo "DAV: $DAV_CAP"
if [[ "$DAV_CAP" != *"1,2,ordered-collections"* && "$DAV_CAP" != *"1,2,ordered-collections"* ]]; then
  echo "DAV 能力不包含 1,2,ordered-collections" >&2; exit 1
fi

echo "[1/15] PUT 文件: $SRC_PATH"
echo "hello-webdav" > "$BODY_FILE"
curl -sS -X PUT --fail \
  --data-binary @"$BODY_FILE" \
  "$BASE_URL$SRC_PATH" >/dev/null

echo "[2/15] PROPFIND Depth:1"
curl -sS -X PROPFIND --fail \
  -H "Depth: 1" \
  "$BASE_URL/interop" >/dev/null

echo "[3/15] MKCOL 创建目录"
DIR_PATH="/interop/dir_${RUN_ID}"
curl -sS --fail -X MKCOL "$BASE_URL$DIR_PATH" >/dev/null

echo "[4/15] PROPFIND Depth:0 (目录)"
curl -sS --fail -X PROPFIND -H "Depth: 0" "$BASE_URL$DIR_PATH" >/dev/null

echo "[5/15] PROPFIND Depth: infinity"
curl -sS --fail -X PROPFIND -H "Depth: infinity" "$BASE_URL/interop" >/dev/null

echo "[6/15] LOCK 独占锁"
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

echo "[7/15] PROPPATCH 设置自定义属性"
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

echo "[8/15] HEAD 获取ETag"
curl -sS --fail -D "$HEADERS_FILE" -o /dev/null -X HEAD "$BASE_URL$SRC_PATH"
ETAG=$(grep -i '^etag:' "$HEADERS_FILE" | awk -F': ' '{print $2}' | tr -d '\r\n')
echo "ETag: $ETAG"
if [[ -z "$ETAG" ]]; then echo "未获取到ETag" >&2; exit 1; fi

echo "[9/15] If-None-Match 命中304 (GET)"
CODE=$(curl -s -o /dev/null -w "%{http_code}" -H "If-None-Match: $ETAG" "$BASE_URL$SRC_PATH")
if [[ "$CODE" != "304" ]]; then echo "GET If-None-Match 预期304 实得$CODE" >&2; exit 1; fi

echo "[10/15] If-None-Match 命中304 (HEAD)"
CODE=$(curl -s -o /dev/null -w "%{http_code}" -I -H "If-None-Match: $ETAG" "$BASE_URL$SRC_PATH")
if [[ "$CODE" != "304" ]]; then echo "HEAD If-None-Match 预期304 实得$CODE" >&2; exit 1; fi

echo "[11/15] GET 下载校验"
curl -sS --fail "$BASE_URL$SRC_PATH" >/dev/null

echo "[12/15] VERSION-CONTROL 标记版本控制"
curl -sS --fail -X VERSION-CONTROL "$BASE_URL$SRC_PATH" >/dev/null

echo "[13/15] REPORT 版本列表"
curl -sS --fail -X REPORT "$BASE_URL$SRC_PATH" >/dev/null

echo "[14/15] REPORT sync-collection (Depth: infinity)"
SYNC_BODY='<?xml version="1.0" encoding="utf-8"?>
<D:sync-collection xmlns:D="DAV:"></D:sync-collection>'
CODE=$(curl -sS -o "$TMP_DIR/sync.xml" -w "%{http_code}" --fail -X REPORT -H "Content-Type: application/xml" -H "Depth: infinity" --data "$SYNC_BODY" "$BASE_URL/interop")
grep -q "sync-token" "$TMP_DIR/sync.xml" || { echo "sync-collection 未返回sync-token" >&2; exit 1; }
if [[ "$CODE" != "207" ]]; then echo "sync-collection 预期207 实得$CODE" >&2; exit 1; fi

echo "[15/15] MOVE 重命名"
curl -sS --fail -X MOVE \
  -H "Destination: $BASE_URL$DST_PATH" \
  -H "If: (<$LOCK_TOKEN>)" \
  "$BASE_URL$SRC_PATH" >/dev/null

echo "COPY 复制到新路径 (校验后删除)"
COPY_PATH="/interop/a_${RUN_ID}_copy.txt"
curl -sS --fail -X COPY -H "Destination: $BASE_URL$COPY_PATH" "$BASE_URL$DST_PATH" >/dev/null
curl -sS --fail -X DELETE "$BASE_URL$COPY_PATH" >/dev/null

echo "PROPPATCH 移除属性"
PROP_BODY_REMOVE='<?xml version="1.0" encoding="utf-8"?>
<D:propertyupdate xmlns:D="DAV:">
  <D:remove>
    <D:prop>
      <Z:category xmlns:Z="urn:x-example"/>
    </D:prop>
  </D:remove>
</D:propertyupdate>'
curl -sS --fail -X PROPPATCH -H "Content-Type: application/xml" -H "If: (<$LOCK_TOKEN>)" --data "$PROP_BODY_REMOVE" "$BASE_URL$DST_PATH" >/dev/null

echo "UNLOCK 解锁"
curl -sS --fail -X UNLOCK \
  -H "Lock-Token: <$LOCK_TOKEN>" \
  "$BASE_URL$DST_PATH" >/dev/null

echo "清理（删除文件与目录）"
curl -sS --fail -X DELETE "$BASE_URL$DST_PATH" >/dev/null || true
curl -sS --fail -X DELETE "$BASE_URL$DIR_PATH" >/dev/null || true

echo "OK: WebDAV 全量接口互通测试通过"
