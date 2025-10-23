# WebDAV 模块说明

本文档概述 Silent‑NAS 中 WebDAV 的模块划分、主要能力与使用要点。

## 模块划分（src/webdav）
- constants.rs
  - 方法常量：PROPFIND/PROPPATCH/LOCK/UNLOCK/MKCOL/MOVE/COPY/…
  - HTTP 与 XML 常量：Allow/DAV/Content‑Type、XML 片段
- types.rs
  - DavLock：锁令牌、独占/过期时间，工具方法 new_exclusive/is_expired
- handler.rs
  - WebDavHandler：核心处理器，包含：
    - 状态持久化：load_persistent_state/persist_locks/persist_props（.webdav/*.json）
    - 锁辅助：parse_timeout/extract_if_lock_token/ensure_lock_ok
    - 路径工具：decode_path/build_full_href
    - Handler::call 方法，将请求分发至各子模块
- files.rs（资源操作）
  - PROPFIND/HEAD/GET/PUT/DELETE/MKCOL/MOVE/COPY
  - 工具：add_prop_response/extract_path_from_url/copy_dir_all
- locks.rs（锁）
  - LOCK/UNLOCK：独占锁、Timeout 解析、持久化、Lock‑Token 返回
- props.rs（属性）
  - PROPPATCH：简化属性处理，属性持久化到 .webdav/props.json
- deltav.rs（版本报告）
  - VERSION‑CONTROL：标记资源受控
  - REPORT：返回文件版本列表（基于 VersionManager）
- routes.rs（路由）
  - register_webdav_methods：注册所有 WebDAV 方法
  - create_webdav_routes：对外导出，供 main 使用

## 能力概览
- 资源操作：HEAD/GET/PUT/DELETE/MKCOL/MOVE/COPY
- 目录/属性：PROPFIND（深度可选）、PROPPATCH（简化）
- 锁管理：LOCK/UNLOCK（独占锁、Lock‑Token、Timeout=Second‑N/Infinite）
- 版本能力（最小闭环）：
  - PUT 写入后创建版本（VersionManager）
  - VERSION‑CONTROL 标记
  - REPORT 列出版本（version‑name/version‑created）

## 路由挂载
```
use silent_nas::webdav::create_webdav_routes;

let route = create_webdav_routes(
    storage,              // Arc<StorageManager>
    notifier,             // Option<Arc<EventNotifier>>
    sync_manager,         // Arc<SyncManager>
    source_http_addr,     // String，如 http://host:port
    version_manager,      // Arc<VersionManager>
);
```

## 快速验证（零配置）
- 使用提供的脚本运行端到端互通测试（会自动读取 config.toml 并在需要时启动本地服务）：

```
./scripts/webdav_interop_test.sh
```

- 流程覆盖：PUT → PROPFIND → LOCK → PROPPATCH → GET → REPORT（版本列表）→ MOVE → UNLOCK → 清理
- 成功后终端输出：OK: WebDAV 互通基础流程通过
- 脚本会在退出时清理临时目录与启动的测试进程

## 锁与并发
- 客户端在修改类请求（PUT/PROPPATCH/MOVE/COPY）需要携带 If: (<opaquelocktoken:…>) 以通过 ensure_lock_ok 检查
- Timeout:
  - Second‑N：解析 N，范围 [1, 3600]
  - Infinite：按 3600s 处理，避免永久锁
- 锁/属性持久化：.webdav/locks.json、.webdav/props.json（进程重启后恢复）

## PROPPATCH（简化）
- 当前版本未做 XML 命名空间下的完整属性解析，示例地写入 "prop:last-proppatch" 时间戳
- 后续可扩展：
  - 解析/校验多命名空间属性
  - 属性持久化结构化与校验

> 当前实现：支持 <set>/<remove> 简化用法，典型示例：
>
> 设置属性：
> ```xml
> <D:propertyupdate xmlns:D="DAV:">
>   <D:set><D:prop><Z:category xmlns:Z="urn:x-example">interop</Z:category></D:prop></D:set>
> </D:propertyupdate>
> ```
> 删除属性：
> ```xml
> <D:propertyupdate xmlns:D="DAV:">
>   <D:remove><D:prop><Z:category xmlns:Z="urn:x-example"/></D:prop></D:remove>
> </D:propertyupdate>
> ```

## REPORT（简化）
- 通过路径查询文件 id 并返回版本列表（version‑name/version‑created）
- 后续可扩展：更多 DeltaV 报告类型与过滤条件

## 互通建议
- PROPFIND：Depth=0/1/Infinity 常见客户端（Cyberduck/Nextcloud）用例验证
- LOCK/UNLOCK：校验 If/Lock‑Token 头链路
- PUT/DELETE/MOVE/COPY：锁语义与错误码一致性

## 注意事项
- 所有 ID 使用 scru128 生成
- 如需时间，统一使用 chrono::Local::now().naive_local()
- 指标与审计：可结合全局 metrics/audit 模块扩展记录（当前模块内未直接上报）
