<h1 align="center">UniLyric - 一个全能的歌词转换器</h1>

<p align="center">
  <img src="https://github.com/user-attachments/assets/3ff25f07-c9cb-4125-90e3-3c409e83ff7b" alt="UniLyric Screenshot" width="600">
</p>

<p align="center">
  <a href="https://github.com/apoint123/Unilyric/releases/latest">
    <img src="https://img.shields.io/github/v/release/apoint123/Unilyric?style=for-the-badge" alt="最新版本">
  </a>
  <a href="LICENSE">
    <img src="https://img.shields.io/github/license/apoint123/Unilyric?style=for-the-badge" alt="许可证">
  </a>
  <a href="https://github.com/apoint123/Unilyric/actions">
    <img src="https://img.shields.io/github/actions/workflow/status/apoint123/Unilyric/release.yml?style=for-the-badge" alt="构建状态">
  </a>
  <a href="https://github.com/apoint123/Unilyric">
    <img src="https://img.shields.io/github/stars/apoint123/Unilyric?style=for-the-badge" alt="星标数量">
  </a>
</p>

<p align="center">
  <strong>一个强大的跨平台歌词处理工具，支持从多个音乐平台搜索、下载和转换歌词格式</strong>
</p>

---

## 🌟 主要特性

- **多平台支持**: 从 QQ音乐、网易云音乐、酷狗音乐和 AMLL TTML 数据库获取歌词
- **格式转换**: 支持 LRC、QRC、KRC、YRC、TTML、Lyricify 等多种歌词格式的相互转换
- **智能搜索**: 基于歌曲元数据（标题、艺术家、专辑）的智能匹配算法
- **批量处理**: 支持批量转换和合并多个歌词文件
- **多语言支持**: 自动处理中文简繁转换，支持翻译和罗马音歌词
- **图形界面**: 基于 egui 的现代化跨平台 GUI 界面
- **高性能**: 使用 Rust 语言开发，快速解析和处理大型歌词文件

## 🚀 快速开始

### 对于普通用户

#### 下载预编译版本

前往 [Releases 页面](https://github.com/apoint123/Unilyric/releases) 下载最新的预编译版本：

- **Windows**: 下载 `.exe` 可执行文件
- **macOS**: 下载 `.dmg` 安装包或 `.app` 文件
- **Linux**: 下载 AppImage 或二进制文件

#### 基本使用

1. 启动 UniLyric 应用程序
2. 在搜索栏输入歌曲名称和艺术家
3. 选择喜欢的歌词版本
4. 选择目标格式进行转换
5. 保存或导出歌词文件

### 对于开发者

#### 环境要求

- **Rust**: 1.70+ (推荐使用 nightly 工具链以获得最佳性能)
- **Cargo**: 最新版本
- **系统**: Windows, macOS, 或 Linux

#### 从源码构建

```bash
# 克隆仓库
git clone https://github.com/apoint123/Unilyric.git
cd Unilyric

# 构建整个工作区
cargo build --release --workspace

# 运行图形界面应用程序
cargo run --release -p Unilyric

# 运行命令行工具
cargo run --release -p lyrics_helper_rs -- --help
```

#### 安装依赖

```bash
# 更新 Rust 工具链
rustup update

# 安装 nightly 工具链（可选，用于性能优化）
rustup toolchain install nightly
rustup default nightly
```

## 📖 详细使用指南

### 图形界面操作

1. **搜索歌曲**: 在主界面的搜索框中输入歌曲信息
2. **选择提供商**: 从下拉菜单选择音乐平台（QQ、网易云、酷狗等）
3. **预览歌词**: 查看搜索结果并选择最匹配的版本
4. **格式转换**: 选择目标输出格式
5. **导出保存**: 将转换后的歌词保存到本地文件

### 命令行使用

UniLyric 也提供命令行接口，适合批量处理和自动化：

```bash
# 搜索歌曲
lyrics_helper_rs search "歌曲名" --artist "艺术家"

# 转换歌词格式
lyrics_helper_rs convert input.lrc --output output.ttml --format ttml

# 批量处理文件夹
lyrics_helper_rs batch-convert ./input_folder/ --output-dir ./output/ --format krc
```

### 配置文件

创建 `config.toml` 文件来自定义设置：

```toml
[providers]
qq_music = true
netease = true
kugou = true
amll_ttml_database = true

[conversion]
default_format = "ttml"
auto_convert_chinese = true

[ui]
theme = "dark"
language = "zh-CN"
```

## 🛠️ 开发者文档

### 项目架构

UniLyric 采用模块化设计，包含多个 Rust crate：

```
UniLyric/
├── lyrics_helper_core/     # 核心数据结构和类型定义
├── lyrics_helper_rs/       # 主要业务逻辑和API
├── ttml_processor/         # TTML 格式专门处理器
└── Unilyric/              # 图形用户界面应用程序
```

### 核心组件

- **lyrics_helper_core**: 定义通用的数据结构、错误类型和特征
- **lyrics_helper_rs**: 实现歌词提供商接口、搜索算法和格式转换
- **ttml_processor**: 高性能的 TTML 解析和生成库
- **Unilyric**: 基于 eframe/egui 的跨平台 GUI 应用

### API 使用示例

```rust
use lyrics_helper_rs::{LyricsHelper, SearchMode};
use lyrics_helper_core::Track;

#[tokio::main]
async fn main() {
    let mut helper = LyricsHelper::new();
    helper.load_providers().await.unwrap();

    let track = Track {
        title: Some("示例歌曲"),
        artists: Some(&["示例艺术家"]),
        album: None,
        duration: None,
    };

    match helper.search_lyrics(&track, SearchMode::Parallel).unwrap().await {
        Ok(Some(lyrics)) => {
            println!("找到歌词: {} 行", lyrics.lyrics.parsed.lines.len());
        }
        Ok(None) => println!("未找到歌词"),
        Err(e) => eprintln!("搜索失败: {}", e),
    }
}
```

### 开发命令

```bash
# 运行测试
cargo test --workspace

# 运行需要网络的集成测试
cargo test -- --ignored

# Clippy 代码检查
cargo clippy --workspace -- -D warnings

# 格式化代码
cargo fmt --all

# 生成文档
cargo doc --workspace --open
```

## 📊 支持的格式

### 输入格式
| 格式 | 解析支持 | 说明 |
|------|----------|------|
| LRC | ✅ | 标准歌词格式 |
| 增强型 LRC | ✅ | 带时间标签的增强格式 |
| QRC | ✅ | QQ音乐专属格式 |
| KRC | ✅ | 酷狗音乐专属格式 |
| YRC | ✅ | 云端歌词格式 |
| TTML | ✅ | Apple Music 时间文本标记语言 |
| Apple Music JSON | ✅ | Apple Music JSON 格式 |
| Lyricify Syllable | ✅ | Lyricify 音节格式 |

### 输出格式
| 格式 | 生成支持 | 说明 |
|------|----------|------|
| LRC | ✅ | 标准歌词格式 |
| 增强型 LRC | ✅ | 带时间标签的增强格式 |
| QRC | ✅ | QQ音乐专属格式 |
| KRC | ✅ | 酷狗音乐专属格式 |
| TTML | ✅ | Apple Music 时间文本标记语言 |
| Lyricify 格式 | ✅ | 多种 Lyricify 导出格式 |

## 🌐 平台支持

| 功能 | QQ音乐 | 网易云音乐 | 酷狗音乐 | AMLL TTML DB |
|------|--------|------------|----------|--------------|
| 搜索歌曲 | ✅ | ✅ | ✅ | ✅ |
| 获取歌词 | ✅ | ✅ | ✅ | ✅ |
| 歌曲信息 | ✅ | ✅ | ✅ | ❌ |
| 专辑信息 | ✅ | ✅ | ✅ | ❌ |
| 专辑封面 | ✅ | ✅ | ✅ | ❌ |
| 歌手歌曲 | ✅ | ✅ | ✅ | ❌ |
| 歌单获取 | ✅ | ✅ | ✅ | ❌ |

## 🤝 贡献指南

我们欢迎各种形式的贡献！请参考以下指南：

### 代码规范

- 遵循 Rust 官方编码规范
- 使用 `cargo fmt` 格式化代码
- 通过 `cargo clippy` 检查
- 编写适当的文档注释

### 提交信息

使用约定式提交格式：

```
feat: 添加新功能
fix: 修复bug
docs: 文档更新
style: 代码格式调整
refactor: 代码重构
test: 测试相关
```

### 开发流程

1. Fork 本项目
2. 创建特性分支 (`git checkout -b feature/AmazingFeature`)
3. 提交更改 (`git commit -m 'feat: Add AmazingFeature'`)
4. 推送到分支 (`git push origin feature/AmazingFeature`)
5. 开启 Pull Request

## 📝 许可证

本项目主要代码根据 [MIT 许可证](LICENSE) 发布。

部分第三方库可能使用不同的许可证，请查看相应的依赖项许可证信息。

## 🆘 问题与支持

如果您遇到问题或有建议：

1. 查看 [常见问题解答](#)（即将添加）
2. 搜索 [Issues](https://github.com/apoint123/Unilyric/issues) 看看是否已有相关讨论
3. 提交新的 [Issue](https://github.com/apoint123/Unilyric/issues/new/choose)
4. 加入讨论 [Discussions](https://github.com/apoint123/Unilyric/discussions)

## 🙏 致谢

感谢所有贡献者和用户的支持！特别感谢：

- [egui](https://github.com/emilk/egui) 团队提供优秀的GUI框架
- 各音乐平台API的开发者
- 所有提交问题和建议的用户

---

<p align="center">
  如果这个项目对您有帮助，请给个 ⭐ 支持一下！
</p>