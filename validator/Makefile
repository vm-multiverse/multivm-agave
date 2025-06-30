# Solana Test Validator Makefile

# 项目路径
PROJECT_PATH := $(shell dirname $(realpath $(firstword $(MAKEFILE_LIST))))
CARGO_TOML := $(PROJECT_PATH)/Cargo.toml
BIN_NAME := solana-test-validator


# 默认目标
.PHONY: all
all: build

# 构建项目
.PHONY: build
build:
	@echo "构建 Solana 测试验证器..."
	cargo build --manifest-path="$(CARGO_TOML)" --bin $(BIN_NAME)

# 构建发布版本
.PHONY: build-release
build-release:
	@echo "构建发布版本的 Solana 测试验证器..."
	cargo build --release --manifest-path="$(CARGO_TOML)" --bin $(BIN_NAME)

# 运行测试验证器
.PHONY: run
run:
	@echo "运行 Solana 测试验证器..."
	cargo run --manifest-path="$(CARGO_TOML)" --bin $(BIN_NAME) $(ARGS)

# 运行测试验证器（带参数示例）
.PHONY: run-ticks
run-ticks:
	@echo "运行 Solana 测试验证器（每槽 2 个 ticks）..."
	cargo run --manifest-path="$(CARGO_TOML)" --bin $(BIN_NAME) -- --ticks-per-slot 2

# 检查代码
.PHONY: check
check:
	@echo "检查代码..."
	cargo check --manifest-path="$(CARGO_TOML)"

# 运行测试
.PHONY: test
test:
	@echo "运行测试..."
	cargo test --manifest-path="$(CARGO_TOML)"

# 清理构建文件
.PHONY: clean
clean:
	@echo "清理构建文件..."
	cargo clean --manifest-path="$(CARGO_TOML)"

# 格式化代码
.PHONY: fmt
fmt:
	@echo "格式化代码..."
	cargo fmt --manifest-path="$(CARGO_TOML)"

# 检查代码风格
.PHONY: clippy
clippy:
	@echo "运行 Clippy 检查..."
	cargo clippy --manifest-path="$(CARGO_TOML)"

# 显示帮助信息
.PHONY: help
help:
	@echo "Solana 测试验证器 Makefile 命令："
	@echo "  build        - 构建项目"
	@echo "  build-release- 构建发布版本"
	@echo "  run          - 运行测试验证器"
	@echo "  run-ticks    - 运行测试验证器（每槽 2 个 ticks）"
	@echo "  check        - 检查代码"
	@echo "  test         - 运行测试"
	@echo "  clean        - 清理构建文件"
	@echo "  fmt          - 格式化代码"
	@echo "  clippy       - 运行 Clippy 检查"
	@echo "  help         - 显示此帮助信息"
	@echo ""
	@echo "使用示例："
	@echo "  make run ARGS=\"--ticks-per-slot 4\""
	@echo "  make build-release"