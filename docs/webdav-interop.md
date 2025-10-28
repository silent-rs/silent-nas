# WebDAV 互通用例（Cyberduck / Nextcloud）

本页记录与常见客户端的互通要点与已验证结果，便于排障与回归。

- 客户端版本
  - Cyberduck 8.x / Mountain Duck 4.x
  - Nextcloud Desktop 3.x

- 发现与列目录
  - PROPFIND 支持 Depth: 0/1/infinity，目录返回 `<D:collection/>`，文件返回 `getcontentlength/getcontenttype/getetag`。
  - Finder/Cyberduck 对 Server/DAV/Allow 头较敏感，已在响应中补足。

- 锁（LOCK/UNLOCK）
  - 支持 `<D:lockinfo>` 请求体解析：`<D:lockscope><D:exclusive|D:shared/>` 与 `<D:owner><D:href>...`。
  - 共享锁可并发，多独占与共享冲突矩阵：
    - 独占 vs 任意锁 → 423 Locked
    - 共享 vs 独占 → 423 Locked
    - 共享 vs 共享 → 允许，共存
  - `Lock-Token` 以 `opaquelocktoken:` 返还；`Timeout`/`Depth` 头生效（Depth=Infinity 记录于锁）。

- 条件请求（If）
  - 解析资源标记 `<...> ( ... )` 与未标记列表；对当前路径筛选相关令牌。
  - 列表内可包含多个令牌，任一匹配即通过。

- PROPPATCH
  - 解析 `xmlns` 声明，属性除原始前缀键外，额外存储 `ns:{URI}#{local}` 结构化键，便于后续检索与校验。

- REPORT
  - 支持 `sync-collection` 基础能力，解析 `<D:limit><D:nresults>` 作为数量过滤。

- 兼容性建议
  - 若客户端要求绝对 href，可通过反向代理改写（当前实现返回相对路径以匹配 Finder 行为）。
  - 若需要差异同步（基于 sync-token），建议结合版本索引与事件监听扩展当前实现。

