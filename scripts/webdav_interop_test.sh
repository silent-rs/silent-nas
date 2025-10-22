#!/usr/bin/env bash
set -euo pipefail

# WebDAV 互通验证脚本（基础流程）
# 依赖：curl

BASE_URL=${BASE_URL:-"http://127.0.0.1:8081/webdav"}
SRC_PATH=${SRC_PATH:-"/interop/a.txt"}
DST_PATH=${DST_PATH:-"/interop/a_renamed.txt"}
TMP_DIR=$(mktemp -d)
HEADERS_FILE="$TMP_DIR/headers.txt"
BODY_FILE="$TMP_DIR/body.bin"

cleanup() {
  rm -rf "$TMP_DIR" || true
}
trap cleanup EXIT

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
  -H "If: (<opaquelocktoken:$LOCK_TOKEN>)" \
  --data "$PROP_BODY" "$BASE_URL$SRC_PATH" >/dev/null

echo "[5/9] GET 下载校验"
curl -sS --fail "$BASE_URL$SRC_PATH" >/dev/null

echo "[6/9] MOVE 重命名"
curl -sS --fail -X MOVE \
  -H "Destination: $BASE_URL$DST_PATH" \
  -H "If: (<opaquelocktoken:$LOCK_TOKEN>)" \
  "$BASE_URL$SRC_PATH" >/dev/null

echo "[7/9] UNLOCK 解锁"
curl -sS --fail -X UNLOCK \
  -H "Lock-Token: <$LOCK_TOKEN>" \
  "$BASE_URL$DST_PATH" >/dev/null

echo "[8/9] REPORT 版本列表"
curl -sS --fail -X REPORT "$BASE_URL$DST_PATH" >/dev/null

echo "[9/9] 清理"
curl -sS --fail -X DELETE "$BASE_URL$DST_PATH" >/dev/null || true

echo "OK: WebDAV 互通基础流程通过"
