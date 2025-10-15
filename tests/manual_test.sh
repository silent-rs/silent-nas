#!/bin/bash

# 简化的手动测试脚本
set -x

export AWS_ACCESS_KEY_ID="minioadmin"
export AWS_SECRET_ACCESS_KEY="minioadmin"
S3="http://127.0.0.1:9000"

echo "=== 测试 1: ListBuckets ==="
aws s3api list-buckets --endpoint-url $S3

echo -e "\n=== 测试 2: CreateBucket ==="
aws s3api create-bucket --bucket test-bucket --endpoint-url $S3

echo -e "\n=== 测试 3: HeadBucket ==="
aws s3api head-bucket --bucket test-bucket --endpoint-url $S3

echo -e "\n=== 测试 4: PutObject ==="
echo "Hello S3" > test.txt
aws s3api put-object --bucket test-bucket --key test.txt --body test.txt --endpoint-url $S3

echo -e "\n=== 测试 5: HeadObject ==="
aws s3api head-object --bucket test-bucket --key test.txt --endpoint-url $S3

echo -e "\n=== 测试 6: GetObject ==="
aws s3api get-object --bucket test-bucket --key test.txt downloaded.txt --endpoint-url $S3
cat downloaded.txt

echo -e "\n=== 测试 7: ListObjectsV2 ==="
aws s3api list-objects-v2 --bucket test-bucket --endpoint-url $S3

echo -e "\n=== 测试 8: DeleteObject ==="
aws s3api delete-object --bucket test-bucket --key test.txt --endpoint-url $S3

echo -e "\n=== 测试 9: DeleteBucket ==="
aws s3api delete-bucket --bucket test-bucket --endpoint-url $S3

# 清理
rm -f test.txt downloaded.txt

echo -e "\n=== 所有测试完成 ==="
