# WebDAV 技术文档

本文档为开发者提供 Silent-NAS 中 WebDAV 实现的完整技术说明，包括模块架构、核心能力、客户端互通测试以及扩展报告的详细示例。

## 目录

1. [模块架构](#模块架构)
2. [核心能力](#核心能力)
3. [路由挂载](#路由挂载)
4. [锁与并发](#锁与并发)
5. [属性管理](#属性管理)
6. [版本报告](#版本报告)
7. [客户端互通](#客户端互通)
8. [报告扩展示例](#报告扩展示例)
9. [测试验证](#测试验证)

---

## 模块架构

### 模块划分（src/webdav）

- **constants.rs**
  - 方法常量：PROPFIND/PROPPATCH/LOCK/UNLOCK/MKCOL/MOVE/COPY/…
  - HTTP 与 XML 常量：Allow/DAV/Content-Type、XML 片段

- **types.rs**
  - DavLock：锁令牌、独占/过期时间，工具方法 new_exclusive/is_expired

- **handler.rs**
  - WebDavHandler：核心处理器，包含：
    - 状态持久化：load_persistent_state/persist_locks/persist_props（.webdav/*.json）
    - 锁辅助：parse_timeout/extract_if_lock_token/ensure_lock_ok
    - 路径工具：decode_path/build_full_href
    - Handler::call 方法，将请求分发至各子模块

- **files.rs**（资源操作）
  - PROPFIND/HEAD/GET/PUT/DELETE/MKCOL/MOVE/COPY
  - 工具：add_prop_response/extract_path_from_url/copy_dir_all

- **locks.rs**（锁）
  - LOCK/UNLOCK：独占锁、Timeout 解析、持久化、Lock-Token 返回

- **props.rs**（属性）
  - PROPPATCH：简化属性处理，属性持久化到 .webdav/props.json

- **deltav.rs**（版本报告）
  - VERSION-CONTROL：标记资源受控
  - REPORT：返回文件版本列表（基于 VersionManager）

- **routes.rs**（路由）
  - register_webdav_methods：注册所有 WebDAV 方法
  - create_webdav_routes：对外导出，供 main 使用

---

## 核心能力

### 基础资源操作
- 资源操作：HEAD/GET/PUT/DELETE/MKCOL/MOVE/COPY
- 目录/属性：PROPFIND（深度可选）、PROPPATCH（简化）
- 锁管理：LOCK/UNLOCK（独占锁、Lock-Token、Timeout=Second-N/Infinite）

### 版本能力（最小闭环）
- PUT 写入后创建版本（VersionManager）
- VERSION-CONTROL 标记
- REPORT 列出版本（version-name/version-created）

### v0.6.0+ 新增能力
- **ETag/Last-Modified**：GET/HEAD 返回 `ETag` 与 `Last-Modified`，并正确处理 `If-None-Match`
- **Depth: infinity**：`PROPFIND` 支持 `Depth: infinity` 递归枚举
- **OPTIONS DAV**：返回 `1, 2, ordered-collections`
- **REPORT**：新增 `sync-collection`（RFC 6578，简化实现，支持 `Depth: 1/infinity`）

---

## 路由挂载

```rust
use silent_nas::webdav::create_webdav_routes;

let route = create_webdav_routes(
    storage,              // Arc<StorageManager>
    notifier,             // Option<Arc<EventNotifier>>
    sync_manager,         // Arc<SyncManager>
    source_http_addr,     // String，如 http://host:port
    version_manager,      // Arc<VersionManager>
);
```

---

## 锁与并发

### 锁机制
- 客户端在修改类请求（PUT/PROPPATCH/MOVE/COPY）需要携带 `If: (<opaquelocktoken:…>)` 以通过 ensure_lock_ok 检查
- `Lock-Token` 以 `opaquelocktoken:` 格式返还
- `Timeout`/`Depth` 头生效（Depth=Infinity 记录于锁）

### Timeout 处理
- **Second-N**：解析 N，范围 [1, 3600]
- **Infinite**：按 3600s 处理，避免永久锁

### 锁冲突矩阵
支持 `<D:lockinfo>` 请求体解析：`<D:lockscope><D:exclusive|D:shared/>` 与 `<D:owner><D:href>...`

- **独占 vs 任意锁** → 423 Locked
- **共享 vs 独占** → 423 Locked
- **共享 vs 共享** → 允许，共存

### 条件请求（If）
- 解析资源标记 `<...> ( ... )` 与未标记列表；对当前路径筛选相关令牌
- 列表内可包含多个令牌，任一匹配即通过

### 持久化
- 锁/属性持久化：.webdav/locks.json、.webdav/props.json（进程重启后恢复）

---

## 属性管理

### PROPPATCH（简化实现）

当前版本未做 XML 命名空间下的完整属性解析，示例地写入 "prop:last-proppatch" 时间戳。

#### 扩展方向
- 解析/校验多命名空间属性
- 属性持久化结构化与校验

#### 属性存储
- 解析 `xmlns` 声明，属性除原始前缀键外，额外存储 `ns:{URI}#{local}` 结构化键，便于后续检索与校验

#### 当前实现示例

**设置属性：**
```xml
<D:propertyupdate xmlns:D="DAV:">
  <D:set><D:prop><Z:category xmlns:Z="urn:x-example">interop</Z:category></D:prop></D:set>
</D:propertyupdate>
```

**删除属性：**
```xml
<D:propertyupdate xmlns:D="DAV:">
  <D:remove><D:prop><Z:category xmlns:Z="urn:x-example"/></D:prop></D:remove>
</D:propertyupdate>
```

---

## 版本报告

### REPORT（简化实现）
- 通过路径查询文件 id 并返回版本列表（version-name/version-created）

### sync-collection
当请求体包含 `<sync-collection>` 时，返回 207 多状态，其中包含：
- `<D:sync-token>`：形如 `urn:sync:<scru128>:<local-time>`
- `<D:response>`：按 Depth 列出资源，包含 `getetag`/`getlastmodified`
- 支持 `<D:limit><D:nresults>` 与增量/删除差异（404）
- 支持 `<D:sync-token>` 基于时间戳的增量同步

### version-tree
返回给定资源的版本列表（DeltaV 简化）

### silent:filter（扩展）
按 `mime` 前缀、`modified-after/before`、`limit`、标签过滤（Depth:1）

### 扩展方向
- 更多 DeltaV 报告类型与过滤条件
- 基于版本索引与事件监听的差异同步优化

---

## 客户端互通

### 验证的客户端版本
- Cyberduck 8.x / Mountain Duck 4.x
- Nextcloud Desktop 3.x
- macOS Finder

### 发现与列目录
- PROPFIND 支持 Depth: 0/1/infinity
- 目录返回 `<D:collection/>`
- 文件返回 `getcontentlength/getcontenttype/getetag`
- Finder/Cyberduck 对 Server/DAV/Allow 头较敏感，已在响应中补足

### 互通建议
- **PROPFIND**：Depth=0/1/Infinity 常见客户端（Cyberduck/Nextcloud）用例验证
- **LOCK/UNLOCK**：校验 If/Lock-Token 头链路
- **PUT/DELETE/MOVE/COPY**：锁语义与错误码一致性

### 兼容性建议
- 若客户端要求绝对 href，可通过反向代理改写（当前实现返回相对路径以匹配 Finder 行为）
- 若需要差异同步（基于 sync-token），建议结合版本索引与事件监听扩展当前实现

---

## 报告扩展示例

### 1. sync-collection（增量 + 删除差异）

#### 请求（携带上次的 sync-token 与属性选择）

```xml
<D:sync-collection xmlns:D="DAV:">
  <D:sync-token>urn:sync:xxxx:2025-10-28 10:00:00</D:sync-token>
  <D:limit><D:nresults>100</D:nresults></D:limit>
  <D:prop>
    <D:displayname/>
    <D:getlastmodified/>
    <x:category xmlns:x="urn:x-example"/>
  </D:prop>
</D:sync-collection>
```

#### 响应（示意）

```xml
<D:multistatus xmlns:D="DAV:">
  <D:sync-token>urn:sync:yyyy:2025-10-28 10:05:00</D:sync-token>
  <D:response>
    <D:href>/docs/a.txt</D:href>
    <D:propstat><D:prop>
      <D:displayname>a.txt</D:displayname>
      <D:getlastmodified>Tue, 28 Oct 2025 10:03:00 GMT</D:getlastmodified>
      <x:category xmlns:x="urn:x-example">work</x:category>
    </D:prop></D:propstat>
  </D:response>
  <!-- 删除差异以 404 表示 -->
  <D:response>
    <D:href>/docs/old.txt</D:href>
    <D:status>HTTP/1.1 404 Not Found</D:status>
  </D:response>
  <!-- 移动差异以 301 + silent:moved-from 表示 -->
  <D:response>
    <D:href>/docs/new-name.txt</D:href>
    <D:status>HTTP/1.1 301 Moved Permanently</D:status>
    <silent:moved-from xmlns:silent="urn:silent-webdav">/docs/old-name.txt</silent:moved-from>
  </D:response>
</D:multistatus>
```

**说明：**
- 仅返回自 `<D:sync-token>` 之后的变化；删除以 404 项返回
- 移动以 301 + `<silent:moved-from>` 扩展字段回显来源路径（命名空间 `urn:silent-webdav`）
- 通过 `<D:prop>` 过滤返回的属性集合（含扩展属性）

### 2. version-tree（版本列表）

#### 请求

```xml
<D:version-tree xmlns:D="DAV:"/>
```

#### 响应（示例）

```xml
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/docs/a.txt</D:href>
    <D:propstat><D:prop>
      <D:version-name>v0001</D:version-name>
      <D:version-created>2025-10-28 09:00:00</D:version-created>
    </D:prop></D:propstat>
  </D:response>
  ...
</D:multistatus>
```

### 3. silent:filter（扩展过滤）

#### 请求（Depth:1，仅单层）

```xml
<silent:filter xmlns:silent="urn:silent-webdav">
  <silent:mime>image/</silent:mime>
  <silent:modified-after>2025-10-28T09:00:00Z</silent:modified-after>
  <silent:limit>50</silent:limit>
  <!-- 标签过滤：ns:{URI}#{local}[=value]，匹配存在或值相等 -->
  <silent:tag>ns:urn:x-example#category=work</silent:tag>
  <silent:tag>ns:urn:x-example#reviewed</silent:tag>
  <D:prop xmlns:D="DAV:">
    <D:displayname/>
    <x:category xmlns:x="urn:x-example"/>
  </D:prop>
</silent:filter>
```

#### 响应

响应为标准 `D:multistatus`，每项仅回显选择的属性。

**注意：**
- 标签过滤使用结构化属性键 `ns:{URI}#{local}`；若带 `=value` 则匹配等值
- 时间支持 RFC3339、RFC2822 或 `YYYY-MM-DD HH:MM:SS`

---

## 测试验证

### 快速验证（零配置）

使用提供的脚本运行端到端互通测试（会自动读取 config.toml 并在需要时启动本地服务）：

```bash
./scripts/webdav_interop_test.sh
```

### 测试覆盖流程
- PUT → PROPFIND → LOCK → PROPPATCH → GET → REPORT（版本列表）→ MOVE → UNLOCK → 清理

### 成功输出
- 终端输出：`OK: WebDAV 互通基础流程通过`
- 脚本会在退出时清理临时目录与启动的测试进程

---

## 开发注意事项

- **ID 生成**：所有 ID 使用 scru128 生成
- **时间处理**：统一使用 `chrono::Local::now().naive_local()`
- **指标与审计**：可结合全局 metrics/audit 模块扩展记录（当前模块内未直接上报）
- **扩展方向**：
  - 完整的 XML 命名空间属性解析
  - 更多 DeltaV 报告类型
  - 基于事件的增量同步优化
  - 更完善的锁超时与清理机制

---

## 参考资料

- [RFC 4918 - WebDAV](https://tools.ietf.org/html/rfc4918)
- [RFC 3253 - DeltaV](https://tools.ietf.org/html/rfc3253)
- [RFC 6578 - Collection Synchronization](https://tools.ietf.org/html/rfc6578)
- [WebDAV 用户指南](../webdav-guide.md)
