.PHONY: help test test-storage test-s3 test-webdav coverage coverage-html clean clean-test check fmt clippy run dev

# 默认目标
help:
	@echo "Silent-NAS Makefile 帮助"
	@echo ""
	@echo "可用命令:"
	@echo "  make help             - 显示此帮助信息"
	@echo "  make test             - 运行所有单元测试"
	@echo "  make test-storage     - 运行存储模块测试"
	@echo "  make test-s3          - 运行 S3 模块测试"
	@echo "  make test-webdav      - 运行 WebDAV 模块测试"
	@echo "  make coverage         - 生成覆盖率报告（排除 silent 框架）"
	@echo "  make coverage-html    - 生成 HTML 覆盖率报告"
	@echo "  make fmt              - 格式化代码"
	@echo "  make clippy           - 运行 clippy 检查"
	@echo "  make check            - 运行所有检查（测试+格式+lint）"
	@echo "  make clean            - 清理构建文件"
	@echo "  make clean-test       - 清理测试生成的文件"
	@echo "  make run              - 运行应用（开发模式）"
	@echo "  make dev              - 开发模式运行（带日志）"

# 运行所有单元测试
test:
	@echo "运行所有单元测试..."
	@cargo test --all-features

# 运行存储模块测试
test-storage:
	@echo "运行存储模块测试..."
	@cargo test storage:: --all-features

# 运行 S3 模块测试
test-s3:
	@echo "运行 S3 模块测试..."
	@cargo test s3:: --all-features

# 运行 WebDAV 模块测试
test-webdav:
	@echo "运行 WebDAV 模块测试..."
	@cargo test webdav:: --all-features

# 生成覆盖率报告（仅统计本项目代码，排除 silent 依赖和 silent-crdt）
coverage:
	@echo "生成覆盖率报告（排除 silent 框架和 silent-crdt）..."
	@cargo llvm-cov --all-features --ignore-filename-regex '.*/silent/silent/.*|.*/silent-crdt/.*' --summary-only

# 生成 HTML 覆盖率报告
coverage-html:
	@echo "生成 HTML 覆盖率报告（排除 silent 框架和 silent-crdt）..."
	@cargo llvm-cov --all-features --ignore-filename-regex '.*/silent/silent/.*|.*/silent-crdt/.*' --html
	@echo "HTML 报告已生成到: target/llvm-cov/html/index.html"
	@echo "在浏览器中打开: open target/llvm-cov/html/index.html"

# 格式化代码
fmt:
	@echo "格式化代码..."
	@cargo fmt

# 运行 clippy 检查
clippy:
	@echo "运行 clippy 检查..."
	@cargo clippy --all-targets --all-features -- -D warnings

# 运行所有检查
check: test fmt clippy
	@echo "运行所有检查..."
	@cargo check --all-features
	@echo "✅ 所有检查通过！"

# 清理构建文件
clean:
	@echo "清理构建文件..."
	@cargo clean
	@rm -rf target/llvm-cov target/llvm-cov-target
	@echo "清理完成"

# 清理测试生成的文件
clean-test:
	@echo "清理测试生成的文件..."
	@rm -rf test_data/ test_storage/ ./test_storage
	@rm -rf ./silent-nas-test
	@rm -f *.log
	@echo "清理完成"

# 运行应用（开发模式）
run:
	@echo "运行 Silent-NAS（开发模式）..."
	@cargo run --all-features

# 开发模式运行（带详细日志）
dev:
	@echo "开发模式运行 Silent-NAS（详细日志）..."
	@RUST_LOG=debug cargo run --all-features

# 构建 release 版本
build-release:
	@echo "构建 release 版本..."
	@cargo build --release --all-features
	@echo "✅ Release 构建完成: target/release/silent-nas"

# 安装依赖
install-deps:
	@echo "安装开发依赖..."
	@cargo install cargo-llvm-cov
	@cargo install cargo-watch
	@echo "✅ 依赖安装完成"

# 监听文件变化并自动测试
watch:
	@echo "监听文件变化并自动运行测试..."
	@cargo watch -x test

# 监听文件变化并自动运行
watch-run:
	@echo "监听文件变化并自动运行应用..."
	@cargo watch -x run
