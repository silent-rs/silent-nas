# 部署指南

本文档提供 Silent-NAS 在生产环境中的部署建议。

## 部署架构

### 单节点部署

适合小规模应用，简单可靠：

```
┌─────────────────────┐
│    Load Balancer    │
│   (Optional: Nginx) │
└──────────┬──────────┘
           │
┌──────────▼──────────┐
│   Silent-NAS Node   │
│  ┌───────────────┐  │
│  │  HTTP API     │  │
│  │  WebDAV       │  │
│  │  S3 API       │  │
│  └───────────────┘  │
│  ┌───────────────┐  │
│  │  Storage      │  │
│  └───────────────┘  │
└─────────────────────┘
```

### 集群部署

适合大规模应用，高可用：

```
                  ┌─────────────────┐
                  │  Load Balancer  │
                  └────────┬─────────┘
         ┌────────────────┼────────────────┐
         │                │                │
    ┌────▼────┐      ┌────▼────┐     ┌────▼────┐
    │  Node1  │      │  Node2  │     │  Node3  │
    └────┬────┘      └────┬────┘     └────┬────┘
         │                │                │
         └────────────────┼────────────────┘
                          │
                   ┌──────▼──────┐
                   │  NATS       │
                   │  Message    │
                   │  Broker     │
                  └─────────────┘
```

> 单节点部署与同步配置提示
>
> - 单节点（未连接 NATS，或 `[node].enable = false`）不会启用跨节点同步与后台补拉任务。
> - 此时可以省略 `[sync]` 段落配置，应用将按默认参数运行，且不会尝试跨节点相关的行为。
> - 需要多节点、联动与收敛时，再启用 NATS 并完整配置 `[node]` 与 `[sync]`。

## 系统资源规划

### 硬件配置建议

#### 小型部署（< 100 用户）

- **CPU**: 4 核心
- **内存**: 8 GB
- **存储**: 500 GB SSD + 2 TB HDD
- **网络**: 千兆以太网

#### 中型部署（100-1000 用户）

- **CPU**: 8 核心
- **内存**: 16 GB
- **存储**: 1 TB SSD + 10 TB HDD
- **网络**: 10 千兆以太网

#### 大型部署（> 1000 用户）

- **CPU**: 16 核心以上
- **内存**: 32 GB 以上
- **存储**: 按需配置，建议使用独立存储阵列
- **网络**: 10 千兆以太网或更高

### 存储规划

#### 存储类型

1. **系统盘**: SSD，用于操作系统和应用
2. **元数据盘**: SSD，用于数据库和索引
3. **数据盘**: HDD/SSD，用于文件存储
4. **缓存盘**: SSD，用于热数据缓存

#### 存储空间计算

```
总存储需求 = 用户数据 + 版本控制 + 缓存 + 系统开销

示例：
- 用户数据: 10 TB
- 版本控制(平均 5 版本): 10 TB × 5 × 0.3 = 15 TB
- 缓存: 100 GB
- 系统开销: 500 GB
- 总计: 10 + 15 + 0.1 + 0.5 = 25.6 TB

建议配置: 30 TB (预留 20% 增长空间)
```

## Docker 部署

### 单节点部署

#### 1. 准备配置文件

```bash
# 创建工作目录
mkdir -p ~/silent-nas/{config,storage,logs}
cd ~/silent-nas

# 创建配置文件
cat > config/config.toml << EOF
[server]
host = "0.0.0.0"
http_port = 8080
webdav_port = 8081
s3_port = 9000

[storage]
root_path = "/data"
max_file_size = 53687091200  # 50GB

[auth]
enable = true
admin_user = "admin"
admin_password = "ChangeMe123!"

[log]
level = "info"
format = "json"
output = "file"
file_path = "/logs/silent-nas.log"
EOF
```

#### 2. 启动容器

```bash
docker run -d \
  --name silent-nas \
  --restart unless-stopped \
  -p 8080:8080 \
  -p 8081:8081 \
  -p 9000:9000 \
  -v $(pwd)/config/config.toml:/config.toml:ro \
  -v $(pwd)/storage:/data \
  -v $(pwd)/logs:/logs \
  silent-rs/silent-nas:latest
```

#### 3. 验证部署

```bash
# 健康检查
curl http://localhost:8080/api/health

# 查看日志
docker logs -f silent-nas
```

### 集群部署（Docker Compose）

#### 1. 下载配置文件

```bash
git clone https://github.com/silent-rs/silent-nas.git
cd silent-nas/docker
```

#### 2. 配置环境变量

```bash
cp .env.example .env
vim .env
```

编辑 `.env` 文件：

```bash
# 同步配置
AUTO_SYNC=true
SYNC_INTERVAL=60

# 认证配置
ADMIN_USER=admin
ADMIN_PASSWORD=ChangeMe123!

# NATS 配置
NATS_URL=nats://nats:4222
```

#### 3. 启动集群

```bash
docker-compose up -d
```

#### 4. 扩展节点

```bash
# 添加更多节点
docker-compose up -d --scale silent-nas-worker=5
```

#### 5. 监控集群

```bash
# 查看所有服务
docker-compose ps

# 查看节点日志
docker-compose logs -f silent-nas-1

# 查看 NATS 状态
curl http://localhost:8222/varz
```

## 使用 Systemd（裸机部署）

### 1. 创建系统用户

```bash
sudo useradd -r -s /bin/false -d /var/lib/silent-nas silent-nas
sudo mkdir -p /var/lib/silent-nas/{storage,logs}
sudo chown -R silent-nas:silent-nas /var/lib/silent-nas
```

### 2. 安装二进制文件

```bash
sudo cp silent-nas /usr/local/bin/
sudo chmod +x /usr/local/bin/silent-nas
```

### 3. 创建配置文件

```bash
sudo mkdir -p /etc/silent-nas
sudo vim /etc/silent-nas/config.toml
```

### 4. 创建 Systemd 服务

```bash
sudo cat > /etc/systemd/system/silent-nas.service << EOF
[Unit]
Description=Silent-NAS Network Attached Storage
After=network.target

[Service]
Type=simple
User=silent-nas
Group=silent-nas
WorkingDirectory=/var/lib/silent-nas
ExecStart=/usr/local/bin/silent-nas --config /etc/silent-nas/config.toml
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal
SyslogIdentifier=silent-nas

# 安全加固
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/silent-nas

# 资源限制
LimitNOFILE=65536
LimitNPROC=4096

[Install]
WantedBy=multi-user.target
EOF
```

### 5. 启动服务

```bash
# 重新加载 systemd
sudo systemctl daemon-reload

# 启动服务
sudo systemctl start silent-nas

# 开机自启
sudo systemctl enable silent-nas

# 查看状态
sudo systemctl status silent-nas

# 查看日志
sudo journalctl -u silent-nas -f
```

## 反向代理配置

### Nginx

#### HTTP + WebDAV

```nginx
upstream silent_nas_http {
    server 127.0.0.1:8080;
}

upstream silent_nas_webdav {
    server 127.0.0.1:8081;
}

server {
    listen 80;
    server_name nas.example.com;

    # HTTP API
    location /api {
        proxy_pass http://silent_nas_http;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # 大文件上传
        client_max_body_size 50G;
        proxy_request_buffering off;
    }

    # WebDAV
    location / {
        proxy_pass http://silent_nas_webdav;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;

        # WebDAV 特殊头
        proxy_set_header Destination $http_destination;
        proxy_set_header Overwrite $http_overwrite;
        proxy_set_header Depth $http_depth;

        client_max_body_size 50G;
    }
}
```

#### HTTPS 配置

```nginx
server {
    listen 443 ssl http2;
    server_name nas.example.com;

    ssl_certificate /etc/letsencrypt/live/nas.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/nas.example.com/privkey.pem;
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers HIGH:!aNULL:!MD5;

    # 其他配置同上
    location /api {
        proxy_pass http://silent_nas_http;
        # ...
    }
}

# HTTP 重定向到 HTTPS
server {
    listen 80;
    server_name nas.example.com;
    return 301 https://$server_name$request_uri;
}
```

### Caddy

```caddyfile
nas.example.com {
    # 自动 HTTPS

    # HTTP API
    route /api/* {
        reverse_proxy localhost:8080
    }

    # WebDAV
    route /* {
        reverse_proxy localhost:8081 {
            header_up Destination {http.request.header.Destination}
            header_up Overwrite {http.request.header.Overwrite}
            header_up Depth {http.request.header.Depth}
        }
    }
}
```

### Traefik（Docker）

```yaml
# docker-compose.yml
version: '3'

services:
  traefik:
    image: traefik:v2.10
    command:
      - "--api.insecure=true"
      - "--providers.docker=true"
      - "--entrypoints.web.address=:80"
      - "--entrypoints.websecure.address=:443"
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock

  silent-nas:
    image: silent-rs/silent-nas:latest
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.silent-nas.rule=Host(`nas.example.com`)"
      - "traefik.http.routers.silent-nas.entrypoints=websecure"
      - "traefik.http.routers.silent-nas.tls.certresolver=letsencrypt"
```

## 高可用配置

### 负载均衡

#### HAProxy 配置

```haproxy
global
    maxconn 4096

defaults
    mode http
    timeout connect 5s
    timeout client 50s
    timeout server 50s

frontend http_front
    bind *:80
    default_backend silent_nas_nodes

backend silent_nas_nodes
    balance roundrobin
    option httpchk GET /api/health
    server node1 192.168.1.101:8080 check
    server node2 192.168.1.102:8080 check
    server node3 192.168.1.103:8080 check
```

### 健康检查

确保负载均衡器正确检测节点状态：

```bash
# 使用 Silent-NAS 健康检查端点
GET /api/health/readiness

# 期望响应
{
  "status": "ready",
  "storage": "ok",
  "nats": "connected"
}
```

## 监控和日志

### Prometheus + Grafana

#### 1. Prometheus 配置

```yaml
# prometheus.yml
scrape_configs:
  - job_name: 'silent-nas'
    static_configs:
      - targets:
        - 'node1:8080'
        - 'node2:8080'
        - 'node3:8080'
    metrics_path: '/api/metrics'
    scrape_interval: 15s
```

#### 2. 启动 Prometheus

```bash
docker run -d \
  -p 9090:9090 \
  -v $(pwd)/prometheus.yml:/etc/prometheus/prometheus.yml \
  prom/prometheus
```

#### 3. 启动 Grafana

```bash
docker run -d \
  -p 3000:3000 \
  grafana/grafana
```

### 日志聚合（ELK Stack）

#### 1. Filebeat 配置

```yaml
# filebeat.yml
filebeat.inputs:
  - type: log
    enabled: true
    paths:
      - /var/lib/silent-nas/logs/*.log
    json.keys_under_root: true

output.elasticsearch:
  hosts: ["elasticsearch:9200"]
```

## 备份策略

### 文件备份

```bash
#!/bin/bash
# backup.sh

BACKUP_DIR="/mnt/backup/silent-nas"
DATE=$(date +%Y%m%d)

# 备份存储目录
rsync -av --progress \
  /var/lib/silent-nas/storage/ \
  $BACKUP_DIR/$DATE/storage/

# 备份配置
cp /etc/silent-nas/config.toml \
  $BACKUP_DIR/$DATE/config.toml

# 清理 30 天前的备份
find $BACKUP_DIR -type d -mtime +30 -exec rm -rf {} \;
```

### 定时备份

```bash
# 添加到 crontab
crontab -e

# 每天凌晨 2 点备份
0 2 * * * /usr/local/bin/backup.sh
```

## 安全加固

### 1. 防火墙配置

```bash
# UFW (Ubuntu)
sudo ufw allow 22/tcp      # SSH
sudo ufw allow 80/tcp      # HTTP
sudo ufw allow 443/tcp     # HTTPS
sudo ufw enable

# 仅允许特定 IP 访问管理端口
sudo ufw allow from 192.168.1.0/24 to any port 8080
```

### 2. 启用 TLS

在配置文件中启用 TLS：

```toml
[server]
enable_tls = true
cert_file = "/etc/ssl/certs/server.crt"
key_file = "/etc/ssl/private/server.key"
```

### 3. 限流配置

```toml
[rate_limit]
enable = true
requests_per_second = 100
burst_size = 200
```

### 4. 定期更新

```bash
# 设置自动更新检查
0 0 * * 0 curl -s https://api.github.com/repos/silent-rs/silent-nas/releases/latest | grep tag_name
```

## 性能优化

### 1. 内核参数调优

```bash
# /etc/sysctl.conf
fs.file-max = 1000000
net.core.somaxconn = 4096
net.ipv4.tcp_max_syn_backlog = 8192
net.ipv4.tcp_tw_reuse = 1
```

### 2. 文件系统优化

```bash
# ext4 挂载选项
/dev/sda1 /var/lib/silent-nas/storage ext4 noatime,nodiratime,data=writeback 0 2
```

### 3. 缓存配置

```toml
[cache]
enable = true
max_cache_size = 10737418240  # 10GB
metadata_ttl = 7200           # 2小时
```

## 故障排查

### 常见问题

1. **节点无法同步**
   - 检查 NATS 连接
   - 验证网络连通性
   - 查看同步日志

2. **性能下降**
   - 检查磁盘 I/O
   - 查看内存使用
   - 分析慢查询

3. **存储空间不足**
   - 清理过期版本
   - 启用自动清理
   - 扩展存储容量

详细故障排查见 [RUNNING.md](../RUNNING.md)

## 下一步

- [配置指南](configuration.md) - 详细配置说明
- [API 使用指南](api-guide.md) - API 使用方法
