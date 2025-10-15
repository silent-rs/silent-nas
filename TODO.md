# Silent-NAS 待办事项

## S3 API 路由问题 [高优先级]

### 问题描述
新增的S3 API（GetBucketLocation、GetBucketVersioning、DeleteObjects）已实现，但存在路由匹配问题：

- `GET /bucket?location` 返回 404 Not Found
- `GET /bucket?versioning` 返回 404 Not Found
- `POST /bucket?delete` 返回 method not allowed

### 已实现的API代码
✅ DeleteObjects - 批量删除对象（POST /bucket?delete）
✅ GetBucketLocation - 获取bucket位置（GET /bucket?location）
✅ GetBucketVersioning - 获取版本控制状态（GET /bucket?versioning）

### 路由配置
当前路由结构：
```rust
Route::new_root().get(root_handler).append(
    Route::new("<bucket>")
        .get(bucket_handler)
        .put(put_bucket)
        .delete(delete_bucket)
        .post(bucket_handler_post)
        .append(Route::new("<key:**>")...)
)
```

### 可能的原因
1. Silent框架的动态路由参数`<bucket>`可能不支持查询参数
2. 路由优先级问题，请求被对象路由捕获
3. POST方法配置问题

### 需要调试
- [ ] 测试不带查询参数的bucket请求是否能匹配
- [ ] 检查Silent框架路由文档
- [ ] 尝试不同的路由配置方式
- [ ] 添加调试日志查看路由匹配过程

### 临时方案
可以考虑：
1. 使用中间件在对象路由前拦截bucket级别的请求
2. 修改路由结构，为bucket操作添加专门的路径前缀
3. 检查是否需要更新Silent框架版本

## 其他待办
- [ ] 实现Multipart Upload（分片上传）
- [ ] 完善认证机制
- [ ] 添加更多S3兼容测试
- [ ] 性能优化
