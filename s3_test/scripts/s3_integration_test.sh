#!/bin/bash

# S3 集成测试脚本
# 测试 Silent-NAS 的 S3 兼容 API 实现

set -e

# 配置
S3_ENDPOINT="http://127.0.0.1:9000"
AWS_ACCESS_KEY_ID="minioadmin"
AWS_SECRET_ACCESS_KEY="minioadmin"
BUCKET="test-bucket"
TEST_FILE="test-file.txt"
LARGE_FILE="large-test-file.bin"

# 颜色输出
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# 测试计数
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

# 打印测试结果
print_test_result() {
    local test_name=$1
    local result=$2
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    if [ "$result" -eq 0 ]; then
        echo -e "${GREEN}✓${NC} $test_name"
        PASSED_TESTS=$((PASSED_TESTS + 1))
    else
        echo -e "${RED}✗${NC} $test_name"
        FAILED_TESTS=$((FAILED_TESTS + 1))
    fi
}

# 检查 aws-cli 是否安装
check_aws_cli() {
    if ! command -v aws &> /dev/null; then
        echo -e "${RED}错误：aws-cli 未安装${NC}"
        echo "请安装 aws-cli: brew install awscli"
        exit 1
    fi
}

# 配置 AWS CLI
configure_aws() {
    export AWS_ACCESS_KEY_ID="$AWS_ACCESS_KEY_ID"
    export AWS_SECRET_ACCESS_KEY="$AWS_SECRET_ACCESS_KEY"
    export AWS_DEFAULT_REGION="us-east-1"
}

# 创建测试文件
create_test_files() {
    echo "Creating test files..."
    echo "Hello Silent-NAS S3!" > "$TEST_FILE"
    # 创建 10MB 测试文件
    dd if=/dev/urandom of="$LARGE_FILE" bs=1M count=10 2>/dev/null
}

# 清理测试文件
cleanup() {
    echo "Cleaning up test files..."
    rm -f "$TEST_FILE" "$LARGE_FILE" downloaded-* copied-*
}

echo "========================================"
echo "Silent-NAS S3 兼容性测试"
echo "========================================"
echo ""

# 前置检查
check_aws_cli
configure_aws
create_test_files

echo "测试端点: $S3_ENDPOINT"
echo ""

# ============================================
# Bucket 操作测试
# ============================================
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📦 Bucket 操作测试"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 测试 1: 创建 Bucket
aws s3api create-bucket \
    --bucket "$BUCKET" \
    --endpoint-url "$S3_ENDPOINT" \
    &>/dev/null
print_test_result "PutBucket - 创建 Bucket" $?

# 测试 2: 检查 Bucket 存在
aws s3api head-bucket \
    --bucket "$BUCKET" \
    --endpoint-url "$S3_ENDPOINT" \
    &>/dev/null
print_test_result "HeadBucket - 检查 Bucket 存在" $?

# 测试 3: 列出所有 Buckets
aws s3api list-buckets \
    --endpoint-url "$S3_ENDPOINT" \
    &>/dev/null
print_test_result "ListBuckets - 列出所有 Bucket" $?

# 测试 4: 获取 Bucket 位置
aws s3api get-bucket-location \
    --bucket "$BUCKET" \
    --endpoint-url "$S3_ENDPOINT" \
    &>/dev/null
print_test_result "GetBucketLocation - 获取 Bucket 位置" $?

# 测试 5: 获取 Bucket 版本控制状态
aws s3api get-bucket-versioning \
    --bucket "$BUCKET" \
    --endpoint-url "$S3_ENDPOINT" \
    &>/dev/null
print_test_result "GetBucketVersioning - 获取版本控制状态" $?

# ============================================
# 对象操作测试
# ============================================
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📄 对象 CRUD 操作测试"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 测试 6: 上传对象
aws s3api put-object \
    --bucket "$BUCKET" \
    --key "$TEST_FILE" \
    --body "$TEST_FILE" \
    --endpoint-url "$S3_ENDPOINT" \
    &>/dev/null
print_test_result "PutObject - 上传对象" $?

# 测试 7: 获取对象元数据
aws s3api head-object \
    --bucket "$BUCKET" \
    --key "$TEST_FILE" \
    --endpoint-url "$S3_ENDPOINT" \
    &>/dev/null
print_test_result "HeadObject - 获取对象元数据" $?

# 测试 8: 下载对象
aws s3api get-object \
    --bucket "$BUCKET" \
    --key "$TEST_FILE" \
    --endpoint-url "$S3_ENDPOINT" \
    "downloaded-$TEST_FILE" \
    &>/dev/null
print_test_result "GetObject - 下载对象" $?

# 测试 9: 验证下载内容
if diff "$TEST_FILE" "downloaded-$TEST_FILE" &>/dev/null; then
    print_test_result "验证下载内容一致性" 0
else
    print_test_result "验证下载内容一致性" 1
fi

# 测试 10: 复制对象
aws s3api copy-object \
    --bucket "$BUCKET" \
    --key "copied-$TEST_FILE" \
    --copy-source "$BUCKET/$TEST_FILE" \
    --endpoint-url "$S3_ENDPOINT" \
    &>/dev/null
print_test_result "CopyObject - 复制对象" $?

# 测试 11: 验证复制的对象存在
aws s3api head-object \
    --bucket "$BUCKET" \
    --key "copied-$TEST_FILE" \
    --endpoint-url "$S3_ENDPOINT" \
    &>/dev/null
print_test_result "验证复制的对象存在" $?

# ============================================
# 列表操作测试
# ============================================
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📋 列表操作测试"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 测试 12: 列出对象 (V2)
aws s3api list-objects-v2 \
    --bucket "$BUCKET" \
    --endpoint-url "$S3_ENDPOINT" \
    &>/dev/null
print_test_result "ListObjectsV2 - 列出对象 (V2)" $?

# 测试 13: 列出对象 (V1)
aws s3api list-objects \
    --bucket "$BUCKET" \
    --endpoint-url "$S3_ENDPOINT" \
    &>/dev/null
print_test_result "ListObjects - 列出对象 (V1)" $?

# ============================================
# HTTP 条件请求测试
# ============================================
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🔒 HTTP 条件请求测试"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 获取 ETag
ETAG=$(aws s3api head-object \
    --bucket "$BUCKET" \
    --key "$TEST_FILE" \
    --endpoint-url "$S3_ENDPOINT" \
    --query 'ETag' \
    --output text 2>/dev/null)

# 测试 14: If-None-Match (应返回 304)
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    -H "If-None-Match: $ETAG" \
    "$S3_ENDPOINT/$BUCKET/$TEST_FILE")
if [ "$HTTP_CODE" = "304" ]; then
    print_test_result "If-None-Match - 304 Not Modified" 0
else
    print_test_result "If-None-Match - 304 Not Modified (got $HTTP_CODE)" 1
fi

# 测试 15: If-Match (应成功)
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    -H "If-Match: $ETAG" \
    "$S3_ENDPOINT/$BUCKET/$TEST_FILE")
if [ "$HTTP_CODE" = "200" ]; then
    print_test_result "If-Match - 200 OK" 0
else
    print_test_result "If-Match - 200 OK (got $HTTP_CODE)" 1
fi

# ============================================
# Range 请求测试 (断点续传)
# ============================================
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "📦 Range 请求测试 (断点续传)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 测试 16: Range 请求
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    -H "Range: bytes=0-9" \
    "$S3_ENDPOINT/$BUCKET/$TEST_FILE")
if [ "$HTTP_CODE" = "206" ]; then
    print_test_result "Range 请求 - 206 Partial Content" 0
else
    print_test_result "Range 请求 - 206 Partial Content (got $HTTP_CODE)" 1
fi

# ============================================
# 分片上传测试
# ============================================
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🔀 分片上传测试 (Multipart Upload)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 测试 17: 初始化分片上传
UPLOAD_ID=$(aws s3api create-multipart-upload \
    --bucket "$BUCKET" \
    --key "$LARGE_FILE" \
    --endpoint-url "$S3_ENDPOINT" \
    --query 'UploadId' \
    --output text 2>/dev/null)
if [ -n "$UPLOAD_ID" ]; then
    print_test_result "InitiateMultipartUpload - 初始化分片上传" 0

    # 测试 18: 上传分片
    ETAG1=$(aws s3api upload-part \
        --bucket "$BUCKET" \
        --key "$LARGE_FILE" \
        --part-number 1 \
        --upload-id "$UPLOAD_ID" \
        --body "$LARGE_FILE" \
        --endpoint-url "$S3_ENDPOINT" \
        --query 'ETag' \
        --output text 2>/dev/null)

    if [ -n "$ETAG1" ]; then
        print_test_result "UploadPart - 上传分片" 0

        # 测试 19: 完成分片上传
        PARTS_JSON=$(cat <<EOF
{
  "Parts": [
    {
      "ETag": $ETAG1,
      "PartNumber": 1
    }
  ]
}
EOF
)
        aws s3api complete-multipart-upload \
            --bucket "$BUCKET" \
            --key "$LARGE_FILE" \
            --upload-id "$UPLOAD_ID" \
            --multipart-upload "$PARTS_JSON" \
            --endpoint-url "$S3_ENDPOINT" \
            &>/dev/null
        print_test_result "CompleteMultipartUpload - 完成分片上传" $?
    else
        print_test_result "UploadPart - 上传分片" 1
        # 测试 20: 取消分片上传
        aws s3api abort-multipart-upload \
            --bucket "$BUCKET" \
            --key "$LARGE_FILE" \
            --upload-id "$UPLOAD_ID" \
            --endpoint-url "$S3_ENDPOINT" \
            &>/dev/null
        print_test_result "AbortMultipartUpload - 取消分片上传" $?
    fi
else
    print_test_result "InitiateMultipartUpload - 初始化分片上传" 1
fi

# ============================================
# 批量删除测试
# ============================================
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🗑️  批量删除测试"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 测试 21: 批量删除对象
DELETE_JSON=$(cat <<EOF
{
  "Objects": [
    {"Key": "$TEST_FILE"},
    {"Key": "copied-$TEST_FILE"}
  ],
  "Quiet": false
}
EOF
)

echo "$DELETE_JSON" | aws s3api delete-objects \
    --bucket "$BUCKET" \
    --delete file:///dev/stdin \
    --endpoint-url "$S3_ENDPOINT" \
    &>/dev/null
print_test_result "DeleteObjects - 批量删除对象" $?

# ============================================
# 删除操作测试
# ============================================
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🗑️  删除操作测试"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# 删除剩余对象
if [ -n "$UPLOAD_ID" ] && [ -n "$ETAG1" ]; then
    aws s3api delete-object \
        --bucket "$BUCKET" \
        --key "$LARGE_FILE" \
        --endpoint-url "$S3_ENDPOINT" \
        &>/dev/null
fi

# 测试 22: 删除 Bucket
aws s3api delete-bucket \
    --bucket "$BUCKET" \
    --endpoint-url "$S3_ENDPOINT" \
    &>/dev/null
print_test_result "DeleteBucket - 删除 Bucket" $?

# ============================================
# 测试总结
# ============================================
echo ""
echo "========================================"
echo "测试总结"
echo "========================================"
echo -e "总测试数: ${YELLOW}$TOTAL_TESTS${NC}"
echo -e "通过: ${GREEN}$PASSED_TESTS${NC}"
echo -e "失败: ${RED}$FAILED_TESTS${NC}"
echo ""

# 清理
cleanup

if [ $FAILED_TESTS -eq 0 ]; then
    echo -e "${GREEN}✓ 所有测试通过！${NC}"
    exit 0
else
    echo -e "${RED}✗ 存在失败的测试${NC}"
    exit 1
fi
