#!/bin/bash
# Silent-NAS 容器入口脚本

set -e

# 生成配置文件（如果不存在）
if [ ! -f /app/config.toml ]; then
    echo "生成配置文件..."

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
topic_prefix = "${NATS_TOPIC_PREFIX:-silent.nas.files}"

[s3]
access_key = "${S3_ACCESS_KEY:-admin}"
secret_key = "${S3_SECRET_KEY:-password}"
enable_auth = ${S3_ENABLE_AUTH:-false}
EOF

    echo "配置文件已生成"
fi

# 启动 Silent-NAS
echo "启动 Silent-NAS..."
exec /app/silent-nas
