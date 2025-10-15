#!/bin/bash

# 扩展功能测试
set -x

export AWS_ACCESS_KEY_ID="minioadmin"
export AWS_SECRET_ACCESS_KEY="minioadmin"
S3="http://127.0.0.1:9000"
BUCKET="testbucket"

echo "=== 准备测试环境 ==="
echo "Test file content" > test1.txt
echo "Another test file" > test2.txt
dd if=/dev/urandom of=large.bin bs=1M count=5 2>/dev/null

echo -e "\n=== 测试 1: CopyObject ==="
aws s3api put-object --bucket $BUCKET --key original.txt --body test1.txt --endpoint-url $S3
aws s3api copy-object --bucket $BUCKET --key copied.txt --copy-source "$BUCKET/original.txt" --endpoint-url $S3
aws s3api head-object --bucket $BUCKET --key copied.txt --endpoint-url $S3

echo -e "\n=== 测试 2: HTTP 条件请求 - If-None-Match ==="
ETAG=$(aws s3api head-object --bucket $BUCKET --key original.txt --endpoint-url $S3 --query 'ETag' --output text)
echo "ETag: $ETAG"
curl -s -o /dev/null -w "HTTP Status: %{http_code}\n" -H "If-None-Match: $ETAG" http://127.0.0.1:9000/$BUCKET/original.txt

echo -e "\n=== 测试 3: Range 请求 ==="
curl -s -H "Range: bytes=0-4" http://127.0.0.1:9000/$BUCKET/original.txt
echo ""

echo -e "\n=== 测试 4: 批量删除 ==="
aws s3api put-object --bucket $BUCKET --key file1.txt --body test1.txt --endpoint-url $S3
aws s3api put-object --bucket $BUCKET --key file2.txt --body test2.txt --endpoint-url $S3
aws s3api delete-objects --bucket $BUCKET --delete '{"Objects":[{"Key":"file1.txt"},{"Key":"file2.txt"}]}' --endpoint-url $S3

echo -e "\n=== 测试 5: 分片上传 ==="
UPLOAD_ID=$(aws s3api create-multipart-upload --bucket $BUCKET --key multipart.bin --endpoint-url $S3 --query 'UploadId' --output text)
echo "Upload ID: $UPLOAD_ID"

if [ -n "$UPLOAD_ID" ]; then
    ETAG1=$(aws s3api upload-part --bucket $BUCKET --key multipart.bin --part-number 1 --upload-id "$UPLOAD_ID" --body large.bin --endpoint-url $S3 --query 'ETag' --output text)
    echo "Part 1 ETag: $ETAG1"

    aws s3api complete-multipart-upload --bucket $BUCKET --key multipart.bin --upload-id "$UPLOAD_ID" --multipart-upload "{\"Parts\":[{\"ETag\":$ETAG1,\"PartNumber\":1}]}" --endpoint-url $S3

    aws s3api head-object --bucket $BUCKET --key multipart.bin --endpoint-url $S3
fi

echo -e "\n=== 测试 6: ListObjects V1 ==="
aws s3api list-objects --bucket $BUCKET --endpoint-url $S3

echo -e "\n=== 测试 7: GetBucketLocation ==="
aws s3api get-bucket-location --bucket $BUCKET --endpoint-url $S3

echo -e "\n=== 测试 8: GetBucketVersioning ==="
aws s3api get-bucket-versioning --bucket $BUCKET --endpoint-url $S3

echo -e "\n=== 清理 ==="
# 删除所有对象
aws s3 rm s3://$BUCKET --recursive --endpoint-url $S3

rm -f test1.txt test2.txt large.bin

echo -e "\n=== 扩展测试完成 ==="
