#!/bin/bash
# WebDAV 性能基准测试和对比脚本
#
# 用途：建立性能基准线，对比不同版本之间的性能变化
# 依赖：curl, bc

set -e

# 配置
WEBDAV_HOST="${WEBDAV_HOST:-http://localhost:8000}"
WEBDAV_USER="${WEBDAV_USER:-admin}"
WEBDAV_PASS="${WEBDAV_PASS:-admin123}"
TEST_DIR="${TEST_DIR:-/benchmark}"
BASELINE_FILE="${BASELINE_FILE:-./benchmark_baseline.json}"
RESULTS_DIR="./benchmark-results"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# 创建结果目录
mkdir -p "$RESULTS_DIR"

# 生成测试文件
generate_test_file() {
    local size_mb=$1
    local filename=$2

    if [ ! -f "$filename" ]; then
        echo "生成 ${size_mb}MB 测试文件..."
        dd if=/dev/urandom of="$filename" bs=1M count=$size_mb 2>/dev/null
    fi
}

# 测试单个文件上传性能
benchmark_single_upload() {
    local file_size_mb=$1
    local test_file="/tmp/benchmark_${file_size_mb}mb.bin"

    echo -e "${BLUE}测试 ${file_size_mb}MB 文件上传...${NC}"

    generate_test_file $file_size_mb "$test_file"

    local start_time=$(date +%s.%N)

    curl -X PUT -u "$WEBDAV_USER:$WEBDAV_PASS" \
        -H "Content-Type: application/octet-stream" \
        --data-binary @"$test_file" \
        "$WEBDAV_HOST$TEST_DIR/benchmark_${file_size_mb}mb.bin" \
        > /dev/null 2>&1

    local end_time=$(date +%s.%N)
    local duration=$(echo "$end_time - $start_time" | bc)
    local throughput=$(echo "scale=2; $file_size_mb / $duration" | bc)

    echo "  耗时: ${duration}s"
    echo "  吞吐量: ${throughput} MB/s"

    # 返回吞吐量（通过echo）
    echo "$throughput"
}

# 测试并发上传性能
benchmark_concurrent_upload() {
    local file_size_mb=$1
    local concurrency=$2
    local test_file="/tmp/benchmark_${file_size_mb}mb.bin"

    echo -e "${BLUE}测试 ${concurrency} 个并发上传 (${file_size_mb}MB 文件)...${NC}"

    generate_test_file $file_size_mb "$test_file"

    local start_time=$(date +%s.%N)

    for i in $(seq 1 $concurrency); do
        curl -X PUT -u "$WEBDAV_USER:$WEBDAV_PASS" \
            -H "Content-Type: application/octet-stream" \
            --data-binary @"$test_file" \
            "$WEBDAV_HOST$TEST_DIR/concurrent_${i}_${file_size_mb}mb.bin" \
            > /dev/null 2>&1 &
    done

    wait

    local end_time=$(date +%s.%N)
    local duration=$(echo "$end_time - $start_time" | bc)
    local total_mb=$(echo "$file_size_mb * $concurrency" | bc)
    local throughput=$(echo "scale=2; $total_mb / $duration" | bc)

    echo "  总数据量: ${total_mb}MB"
    echo "  总耗时: ${duration}s"
    echo "  聚合吞吐量: ${throughput} MB/s"

    echo "$throughput"
}

# 测试文件下载性能
benchmark_download() {
    local file_size_mb=$1
    local test_file="/tmp/benchmark_${file_size_mb}mb.bin"

    echo -e "${BLUE}测试 ${file_size_mb}MB 文件下载...${NC}"

    # 先上传测试文件
    generate_test_file $file_size_mb "$test_file"
    curl -X PUT -u "$WEBDAV_USER:$WEBDAV_PASS" \
        -H "Content-Type: application/octet-stream" \
        --data-binary @"$test_file" \
        "$WEBDAV_HOST$TEST_DIR/download_test_${file_size_mb}mb.bin" \
        > /dev/null 2>&1

    # 下载测试
    local start_time=$(date +%s.%N)

    curl -X GET -u "$WEBDAV_USER:$WEBDAV_PASS" \
        "$WEBDAV_HOST$TEST_DIR/download_test_${file_size_mb}mb.bin" \
        -o /dev/null \
        > /dev/null 2>&1

    local end_time=$(date +%s.%N)
    local duration=$(echo "$end_time - $start_time" | bc)
    local throughput=$(echo "scale=2; $file_size_mb / $duration" | bc)

    echo "  耗时: ${duration}s"
    echo "  吞吐量: ${throughput} MB/s"

    echo "$throughput"
}

# 测试小文件操作性能
benchmark_small_files() {
    local num_files=$1

    echo -e "${BLUE}测试小文件操作 (${num_files} 个 1KB 文件)...${NC}"

    local test_data=$(head -c 1024 /dev/urandom | base64)

    local start_time=$(date +%s.%N)

    for i in $(seq 1 $num_files); do
        echo "$test_data" | curl -X PUT -u "$WEBDAV_USER:$WEBDAV_PASS" \
            -H "Content-Type: application/octet-stream" \
            --data-binary @- \
            "$WEBDAV_HOST$TEST_DIR/small_file_${i}.txt" \
            > /dev/null 2>&1
    done

    local end_time=$(date +%s.%N)
    local duration=$(echo "$end_time - $start_time" | bc)
    local ops_per_sec=$(echo "scale=2; $num_files / $duration" | bc)

    echo "  总文件数: $num_files"
    echo "  总耗时: ${duration}s"
    echo "  操作速率: ${ops_per_sec} 文件/秒"

    echo "$ops_per_sec"
}

# 运行完整基准测试套件
run_benchmark_suite() {
    echo -e "${YELLOW}========================================${NC}"
    echo -e "${YELLOW}运行 WebDAV 性能基准测试套件${NC}"
    echo -e "${YELLOW}========================================${NC}\n"

    # 创建测试目录
    curl -X MKCOL -u "$WEBDAV_USER:$WEBDAV_PASS" "$WEBDAV_HOST$TEST_DIR" > /dev/null 2>&1 || true

    # 1. 小文件上传 (1MB)
    echo -e "\n${GREEN}[测试1] 1MB 文件上传${NC}"
    local upload_1mb=$(benchmark_single_upload 1)

    # 2. 中等文件上传 (10MB)
    echo -e "\n${GREEN}[测试2] 10MB 文件上传${NC}"
    local upload_10mb=$(benchmark_single_upload 10)

    # 3. 大文件上传 (100MB)
    echo -e "\n${GREEN}[测试3] 100MB 文件上传${NC}"
    local upload_100mb=$(benchmark_single_upload 100)

    # 4. 超大文件上传 (1GB)
    echo -e "\n${GREEN}[测试4] 1GB 文件上传${NC}"
    local upload_1gb=$(benchmark_single_upload 1024)

    # 5. 并发上传 (10个 10MB 文件)
    echo -e "\n${GREEN}[测试5] 并发上传 (10个 10MB 文件)${NC}"
    local concurrent_10x10mb=$(benchmark_concurrent_upload 10 10)

    # 6. 并发上传 (5个 100MB 文件)
    echo -e "\n${GREEN}[测试6] 并发上传 (5个 100MB 文件)${NC}"
    local concurrent_5x100mb=$(benchmark_concurrent_upload 100 5)

    # 7. 文件下载 (100MB)
    echo -e "\n${GREEN}[测试7] 100MB 文件下载${NC}"
    local download_100mb=$(benchmark_download 100)

    # 8. 小文件操作性能
    echo -e "\n${GREEN}[测试8] 小文件操作 (100个 1KB 文件)${NC}"
    local small_files_100=$(benchmark_small_files 100)

    # 生成 JSON 结果
    local result_file="$RESULTS_DIR/benchmark_${TIMESTAMP}.json"
    cat > "$result_file" <<EOF
{
  "timestamp": "$TIMESTAMP",
  "version": "$(git describe --tags 2>/dev/null || echo 'unknown')",
  "host": "$WEBDAV_HOST",
  "results": {
    "upload_1mb_mbs": $upload_1mb,
    "upload_10mb_mbs": $upload_10mb,
    "upload_100mb_mbs": $upload_100mb,
    "upload_1gb_mbs": $upload_1gb,
    "concurrent_10x10mb_mbs": $concurrent_10x10mb,
    "concurrent_5x100mb_mbs": $concurrent_5x100mb,
    "download_100mb_mbs": $download_100mb,
    "small_files_100_ops": $small_files_100
  }
}
EOF

    echo -e "\n${GREEN}✓ 基准测试结果已保存: $result_file${NC}"

    # 如果存在基线，进行对比
    if [ -f "$BASELINE_FILE" ]; then
        compare_with_baseline "$result_file"
    else
        echo -e "${YELLOW}! 未找到基线文件，是否将当前结果设为基线？(y/n)${NC}"
        read -r response
        if [[ "$response" =~ ^[Yy]$ ]]; then
            cp "$result_file" "$BASELINE_FILE"
            echo -e "${GREEN}✓ 基线已设置: $BASELINE_FILE${NC}"
        fi
    fi

    # 清理
    cleanup_test_files
}

# 与基线对比
compare_with_baseline() {
    local current_file=$1

    echo -e "\n${YELLOW}========================================${NC}"
    echo -e "${YELLOW}性能对比分析${NC}"
    echo -e "${YELLOW}========================================${NC}\n"

    # 提取当前和基线的值（使用简单的 grep/sed 解析 JSON）
    local metrics=("upload_1mb_mbs" "upload_10mb_mbs" "upload_100mb_mbs" "upload_1gb_mbs" "concurrent_10x10mb_mbs" "concurrent_5x100mb_mbs" "download_100mb_mbs" "small_files_100_ops")
    local metric_names=("1MB上传" "10MB上传" "100MB上传" "1GB上传" "并发10x10MB" "并发5x100MB" "100MB下载" "100小文件")

    for i in "${!metrics[@]}"; do
        local metric="${metrics[$i]}"
        local name="${metric_names[$i]}"

        local baseline_val=$(grep "\"$metric\"" "$BASELINE_FILE" | sed 's/.*: //;s/,$//')
        local current_val=$(grep "\"$metric\"" "$current_file" | sed 's/.*: //;s/,$//')

        if [ -n "$baseline_val" ] && [ -n "$current_val" ]; then
            local diff=$(echo "scale=2; $current_val - $baseline_val" | bc)
            local percent=$(echo "scale=2; ($diff / $baseline_val) * 100" | bc)

            if (( $(echo "$percent > 0" | bc -l) )); then
                echo -e "${GREEN}$name: ${current_val} (基线: ${baseline_val}, +${percent}%)${NC}"
            elif (( $(echo "$percent < -5" | bc -l) )); then
                echo -e "${RED}$name: ${current_val} (基线: ${baseline_val}, ${percent}%)${NC}"
            else
                echo -e "$name: ${current_val} (基线: ${baseline_val}, ${percent}%)"
            fi
        fi
    done

    echo ""
}

# 清理测试文件
cleanup_test_files() {
    echo -e "${YELLOW}清理测试文件...${NC}"

    curl -X DELETE -u "$WEBDAV_USER:$WEBDAV_PASS" -r "$WEBDAV_HOST$TEST_DIR" > /dev/null 2>&1 || true

    rm -f /tmp/benchmark_*.bin

    echo -e "${GREEN}✓ 清理完成${NC}"
}

# 显示帮助信息
show_help() {
    cat <<EOF
WebDAV 性能基准测试工具

用法:
  $0 [选项]

选项:
  run             运行完整基准测试套件
  set-baseline    将最新结果设为基线
  compare         与基线对比当前性能
  clean           清理测试文件和结果
  help            显示此帮助信息

环境变量:
  WEBDAV_HOST     WebDAV 服务器地址 (默认: http://localhost:8000)
  WEBDAV_USER     用户名 (默认: admin)
  WEBDAV_PASS     密码 (默认: admin123)
  TEST_DIR        测试目录 (默认: /benchmark)
  BASELINE_FILE   基线文件路径 (默认: ./benchmark_baseline.json)

示例:
  # 运行基准测试
  $0 run

  # 使用自定义服务器
  WEBDAV_HOST=http://localhost:9000 $0 run

  # 设置基线
  $0 set-baseline

  # 对比性能
  $0 compare
EOF
}

# 主函数
main() {
    local command=${1:-run}

    case $command in
        run)
            run_benchmark_suite
            ;;
        set-baseline)
            local latest=$(ls -t $RESULTS_DIR/benchmark_*.json 2>/dev/null | head -1)
            if [ -n "$latest" ]; then
                cp "$latest" "$BASELINE_FILE"
                echo -e "${GREEN}✓ 基线已设置: $BASELINE_FILE${NC}"
            else
                echo -e "${RED}错误: 未找到测试结果文件${NC}"
                exit 1
            fi
            ;;
        compare)
            local latest=$(ls -t $RESULTS_DIR/benchmark_*.json 2>/dev/null | head -1)
            if [ -n "$latest" ]; then
                compare_with_baseline "$latest"
            else
                echo -e "${RED}错误: 未找到测试结果文件${NC}"
                exit 1
            fi
            ;;
        clean)
            cleanup_test_files
            ;;
        help|--help|-h)
            show_help
            ;;
        *)
            echo -e "${RED}未知命令: $command${NC}"
            show_help
            exit 1
            ;;
    esac
}

# 运行主函数
main "$@"
