#!/bin/bash
# Silent-NAS 容器入口脚本

set -e

# 生成配置文件（如果不存在）
if [ ! -f /app/config.toml ]; then
    echo "生成配置文件..."

    # 解析种子节点列表
    SEED_NODES_CONFIG=""
    if [ -n "${SEED_NODES:-}" ]; then
        # 将逗号分隔的种子节点转换为 TOML 数组格式
        IFS=',' read -ra SEEDS <<< "$SEED_NODES"
        for seed in "${SEEDS[@]}"; do
            seed=$(echo "$seed" | xargs)  # 去除空格
            if [ -n "$seed" ]; then
                if [ -z "$SEED_NODES_CONFIG" ]; then
                    SEED_NODES_CONFIG="\"$seed\""
                else
                    SEED_NODES_CONFIG="$SEED_NODES_CONFIG, \"$seed\""
                fi
            fi
        done
    fi

    # 从模板创建配置文件
    cat > /app/config.toml << EOF
[server]
host = "${HTTP_HOST:-0.0.0.0}"
http_port = ${HTTP_PORT:-8080}
grpc_port = ${GRPC_PORT:-50051}
quic_port = ${QUIC_PORT:-4433}
webdav_port = ${WEBDAV_PORT:-8081}
s3_port = ${S3_PORT:-9001}

[storage]
root_path = "${STORAGE_PATH:-/data}"
chunk_size = ${CHUNK_SIZE:-4194304}

[nats]
url = "${NATS_URL:-nats://nats:4222}"
topic_prefix = "${NATS_TOPIC_PREFIX:-bench.nas}"

[s3]
access_key = "${S3_ACCESS_KEY:-admin}"
secret_key = "${S3_SECRET_KEY:-password}"
enable_auth = ${S3_ENABLE_AUTH:-false}

[auth]
enable = ${AUTH_ENABLE:-false}
db_path = "${AUTH_DB_PATH:-/data/auth.db}"
jwt_secret = "${JWT_SECRET:-secret}"
access_token_exp = ${ACCESS_TOKEN_EXP:-3600}
refresh_token_exp = ${REFRESH_TOKEN_EXP:-604800}

[node]
enable = ${NODE_ENABLE:-true}
seed_nodes = [$SEED_NODES_CONFIG]
heartbeat_interval = ${HEARTBEAT_INTERVAL:-2}
node_timeout = ${NODE_TIMEOUT:-120}

[sync]
auto_sync = ${AUTO_SYNC:-true}
sync_interval = ${SYNC_INTERVAL:-2}
max_files_per_sync = ${MAX_FILES_PER_SYNC:-200}
max_retries = ${MAX_RETRIES:-3}
EOF

    echo "配置文件已生成"
    cat /app/config.toml
fi

# 启动 Silent-NAS
echo "启动 Silent-NAS..."
exec /app/silent-nas
