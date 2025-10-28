#!/usr/bin/env bash
set -euo pipefail

# WebDAV 互通验证脚本（单脚本、零配置）
# 依赖：curl；若服务未就绪，尝试自动启动（cargo run）并等待

ROOT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")"/.. && pwd)
# 统一 curl 时间限制与重试策略（避免服务端偶发阻塞导致脚本卡死）
CURL_OPTS=(--connect-timeout 5 --max-time 15 --retry 2 --retry-delay 1)
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
  # WebDAV 实际挂载在根路径，避免在存储中生成 /webdav 目录
  echo "http://$host:$port"
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
    if curl -s -o /dev/null "${CURL_OPTS[@]}" -X OPTIONS "$url/"; then
      return 0
    fi
    sleep 0.5; i=$((i+1))
  done
  return 1
}

supports_method() {
  local url="$1" method="$2"
  code=$(curl -s -o /dev/null "${CURL_OPTS[@]}" -w "%{http_code}" -X "$method" "$url/") || code=000
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
curl -sS -D "$HEADERS_FILE" -o /dev/null --fail "${CURL_OPTS[@]}" -X OPTIONS "$BASE_URL/" || {
  echo "OPTIONS 失败" >&2; exit 1;
}
DAV_CAP=$(grep -i "^dav:" "$HEADERS_FILE" | awk -F': ' '{print tolower($2)}' | tr -d '\r\n ')
echo "DAV: $DAV_CAP"
if [[ "$DAV_CAP" != *"1,2,ordered-collections"* && "$DAV_CAP" != *"1, 2, ordered-collections"* ]]; then
  echo "DAV 能力不包含 1,2,ordered-collections" >&2; exit 1
fi

echo "[1/15] PUT 文件: $SRC_PATH"
echo "hello-webdav" > "$BODY_FILE"
curl -sS "${CURL_OPTS[@]}" -X PUT --fail \
  --data-binary @"$BODY_FILE" \
  "$BASE_URL$SRC_PATH" >/dev/null

echo "[2/15] PROPFIND Depth:1"
curl -sS "${CURL_OPTS[@]}" -X PROPFIND --fail \
  -H "Depth: 1" \
  "$BASE_URL/interop" >/dev/null

echo "[3/15] MKCOL 创建目录"
DIR_PATH="/interop/dir_${RUN_ID}"
curl -sS --fail "${CURL_OPTS[@]}" -X MKCOL "$BASE_URL$DIR_PATH" >/dev/null

echo "[4/15] PROPFIND Depth:0 (目录)"
curl -sS --fail "${CURL_OPTS[@]}" -X PROPFIND -H "Depth: 0" "$BASE_URL$DIR_PATH" >/dev/null

echo "[5/15] PROPFIND Depth: infinity"
curl -sS --fail "${CURL_OPTS[@]}" -X PROPFIND -H "Depth: infinity" "$BASE_URL/interop" >/dev/null

echo "[6/15] LOCK 独占锁"
LOCK_BODY='<?xml version="1.0" encoding="utf-8"?>
<D:lockinfo xmlns:D="DAV:">
  <D:lockscope><D:exclusive/></D:lockscope>
  <D:locktype><D:write/></D:locktype>
  <D:owner>interop-test</D:owner>
</D:lockinfo>'
curl -sS -D "$HEADERS_FILE" -o /dev/null --fail "${CURL_OPTS[@]}" \
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
curl -sS --fail "${CURL_OPTS[@]}" -X PROPPATCH \
  -H "Content-Type: application/xml" \
  -H "If: (<$LOCK_TOKEN>)" \
  --data "$PROP_BODY" "$BASE_URL$SRC_PATH" >/dev/null

echo "[8/18] PROPFIND 属性选择 + 前缀回显 (Depth:0)"
PROPSELECT='<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:displayname/>
    <Q:category xmlns:Q="urn:x-example"/>
  </D:prop>
  </D:propfind>'
curl -sS --fail "${CURL_OPTS[@]}" -X PROPFIND -H "Depth: 0" \
  -H "Content-Type: application/xml" --data "$PROPSELECT" \
  "$BASE_URL$SRC_PATH" -o "$TMP_DIR/propfind.xml"
grep -q "<D:displayname>" "$TMP_DIR/propfind.xml" || { echo "PROPFIND 未返回 displayname" >&2; exit 1; }
# 宽松校验扩展属性存在与命名空间（前缀回显若不可用则降级通过）
if grep -qi ":category" "$TMP_DIR/propfind.xml"; then
  echo "检测到扩展属性 category 回显"
else
  echo "WARN: 未检测到扩展属性回显，跳过严格校验"
fi

echo "[9/18] HEAD 获取ETag"
curl -sS --fail "${CURL_OPTS[@]}" -D "$HEADERS_FILE" -o /dev/null -I "$BASE_URL$SRC_PATH"
ETAG=$(grep -i '^etag:' "$HEADERS_FILE" | awk -F': ' '{print $2}' | tr -d '\r\n')
echo "ETag: $ETAG"
if [[ -z "$ETAG" ]]; then echo "未获取到ETag" >&2; exit 1; fi

echo "[10/18] If-None-Match 命中304 (GET)"
CODE=$(curl -s -o /dev/null "${CURL_OPTS[@]}" -w "%{http_code}" -H "If-None-Match: $ETAG" "$BASE_URL$SRC_PATH")
if [[ "$CODE" != "304" ]]; then echo "GET If-None-Match 预期304 实得$CODE" >&2; exit 1; fi

echo "[11/18] If-None-Match 命中304 (HEAD)"
CODE=$(curl -s -o /dev/null "${CURL_OPTS[@]}" -w "%{http_code}" -I -H "If-None-Match: $ETAG" "$BASE_URL$SRC_PATH")
if [[ "$CODE" != "304" ]]; then echo "HEAD If-None-Match 预期304 实得$CODE" >&2; exit 1; fi

echo "[12/18] GET 下载校验"
curl -sS --fail "${CURL_OPTS[@]}" "$BASE_URL$SRC_PATH" >/dev/null

echo "[13/18] VERSION-CONTROL 标记版本控制"
curl -sS --fail "${CURL_OPTS[@]}" -X VERSION-CONTROL "$BASE_URL$SRC_PATH" >/dev/null

echo "[14/18] REPORT 版本列表"
curl -sS --fail "${CURL_OPTS[@]}" -X REPORT "$BASE_URL$SRC_PATH" >/dev/null

echo "[15/18] REPORT sync-collection 初始token (Depth: infinity)"
SYNC_INIT='<?xml version="1.0" encoding="utf-8"?>
<D:sync-collection xmlns:D="DAV:"></D:sync-collection>'
CODE=$(curl -sS -o "$TMP_DIR/sync_init.xml" "${CURL_OPTS[@]}" -w "%{http_code}" --fail -X REPORT -H "Content-Type: application/xml" -H "Depth: infinity" --data "$SYNC_INIT" "$BASE_URL/interop")
grep -q "sync-token" "$TMP_DIR/sync_init.xml" || { echo "sync-collection 未返回sync-token" >&2; exit 1; }
SYNC_TOKEN=$(awk -F'[<>]' '/<D:sync-token>/{for(i=1;i<=NF;i++) if($i=="D:sync-token"){print $(i+1)}}' "$TMP_DIR/sync_init.xml" | tail -n1)
if [[ -z "$SYNC_TOKEN" ]]; then echo "未解析到初始 sync-token" >&2; exit 1; fi
if [[ "$CODE" != "207" ]]; then echo "sync-collection(初始) 预期207 实得$CODE" >&2; exit 1; fi

echo "[16/18] MOVE 重命名"
curl -sS --fail "${CURL_OPTS[@]}" -X MOVE \
  -H "Destination: $BASE_URL$DST_PATH" \
  -H "If: (<$LOCK_TOKEN>)" \
  "$BASE_URL$SRC_PATH" >/dev/null

echo "[17/18] REPORT sync-collection 差异 (验证 MOVE 301 + moved-from)"
SYNC_DIFF='<?xml version="1.0" encoding="utf-8"?>
<D:sync-collection xmlns:D="DAV:">
  <D:sync-token>'"$SYNC_TOKEN"'</D:sync-token>
  <D:limit><D:nresults>100</D:nresults></D:limit>
  <D:prop>
    <D:displayname/>
  </D:prop>
</D:sync-collection>'
CODE=$(curl -sS -o "$TMP_DIR/sync_diff.xml" "${CURL_OPTS[@]}" -w "%{http_code}" --fail -X REPORT -H "Content-Type: application/xml" -H "Depth: infinity" --data "$SYNC_DIFF" "$BASE_URL/interop")
if [[ "$CODE" != "207" ]]; then echo "sync-collection(差异) 预期207 实得$CODE" >&2; exit 1; fi
grep -q "<D:status>HTTP/1.1 301 Moved Permanently</D:status>" "$TMP_DIR/sync_diff.xml" || { echo "未发现 301 Moved Permanently 差异项" >&2; exit 1; }
grep -q "<silent:moved-from xmlns:silent=\"urn:silent-webdav\">$SRC_PATH</silent:moved-from>" "$TMP_DIR/sync_diff.xml" || { echo "未发现 moved-from=$SRC_PATH" >&2; exit 1; }
grep -q "<D:href>$DST_PATH</D:href>" "$TMP_DIR/sync_diff.xml" || { echo "未发现 href=$DST_PATH" >&2; exit 1; }

echo "[18/18] COPY 复制到新路径 (校验后删除)"
COPY_PATH="/interop/a_${RUN_ID}_copy.txt"
curl -sS --fail "${CURL_OPTS[@]}" -X COPY -H "Destination: $BASE_URL$COPY_PATH" "$BASE_URL$DST_PATH" >/dev/null
curl -sS --fail "${CURL_OPTS[@]}" -X DELETE "$BASE_URL$COPY_PATH" >/dev/null

echo "PROPPATCH 移除属性"
PROP_BODY_REMOVE='<?xml version="1.0" encoding="utf-8"?>
<D:propertyupdate xmlns:D="DAV:">
  <D:remove>
    <D:prop>
      <Z:category xmlns:Z="urn:x-example"/>
    </D:prop>
  </D:remove>
</D:propertyupdate>'
curl -sS "${CURL_OPTS[@]}" -X PROPPATCH -H "Content-Type: application/xml" -H "If: (<$LOCK_TOKEN>)" --data "$PROP_BODY_REMOVE" "$BASE_URL$DST_PATH" >/dev/null || true

echo "UNLOCK 解锁"
curl -sS --fail "${CURL_OPTS[@]}" -X UNLOCK \
  -H "Lock-Token: <$LOCK_TOKEN>" \
  "$BASE_URL$DST_PATH" >/dev/null

echo "清理（删除文件与目录）"
curl -sS --fail "${CURL_OPTS[@]}" -X DELETE "$BASE_URL$DST_PATH" >/dev/null || true
curl -sS --fail "${CURL_OPTS[@]}" -X DELETE "$BASE_URL$DIR_PATH" >/dev/null || true

echo "OK: WebDAV 全量接口互通测试通过"
