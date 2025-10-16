#!/bin/bash
# Silent-NAS 快速启动脚本

set -e

echo "🚀 Silent-NAS Docker 部署脚本"
echo "================================"
echo ""

# 检查 Docker
if ! command -v docker &> /dev/null; then
    echo "❌ 错误: Docker 未安装"
    exit 1
fi

if ! command -v docker-compose &> /dev/null; then
    echo "❌ 错误: Docker Compose 未安装"
    exit 1
fi

# 检查 silent 子模块
if [ ! -f "../silent/Cargo.toml" ]; then
    echo "📦 初始化 git submodule..."
    cd ..
    git submodule update --init --recursive
    cd docker
    echo "✅ submodule 初始化完成"
fi

# 创建数据目录
echo "📁 创建数据目录..."
mkdir -p data/{node1,node2,node3}

# 复制环境变量配置
if [ ! -f .env ]; then
    echo "⚙️ 创建环境配置..."
    cp ./docker/.env.example .env
    echo "✅ 已创建 .env 文件，请根据需要修改"
fi

# 构建镜像
echo ""
echo "🔨 构建 Docker 镜像..."
docker-compose build

# 启动服务
echo ""
echo "🚀 启动服务..."
docker-compose up -d

# 等待服务就绪
echo ""
echo "⏳ 等待服务启动 (30秒)..."
sleep 30

# 检查服务状态
echo ""
echo "📊 服务状态:"
docker-compose ps

# 检查健康状态
echo ""
echo "🏥 健康检查:"
echo "  Node1:" $(curl -s http://localhost:8080/health 2>/dev/null | grep -o '"status":"[^"]*"' || echo "未就绪")
echo "  Node2:" $(curl -s http://localhost:8090/health 2>/dev/null | grep -o '"status":"[^"]*"' || echo "未就绪")
echo "  Node3:" $(curl -s http://localhost:8100/health 2>/dev/null | grep -o '"status":"[^"]*"' || echo "未就绪")
echo "  NATS:" $(curl -s http://localhost:8222/healthz 2>/dev/null || echo "未就绪")

# 显示访问信息
echo ""
echo "✅ 部署完成！"
echo ""
echo "📍 服务访问地址:"
echo "  Node1 HTTP API:  http://localhost:8080"
echo "  Node1 WebDAV:    http://localhost:8081"
echo "  Node1 S3:        http://localhost:9001"
echo ""
echo "  Node2 HTTP API:  http://localhost:8090"
echo "  Node3 HTTP API:  http://localhost:8100"
echo ""
echo "  NATS Monitor:    http://localhost:8222"
echo ""
echo "📝 查看日志: docker-compose logs -f"
echo "🛑 停止服务: docker-compose down"
echo ""
