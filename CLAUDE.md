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
cargo build -p lyrics_helper_core
cargo build -p ttml_processor

# 发布构建（优化性能）
cargo build --release --workspace

# 为特定target构建
cargo build --release --target x86_64-pc-windows-msvc
```

### 测试命令
```bash
# 运行所有测试
cargo test --workspace

# 运行特定包的测试
cargo test -p lyrics_helper_rs
cargo test -p ttml_processor
cargo test -p lyrics_helper_core

# 运行集成测试（需要网络连接）
cargo test -- --ignored

# 运行特定测试文件
cargo test --test test_name

# 运行特定测试函数
cargo test test_function_name

# 并发运行测试
cargo test --workspace -- --test-threads=4
```

### 格式化和Lint检查
```bash
# 格式化所有代码
cargo fmt --all

# Clippy 检查所有包
cargo clippy --workspace -- -D warnings

# 单个包的 clippy 检查
cargo clippy -p lyrics_helper_rs -- -D warnings
cargo clippy -p Unilyric -- -D warnings
```

### 文档生成
```bash
# 生成所有文档
cargo doc --workspace --no-deps

# 打开生成的文档
cargo doc --workspace --open

# 生成特定包的文档
cargo doc -p lyrics_helper_rs --open
```

### 依赖管理
```bash
# 生成依赖树
cargo tree --workspace

# 更新所有依赖
cargo update

# 查看过时的依赖
cargo outdated
```

### 运行应用程序
```bash
# 运行图形界面应用
cargo run --release -p Unilyric

# 运行命令行工具
cargo run --release -p lyrics_helper_rs -- --help

# 调试模式运行
cargo run -p Unilyric
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

## 核心架构和设计模式

### 核心特性设计
- **异步处理**: 使用 Tokio runtime 处理网络请求，支持高并发搜索
- **模块化提供商**: 每个音乐平台作为独立的 Provider 实现，易于扩展
- **格式抽象**: 统一的歌词格式转换系统，支持多输入多输出格式
- **智能匹配**: 基于歌曲元数据的智能搜索算法，支持多种匹配策略

### 关键设计模式
- **策略模式**: 搜索模式（Ordered, Parallel, Specific, Subset）
- **工厂模式**: 提供商动态加载和初始化
- **适配器模式**: 不同歌词格式的统一处理接口
- **观察者模式**: 登录流程的事件处理机制

### 主要入口点和关键文件
- **主要库入口**: `lyrics_helper_rs/src/lib.rs:276` - `LyricsHelper` 结构体
- **GUI 应用入口**: `Unilyric/src/main.rs`
- **核心数据结构**: `lyrics_helper_core/src/model/`
- **格式转换器**: `lyrics_helper_rs/src/converter/`
- **提供商实现**: `lyrics_helper_rs/src/providers/`
- **网络客户端**: `lyrics_helper_rs/src/http/`

### 网络与并发
- **HTTP 客户端**: 自定义抽象 HTTP 客户端接口，支持 cookie 管理
- **并发搜索**: 并行搜索多个提供商，自动选择最佳匹配
- **会话管理**: 持久化登录状态，支持会话导入导出
- **取消支持**: 使用 `CancellationToken` 支持操作取消

### 提供商支持详情
- **QQ音乐**: 完整支持搜索、歌词、专辑信息、登录
- **网易云音乐**: 完整支持搜索、歌词、专辑信息、登录
- **酷狗音乐**: 支持搜索和歌词获取
- **AMLL TTML 数据库**: 专门处理 Apple Music 歌词数据库

## 开发要点

- **主要入口点**: `lyrics_helper_rs/src/lib.rs:276` 和 `Unilyric/src/main.rs`
- **异步架构**: 使用 async/await 和 Tokio 处理所有网络请求
- **日志调试**: 集成 tracing 库，设置日志级别: `RUST_LOG=lyrics_helper_rs=debug`
- **WASM 支持**: 支持编译为 WebAssembly，可用于 Web 应用
- **错误处理**: 统一的错误类型 `LyricsHelperError`，包含详细的错误上下文
- **测试策略**: 单元测试 + 集成测试（带网络请求的需要 `-- --ignored`）
- **性能优化**: 使用 nightly Rust 工具链进行最大优化，release 构建启用 LTO