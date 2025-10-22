# Silent-NAS 文档

欢迎使用 Silent-NAS 文档！本目录包含所有用户文档和使用指南。

## 📖 文档导航

### 新手入门

1. **[安装指南](installation.md)**
   - 系统要求
   - 多种安装方式（二进制/源码/Docker）
   - 依赖服务安装（NATS）
   - 验证安装

2. **[快速开始](../README.md#快速开始)**
   - 5分钟快速体验
   - 基本操作示例

### 配置和部署

3. **[配置指南](configuration.md)**
   - 完整配置参数说明
   - 配置模板（开发/生产/集群）
   - 环境变量配置
   - 配置最佳实践

4. **[部署指南](deployment.md)**
   - 单节点部署
   - 集群部署
   - Docker 部署
   - Systemd 服务配置
   - 反向代理配置（Nginx/Caddy/Traefik）
   - 高可用配置
   - 监控和日志
   - 备份策略
   - 安全加固

### API 使用

5. **[API 使用指南](api-guide.md)**
   - HTTP REST API
   - WebDAV 协议
   - S3 兼容 API
   - gRPC API
   - 节点同步（管理员 API）
   - 性能监控
   - 错误处理
   - 最佳实践

### 运维指南

6. **[运行指南](../RUNNING.md)**
   - 启动和停止服务
   - 日志查看
   - 故障排查
   - 性能测试

## 🎯 根据使用场景选择文档

### 我想快速测试 Silent-NAS
→ [README 快速开始](../README.md#快速开始) → [运行指南](../RUNNING.md)

### 我想在生产环境部署
→ [安装指南](installation.md) → [配置指南](configuration.md) → [部署指南](deployment.md)

### 我想使用 Docker 部署
→ [Docker 部署](deployment.md#docker-部署) → [Docker README](../docker/README.md)

### 我想开发应用对接 Silent-NAS
→ [API 使用指南](api-guide.md)

### 我想部署集群
→ [多节点部署与联调](deployment-multi-node.md) → [部署指南 - 集群部署](deployment.md#集群部署docker-compose)

## 🔗 外部文档

- [Docker 部署文档](../docker/README.md)
- [Docker 快速开始](../docker/QUICK_START.md)
- [项目 GitHub](https://github.com/silent-rs/silent-nas)
- [问题反馈](https://github.com/silent-rs/silent-nas/issues)

## 📋 文档版本

当前文档版本: **v0.6.0**

最后更新: 2025-10-21

## 🤝 贡献文档

发现文档问题或想要改进？欢迎提交 Pull Request！

文档源文件位置: `docs/`

## 💡 获取帮助

- **GitHub Issues**: [提交问题](https://github.com/silent-rs/silent-nas/issues)
- **讨论区**: [参与讨论](https://github.com/silent-rs/silent-nas/discussions)
- **文档反馈**: 在相关文档页面提交 Issue

## 📚 文档结构

```
docs/
├── README.md           # 本文档索引
├── installation.md     # 安装指南
├── configuration.md    # 配置指南
├── api-guide.md        # API 使用指南
└── deployment.md       # 部署指南
```

---

**提示**: 所有文档均使用 Markdown 格式编写，可以在 GitHub 或任何 Markdown 查看器中阅读。
