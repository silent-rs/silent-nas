# WebDAV 使用指南（Silent‑NAS）

本文汇总 Silent‑NAS 当前 WebDAV 能力、用法与最佳实践，便于快速对接 Finder、Cyberduck、Nextcloud 等客户端。

## 支持的协议与能力
- 基础方法：OPTIONS、PROPFIND、PROPPATCH、HEAD、GET、PUT、DELETE、MKCOL、MOVE、COPY
- 锁（LOCK/UNLOCK）：
  - 支持独占/共享锁（exclusive/shared）、owner(href)、Depth（infinity/0）与 Timeout
  - 资源被锁时需携带 If 条件（锁令牌或 ETag）
- 条件请求（If）：
  - 解析资源标记与多令牌；列表内 AND、列表间 OR；支持 Not 取反
  - 支持 Lock‑Token 与 ETag 条件（祖先 Depth: infinity 锁同样生效）
- 属性模型：
  - PROPPATCH 解析 xmlns 并存储结构化键 ns:{URI}#{local}
  - 禁止修改 DAV: 命名空间；属性值长度限制（≤4096）
  - 命名空间冲突检测（同 local 名在不同 URI 下且值不一致 → 409）
- 报告（REPORT）：
  - sync-collection：支持 <D:limit><D:nresults> 与 <D:sync-token>，返回增量与删除差异（404）
  - version-tree：返回资源的版本列表
  - silent:filter（扩展）：支持 mime 前缀、modified-after/before、limit、标签过滤（ns:{URI}#{local}[=value]）
- 属性选择：
  - 在 REPORT 请求体内用 <D:prop> 限定返回属性（标准/扩展均可）

## 典型用法

### 1) 发现与浏览（Finder/Cyberduck）
- 客户端发送 `PROPFIND /path`，Depth 可取 0/1/infinity
- 响应中包含：displayname、resourcetype、getcontentlength/getcontenttype、getetag、getlastmodified

### 2) 上下行文件
- 上传：`PUT /path/file.ext`（需要确保父目录存在或先 `MKCOL`）
- 下载：`GET /path/file.ext`
- 删除：`DELETE /path/file_or_dir`
- 移动/复制：`MOVE`/`COPY`，携带 `Destination: /target/path`

### 3) 锁与条件请求
- 上锁：
```xml
<lockinfo xmlns="DAV:">
  <lockscope><exclusive/></lockscope>
  <locktype><write/></locktype>
  <owner><href>user@example</href></owner>
  <!-- 请求头可带：Depth: infinity / Timeout: Second-300 -->
  <!-- 响应头返回：Lock-Token、Timeout -->
</lockinfo>
```
- 解锁：`UNLOCK`，携带 `Lock-Token: <opaquelocktoken:...>`
- 被锁资源后续请求需携带 If 条件，示例：
```
If: ( <opaquelocktoken:xxxx> )
If: ( "\"etag-value\"" )
```

### 4) PROPPATCH（设置自定义属性）
- 请求示例：
```xml
<D:propertyupdate xmlns:D="DAV:">
  <D:set>
    <D:prop>
      <x:category xmlns:x="urn:x-example">work</x:category>
      <x:reviewed.bool xmlns:x="urn:x-example">true</x:reviewed.bool>
    </D:prop>
  </D:set>
</D:propertyupdate>
```
- 约束：
  - 只读：DAV: 命名空间属性不允许修改
  - 类型：`.bool` 必须为 true/false；`.int` 必须为整数
  - 冲突：同名 `local` 存在不同 URI 且值不一致时返回 409

### 5) REPORT（同步/过滤/版本）
- 增量同步（sync-collection）：
```xml
<D:sync-collection xmlns:D="DAV:">
  <D:sync-token>urn:sync:...:2025-10-28 10:00:00</D:sync-token>
  <D:limit><D:nresults>100</D:nresults></D:limit>
  <D:prop>
    <D:displayname/>
    <x:category xmlns:x="urn:x-example"/>
  </D:prop>
</D:sync-collection>
```
- 版本树（version-tree）：
```xml
<D:version-tree xmlns:D="DAV:"/>
```
- 扩展过滤（silent:filter）：
```xml
<silent:filter xmlns:silent="urn:silent-webdav">
  <silent:mime>image/</silent:mime>
  <silent:modified-after>2025-10-28T09:00:00Z</silent:modified-after>
  <silent:limit>50</silent:limit>
  <silent:tag>ns:urn:x-example#category=work</silent:tag>
  <D:prop xmlns:D="DAV:">
    <D:displayname/>
    <x:category xmlns:x="urn:x-example"/>
  </D:prop>
</silent:filter>
```

## 最佳实践
- 建议客户端携带 ETag 做条件请求，减少并发写入冲突
- 大批量属性写入建议分批，避免单次 PROPPATCH 体过大
- 增量同步建议结合 sync-token，按需叠加 `limit` 控制流量

## 兼容性提示
- Finder 对响应头 `Server/DAV/Allow` 较敏感，已在实现中补足
- href 返回相对路径（/path 形式），若需绝对 URL 可在反向代理层做改写

