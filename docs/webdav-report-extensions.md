# WebDAV 报告扩展示例

本文提供 sync-collection、version-tree 与 silent:filter 的请求/响应示例，并说明属性选择 <D:prop> 与标签过滤的用法。

## 1. sync-collection（增量 + 删除差异）

请求（携带上次的 `<D:sync-token>` 与属性选择）：

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

响应（示意）：

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
</D:multistatus>
```

说明：
- 仅返回自 `<D:sync-token>` 之后的变化；删除以 404 项返回。
- 通过 `<D:prop>` 过滤返回的属性集合（含扩展属性）。

## 2. version-tree（版本列表）

请求：

```xml
<D:version-tree xmlns:D="DAV:"/>
```

响应（示例）：

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

## 3. silent:filter（扩展过滤）

请求（Depth:1，仅单层）：

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

响应为标准 `D:multistatus`，每项仅回显选择的属性。

注意：
- 标签过滤使用结构化属性键 `ns:{URI}#{local}`；若带 `=value` 则匹配等值。
- 时间支持 RFC3339、RFC2822 或 `YYYY-MM-DD HH:MM:SS`。

