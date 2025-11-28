#!/bin/bash
# WebDAV 大文件上传压力测试脚本
#
# 用途：测试 WebDAV 服务器在高并发场景下的性能表现
# 依赖：wrk（HTTP 压力测试工具）
#
# 安装 wrk:
#   macOS: brew install wrk
#   Ubuntu: sudo apt-get install wrk
#   手动编译: git clone https://github.com/wg/wrk && cd wrk && make

set -e

# 配置
WEBDAV_HOST="${WEBDAV_HOST:-http://localhost:8000}"
WEBDAV_USER="${WEBDAV_USER:-admin}"
WEBDAV_PASS="${WEBDAV_PASS:-admin123}"
TEST_DIR="${TEST_DIR:-/stress-test}"
RESULTS_DIR="./performance-results"

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# 检查依赖
check_dependencies() {
    echo -e "${YELLOW}[检查依赖]${NC}"

    if ! command -v wrk &> /dev/null; then
        echo -e "${RED}错误: 未找到 wrk 工具${NC}"
        echo "请安装 wrk: brew install wrk (macOS) 或 sudo apt-get install wrk (Ubuntu)"
        exit 1
    fi

    if ! command -v curl &> /dev/null; then
        echo -e "${RED}错误: 未找到 curl 工具${NC}"
        exit 1
    fi

    echo -e "${GREEN}✓ 依赖检查通过${NC}\n"
}

# 检查服务器连接
check_server() {
    echo -e "${YELLOW}[检查服务器连接]${NC}"
    echo "服务器地址: $WEBDAV_HOST"

    if curl -s -f -u "$WEBDAV_USER:$WEBDAV_PASS" "$WEBDAV_HOST" > /dev/null 2>&1; then
        echo -e "${GREEN}✓ 服务器连接正常${NC}\n"
    else
        echo -e "${RED}错误: 无法连接到服务器 $WEBDAV_HOST${NC}"
        echo "请确保服务器正在运行并且认证信息正确"
        exit 1
    fi
}

# 创建测试目录
setup_test_dir() {
    echo -e "${YELLOW}[创建测试目录]${NC}"

    curl -X MKCOL -u "$WEBDAV_USER:$WEBDAV_PASS" "$WEBDAV_HOST$TEST_DIR" > /dev/null 2>&1 || true

    mkdir -p "$RESULTS_DIR"
    echo -e "${GREEN}✓ 测试目录创建完成${NC}\n"
}

# 生成测试数据
generate_test_file() {
    local size_mb=$1
    local filename=$2

    echo "生成 ${size_mb}MB 测试文件: $filename"
    dd if=/dev/urandom of="$filename" bs=1M count=$size_mb 2>/dev/null
}

# Lua 脚本：小文件上传压力测试
create_wrk_script_small_file() {
    cat > /tmp/wrk_upload_small.lua <<'EOF'
wrk.method = "PUT"
wrk.headers["Authorization"] = "Basic YWRtaW46YWRtaW4xMjM="  -- admin:admin123
wrk.headers["Content-Type"] = "application/octet-stream"

-- 1KB 测试数据
wrk.body = string.rep("x", 1024)

counter = 0

request = function()
    counter = counter + 1
    local path = "/stress-test/file_" .. counter .. ".bin"
    return wrk.format(nil, path)
end
EOF
}

# Lua 脚本：中等文件上传压力测试
create_wrk_script_medium_file() {
    cat > /tmp/wrk_upload_medium.lua <<'EOF'
wrk.method = "PUT"
wrk.headers["Authorization"] = "Basic YWRtaW46YWRtaW4xMjM="  -- admin:admin123
wrk.headers["Content-Type"] = "application/octet-stream"

-- 100KB 测试数据
wrk.body = string.rep("x", 102400)

counter = 0

request = function()
    counter = counter + 1
    local path = "/stress-test/file_" .. counter .. ".bin"
    return wrk.format(nil, path)
end
EOF
}

# 测试1: 小文件高并发上传（1KB文件）
test_small_file_concurrency() {
    echo -e "${YELLOW}[测试1: 小文件高并发上传]${NC}"
    echo "场景: 1KB 文件，1000 并发连接，持续 30 秒"

    create_wrk_script_small_file

    wrk -t8 -c1000 -d30s -s /tmp/wrk_upload_small.lua "$WEBDAV_HOST" \
        | tee "$RESULTS_DIR/test1_small_file_1000conn.txt"

    echo -e "${GREEN}✓ 测试1完成${NC}\n"
}

# 测试2: 中等文件并发上传（100KB文件）
test_medium_file_concurrency() {
    echo -e "${YELLOW}[测试2: 中等文件并发上传]${NC}"
    echo "场景: 100KB 文件，500 并发连接，持续 30 秒"

    create_wrk_script_medium_file

    wrk -t8 -c500 -d30s -s /tmp/wrk_upload_medium.lua "$WEBDAV_HOST" \
        | tee "$RESULTS_DIR/test2_medium_file_500conn.txt"

    echo -e "${GREEN}✓ 测试2完成${NC}\n"
}

# 测试3: 大文件上传吞吐量测试
test_large_file_throughput() {
    echo -e "${YELLOW}[测试3: 大文件上传吞吐量]${NC}"
    echo "场景: 10MB 文件，并发上传 10 次"

    generate_test_file 10 /tmp/test_10mb.bin

    local start_time=$(date +%s)

    for i in {1..10}; do
        curl -X PUT -u "$WEBDAV_USER:$WEBDAV_PASS" \
            -H "Content-Type: application/octet-stream" \
            --data-binary @/tmp/test_10mb.bin \
            "$WEBDAV_HOST$TEST_DIR/large_file_${i}.bin" \
            > /dev/null 2>&1 &
    done

    wait

    local end_time=$(date +%s)
    local duration=$((end_time - start_time))
    local total_mb=100
    local throughput=$(echo "scale=2; $total_mb / $duration" | bc)

    echo "总数据量: ${total_mb}MB"
    echo "总耗时: ${duration}秒"
    echo "吞吐量: ${throughput} MB/s"
    echo "$throughput" > "$RESULTS_DIR/test3_throughput_mbs.txt"

    rm /tmp/test_10mb.bin
    echo -e "${GREEN}✓ 测试3完成${NC}\n"
}

# 测试4: 逐步增加并发连接数
test_gradual_concurrency() {
    echo -e "${YELLOW}[测试4: 逐步增加并发连接数]${NC}"

    create_wrk_script_small_file

    for connections in 100 250 500 750 1000 1500 2000; do
        echo "测试并发数: $connections"

        wrk -t8 -c$connections -d15s -s /tmp/wrk_upload_small.lua "$WEBDAV_HOST" \
            > "$RESULTS_DIR/test4_concurrency_${connections}.txt" 2>&1

        # 提取关键指标
        local requests_per_sec=$(grep "Requests/sec:" "$RESULTS_DIR/test4_concurrency_${connections}.txt" | awk '{print $2}')
        echo "$connections,$requests_per_sec" >> "$RESULTS_DIR/test4_concurrency_summary.csv"

        sleep 2  # 让服务器恢复
    done

    echo -e "${GREEN}✓ 测试4完成${NC}\n"
}

# 测试5: 长时间稳定性测试
test_stability() {
    echo -e "${YELLOW}[测试5: 长时间稳定性测试]${NC}"
    echo "场景: 200 并发连接，持续 5 分钟"

    create_wrk_script_small_file

    wrk -t8 -c200 -d5m -s /tmp/wrk_upload_small.lua "$WEBDAV_HOST" \
        | tee "$RESULTS_DIR/test5_stability_5min.txt"

    echo -e "${GREEN}✓ 测试5完成${NC}\n"
}

# 清理测试数据
cleanup() {
    echo -e "${YELLOW}[清理测试数据]${NC}"

    curl -X DELETE -u "$WEBDAV_USER:$WEBDAV_PASS" -r "$WEBDAV_HOST$TEST_DIR" > /dev/null 2>&1 || true

    rm -f /tmp/wrk_upload_*.lua
    rm -f /tmp/test_*.bin

    echo -e "${GREEN}✓ 清理完成${NC}\n"
}

# 生成测试报告
generate_report() {
    echo -e "${YELLOW}[生成测试报告]${NC}"

    local report_file="$RESULTS_DIR/summary_report.txt"

    cat > "$report_file" <<EOF
========================================
WebDAV 大文件上传压力测试报告
========================================
测试时间: $(date)
服务器地址: $WEBDAV_HOST
测试工具: wrk

========================================
测试1: 小文件高并发上传（1KB，1000并发）
========================================
EOF

    if [ -f "$RESULTS_DIR/test1_small_file_1000conn.txt" ]; then
        cat "$RESULTS_DIR/test1_small_file_1000conn.txt" >> "$report_file"
    fi

    cat >> "$report_file" <<EOF

========================================
测试2: 中等文件并发上传（100KB，500并发）
========================================
EOF

    if [ -f "$RESULTS_DIR/test2_medium_file_500conn.txt" ]; then
        cat "$RESULTS_DIR/test2_medium_file_500conn.txt" >> "$report_file"
    fi

    cat >> "$report_file" <<EOF

========================================
测试3: 大文件上传吞吐量（10MB x 10）
========================================
EOF

    if [ -f "$RESULTS_DIR/test3_throughput_mbs.txt" ]; then
        echo "吞吐量: $(cat $RESULTS_DIR/test3_throughput_mbs.txt) MB/s" >> "$report_file"
    fi

    cat >> "$report_file" <<EOF

========================================
测试4: 并发连接数梯度测试
========================================
EOF

    if [ -f "$RESULTS_DIR/test4_concurrency_summary.csv" ]; then
        echo "并发数,请求/秒" >> "$report_file"
        cat "$RESULTS_DIR/test4_concurrency_summary.csv" >> "$report_file"
    fi

    echo -e "${GREEN}✓ 测试报告已生成: $report_file${NC}\n"

    # 显示摘要
    echo -e "${YELLOW}========================================${NC}"
    echo -e "${YELLOW}测试摘要${NC}"
    echo -e "${YELLOW}========================================${NC}"
    cat "$report_file" | grep -A 5 "测试1:\|测试2:\|测试3:\|测试4:" || true
}

# 主函数
main() {
    echo -e "${GREEN}"
    echo "========================================="
    echo "   WebDAV 大文件上传压力测试套件"
    echo "========================================="
    echo -e "${NC}\n"

    check_dependencies
    check_server
    setup_test_dir

    # 运行所有测试
    test_small_file_concurrency
    test_medium_file_concurrency
    test_large_file_throughput
    test_gradual_concurrency
    # test_stability  # 可选：取消注释以运行5分钟稳定性测试

    generate_report

    # 询问是否清理
    read -p "是否清理测试数据？(y/n) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        cleanup
    fi

    echo -e "${GREEN}=========================================${NC}"
    echo -e "${GREEN}所有测试完成！${NC}"
    echo -e "${GREEN}结果保存在: $RESULTS_DIR${NC}"
    echo -e "${GREEN}=========================================${NC}"
}

# 运行主函数
main "$@"
