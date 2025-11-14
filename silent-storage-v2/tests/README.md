# Storage V2 集成测试

本目录包含 Silent-NAS Storage V2 的集成测试，用于验证文件存储系统的完整功能。

## 测试覆盖

### ✅ test_basic_file_storage
测试基本的文件存储和读取功能。
- 保存文件
- 读取文件
- 验证数据一致性

### ✅ test_version_management
测试文件版本管理功能。
- 创建多个文件版本
- 读取指定版本
- 列出所有版本
- 验证版本数据正确性

### ✅ test_large_file_chunking
测试大文件的 CDC 分块功能。
- 保存大文件（100KB+）
- 验证自动分块
- 验证数据完整性

### ✅ test_deduplication
测试块级去重功能。
- 保存相同内容的多个文件
- 验证使用相同的块 ID
- 验证去重效果

### ✅ test_incremental_storage
测试文件的存储功能。
- 保存基础版本
- 保存修改版本
- 验证两个版本都能正确读取

### ✅ test_file_deletion_and_gc
测试文件删除和垃圾回收功能。
- 删除文件
- 执行垃圾回收
- 验证孤立块被清理
- 验证空间回收

### ✅ test_persistence_and_recovery
测试数据持久化和恢复功能。
- 保存数据后关闭存储
- 重新打开存储
- 验证数据完整恢复

### ✅ test_concurrent_operations
测试并发操作的安全性。
- 同时保存 10 个文件
- 验证所有文件都正确保存
- 验证并发读取正确性

### ✅ test_compression
测试数据压缩功能。
- 保存高度重复的数据
- 验证压缩生效
- 验证数据完整性

## 运行测试

### 运行所有集成测试
```bash
cd silent-storage-v2
cargo test --test storage_integration_test
```

### 运行单个测试
```bash
cargo test --test storage_integration_test test_basic_file_storage
```

### 显示测试输出
```bash
cargo test --test storage_integration_test -- --nocapture
```

### 运行并显示详细日志
```bash
RUST_LOG=debug cargo test --test storage_integration_test -- --nocapture
```

## 测试结果

当前所有测试都通过 ✅：

```
test test_basic_file_storage ... ok
test test_compression ... ok
test test_concurrent_operations ... ok
test test_deduplication ... ok
test test_file_deletion_and_gc ... ok
test test_incremental_storage ... ok
test test_large_file_chunking ... ok
test test_persistence_and_recovery ... ok
test test_version_management ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## 测试配置

测试使用以下配置：

- **压缩**: 启用 LZ4 压缩
- **分块大小**:
  - 最小: 1KB
  - 平均: 4KB
  - 最大: 16KB
- **去重**: 启用
- **临时目录**: 每个测试使用独立的临时目录，测试结束自动清理

## 注意事项

1. **测试隔离**: 每个测试使用独立的临时目录，互不干扰
2. **自动清理**: 测试完成后临时文件会自动清理
3. **Sled 数据库**: 测试会自动初始化 Sled 元数据数据库
4. **并发安全**: 并发测试验证了系统的线程安全性

## 添加新测试

在 `storage_integration_test.rs` 中添加新的测试函数：

```rust
#[tokio::test]
async fn test_new_feature() {
    let (storage, _temp_dir) = create_test_storage().await;

    // 测试逻辑...

    assert!(condition, "失败消息");
    println!("✅ 测试通过");
}
```

## 性能基准

测试结果显示：

- **基本文件操作**: < 50ms
- **大文件分块**: ~200ms (41KB)
- **并发操作**: 10 个并发任务 ~200ms
- **垃圾回收**: < 50ms
- **持久化恢复**: < 100ms
