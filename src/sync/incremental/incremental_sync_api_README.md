# 增量同步 API 模块

## 概述

`incremental_sync_api.rs` 模块提供了增量同步功能的HTTP API处理逻辑，将复杂的业务逻辑从`main.rs`中分离出来，提高代码的可测试性和可维护性。

## 模块结构

### 函数

#### `handle_get_signature`
获取文件的签名信息。

**参数**：
- `handler: &IncrementalSyncHandler` - 增量同步处理器
- `file_id: &str` - 文件ID

**返回**：`Result<FileSignature>` - 文件签名信息

**用途**：在main.rs的HTTP处理函数中被调用，用于计算并返回文件的块级签名。

#### `handle_get_delta`
生成文件的差异块。

**参数**：
- `handler: &IncrementalSyncHandler` - 增量同步处理器
- `file_id: &str` - 文件ID
- `target_signature: &FileSignature` - 目标文件的签名

**返回**：`Result<Vec<DeltaChunk>>` - 差异块列表

**用途**：在main.rs的HTTP处理函数中被调用，用于生成需要传输的差异块。

## 设计原则

1. **关注点分离**：将业务逻辑与HTTP处理分离
2. **易于测试**：独立的函数便于编写单元测试
3. **简单清晰**：每个函数职责单一，易于理解和维护

## 使用示例

在`main.rs`中的使用方式：

```rust
// 创建增量同步处理器
let inc_sync_handler = Arc::new(IncrementalSyncHandler::new(storage.clone(), 64 * 1024));

// 定义HTTP处理函数
let get_file_signature = move |req: Request| {
    let handler = inc_sync_handler.clone();
    async move {
        let file_id: String = req.get_path_params("id")?;
        let signature = incremental_sync_api::handle_get_signature(&handler, &file_id)
            .await
            .map_err(|e| { /* 错误处理 */ })?;
        Ok(serde_json::to_value(signature).unwrap())
    }
};

// 添加到路由
Route::new("sync/signature/<id>").get(get_file_signature)
```

## 测试

模块包含完整的单元测试：

```bash
cargo test --bin silent-nas incremental_sync_api::tests
```

测试覆盖：
- `test_handle_get_signature` - 测试签名计算
- `test_handle_get_delta` - 测试差异块生成

## API端点

配合main.rs中的路由，提供以下HTTP端点：

- `GET /api/sync/signature/{id}` - 获取文件签名
- `POST /api/sync/delta/{id}` - 获取文件差异块（请求体包含target_signature）

## 未来改进

- [ ] 添加请求参数验证
- [ ] 支持更多的差异算法
- [ ] 添加性能指标收集
- [ ] 支持批量文件处理
