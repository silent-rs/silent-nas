# 存储性能测试工具

本目录包含用于对比普通存储(v0.6)与增量分片存储(v0.7)性能的测试工具。

## 测试脚本

### 1. 快速性能测试

```bash
./scripts/quick_storage_benchmark.sh
```

运行快速性能测试，包括：
- 写入性能对比（1MB文件）
- 读取性能对比（1MB文件）
- 去重效率测试

**耗时**: 约 30 秒 - 1 分钟

### 2. 完整性能测试

```bash
./scripts/storage_performance_comparison.sh
```

运行完整的性能对比测试套件，包括：
- 写入速度测试（多种文件大小）
- 读取速度测试
- 存储空间效率测试（去重率）
- 增量更新性能测试
- 并发操作性能测试
- 内存使用对比

**耗时**: 约 10-15 分钟

#### 可选参数

```bash
# 仅测试写入性能
./scripts/storage_performance_comparison.sh --write-only

# 仅测试读取性能
./scripts/storage_performance_comparison.sh --read-only

# 仅测试存储效率
./scripts/storage_performance_comparison.sh --storage-only

# 仅测试增量更新
./scripts/storage_performance_comparison.sh --incremental-only

# 仅测试并发性能
./scripts/storage_performance_comparison.sh --concurrent-only

# 仅测试内存使用
./scripts/storage_performance_comparison.sh --memory-only

# 清理测试数据
./scripts/storage_performance_comparison.sh --clean

# 仅生成测试数据
./scripts/storage_performance_comparison.sh --generate-data

# 显示帮助
./scripts/storage_performance_comparison.sh --help
```

## Rust 测试模块

除了 shell 脚本，还可以直接运行 Rust 测试：

```bash
# 运行所有性能测试
cargo test --release --test storage_performance_test -- --nocapture

# 运行特定测试
cargo test --release --test storage_performance_test test_v2_write_performance -- --nocapture

# 运行完整对比（忽略的测试）
cargo test --release --test storage_performance_test benchmark_full_comparison -- --ignored --nocapture
```

### 可用的测试

- `test_v1_write_performance` - v0.6 写入性能
- `test_v2_write_performance` - v0.7 写入性能
- `test_v1_read_performance` - v0.6 读取性能
- `test_v2_read_performance` - v0.7 读取性能
- `test_storage_efficiency` - 存储空间效率（去重）
- `test_incremental_update_performance` - 增量更新性能
- `test_concurrent_operations` - 并发操作性能
- `benchmark_full_comparison` - 完整对比测试（需要 --ignored）

## 测试结果

测试结果将保存在 `benchmark_results/` 目录下，文件名格式为：
```
comparison_YYYYMMDD_HHMMSS.md
```

结果报告包含：
- 测试环境信息
- 各项性能指标对比表格
- 性能改进百分比
- 测试结论和建议

## 测试数据

测试数据存储在 `test_data/` 目录，包含不同大小的测试文件：
- 1KB
- 10KB
- 100KB
- 1MB
- 10MB

每种大小生成 10 个测试文件。

## 性能指标说明

### 1. 写入速度
测量单次完整写入操作的耗时和吞吐量（MB/s）。

### 2. 读取速度
测量单次完整读取操作的耗时和吞吐量（MB/s）。

### 3. 存储效率
测量实际占用的磁盘空间，计算去重率：
```
去重率 = (v1存储大小 - v2存储大小) / v1存储大小 × 100%
```

### 4. 增量更新
测量文件修改后生成增量差异的耗时，以及增量大小占比。

### 5. 并发性能
测量不同并发级别（1, 4, 8, 16, 32）下的操作吞吐量（ops/s）。

### 6. 内存使用
测量各种操作（写入、读取、更新、删除）的峰值内存占用。

## 示例输出

```
=== 写入速度对比 ===

| 文件大小 | 普通存储 (ms) | 分片存储 (ms) | 改进 (%) |
|---------|--------------|--------------|----------|
| 1KB     | 5            | 3            | 40.00    |
| 10KB    | 12           | 8            | 33.33    |
| 100KB   | 45           | 28           | 37.78    |
| 1MB     | 156          | 98           | 37.18    |
| 10MB    | 1520         | 980          | 35.53    |

=== 存储效率对比 ===

| 文件大小 | 原始大小 (KB) | 普通存储 (KB) | 分片存储 (KB) | 去重率 (%) |
|---------|--------------|--------------|--------------|-----------|
| 1MB     | 10240        | 10240        | 1024         | 90.00     |
```

## 注意事项

1. **环境要求**
   - 需要安装 Rust 工具链
   - 建议在 release 模式下运行测试
   - 确保有足够的磁盘空间（至少 500MB）

2. **测试干扰**
   - 测试期间避免运行其他高 I/O 负载程序
   - 多次运行取平均值以获得更准确的结果
   - 首次运行可能较慢（需要编译）

3. **清理**
   - 测试完成后可运行清理命令：
     ```bash
     ./scripts/storage_performance_comparison.sh --clean
     ```

4. **自定义**
   - 可修改脚本中的配置项：
     - `FILE_SIZES` - 测试文件大小
     - `NUM_FILES` - 每种大小的文件数量
     - `NUM_ITERATIONS` - 测试迭代次数

## 故障排查

### 问题：编译失败
**解决**: 确保依赖安装完整
```bash
cargo check --all-features
```

### 问题：测试超时
**解决**: 增加超时时间或减少测试数据量

### 问题：权限错误
**解决**: 确保脚本有执行权限
```bash
chmod +x scripts/*.sh
```
