#!/usr/bin/env bash
set -euo pipefail

# 用于对指定路径加锁（独占），输出 Lock-Token

BASE_URL=${BASE_URL:-"http://127.0.0.1:8081/webdav"}
PATH_IN=${1:?"usage: $0 /path/to/lock"}
TIMEOUT=${TIMEOUT:-60}
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

LOCK_BODY='<?xml version="1.0" encoding="utf-8"?>
<D:lockinfo xmlns:D="DAV:">
  <D:lockscope><D:exclusive/></D:lockscope>
  <D:locktype><D:write/></D:locktype>
  <D:owner>script</D:owner>
</D:lockinfo>'

curl -sS -D "$TMP" -o /dev/null --fail \
  -X LOCK -H "Content-Type: application/xml" \
  -H "Timeout: Second-$TIMEOUT" \
  --data "$LOCK_BODY" "$BASE_URL$PATH_IN"
grep -i "^lock-token:" "$TMP" | awk '{print $2}' | tr -d '\r\n<>'
