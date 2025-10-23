#!/usr/bin/env bash
# ===========================================================
# Finder WebDAV Compatibility Test Script
# ===========================================================
# 用于验证 macOS Finder 是否可以成功连接 Silent WebDAV 服务。
# 将此脚本放置在 silent-nas/scripts/ 目录下执行。
# ===========================================================

set -e

# -------------------------------
# 配置参数
# -------------------------------
HOST="${1:-http://127.0.0.1:8081/}"
AUTH="${AUTH:-}"    # 例如 "user:password"
CURL="curl -s -D -"

echo "🔍 Testing WebDAV endpoint: $HOST"
echo "------------------------------------------------------------"

# -------------------------------
# OPTIONS 请求
# -------------------------------
echo "1️⃣ OPTIONS /"
$CURL -X OPTIONS "$HOST" | tee /tmp/webdav_opts.txt

echo "------------------------------------------------------------"
grep -i "DAV" /tmp/webdav_opts.txt >/dev/null || echo "❌ Missing DAV header"
grep -i "Allow" /tmp/webdav_opts.txt >/dev/null || echo "❌ Missing Allow header"

# -------------------------------
# PROPFIND Depth:0
# -------------------------------
echo "2️⃣ PROPFIND Depth:0"
$CURL -X PROPFIND "$HOST" \
  -H "Depth: 0" \
  -H "Content-Type: text/xml" \
  -d '<?xml version="1.0"?><propfind xmlns="DAV:"><allprop/></propfind>' \
  -o /tmp/webdav_depth0.xml -w "\nHTTP code: %{http_code}\n"

grep -q "207 Multi-Status" /tmp/webdav_depth0.xml && echo "✅ Depth:0 returned 207"
grep -q "displayname" /tmp/webdav_depth0.xml || echo "⚠️ Missing <D:displayname>"
grep -q "resourcetype" /tmp/webdav_depth0.xml || echo "⚠️ Missing <D:resourcetype>"
grep -q "getlastmodified" /tmp/webdav_depth0.xml || echo "⚠️ Missing <D:getlastmodified>"

# -------------------------------
# PROPFIND Depth:1
# -------------------------------
echo "3️⃣ PROPFIND Depth:1"
$CURL -X PROPFIND "$HOST" \
  -H "Depth: 1" \
  -H "Content-Type: text/xml" \
  -d '<?xml version="1.0"?><propfind xmlns="DAV:"><allprop/></propfind>' \
  -o /tmp/webdav_depth1.xml -w "\nHTTP code: %{http_code}\n"

grep -q "207 Multi-Status" /tmp/webdav_depth1.xml && echo "✅ Depth:1 returned 207"
grep -q "displayname" /tmp/webdav_depth1.xml || echo "⚠️ Missing <D:displayname>"
grep -q "getcontentlength" /tmp/webdav_depth1.xml || echo "⚠️ Missing <D:getcontentlength>"
grep -q "creationdate" /tmp/webdav_depth1.xml || echo "⚠️ Missing <D:creationdate>"

# -------------------------------
# LOCK / UNLOCK 测试
# -------------------------------
echo "4️⃣ LOCK /"
$CURL -X LOCK "$HOST" -H "Content-Type: text/xml" \
  -d '<?xml version="1.0"?><lockinfo xmlns="DAV:"><lockscope><exclusive/></lockscope><locktype><write/></locktype><owner><href>silent-nas</href></owner></lockinfo>' \
  -o /tmp/webdav_lock.xml -w "\nHTTP code: %{http_code}\n"

grep -q "200" /tmp/webdav_lock.xml && echo "✅ LOCK returned 200" || echo "⚠️ LOCK not supported"

LOCK_TOKEN=$(grep -oE "opaquelocktoken:[0-9a-fA-F-]+" /tmp/webdav_lock.xml | head -n1)
if [ -n "$LOCK_TOKEN" ]; then
  echo "Found lock token: $LOCK_TOKEN"
  echo "5️⃣ UNLOCK /"
  $CURL -X UNLOCK "$HOST" -H "Lock-Token: <$LOCK_TOKEN>" -o /tmp/webdav_unlock.txt -w "\nHTTP code: %{http_code}\n"
else
  echo "⚠️ No lock token found, skipping UNLOCK test"
fi

# -------------------------------
# 结果汇总
# -------------------------------
echo "------------------------------------------------------------"
echo "✅ 测试完成"
echo "可手动挂载 Finder："
echo "   ⌘K → 连接服务器 → 输入：$HOST"
echo "   若仍失败，请执行：sudo log stream --predicate 'process == \"mount_webdav\"' --style syslog"
echo "------------------------------------------------------------"
