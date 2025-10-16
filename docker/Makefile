# Silent-NAS Docker 管理 Makefile

.PHONY: help build up down restart logs ps clean

help: ## 显示帮助信息
	@echo "Silent-NAS Docker 管理命令:"
	@echo ""
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2}'

build: ## 构建镜像
	docker-compose build

up: ## 启动所有服务
	docker-compose up -d
	@echo "等待服务就绪..."
	@sleep 10
	@make ps

down: ## 停止所有服务
	docker-compose down

restart: ## 重启所有服务
	docker-compose restart

logs: ## 查看所有日志
	docker-compose logs -f

logs-node1: ## 查看节点1日志
	docker-compose logs -f node1

logs-node2: ## 查看节点2日志
	docker-compose logs -f node2

logs-node3: ## 查看节点3日志
	docker-compose logs -f node3

ps: ## 查看服务状态
	docker-compose ps

status: ## 检查集群状态
	@echo "=== NATS 状态 ==="
	@curl -s http://localhost:8222/varz | grep -o '"connections":[0-9]*'
	@echo ""
	@echo "=== 节点状态 ==="
	@curl -s http://localhost:8080/api/nodes 2>/dev/null || echo "节点1未就绪"
	@echo ""

clean: ## 清理所有数据和容器
	docker-compose down -v
	rm -rf data/node*/*

test: ## 运行测试
	@echo "上传测试文件到节点1..."
	@echo "Hello, Silent-NAS!" > /tmp/test.txt
	@curl -X POST -F "file=@/tmp/test.txt" http://localhost:8080/api/files/upload
	@echo ""
	@sleep 5
	@echo "从节点2查询文件..."
	@curl -s http://localhost:8090/api/files/list
	@rm /tmp/test.txt
