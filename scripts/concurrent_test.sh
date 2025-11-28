#!/bin/bash

# 并发连接测试脚本
# 测试高并发场景下的系统稳定性

set -e

WEBDAV_HOST="${WEBDAV_HOST:-http://localhost:8081}"
WEBDAV_USER="${WEBDAV_USER:-admin}"
WEBDAV_PASS="${WEBDAV_PASS:-admin123}"

echo "========================================"
echo "并发连接测试"
echo "========================================"
echo ""

# 创建测试目录
TEST_DIR="/concurrent_test_$(date +%s)"
echo "测试目录: $TEST_DIR"

# 创建一个小测试文件
TEST_FILE="/tmp/concurrent_test_1kb.dat"
dd if=/dev/urandom of="$TEST_FILE" bs=1024 count=1 2>/dev/null

echo ""
echo "[测试1] 100个并发上传"
echo "--------------------"
start_time=$(date +%s.%N)
success_count=0
failed_count=0

for i in $(seq 1 100); do
    (
        if curl -X PUT -u "$WEBDAV_USER:$WEBDAV_PASS" \
            --data-binary @"$TEST_FILE" \
            -s -o /dev/null -w "%{http_code}" \
            "$WEBDAV_HOST$TEST_DIR/file_$i.dat" | grep -q "20[01]"; then
            echo "." > /dev/null
        else
            echo "!" > /dev/null
        fi
    ) &
done

# 等待所有后台任务完成
wait

end_time=$(date +%s.%N)
duration=$(echo "$end_time - $start_time" | bc)

# 检查成功上传的文件数
success_count=$(curl -X PROPFIND -u "$WEBDAV_USER:$WEBDAV_PASS" \
    -s "$WEBDAV_HOST$TEST_DIR/" | grep -c "file_" || echo "0")

echo "  总耗时: ${duration}s"
echo "  成功上传: $success_count/100"
echo "  平均速率: $(echo "scale=2; 100 / $duration" | bc) 请求/秒"

# 计算平均响应时间
avg_response_time=$(echo "scale=3; $duration / 100 * 1000" | bc)
echo "  平均响应时间: ${avg_response_time}ms"

echo ""
echo "[测试2] 500个并发上传"
echo "--------------------"
start_time=$(date +%s.%N)

for i in $(seq 101 600); do
    (
        curl -X PUT -u "$WEBDAV_USER:$WEBDAV_PASS" \
            --data-binary @"$TEST_FILE" \
            -s -o /dev/null \
            "$WEBDAV_HOST$TEST_DIR/file_$i.dat"
    ) &

    # 每50个请求暂停一下，避免太多同时连接
    if [ $((i % 50)) -eq 0 ]; then
        sleep 0.1
    fi
done

wait

end_time=$(date +%s.%N)
duration=$(echo "$end_time - $start_time" | bc)

success_count=$(curl -X PROPFIND -u "$WEBDAV_USER:$WEBDAV_PASS" \
    -s "$WEBDAV_HOST$TEST_DIR/" | grep -c "file_" || echo "0")

echo "  总耗时: ${duration}s"
echo "  成功上传: $success_count/600 (包含之前的100个)"
echo "  平均速率: $(echo "scale=2; 500 / $duration" | bc) 请求/秒"

avg_response_time=$(echo "scale=3; $duration / 500 * 1000" | bc)
echo "  平均响应时间: ${avg_response_time}ms"

echo ""
echo "[测试3] 单个请求响应时间测试"
echo "-----------------------------"

# 测试10次取平均值
total_time=0
for i in $(seq 1 10); do
    start=$(date +%s.%N)
    curl -X PUT -u "$WEBDAV_USER:$WEBDAV_PASS" \
        --data-binary @"$TEST_FILE" \
        -s -o /dev/null \
        "$WEBDAV_HOST$TEST_DIR/response_test_$i.dat"
    end=$(date +%s.%N)
    duration=$(echo "($end - $start) * 1000" | bc)
    total_time=$(echo "$total_time + $duration" | bc)
    echo "  请求 $i: ${duration}ms"
done

avg_time=$(echo "scale=2; $total_time / 10" | bc)
echo ""
echo "  平均响应时间: ${avg_time}ms"

if (( $(echo "$avg_time < 100" | bc -l) )); then
    echo "  ✅ 响应时间 < 100ms 目标达成"
else
    echo "  ⚠️  响应时间 > 100ms"
fi

# 清理
rm -f "$TEST_FILE"

echo ""
echo "========================================"
echo "测试完成"
echo "========================================"
echo ""
echo "总结："
echo "- 100并发: 平均响应时间可通过上述数据计算"
echo "- 500并发: 系统稳定性验证"
echo "- 单请求: 响应时间基准"
echo ""

# 注意：由于我们使用的是 bash 后台进程，实际并发数受系统限制
# 真正的 1000+ 并发测试需要使用 wrk 或 ab 等专业工具
