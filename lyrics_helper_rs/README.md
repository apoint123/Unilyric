# Lyrics Helper RS

> [!CAUTION]
> 本库的功能尚未完善，可能会有许多 bug。

一个强大的 Rust 歌词处理库，支持从多个音乐提供商获取歌词并进行转换。

## 支持的歌词转换格式

|           格式名称            | 解析支持 | 生成支持 |
|:-------------------------:|:----:|:----:|
|          标准 LRC           |  ✅   |  ✅   |
|          增强型 LRC          |  ✅   |  ✅   |
|            QRC            |  ✅   |  ✅   |
|            KRC            |  ✅   |  ✅   |
|            YRC            |  ✅   |  ✅   |
|     Apple Music JSON      |  ✅   |  ✅   |
|           TTML            |  ✅   |  ✅   |
|     Lyricify Syllable     |  ✅   |  ✅   |
|   Lyricify Quick Export   |  ✅   |  ✅   |
|      Lyricify Lines       |  ✅   |  ✅   |
|    Salt Player Lyrics     |  ✅   |  ✅   |
| Advanced SubStation Alpha |  ✅   |  ✅   |

## 各提供商支持情况

| 功能                             | QQ音乐 | 网易云音乐 | 酷狗音乐 | AMLL TTML DB |
|:-------------------------------|:----:|:-----:|:----:|:------------:|
| `search_songs` (搜索歌曲)          |  ✅   |   ✅   |  ✅   |      ✅       |
| `get_full_lyrics` (获取歌词)       |  ✅   |   ✅   |  ✅   |      ✅       |
| `get_song_info` (获取歌曲信息)       |  ✅   |   ✅   |  ✅   |      ❌       |
| `get_album_info` (获取专辑信息)      |  ✅   |   ✅   |  ✅   |      ❌       |
| `get_album_songs` (获取专辑歌曲)     |  ✅   |   ✅   |  ✅   |      ❌       |
| `get_album_cover_url` (获取专辑封面) |  ✅   |   ✅   |  ✅   |      ❌       |
| `get_singer_songs` (获取歌手歌曲)    |  ✅   |   ✅   |  ✅   |      ❌       |
| `get_playlist` (获取歌单)          |  ✅   |   ✅   |  ✅   |      ❌       |
| `get_song_link` (获取歌曲播放链接)[^1] |  ✅   |   ✅   |  ✅   |      ❌       |

[^1]: 无法获取需要 VIP 或者付费的歌曲链接。

--- 
## 项目架构

```
src/
├── lib.rs              # 顶层入口。
├── error.rs            # 定义了自定义错误类型。
│
├── providers/          # 在线歌词源提供者
│   ├── mod.rs          #    - 定义了所有 Provider Trait。
│   ├── qq/             #    - QQ音乐源的实现。
│   ├── netease/        #    - 网易云音乐源的实现。
│   ├── kugou/          #    - 酷狗音乐源的实现。
│   └── amll_ttml_database/ - # AMLL TTML Database 源的实现。
│
├── converter/          # 核心转换与处理模块。
│   ├── mod.rs          #    - 转换功能的总入口。
│   ├── types.rs        #    - 内部的核心数据结构。
│   ├── utils.rs        #    - 包含了一些辅助函数。
│   ├── parsers/        #    - 包含所有格式的解析器。
│   ├── generators/     #    - 包含所有格式的生成器。
│   └── processors/     #    - 中间处理器，用于优化歌词。
│       ├── agent_recognizer.rs                 # - 对唱识别器。
│       ├── batch_processor.rs                  # - 批量转换器。
│       ├── chinese_conversion_processor.rs     # - 简繁转换器。
│       ├── metadata_processor.rs               # - 元数据管理器。
│       ├── metadata_stripper.rs                # - 元数据行移除器。
│       └── syllable_smoothing.rs               # - 音节平滑器。
│
├── search/             # 平台搜索与匹配
│   ├── mod.rs          #    - 智能搜索逻辑，用于聚合来自不同平台的搜索结果。
│   └── matcher.rs      #    - 具体的歌曲元信息匹配与评分算法。
│
└── model/              # 业务逻辑数据模型
    ├── mod.rs          #    - 模块声明。
    ├── track.rs        #    - 定义 `Track` (曲目信息) 和 `SearchResult` (搜索结果)。
    └── generic.rs      #    - 其他通用模型定义。
```

## 许可证

本项目采用**MIT许可**。