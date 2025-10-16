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

# 先仅复制清单文件与工作区依赖以利用缓存
COPY Cargo.toml Cargo.lock ./
COPY build.rs ./
COPY proto/ ./proto/
COPY silent/ ./silent/
COPY silent-crdt/ ./silent-crdt/
COPY odyssey/ ./odyssey/

# 预热依赖缓存（使用临时 main）
RUN mkdir -p src && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src

# 复制实际源码
COPY src/ ./src/

# 为确保主二进制使用真实源码，清理该包缓存后再构建
RUN cargo clean \
    && cargo build --release

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
