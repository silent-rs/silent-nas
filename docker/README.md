# Silent-NAS Docker 部署文档

## 📋 目录结构

```
docker/
├── Dockerfile              # 镜像构建文件
├── docker-compose.yml      # 编排配置文件
├── config.toml.template    # 配置模板
├── .env.example            # 环境变量示例
├── .dockerignore           # Docker 忽略文件
├── README.md               # 本文档
└── data/                   # 数据目录
    ├── node1/              # 节点1数据
    ├── node2/              # 节点2数据
    └── node3/              # 节点3数据
```

## 🚀 快速开始

### 1. 准备环境

```bash
# 进入 docker 目录
cd docker

# 创建数据目录
mkdir -p data/{node1,node2,node3}

# 复制环境变量配置
cp .env.example .env
```

### 2. 启动集群

```bash
# 构建并启动所有服务
docker-compose up -d

# 查看服务状态
docker-compose ps

# 查看日志
docker-compose logs -f
```

### 3. 验证部署

```bash
# 检查节点状态
curl http://localhost:8080/api/nodes

# 上传测试文件
curl -X POST -F "file=@test.txt" http://localhost:8080/api/files/upload

# 从其他节点查询
curl http://localhost:8090/api/files/list
```

## 🎯 服务端口映射

| 服务 | 节点 | gRPC | HTTP | WebDAV | S3 |
|------|------|------|------|--------|-----|
| node1 | 种子节点 | 9000 | 8080 | 8081 | 9001 |
| node2 | 对等节点 | 9010 | 8090 | 8091 | 9011 |
| node3 | 对等节点 | 9020 | 8100 | 8101 | 9021 |
| nats | 消息总线 | 4222 | 8222 | - | - |

## ⚙️ 配置说明

环境变量在 `.env` 文件中配置。主要配置项：

```bash
# 同步配置
AUTO_SYNC=true              # 启用自动同步
SYNC_INTERVAL=60            # 同步间隔(秒)
MAX_FILES_PER_SYNC=100      # 每次最大同步数

# 心跳配置
HEARTBEAT_INTERVAL=10       # 心跳间隔(秒)
NODE_TIMEOUT=30             # 节点超时(秒)
```

## 📊 管理命令

```bash
# 启动服务
docker-compose up -d

# 停止服务
docker-compose down

# 重启服务
docker-compose restart

# 查看日志
docker-compose logs -f node1

# 扩容（添加节点4）
docker-compose up -d --scale node2=2

# 进入容器
docker-compose exec node1 bash

# 清理数据
docker-compose down -v
```

## 🔧 故障排查

### 节点无法连接

```bash
# 检查网络
docker network ls
docker network inspect silent-nas_nas-network

# 检查日志
docker-compose logs node1 | grep ERROR
```

### 文件同步失败

```bash
# 检查NATS状态
curl http://localhost:8222/varz

# 手动触发同步
curl -X POST http://localhost:8080/api/sync/trigger
```

## 📚 更多文档

- [分布式部署指南](../docs/分布式部署指南.md)
- [跨节点同步实现报告](../docs/跨节点同步实现报告.md)
