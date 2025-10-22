#!/usr/bin/env bash
set -euo pipefail

# 对资源进行 PROPPATCH（设置/移除属性）
# 用法：
#   设置：KEY=Z:category VAL=interop ./scripts/webdav_proppatch.sh /path token
#   移除：KEY=Z:category ./scripts/webdav_proppatch.sh /path token

BASE_URL=${BASE_URL:-"http://127.0.0.1:8081/webdav"}
PATH_IN=${1:?"usage: KEY=xxx [VAL=yyy] $0 /path lock_token"}
LOCK_TOKEN=${2:?"usage: KEY=xxx [VAL=yyy] $0 /path lock_token"}
KEY=${KEY:?"env KEY is required, like Z:category"}
VAL=${VAL:-}

if [[ -n "$VAL" ]]; then
  BODY="<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:propertyupdate xmlns:D=\"DAV:\"><D:set><D:prop><$KEY>$VAL</$KEY></D:prop></D:set></D:propertyupdate>"
else
  BODY="<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:propertyupdate xmlns:D=\"DAV:\"><D:remove><D:prop><$KEY/></D:prop></D:remove></D:propertyupdate>"
fi

curl -sS --fail -X PROPPATCH \
  -H "Content-Type: application/xml" \
  -H "If: (<opaquelocktoken:$LOCK_TOKEN>)" \
  --data "$BODY" "$BASE_URL$PATH_IN" >/dev/null
echo OK
