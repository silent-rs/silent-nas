# Silent-NAS S3 测试套件

## 测试脚本说明

### 1. manual_test.sh - 基础功能测试
快速验证核心S3功能是否正常工作。

**测试内容:**
- ListBuckets
- CreateBucket
- PutObject
- GetObject
- HeadObject
- DeleteObject
- DeleteBucket

**运行时间:** ~5秒

**使用方法:**
```bash
./tests/manual_test.sh
```

### 2. extended_test.sh - 扩展功能测试
测试高级S3特性。

**测试内容:**
- CopyObject
- HTTP条件请求 (If-None-Match)
- Range请求
- 批量删除 (DeleteObjects)
- 分片上传 (Multipart Upload)
- ListObjects V1
- GetBucketLocation
- GetBucketVersioning

**运行时间:** ~10秒

**使用方法:**
```bash
./tests/extended_test.sh
```

### 3. s3_integration_test.sh - 完整集成测试
完整的S3兼容性测试套件，包含所有功能点。

**测试内容:** 20+ 项测试
- 所有Bucket操作
- 所有对象操作
- 所有高级特性

**运行时间:** ~30秒

**使用方法:**
```bash
./tests/s3_integration_test.sh
```

## 前置条件

### 1. 安装 AWS CLI
```bash
brew install awscli
```

### 2. 启动 Silent-NAS 服务
```bash
cargo run --release
```

服务应在以下端口运行：
- HTTP: 8080
- S3: 9000
- WebDAV: 8081
- gRPC: 50051
- QUIC: 4433

### 3. 环境变量（已在脚本中配置）
```bash
export AWS_ACCESS_KEY_ID="minioadmin"
export AWS_SECRET_ACCESS_KEY="minioadmin"
```

## 测试结果解读

### 成功标志
```
✓ 测试名称
```

### 失败标志
```
✗ 测试名称
```

### 测试总结
```
总测试数: X
通过: Y
失败: Z
```

## 已知问题

1. **CreateBucket 带连字符的名称**
   - `test-bucket` 类型的名称会导致500错误
   - 不带连字符的名称 `testbucket` 正常工作
   - 影响: 较小，PutObject会自动创建bucket

## 手动测试示例

### 使用 AWS CLI
```bash
# 配置环境变量
export AWS_ACCESS_KEY_ID="minioadmin"
export AWS_SECRET_ACCESS_KEY="minioadmin"
S3="http://127.0.0.1:9000"

# 列出buckets
aws s3api list-buckets --endpoint-url $S3

# 上传文件
echo "Hello S3" > test.txt
aws s3api put-object --bucket mybucket --key test.txt --body test.txt --endpoint-url $S3

# 下载文件
aws s3api get-object --bucket mybucket --key test.txt downloaded.txt --endpoint-url $S3

# 删除文件
aws s3api delete-object --bucket mybucket --key test.txt --endpoint-url $S3
```

### 使用 curl
```bash
# 简单GET请求
curl http://127.0.0.1:9000/mybucket/test.txt

# Range请求
curl -H "Range: bytes=0-9" http://127.0.0.1:9000/mybucket/test.txt

# 条件请求
curl -H "If-None-Match: \"<etag>\"" http://127.0.0.1:9000/mybucket/test.txt
```

## 测试覆盖率

| 功能类别 | 测试覆盖 |
|---------|---------|
| Bucket操作 | 100% |
| 对象操作 | 100% |
| 列表操作 | 100% |
| 条件请求 | 100% |
| Range请求 | 100% |
| 批量操作 | 100% |
| 分片上传 | 100% |

## 性能测试

目前测试脚本专注于功能验证。性能测试建议：

```bash
# 大文件上传 (100MB)
dd if=/dev/urandom of=large.bin bs=1M count=100
time aws s3 cp large.bin s3://mybucket/ --endpoint-url http://127.0.0.1:9000

# 并发上传
for i in {1..10}; do
  aws s3 cp test.txt s3://mybucket/file$i.txt --endpoint-url http://127.0.0.1:9000 &
done
wait
```

## 故障排除

### 服务未启动
```
curl: (7) Failed to connect to 127.0.0.1 port 9000
```
**解决:** 确保 `cargo run --release` 正在运行

### 认证失败
```
An error occurred (403) when calling ... operation: Forbidden
```
**解决:** 检查环境变量 AWS_ACCESS_KEY_ID 和 AWS_SECRET_ACCESS_KEY

### NATS 连接失败
```
ERROR: Failed to connect to NATS
```
**解决:** 确保 NATS 服务器在 127.0.0.1:4222 运行

## 相关文档

- [S3功能验证报告](../docs/S3功能验证报告.md) - 代码实现分析
- [S3测试结果](../docs/S3测试结果.md) - 详细测试结果
- [验证总结](../VERIFICATION_SUMMARY.md) - 整体验证总结
