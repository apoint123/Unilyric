# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概览

UniLyric 是一个专业的歌词转换器，支持从多个音乐平台获取歌词并进行格式转换。项目采用 Rust 语言开发，使用 Cargo 工作区管理多个 crate。

## 代码架构

项目采用分层架构，包含以下主要组件：

### 核心 crate
- **lyrics_helper_core**: 数据结构和基础类型定义，被其他 crate 依赖
- **lyrics_helper_rs**: 主要业务逻辑，包含歌词提供者、搜索、转换功能
- **ttml_processor**: 专门处理 Apple Music TTML 格式的库
- **Unilyric**: 图形界面应用程序，基于 eframe/egui

### 歌词提供者支持
- QQ音乐、网易云音乐、酷狗音乐、AMLL TTML 数据库
- 具有搜索、歌词获取、专辑信息查询等功能

### 格式转换支持
支持多种歌词格式的输入和输出：LRC、QRC、KRC、YRC、TTML、Lyricify 格式等

## 开发命令

### 构建命令
```bash
# 构建整个工作区
cargo build --workspace

# 构建特定包
cargo build -p lyrics_helper_rs
cargo build -p Unilyric

# 发布构建
cargo build --release --workspace
```

### 测试命令
```bash
# 运行所有测试
cargo test --workspace

# 运行特定包的测试
cargo test -p lyrics_helper_rs
cargo test -p ttml_processor

# 运行集成测试（需要网络连接）
cargo test -- --ignored

# 运行特定测试文件
cargo test --test test_name
```

### Lint 检查
```bash
# Clippy 检查
cargo clippy --workspace -- -D warnings

# 单个包的 clippy 检查
cargo clippy -p lyrics_helper_rs -- -D warnings
```

### 其他命令
```bash
# 生成依赖树
cargo tree --workspace

# 更新依赖
cargo update
```

## CI/CD 配置

GitHub Actions 配置位于 `Unilyric/.github/workflows/release.yml`：
- 在 main 分支推送时自动构建发布版本
- 使用 nightly Rust 工具链
- 生成 Windows 可执行文件并创建发布

## 重要说明

1. **网络依赖**: 测试和功能需要使用网络连接访问音乐 API
2. **提供商限制**: 某些功能可能需要登录或受提供商限制
3. **性能优化**: 使用 Rust nightly 工具链进行最大优化
4. **多语言支持**: 主要支持中文歌曲和相关功能

## 开发要点

- 主要入口点：`lyrics_helper_rs/src/lib.rs` 和 `Unilyric/src/main.rs`
- 遵循 Rust 最佳实践，使用 async/await 处理网络请求
- 代码包含详细的 tracing 日志用于调试
- 支持 WASM target，可用于 Web 应用