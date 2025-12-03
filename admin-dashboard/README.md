# Silent-NAS 管理控制台

基于 Vue 3 + Vite + Element Plus + TypeScript 的 Web 管理控制台。

## 技术栈

- **框架**: Vue 3 (Composition API)
- **构建工具**: Vite 5
- **UI 组件库**: Element Plus
- **状态管理**: Pinia
- **路由**: Vue Router 4
- **HTTP 客户端**: Axios
- **图表库**: ECharts
- **类型支持**: TypeScript

## 开发

```bash
# 安装依赖
yarn install

# 启动开发服务器
yarn dev

# 构建生产版本
yarn build

# 预览生产构建
yarn preview
```

## 项目结构

```
admin-dashboard/
├── public/               # 静态资源
├── src/
│   ├── api/             # API 请求封装
│   ├── assets/          # 图片、字体等资源
│   ├── components/      # 公共组件
│   ├── layouts/         # 布局组件
│   ├── router/          # 路由配置
│   ├── store/           # Pinia 状态管理
│   ├── styles/          # 全局样式
│   ├── types/           # TypeScript 类型定义
│   ├── utils/           # 工具函数
│   ├── views/           # 页面视图
│   ├── App.vue
│   └── main.ts
├── .env.development     # 开发环境配置
├── .env.production      # 生产环境配置
├── index.html
├── package.json
├── tsconfig.json        # TypeScript 配置
├── vite.config.ts       # Vite 配置
└── README.md
```

## 功能说明

### 当前已实现

- ✅ 项目基础架构
- ✅ 用户登录认证
- ✅ 路由守卫和权限控制
- ✅ Axios 请求封装
- ✅ 基础仪表盘页面

### 开发中

- 🔄 系统仪表盘（监控数据）
- 🔄 文件管理功能
- 🔄 用户管理功能
- 🔄 系统配置功能

## 配置说明

### 开发环境

开发服务器运行在 `http://localhost:5173`，API 请求会自动代理到 `http://localhost:8080`。

### API 代理配置

在 `vite.config.ts` 中配置了 API 代理：

```typescript
server: {
  port: 5173,
  proxy: {
    '/api': {
      target: 'http://localhost:8080',
      changeOrigin: true,
    },
  },
}
```

### 路径别名

项目配置了 `@` 路径别名，指向 `src` 目录：

```typescript
import { useAuthStore } from '@/store/modules/auth'
```

## 开发规范

- 使用 TypeScript 进行类型检查
- 使用 Composition API 编写组件
- 使用 SCSS 编写样式
- API 请求统一使用 `src/utils/request.ts` 封装的 axios 实例

## 相关文档

- [开发计划](../docs/admin-dashboard-plan.md)
- [项目规划](../PLAN.md)
- [任务清单](../TODO.md)
