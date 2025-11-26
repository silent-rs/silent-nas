# WebDAV 需求整理（Finder 兼容）

目标：macOS Finder 可直接通过 “连接服务器 (⌘K)” 挂载并读写。

## 协议与路由
- 基础路径：`/`（根路径挂载，例如：`http://host:port/file.txt` 可直接访问根目录下文件）。
- DAV 能力：`DAV: 1, 2, ordered-collections`。
- Allow 方法：`OPTIONS, GET, HEAD, PUT, DELETE, PROPFIND, PROPPATCH, MKCOL, MOVE, COPY, LOCK, UNLOCK, VERSION-CONTROL, REPORT`。

## PROPFIND 响应
- 必含命名空间根：`<D:multistatus xmlns:D="DAV:">`。
- 资源属性：
  - `D:displayname`
  - `D:resourcetype`（目录含 `<D:collection/>`）
  - `D:getcontentlength`：仅文件返回实际大小；目录不返回该字段（Finder 要求）
  - `D:creationdate`（ISO8601）
  - `D:getlastmodified`（HTTP-date, GMT）
  - `D:supportedlock`：可选；为兼容 Finder 初始握手可暂不返回
- 支持 `Depth: 0/1/infinity`。
 - 支持请求体内 `<D:prop>` 属性选择（标准/扩展属性）。
 - 扩展属性（自定义命名空间）在响应中尽量回显客户端在 `<D:prop>` 中声明的 xmlns 前缀；未声明时使用 `x` 前缀。

### 命名空间与 href 规则（Finder 关键要求）
- DAV 命名空间使用带前缀格式：在根元素声明 `xmlns:D="DAV:"`，所有 DAV 元素使用 `D:` 前缀（如 `D:multistatus/D:response/D:resourcetype/D:collection`）。
- `D:href` 必须为相对路径（不包含 schema/host/port）。
- 目录必须以尾斜杠结尾：`/`、`/dir/`。
- 根目录使用 `/`。

## 锁（Class 2 WebDAV）
- `LOCK`：返回 200，响应体包含 `lockdiscovery`，响应头返回 `Lock-Token: <opaquelocktoken:...>`。
- `UNLOCK`：校验 `Lock-Token` 头，成功移除锁。
- 锁 Token：使用 `scru128` 生成，形如 `opaquelocktoken:<scru128>`。

## 其他兼容性
- `HEAD`/`GET`：文件返回 `Content-Length`、`ETag`、`Last-Modified`，并声明 `Accept-Ranges: bytes`。
- 目录 `GET`：返回简单 HTML（提示使用 PROPFIND）。
 - REPORT 增量同步（sync-collection）：
   - 删除以 `404` 形式返回；
   - 移动以 `301 Moved Permanently` + `<silent:moved-from>`（命名空间 `urn:silent-webdav`）返回来源路径。

## 测试脚本期望
- `scripts/finder_webdav_test.sh`：
  - 从 `Lock-Token` 响应头获取锁令牌；
  - 回退时从 body 解析，正则需覆盖 `[0-9A-Za-z-]`（兼容 scru128）。
  - `LOCK/UNLOCK` 以 HTTP 状态码判定成功。

## 约束与实现约定
- ID 一律使用 `scru128`。
- 涉及 DAV 时间字段：遵循 HTTP 规范；创建/修改时间取文件系统时间。
- 不改变已有对外行为的前提下，最小化改动以提升 Finder 兼容性。
