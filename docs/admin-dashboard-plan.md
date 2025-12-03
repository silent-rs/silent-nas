# Silent-NAS 管理端开发计划

## 项目概述

为 Silent-NAS 开发基于 Web 的管理控制台，提供可视化的系统管理、文件管理、用户管理、监控和配置功能。

## 技术栈选型

### 前端框架
- **框架**: Vue 3 (Composition API)
- **构建工具**: Vite 5
- **包管理**: yarn
- **UI 框架**: Element Plus（成熟的 Vue 3 UI 组件库）
- **状态管理**: Pinia
- **路由**: Vue Router 4
- **HTTP 客户端**: Axios
- **图表库**: ECharts（用于监控面板）
- **Markdown 渲染**: markdown-it（用于文件预览）

### 开发工具
- **TypeScript**: 类型安全
- **ESLint**: 代码规范检查
- **Prettier**: 代码格式化
- **Sass/SCSS**: CSS 预处理器

### 后端支持
- 复用现有 HTTP API（`src/http/`）
- 需要新增的管理端专用 API 端点
- JWT Token 认证（复用 `src/auth/jwt.rs`）

## 功能规划

### 第一阶段：基础框架与登录认证（v0.8.0）

#### 1.1 项目初始化
- [ ] 创建 `admin-dashboard` 前端项目目录
- [ ] 配置 Vite + Vue 3 + TypeScript
- [ ] 配置 Element Plus UI 库
- [ ] 配置 Pinia 状态管理
- [ ] 配置 Vue Router 路由
- [ ] 配置 Axios 和 API 请求封装
- [ ] 配置开发环境代理（指向后端服务）

#### 1.2 基础布局与导航
- [ ] 实现主布局组件（侧边栏 + 顶栏 + 内容区）
- [ ] 实现侧边栏菜单组件（可折叠）
- [ ] 实现顶栏组件（用户信息、退出登录）
- [ ] 实现面包屑导航
- [ ] 实现响应式布局（适配桌面和平板）

#### 1.3 用户认证与授权
- [ ] 登录页面（用户名/密码登录）
- [ ] 登录状态管理（Pinia store）
- [ ] Token 存储与自动刷新
- [ ] 路由守卫（未登录跳转登录页）
- [ ] 基于角色的权限控制（Admin/User/ReadOnly）
- [ ] 退出登录功能

**验收标准**：
- 用户可以通过 Web 界面登录系统
- 登录后显示管理控制台主界面
- Token 过期自动跳转登录页
- 不同角色看到不同的菜单权限

---

### 第二阶段：仪表盘与监控（v0.8.1）

#### 2.1 系统仪表盘
- [ ] 系统概览卡片
  - 存储空间使用情况（总容量/已用/可用）
  - 文件总数统计
  - 用户总数统计
  - 在线节点数（多节点部署时）
- [ ] 实时性能指标
  - CPU 使用率
  - 内存使用率
  - 网络流量（上传/下载）
  - 请求 QPS
- [ ] 最近活动日志（最近 10 条操作记录）
- [ ] 快捷操作入口

#### 2.2 监控面板
- [ ] 集成 Prometheus 指标
- [ ] 存储性能图表（读写吞吐量）
- [ ] 请求延迟分布图
- [ ] API 调用统计（各端点调用次数）
- [ ] WebDAV/S3/HTTP 协议使用统计
- [ ] 节点健康状态（多节点部署时）
- [ ] 自定义时间范围查询（最近 1 小时/24 小时/7 天/30 天）

**验收标准**：
- 仪表盘页面加载时间 < 2 秒
- 监控数据实时更新（30 秒轮询或 WebSocket）
- 图表渲染流畅，支持缩放和时间范围选择
- 移动端可正常查看仪表盘

---

### 第三阶段：文件管理（v0.8.2）

#### 3.1 文件浏览器
- [ ] 文件列表视图（表格模式/网格模式切换）
- [ ] 文件夹树形导航
- [ ] 文件/文件夹详情展示
  - 文件名、大小、修改时间、创建者
  - 文件类型图标
  - 文件权限信息
- [ ] 分页加载（虚拟滚动优化大目录）
- [ ] 排序和筛选（按名称/大小/时间/类型）
- [ ] 搜索功能（基于 `src/search/`）

#### 3.2 文件操作
- [ ] 上传文件（支持拖拽上传）
- [ ] 下载文件
- [ ] 删除文件/文件夹
- [ ] 重命名
- [ ] 移动/复制文件
- [ ] 新建文件夹
- [ ] 批量操作（批量删除/移动）
- [ ] 上传进度显示
- [ ] 大文件上传（分片上传，基于 WebDAV 上传会话）

#### 3.3 文件预览
- [ ] 图片预览（支持缩放）
- [ ] 文本文件预览（代码高亮）
- [ ] Markdown 文件预览（渲染）
- [ ] PDF 预览
- [ ] 视频/音频预览

#### 3.4 版本管理
- [ ] 文件版本列表
- [ ] 版本对比（文本文件）
- [ ] 版本恢复
- [ ] 版本删除

**验收标准**：
- 支持 1000+ 文件的目录快速加载
- 文件上传支持断点续传
- 预览功能支持常见文件类型（图片/文本/Markdown/PDF）
- 文件操作响应时间 < 500ms

---

### 第四阶段：用户与权限管理（v0.8.3）

#### 4.1 用户管理
- [ ] 用户列表页面
- [ ] 用户搜索和筛选
- [ ] 创建新用户
- [ ] 编辑用户信息
- [ ] 删除用户
- [ ] 修改用户密码（管理员操作）
- [ ] 用户角色管理（Admin/User/ReadOnly）
- [ ] 用户状态管理（启用/禁用）

#### 4.2 权限管理
- [ ] 角色权限配置界面
- [ ] 路径级 ACL 配置（路径权限设置）
- [ ] 用户配额管理（存储空间限制）
- [ ] API Token 管理（为用户生成/撤销 Token）

#### 4.3 审计日志
- [ ] 操作日志查询（基于 `src/audit.rs`）
- [ ] 日志筛选（按用户/操作类型/时间范围）
- [ ] 日志导出（CSV/JSON）
- [ ] 登录历史记录
- [ ] 敏感操作告警

**验收标准**：
- 支持 1000+ 用户管理
- 权限变更实时生效
- 审计日志查询响应时间 < 1 秒
- 日志导出支持大数据量（10 万条+）

---

### 第五阶段：系统配置与节点管理（v0.8.4）

#### 5.1 系统配置
- [ ] 配置项可视化编辑（基于 `config.toml`）
- [ ] 存储配置（存储路径、缓存大小）
- [ ] 协议配置（HTTP/WebDAV/S3/gRPC/QUIC 端口）
- [ ] 安全配置（JWT 密钥、Token 过期时间）
- [ ] 性能配置（并发限制、内存限制）
- [ ] 配置验证和错误提示
- [ ] 配置导入/导出
- [ ] 配置变更历史

#### 5.2 节点管理（多节点部署）
- [ ] 节点列表和状态
- [ ] 节点详情（地址、心跳、存储使用）
- [ ] 节点添加/删除
- [ ] 节点同步状态监控
- [ ] 节点间文件同步任务管理
- [ ] 节点健康检查配置

#### 5.3 任务管理
- [ ] 后台任务列表（上传会话、同步任务）
- [ ] 任务状态查询（运行中/完成/失败）
- [ ] 任务取消
- [ ] 任务重试
- [ ] 定时任务配置（GC、索引重建）

**验收标准**：
- 配置变更无需重启即可生效（支持热重载的配置）
- 多节点部署时可视化节点拓扑
- 任务管理支持 100+ 并发任务

---

### 第六阶段：高级功能（v0.9.0）

#### 6.1 搜索增强
- [ ] 高级搜索界面（基于 `src/search/`）
- [ ] 全文搜索（文件内容）
- [ ] 元数据搜索（文件名/标签/创建者）
- [ ] 搜索结果高亮
- [ ] 搜索历史记录
- [ ] 搜索结果导出

#### 6.2 分享与协作
- [ ] 生成分享链接
- [ ] 分享权限控制（只读/读写）
- [ ] 分享过期时间设置
- [ ] 分享密码保护
- [ ] 分享访问统计

#### 6.3 系统诊断与工具
- [ ] 系统健康检查
- [ ] 存储完整性检查
- [ ] 数据库清理工具（GC）
- [ ] 索引重建工具
- [ ] 日志收集与下载
- [ ] 系统备份/恢复

#### 6.4 通知与告警
- [ ] 系统通知中心
- [ ] 存储空间告警（使用率 > 80%）
- [ ] 节点离线告警
- [ ] 异常操作告警
- [ ] 邮件/Webhook 通知集成

**验收标准**：
- 搜索结果准确率 > 95%
- 分享链接访问无需登录
- 系统诊断工具执行时间 < 30 秒
- 告警响应时间 < 10 秒

---

## 目录结构设计

```
admin-dashboard/
├── public/               # 静态资源
│   ├── favicon.ico
│   └── logo.png
├── src/
│   ├── assets/          # 图片、字体等资源
│   ├── components/      # 公共组件
│   │   ├── FileIcon.vue          # 文件类型图标
│   │   ├── FileUploader.vue      # 文件上传器
│   │   ├── UserAvatar.vue        # 用户头像
│   │   ├── PermissionTag.vue     # 权限标签
│   │   └── ...
│   ├── layouts/         # 布局组件
│   │   ├── DefaultLayout.vue     # 默认布局
│   │   ├── BlankLayout.vue       # 空白布局（登录页）
│   │   └── components/
│   │       ├── Sidebar.vue
│   │       ├── Header.vue
│   │       └── Breadcrumb.vue
│   ├── views/           # 页面视图
│   │   ├── Login.vue             # 登录页
│   │   ├── Dashboard/            # 仪表盘
│   │   │   ├── index.vue
│   │   │   └── components/
│   │   ├── Files/                # 文件管理
│   │   │   ├── index.vue
│   │   │   ├── FileList.vue
│   │   │   ├── FilePreview.vue
│   │   │   └── VersionHistory.vue
│   │   ├── Users/                # 用户管理
│   │   │   ├── index.vue
│   │   │   ├── UserList.vue
│   │   │   └── UserEdit.vue
│   │   ├── Monitoring/           # 监控面板
│   │   │   └── index.vue
│   │   ├── Settings/             # 系统配置
│   │   │   ├── index.vue
│   │   │   ├── SystemConfig.vue
│   │   │   └── SecurityConfig.vue
│   │   ├── Nodes/                # 节点管理
│   │   │   └── index.vue
│   │   └── AuditLog/             # 审计日志
│   │       └── index.vue
│   ├── router/          # 路由配置
│   │   └── index.ts
│   ├── store/           # Pinia 状态管理
│   │   ├── modules/
│   │   │   ├── auth.ts           # 认证状态
│   │   │   ├── user.ts           # 用户信息
│   │   │   ├── files.ts          # 文件状态
│   │   │   └── system.ts         # 系统状态
│   │   └── index.ts
│   ├── api/             # API 请求封装
│   │   ├── auth.ts
│   │   ├── files.ts
│   │   ├── users.ts
│   │   ├── monitoring.ts
│   │   ├── config.ts
│   │   └── index.ts
│   ├── utils/           # 工具函数
│   │   ├── request.ts            # Axios 封装
│   │   ├── auth.ts               # 认证工具
│   │   ├── format.ts             # 格式化工具
│   │   ├── file.ts               # 文件处理工具
│   │   └── permission.ts         # 权限检查
│   ├── types/           # TypeScript 类型定义
│   │   ├── api.ts
│   │   ├── file.ts
│   │   ├── user.ts
│   │   └── index.ts
│   ├── styles/          # 全局样式
│   │   ├── variables.scss        # SCSS 变量
│   │   ├── mixins.scss           # SCSS 混入
│   │   └── global.scss           # 全局样式
│   ├── App.vue
│   └── main.ts
├── .env.development     # 开发环境配置
├── .env.production      # 生产环境配置
├── .eslintrc.js         # ESLint 配置
├── .prettierrc.js       # Prettier 配置
├── index.html
├── package.json
├── tsconfig.json        # TypeScript 配置
├── vite.config.ts       # Vite 配置
└── README.md
```

## 后端 API 需求

### 需要新增的管理端 API

```rust
// src/http/admin/mod.rs
pub mod dashboard;      // 仪表盘数据 API
pub mod users;          // 用户管理 API
pub mod config;         // 配置管理 API
pub mod nodes;          // 节点管理 API
pub mod tasks;          // 任务管理 API
pub mod audit;          // 审计日志 API
```

### API 端点清单

#### 仪表盘 API (`/api/admin/dashboard`)
- `GET /api/admin/dashboard/overview` - 系统概览数据
- `GET /api/admin/dashboard/metrics` - 性能指标
- `GET /api/admin/dashboard/activities` - 最近活动

#### 用户管理 API (`/api/admin/users`)
- `GET /api/admin/users` - 用户列表
- `POST /api/admin/users` - 创建用户
- `GET /api/admin/users/:id` - 用户详情
- `PUT /api/admin/users/:id` - 更新用户
- `DELETE /api/admin/users/:id` - 删除用户
- `PUT /api/admin/users/:id/password` - 修改密码
- `PUT /api/admin/users/:id/status` - 修改状态

#### 配置管理 API (`/api/admin/config`)
- `GET /api/admin/config` - 获取配置
- `PUT /api/admin/config` - 更新配置
- `POST /api/admin/config/validate` - 验证配置
- `GET /api/admin/config/history` - 配置历史

#### 节点管理 API (`/api/admin/nodes`)
- `GET /api/admin/nodes` - 节点列表
- `GET /api/admin/nodes/:id` - 节点详情
- `POST /api/admin/nodes` - 添加节点
- `DELETE /api/admin/nodes/:id` - 删除节点
- `GET /api/admin/nodes/:id/sync-status` - 同步状态

#### 任务管理 API (`/api/admin/tasks`)
- `GET /api/admin/tasks` - 任务列表
- `GET /api/admin/tasks/:id` - 任务详情
- `POST /api/admin/tasks/:id/cancel` - 取消任务
- `POST /api/admin/tasks/:id/retry` - 重试任务

#### 监控 API (`/api/admin/monitoring`)
- `GET /api/admin/monitoring/storage` - 存储监控
- `GET /api/admin/monitoring/performance` - 性能监控
- `GET /api/admin/monitoring/protocols` - 协议统计

## 开发阶段规划

### 阶段一：基础框架与登录认证（v0.8.0）
**预计时间**: 2 周

**前端任务**:
- 项目初始化和基础布局
- 登录认证功能
- 路由和权限控制

**后端任务**:
- 完善用户认证 API（已有基础）
- 添加静态文件服务支持

### 阶段二：仪表盘与监控（v0.8.1）
**预计时间**: 2 周

**前端任务**:
- 系统仪表盘开发
- 监控面板开发

**后端任务**:
- 后端仪表盘 API 开发
- 监控指标聚合 API

### 阶段三：文件管理（v0.8.2）
**预计时间**: 3 周

**前端任务**:
- 文件浏览器开发
- 文件操作功能
- 文件预览和版本管理

**后端任务**:
- 文件管理 API 增强
- 文件预览服务

### 阶段四：用户管理（v0.8.3）
**预计时间**: 2 周

**前端任务**:
- 用户管理界面
- 权限管理功能
- 审计日志查询

**后端任务**:
- 用户管理 API 完善
- 审计日志查询 API

### 阶段五：系统配置（v0.8.4）
**预计时间**: 2 周

**前端任务**:
- 配置管理界面
- 节点管理功能
- 任务管理功能

**后端任务**:
- 配置管理 API
- 节点管理 API
- 任务管理 API

### 阶段六：高级功能（v0.9.0）
**预计时间**: 3 周

**前端任务**:
- 搜索增强
- 分享与协作
- 系统诊断工具
- 通知与告警

**后端任务**:
- 分享链接 API
- 系统诊断工具 API
- 通知系统集成

## 非功能性需求

### 性能要求
- 首屏加载时间 < 3 秒
- 页面切换响应时间 < 500ms
- 大列表虚拟滚动支持 10000+ 条目
- API 响应时间 < 1 秒

### 兼容性要求
- 浏览器：Chrome 90+, Firefox 88+, Safari 14+, Edge 90+
- 屏幕分辨率：最低 1280x720
- 移动端适配：平板（768px+）

### 安全要求
- 所有请求携带 JWT Token
- Token 自动刷新机制
- XSS 防护（输入验证和转义）
- CSRF 防护
- 敏感操作二次确认

### 可访问性
- 支持键盘导航
- ARIA 标签支持
- 色彩对比度符合 WCAG 2.1 AA 标准

## 部署方案

### 开发环境
```bash
cd admin-dashboard
yarn install
yarn dev
```

### 生产环境
```bash
cd admin-dashboard
yarn build
# 生成的静态文件在 dist/ 目录
# 可通过 Nginx 或 Silent-NAS 内置静态文件服务提供
```

### Docker 部署
在主 Dockerfile 中添加前端构建步骤，将构建产物集成到最终镜像中。

### 静态文件服务
在 `src/http/mod.rs` 中添加静态文件路由，为管理端提供静态文件服务。

## 风险与应对

### 技术风险
- **大文件上传**: 需要实现分片上传和断点续传
  - 应对：使用成熟的文件上传库
- **实时监控**: WebSocket 或长轮询的性能开销
  - 应对：优先使用轮询，后续优化为 WebSocket
- **大列表渲染**: 虚拟滚动实现复杂度高
  - 应对：使用 Element Plus Table 的虚拟滚动功能

### 兼容性风险
- 不同浏览器的 File API 兼容性
  - 应对：使用 polyfill 和 browserslist 配置
- 移动端触摸操作适配
  - 应对：优先支持桌面端，移动端仅支持查看功能

## 参考资料

### 类似项目
- **Nextcloud Web UI**: 文件管理界面设计参考
- **MinIO Console**: 对象存储管理界面参考
- **Grafana**: 监控面板设计参考
- **Ant Design Pro**: 后台管理系统参考

### 技术文档
- [Vue 3 官方文档](https://vuejs.org/)
- [Element Plus 文档](https://element-plus.org/)
- [ECharts 文档](https://echarts.apache.org/)
- [Vite 官方文档](https://vitejs.dev/)

---

**最后更新**: 2025-12-03
**文档版本**: v1.0
**状态**: 规划中
