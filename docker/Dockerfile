# Silent-NAS Docker 镜像构建文件
# 多阶段构建以减小镜像大小

# ==================== 构建阶段 ====================
# 与运行时保持一致的 Debian 版以避免 GLIBC 不匹配
FROM rust:1.90-slim-bookworm as builder

# 安装构建依赖
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# 设置工作目录
WORKDIR /build

# 复制依赖文件（利用 Docker 层缓存）
COPY Cargo.toml Cargo.lock ./
COPY proto/ ./proto/
COPY build.rs ./
COPY silent/ ./silent/
COPY silent-crdt/ ./silent-crdt/
COPY odyssey/ ./odyssey/

# 创建虚拟 src 目录以预编译依赖
RUN mkdir src && \
    echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

# 复制实际源代码
COPY src/ ./src/

# 重新构建应用（此时依赖已缓存）
RUN cargo build --release

# ==================== 运行阶段 ====================
FROM debian:bookworm-slim

# 安装运行时依赖
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# 创建非 root 用户
RUN useradd -m -u 1000 -s /bin/bash nas && \
    mkdir -p /data /app && \
    chown -R nas:nas /data /app

# 设置工作目录
WORKDIR /app

# 从构建阶段复制二进制文件
COPY --from=builder /build/target/release/silent-nas /app/silent-nas

# 复制入口脚本
COPY docker/entrypoint.sh /app/entrypoint.sh
RUN chmod +x /app/entrypoint.sh

# 切换到非 root 用户
USER nas

# 暴露端口
EXPOSE 9000 8080 8081 9001

# 设置环境变量
ENV RUST_LOG=info
ENV RUST_BACKTRACE=1

# 健康检查（暂时禁用，让容器先能启动）
# HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
#     CMD curl -f http://localhost:8080/health || exit 1

# 入口点
ENTRYPOINT ["/app/entrypoint.sh"]
