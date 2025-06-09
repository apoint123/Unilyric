use crate::kugou_lyrics_fetcher;
use quick_xml::Error as QuickXmlErrorMain;
use quick_xml::events::attributes::AttrError as QuickXmlAttrError;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use strum_macros::EnumString;
use thiserror::Error;

//=============================================================================
// 1. 错误枚举 (Error Enums)
//=============================================================================

/// 定义项目中所有转换和处理过程中可能发生的错误。
#[derive(Error, Debug)]
pub enum ConvertError {
    #[error("生成 XML 错误: {0}")] // XML 生成错误，源自 quick_xml
    Xml(#[from] QuickXmlErrorMain),
    #[error("XML 属性错误: {0}")] // XML 属性解析错误，源自 quick_xml
    Attribute(#[from] QuickXmlAttrError),
    #[error("行 {0}: 无效的 ASS 字幕时间格式: '{1}'")] // ASS 时间格式无效
    InvalidAssTime(usize, String),
    #[error("行 {0}: 无效的 K 格式: {1}")] // ASS 卡拉OK标签格式无效
    InvalidAssKaraoke(usize, String),
    #[error("行 {line_num} 说话人冲突: {tags:?}")] // ASS 文件中同一行出现冲突的说话人标签
    ConflictingActorTags { line_num: usize, tags: Vec<String> },
    #[error("解析错误: {0}")] // 通用整数解析错误
    ParseInt(#[from] std::num::ParseIntError),
    #[error("无效的时间格式: {0}")] // 通用时间格式无效
    InvalidTime(String),
    #[error("格式错误: {0}")] // 字符串格式化错误
    Format(#[from] std::fmt::Error),
    #[error("错误: {0}")] // 内部逻辑错误或未分类错误
    Internal(String),
    #[error("无效的 LYS 行格式 (行 {line_num}): {message}")] // LYS 文件行格式无效
    InvalidLysFormat { line_num: usize, message: String },
    #[error("无效的 LYS 属性 (行 {line_num}): '{property_str}'")] // LYS 文件属性无效
    InvalidLysProperty {
        line_num: usize,
        property_str: String,
    },
    #[error("无效的 LYS 音节 (行 {line_num}, 文本 '{text}'): {message}")] // LYS 音节数据无效
    InvalidLysSyllable {
        line_num: usize,
        text: String,
        message: String,
    },
    #[error("无效的 QRC 行格式 (行 {line_num}): {message}")] // QRC 文件行格式无效
    InvalidQrcFormat { line_num: usize, message: String },
    #[error("无效的 QRC 行时间戳 (行 {line_num}): '{timestamp_str}'")] // QRC 行时间戳无效
    InvalidQrcLineTimestamp {
        line_num: usize,
        timestamp_str: String,
    },
    #[error("IO 错误: {0}")] // 文件读写等IO错误
    Io(#[from] io::Error),
    #[error("JSON 解析错误: {0}")] // JSON 解析错误
    JsonParse(#[from] serde_json::Error),
    #[error("JSON 结构无效: {0}")] // JSON 结构不符合预期
    InvalidJsonStructure(String),
    #[error("网络请求错误: {0}")] // 网络请求错误 (例如，下载歌词时)
    NetworkRequest(#[from] reqwest::Error),
    #[error("QQ音乐API返回错误: {0}")] // QQ音乐API特定错误
    QqMusicApiError(String),
    #[error("歌词内容未找到")] // 未找到歌词
    LyricNotFound,
    #[error("Base64 解码错误: {0}")] // Base64 解码失败
    Base64Decode(#[from] base64::DecodeError),
    #[error("UTF-8 转换错误: {0}")] // 从字节序列转换为 UTF-8 字符串失败
    FromUtf8(#[from] std::string::FromUtf8Error),
    #[error("时间转换错误 (SystemTime): {0}")] // 系统时间相关的错误
    SystemTime(#[from] std::time::SystemTimeError),
    #[error("无效的十六进制字符串: {0}")] // 十六进制字符串解析失败
    InvalidHex(String),
    #[error("解压缩错误: {0}")] // Zlib等解压缩失败
    Decompression(#[source] std::io::Error),
    #[error("QQ音乐服务器拒绝了你的搜索请求 (代码2001)，请稍后再试")] // QQ音乐API特定错误代码
    RequestRejected,
    #[error("酷狗歌词获取/处理错误: {0}")] // 酷狗歌词获取器返回的错误
    KugouFetcher(#[from] kugou_lyrics_fetcher::error::KugouError),
}

/// 定义歌词获取函数可能返回的错误类型。
#[derive(Debug, Clone, thiserror::Error)]
pub enum LyricFetchError {
    #[error("歌曲未找到")]
    NotFound,
    #[error("请求超时")]
    Timeout,
    #[error("网络错误: {0}")]
    NetworkError(String),
    #[error("解析错误: {0}")]
    ParseError(String),
    #[error("API客户端错误: {0}")]
    ApiClientError(String),
    #[error("内部错误: {0}")]
    InternalError(String),
}

/// 定义从字符串解析 CanonicalMetadataKey 时可能发生的错误。
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ParseCanonicalMetadataKeyError(String); // 存储无法解析的原始键字符串

impl std::fmt::Display for ParseCanonicalMetadataKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "未知或无效的元数据键: {}", self.0)
    }
}

impl std::error::Error for ParseCanonicalMetadataKeyError {}

//=============================================================================
// 2. 核心歌词格式枚举及相关 (Core Lyric Format Enum & Related)
//=============================================================================

/// 枚举：表示支持的歌词格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString)]
#[strum(ascii_case_insensitive)]
pub enum LyricFormat {
    Ass,  // Advanced SubStation Alpha
    Ttml, // Timed Text Markup Language
    Json, // Apple Music JSON (内嵌TTML)
    Lys,  // Lyricify Syllable Format
    Lrc,  // Standard LRC
    Qrc,  // QQ Music QRC
    Yrc,  // NetEase YRC
    Lyl,  // Lyricify Lines Format
    Spl,  // Salt Player Lyrics
    Lqe,  // Lyricify Quick Export
    Krc,  // Kugou KRC
}

impl fmt::Display for LyricFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LyricFormat::Ass => write!(f, "ASS"),
            LyricFormat::Ttml => write!(f, "TTML"),
            LyricFormat::Json => write!(f, "JSON"),
            LyricFormat::Lys => write!(f, "Lyricify Syllable"),
            LyricFormat::Lrc => write!(f, "LRC"),
            LyricFormat::Qrc => write!(f, "QRC"),
            LyricFormat::Yrc => write!(f, "YRC"),
            LyricFormat::Lyl => write!(f, "Lyricify Lines"),
            LyricFormat::Spl => write!(f, "SPL"),
            LyricFormat::Lqe => write!(f, "Lyricify Quick Export"),
            LyricFormat::Krc => write!(f, "KRC"),
        }
    }
}

impl LyricFormat {
    /// 返回所有支持的歌词格式的列表。
    pub fn all() -> Vec<Self> {
        vec![
            LyricFormat::Ass,
            LyricFormat::Ttml,
            LyricFormat::Json,
            LyricFormat::Lys,
            LyricFormat::Lrc,
            LyricFormat::Qrc,
            LyricFormat::Yrc,
            LyricFormat::Lyl,
            LyricFormat::Spl,
            LyricFormat::Lqe,
            LyricFormat::Krc,
        ]
    }

    /// 将歌词格式枚举转换为文件扩展名字符串。
    pub fn to_extension_str(self) -> &'static str {
        match self {
            LyricFormat::Ass => "ass",
            LyricFormat::Ttml => "ttml",
            LyricFormat::Json => "json",
            LyricFormat::Lys => "lys",
            LyricFormat::Lrc => "lrc",
            LyricFormat::Qrc => "qrc",
            LyricFormat::Yrc => "yrc",
            LyricFormat::Lyl => "lyl",
            LyricFormat::Spl => "spl",
            LyricFormat::Lqe => "lqe",
            LyricFormat::Krc => "krc",
        }
    }

    /// 从字符串（通常是文件扩展名或用户输入）解析歌词格式枚举。
    /// 不区分大小写，并移除空格和点。
    pub fn from_string(s: &str) -> Option<Self> {
        let normalized_s = s.to_uppercase().replace([' ', '.'], "");

        match normalized_s.as_str() {
            "ASS" | "SUBSTATIONALPHA" | "SSA" => Some(LyricFormat::Ass),
            "TTML" | "XML" => Some(LyricFormat::Ttml),
            "JSON" => Some(LyricFormat::Json),
            "LYS" | "LYRICIFYSYLLABLE" => Some(LyricFormat::Lys),
            "LRC" => Some(LyricFormat::Lrc),
            "QRC" => Some(LyricFormat::Qrc),
            "YRC" => Some(LyricFormat::Yrc),
            "LYL" | "LYRICIFYLINES" => Some(LyricFormat::Lyl),
            "SPL" => Some(LyricFormat::Spl),
            "LQE" | "LYRICIFYQUICKEXPORT" => Some(LyricFormat::Lqe),
            "KRC" => Some(LyricFormat::Krc),
            _ => {
                log::error!("[UniLyric] 未知的格式: {}", s);
                None
            }
        }
    }
}

//=============================================================================
// 3. 特定格式结构体 (Format-Specific Structs)
//=============================================================================

//-----------------------------------------------------------------------------
// 3.1 ASS (Advanced SubStation Alpha) 相关结构体
//-----------------------------------------------------------------------------

/// 定义 ASS 文件中说话人的角色。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Copy)]
pub enum ActorRole {
    Vocal1,     // 主唱1
    Vocal2,     // 主唱2
    Background, // 背景和声/伴唱
    Chorus,     // 合唱 (通常由 v1000 表示)
}

/// 表示 ASS 文件中一个逐字音节。
#[derive(Debug, Clone, PartialEq)]
pub struct AssSyllable {
    pub text: String,          // 音节文本
    pub start_ms: u64,         // 音节开始时间 (毫秒)
    pub end_ms: u64,           // 音节结束时间 (毫秒)
    pub ends_with_space: bool, // 该音节后是否应有空格 (用于TTML转换时处理空格)
}

/// 表示 ASS 文件中一行的内容。
/// 可以是一行歌词，也可以是主翻译或主罗马音。
#[derive(Debug, Clone, PartialEq)]
pub enum AssLineContent {
    LyricLine {
        // 歌词行
        role: ActorRole,                                  // 演唱者角色
        syllables: Vec<AssSyllable>,                      // 音节列表
        bg_translation: Option<(Option<String>, String)>, // 背景歌词的翻译 (可选语言代码, 翻译文本)
        bg_romanization: Option<String>,                  // 背景歌词的罗马音
    },
    MainTranslation {
        // 主翻译行
        lang_code: Option<String>, // 翻译的语言代码 (例如 "en", "ja")
        text: String,              // 翻译文本
    },
    MainRomanization {
        // 主罗马音行
        text: String, // 罗马音文本
    },
}

/// 表示从 ASS 文件解析出的一行完整信息。
#[derive(Debug, Clone, PartialEq)]
pub struct AssLineInfo {
    pub line_num: usize,                 // 原始文件中的行号
    pub start_ms: u64,                   // 行开始时间 (毫秒)
    pub end_ms: u64,                     // 行结束时间 (毫秒)
    pub content: Option<AssLineContent>, // 行的具体内容
    pub song_part: Option<String>,       // 歌词组成部分标记 (例如 "Verse 1", "Chorus")
}

/// 表示从 ASS 说话人字段解析出的信息。
#[derive(Debug)]
pub struct ParsedActor {
    pub role: Option<ActorRole>,   // 演唱者角色
    pub is_background: bool,       // 是否为背景
    pub lang_code: Option<String>, // 语言代码 (用于翻译/罗马音)
    pub is_marker: bool,           // 是否为标记行
    pub song_part: Option<String>, // 歌曲组成部分
}

/// 包含从 ASS 文件解析出的所有相关数据。
#[derive(Debug)]
pub struct ProcessedAssData {
    pub lines: Vec<AssLineInfo>,                       // 解析出的所有行信息
    pub metadata: Vec<AssMetadata>,                    // 文件级元数据
    pub markers: Vec<MarkerInfo>,                      // 标记信息
    pub apple_music_id: String,                        // Apple Music ID (如果找到)
    pub songwriters: Vec<String>,                      // 创作者列表
    pub language_code: Option<String>,                 // 主语言代码
    pub agent_names: HashMap<String, String>, // 演唱者 ID 到 演唱者 名称的映射 (例如 "v1" -> "歌手A")
    pub detected_translation_language: Option<String>, // 检测到的翻译语言
}

//-----------------------------------------------------------------------------
// 3.2 TTML (Timed Text Markup Language) 相关结构体
//-----------------------------------------------------------------------------

/// 表示 TTML 文件中一个逐字音节。
/// 这是项目内部处理逐字歌词时的一个核心结构。
#[derive(Default, Debug, Clone, PartialEq)]
pub struct TtmlSyllable {
    pub text: String,          // 音节文本
    pub start_ms: u64,         // 音节开始时间 (毫秒)
    pub end_ms: u64,           // 音节结束时间 (毫秒)
    pub ends_with_space: bool, // 该音节后是否应有空格
}

/// 表示 TTML 中背景歌词的部分 (`<span ttm:role="x-bg">...</span>`)。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct BackgroundSection {
    pub start_ms: u64,                                 // 背景部分开始时间
    pub end_ms: u64,                                   // 背景部分结束时间
    pub syllables: Vec<TtmlSyllable>,                  // 背景音节列表
    pub translation: Option<(String, Option<String>)>, // 背景部分的翻译 (文本, 可选语言代码)
    pub romanization: Option<String>,                  // 背景部分的罗马音
}

/// 表示 TTML 中的一个段落 (`<p>` 标签)。
/// 这是项目内部表示歌词数据的主要结构之一，用于在不同格式间转换。
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TtmlParagraph {
    pub p_start_ms: u64,                               // 段落开始时间 (毫秒)
    pub p_end_ms: u64,                                 // 段落结束时间 (毫秒)
    pub agent: String,                                 // 演唱者 (例如 "v1", "v2")
    pub main_syllables: Vec<TtmlSyllable>,             // 主歌词音节列表
    pub background_section: Option<BackgroundSection>, // 可选的背景歌词部分
    pub translation: Option<(String, Option<String>)>, // 可选的主翻译 (翻译文本, 可选语言代码)
    pub romanization: Option<String>,                  // 可选的主罗马音
    pub song_part: Option<String>,                     // 可选的歌曲组成部分标记
}

//-----------------------------------------------------------------------------
// 3.3 Apple Music JSON 相关结构体
//-----------------------------------------------------------------------------
// 这些结构用于解析从 Apple Music 获取的 JSON 文件，该响应内嵌 TTML。

#[derive(Serialize, Debug, Deserialize)]
pub struct AppleMusicPlayParams {
    // 播放参数
    pub id: String,
    pub kind: String,
    #[serde(rename = "catalogId")]
    pub catalog_id: String,
    #[serde(rename = "displayType")]
    pub display_type: u8,
}

#[derive(Serialize, Debug, Deserialize)]
pub struct AppleMusicAttributes {
    // 属性，包含 TTML 字符串和播放参数
    pub ttml: String, // 内嵌的 TTML 歌词内容
    #[serde(rename = "playParams")]
    pub play_params: AppleMusicPlayParams,
}

#[derive(Serialize, Debug, Deserialize)]
pub struct AppleMusicDataObject {
    // 数据对象
    pub id: String,
    #[serde(rename = "type")]
    pub data_type: String, // 总是为 "syllable-lyrics"
    pub attributes: AppleMusicAttributes,
}

#[derive(Serialize, Debug, Deserialize)]
pub struct AppleMusicRoot {
    // JSON 根结构
    pub data: Vec<AppleMusicDataObject>,
}

//-----------------------------------------------------------------------------
// 3.4 LYS (Lyricify Syllable) / QRC / KRC 音节结构体
//-----------------------------------------------------------------------------

/// 表示 LYS, QRC, KRC 等格式中的一个音节。
/// 注意：对于 QRC 和 LYS，`start_ms` 是相对于歌曲开始的绝对时间。
/// 对于 KRC，`start_ms` 是相对于该行第一个音节的偏移时间。
#[derive(Debug, Clone, PartialEq)]
pub struct LysSyllable {
    pub text: String,     // 音节文本
    pub start_ms: u64,    // 开始时间 (毫秒)，具体含义取决于格式
    pub duration_ms: u64, // 持续时间 (毫秒)
}

//-----------------------------------------------------------------------------
// 3.5 LYS (Lyricify Syllable) 相关结构体
//-----------------------------------------------------------------------------

/// 表示 LYS 文件中的一行歌词。
#[derive(Debug, Clone, PartialEq)]
pub struct LysLine {
    pub property: u8,                // 行属性 (决定左右、是否背景)
    pub syllables: Vec<LysSyllable>, // 该行的音节列表
}

//-----------------------------------------------------------------------------
// 3.6 QRC (QQ Music) / YRC (NetEase) 相关结构体
//-----------------------------------------------------------------------------

/// 表示 QRC 或 YRC 文件中的一行歌词。
/// QRC 和 YRC 的行结构和音节结构（使用 LysSyllable）相似。
#[derive(Debug, Clone, PartialEq)]
pub struct QrcLine {
    // 也可用于 YRC
    pub line_start_ms: u64,          // 行开始的绝对时间 (毫秒)
    pub line_duration_ms: u64,       // 行的整体持续时间 (毫秒)
    pub syllables: Vec<LysSyllable>, // 音节列表 (音节的 start_ms 是绝对时间)
}

//-----------------------------------------------------------------------------
// 3.7 LRC (LyRiCs) 相关结构体
//-----------------------------------------------------------------------------

/// 表示 LRC 文件中的一行歌词。
#[derive(Debug, Clone, PartialEq)]
pub struct LrcLine {
    pub timestamp_ms: u64, // 时间戳 (毫秒)
    pub text: String,      // 该时间戳对应的歌词文本
}

/// 表示在UI可编辑LRC面板中的一行，可以是已解析的或原始文本
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayLrcLine {
    Parsed(LrcLine),               // 成功解析的行
    Raw { original_text: String }, // 原始未解析的行
}

/// 枚举：用于指示加载的 LRC 文件内容是翻译还是罗马音。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LrcContentType {
    Translation,
    Romanization,
}

//-----------------------------------------------------------------------------
// 3.8 LYL (Lyricify Lines) 相关结构体
//-----------------------------------------------------------------------------

/// 表示从 Lyricify Lines (LYL) 格式解析出的一行。
#[derive(Debug, Clone)]
pub struct ParsedLyricifyLine {
    pub start_ms: u64, // 行开始时间 (毫秒)
    pub end_ms: u64,   // 行结束时间 (毫秒)
    pub text: String,  // 行文本
}

//-----------------------------------------------------------------------------
// 3.9 LQE (Lyricify Quick Export) 相关结构体
//-----------------------------------------------------------------------------

/// 表示 LQE 文件中的一个区段 (例如 [lyrics:...], [translation:...])。
#[derive(Debug, Clone, Default)]
pub struct LqeSection {
    pub format: Option<LyricFormat>, // 该区段内容的格式 (例如 LYS, LRC)
    pub language: Option<String>,    // 该区段内容的语言代码
    pub content: String,             // 区段的原始文本内容
}

/// 表示从 LQE 文件解析出的完整数据。
#[derive(Debug, Clone, Default)]
pub struct ParsedLqeData {
    pub version: Option<String>,                   // LQE 文件版本
    pub global_metadata: Vec<AssMetadata>,         // 全局元数据 (文件头部)
    pub lyrics_section: Option<LqeSection>,        // 主歌词区段
    pub translation_section: Option<LqeSection>,   // 翻译区段
    pub pronunciation_section: Option<LqeSection>, // 音译/罗马音区段
}

// KRC (Kugou) 格式通常直接使用 QrcLine 和 LysSyllable 结构，
// 但音节的 start_ms 是相对行首的偏移。

//=============================================================================
// 4. 元数据结构体 (Metadata Structs)
//=============================================================================

/// 表示一条元数据，通常是从注释行解析出来的。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct AssMetadata {
    pub key: String,   // 元数据键 (例如 "title", "artist")
    pub value: String, // 元数据值
}

/// 定义元数据的标准（规范化）键。
/// 用于在内部统一表示从不同来源获取的元数据，方便查询和处理。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CanonicalMetadataKey {
    Title,                  // 歌曲名
    Artist,                 // 艺术家
    Album,                  // 专辑
    Author,                 // 作者/LRC制作者 (通常对应 [by:] 或 ttmlAuthor)
    Songwriter,             // 作词/作曲 (通常对应 TTML <songwriter>)
    Language,               // 主语言代码
    Offset,                 // 时间偏移
    Length,                 // 歌曲总时长
    Editor,                 // 编辑器/工具版本 (通常对应 [re:])
    Version,                // 歌词文件版本 (通常对应 [ve:])
    AppleMusicId,           // Apple Music ID
    KrcInternalTranslation, // KRC内部的 [language:xxx] 标签值 (Base64编码)
    Custom(String),         // 其他自定义或特定平台的元数据键
}

impl fmt::Display for CanonicalMetadataKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CanonicalMetadataKey::Title => write!(f, "Title"),
            CanonicalMetadataKey::Artist => write!(f, "Artist"),
            CanonicalMetadataKey::Album => write!(f, "Album"),
            CanonicalMetadataKey::Author => write!(f, "Author"),
            CanonicalMetadataKey::Songwriter => write!(f, "Songwriter"),
            CanonicalMetadataKey::Language => write!(f, "Language"),
            CanonicalMetadataKey::Offset => write!(f, "Offset"),
            CanonicalMetadataKey::Length => write!(f, "Length"),
            CanonicalMetadataKey::Editor => write!(f, "Editor"),
            CanonicalMetadataKey::Version => write!(f, "Version"),
            CanonicalMetadataKey::AppleMusicId => write!(f, "AppleMusicId"),
            CanonicalMetadataKey::KrcInternalTranslation => write!(f, "KrcInternalTranslation"),
            CanonicalMetadataKey::Custom(s) => write!(f, "Custom({})", s),
        }
    }
}

impl FromStr for CanonicalMetadataKey {
    type Err = ParseCanonicalMetadataKeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "ti" | "title" | "musicname" | "songname" => Ok(Self::Title),
            "ar" | "artist" | "artists" | "singer" | "singername" => Ok(Self::Artist),
            "al" | "album" | "album_name" => Ok(Self::Album),
            "by" | "author" | "creator" | "ttmlauthorgithublogin" | "lyricist" => Ok(Self::Author),
            "songwriter" | "songwriters" => Ok(Self::Songwriter),
            "offset" => Ok(Self::Offset),
            "length" | "duration" => Ok(Self::Length),
            "ve" | "version" => Ok(Self::Version),
            "re" | "editor" => Ok(Self::Editor),
            "lang" | "language" | "xml:lang" | "lyrics_language" => Ok(Self::Language),
            "krcinternaltranslationbase64value" | "krc_internal_translation" => {
                Ok(Self::KrcInternalTranslation)
            }
            "applemusicid" | "apple_music_id" => Ok(Self::AppleMusicId),
            "v1" => Ok(Self::Custom("v1".to_string())), // 通常用于 TTML agent
            "v2" => Ok(Self::Custom("v2".to_string())),
            "v1000" => Ok(Self::Custom("v1000".to_string())),
            "isrc" => Ok(Self::Custom("isrc".to_string())),
            "ncmmusicid" => Ok(Self::Custom("ncmMusicId".to_string())),
            "qqmusicid" => Ok(Self::Custom("qqMusicId".to_string())),
            "spotifymusicid" | "spotifyid" => Ok(Self::Custom("spotifyId".to_string())),
            "ttmlauthorgithub" => Ok(Self::Custom("ttmlAuthorGithub".to_string())),
            // 其他非空字符串都被视为自定义键
            custom_key if !custom_key.is_empty() => Ok(Self::Custom(custom_key.to_string())),
            _ => Err(ParseCanonicalMetadataKeyError(s.to_string())), // 解析失败
        }
    }
}

impl CanonicalMetadataKey {
    /// 将 CanonicalMetadataKey 转换为一个用于显示或作为元数据编辑器中键的代表性字符串。
    /// 这个字符串也应该能够被 `Self::from_str` 解析（如果适用）。
    pub fn to_display_key(&self) -> String {
        match self {
            Self::Title => "musicName".to_string(), // 倾向于使用 Apple Music TTML 中的键名
            Self::Artist => "artists".to_string(),  // 使用复数形式
            Self::Album => "album".to_string(),
            Self::Author => "ttmlAuthorGithubLogin".to_string(), // TTML中作者的Github登录名
            Self::Songwriter => "songwriters".to_string(),       // 作曲作词者
            Self::Language => "language".to_string(),
            Self::Offset => "offset".to_string(),
            Self::Length => "length".to_string(),
            Self::Editor => "editor".to_string(),
            Self::Version => "version".to_string(),
            Self::KrcInternalTranslation => "krcInternalTranslationBase64Value".to_string(),
            Self::AppleMusicId => "appleMusicId".to_string(),
            Self::Custom(s) => s.clone(), // 自定义键直接返回其字符串
        }
    }

    /// 获取此标准键在 Group 1 格式 (LRC, QRC, KRC, YRC, LYS) 中对应的标签名。
    /// LQE 的全局元数据也可能使用这些。返回 Option<&str>。
    pub fn get_group1_tag_name_for_lrc_qrc(&self) -> Option<&str> {
        match self {
            Self::Title => Some("ti"),
            Self::Artist => Some("ar"),
            Self::Album => Some("al"),
            Self::Author => Some("by"), // LRC 'by' 通常指LRC制作者
            Self::Offset => Some("offset"),
            Self::Length => Some("length"),
            Self::Version => Some("ve"),
            Self::Editor => Some("re"),
            Self::Language => Some("lang"), // 有些格式可能用 [language:xx]
            Self::KrcInternalTranslation => None, // KRC 的 [language:xxx] 标签
            // 以下通常不在 Group 1 格式的头部标签中，或者有特定处理逻辑
            Self::Songwriter => None,
            Self::AppleMusicId => None,
            Self::Custom(_) => None,
        }
    }
}

/// 元数据编辑器中的一个可编辑条目
#[derive(Clone, Debug)]
pub struct EditableMetadataEntry {
    pub key: String,        // 元数据键名 (显示用)
    pub value: String,      // 元数据值
    pub is_pinned: bool,    // 此条目是否被用户标记为“固定”
    pub is_from_file: bool, // 此条目是否来自当前加载的文件 (或为固定项的初始状态)
    pub id: egui::Id,       // egui 用于追踪UI元素的唯一ID
}

//=============================================================================
// 5. 处理与数据结构体 (Processing & Data Structs)
//=============================================================================

/// 存储从源文件解析出的、准备进行进一步处理或转换的歌词数据。
/// 这是一个非常核心的中间数据结构。
#[derive(Debug, Default, Clone)]
pub struct ParsedSourceData {
    pub paragraphs: Vec<TtmlParagraph>, // 主要的歌词内容，以 TTML 段落列表形式存储
    pub language_code: Option<String>,  // 主歌词语言代码
    pub songwriters: Vec<String>,       // 作曲者列表 (用于TTML iTunesMetadata)
    pub agent_names: HashMap<String, String>, // Agent ID (如 "v1") 到显示名称的映射
    pub apple_music_id: String,         // Apple Music ID
    pub general_metadata: Vec<AssMetadata>, // 从源文件解析的其他通用元数据
    pub markers: Vec<MarkerInfo>,       // 标记信息 (例如从 ASS 的 Comment 行提取)
    pub is_line_timed_source: bool,     // 指示源文件是否为逐行歌词 (如 LRC, LYL)
    pub raw_ttml_from_input: Option<String>, // 如果源是 TTML 或 JSON(内嵌TTML)，则存储原始TTML字符串

    // LQE (Lyricify Quick Export) 格式特有的字段
    // 用于存储当主歌词部分为空时，从 LQE 文件中提取的、但无法立即合并的 LRC 内容
    pub lqe_extracted_translation_lrc_content: Option<String>, // 提取的翻译LRC文本
    pub lqe_extracted_romanization_lrc_content: Option<String>, // 提取的罗马音LRC文本
    pub lqe_translation_language: Option<String>,              // LQE 翻译区段的语言
    pub lqe_romanization_language: Option<String>,             // LQE 音译区段的语言

    pub detected_formatted_input: Option<bool>, // 指示输入的 TTML/JSON 是否可能被格式化过 (影响空格处理)
    pub _source_translation_language: Option<String>, // 从源文件（如ASS）检测到的翻译语言
    pub lqe_main_lyrics_as_lrc: bool,           // 当LQE作为输出时，指示主歌词是否应为LRC格式
    pub lqe_direct_main_lrc_content: Option<String>, // LQE生成时直接使用的主LRC内容（如果适用）

    /// 用于存储从双语LRC主输入中直接解析出的翻译行
    pub bilingual_extracted_translations: Option<Vec<LrcLine>>,
}

/// 封装从各个歌词源获取并初步处理后的统一数据结构。
#[derive(Debug, Clone)]
pub struct ProcessedLyricsSourceData {
    pub format: LyricFormat,
    pub main_lyrics: String,
    pub translation_lrc: Option<String>,  // 预处理后的翻译LRC
    pub romanization_qrc: Option<String>, // QRC格式罗马音
    pub romanization_lrc: Option<String>, // LRC格式罗马音
    pub krc_translation_lines: Option<Vec<String>>, // KRC内嵌行翻译
    pub platform_metadata: HashMap<String, String>, // 从平台获取的元数据
}

//=============================================================================
// 6. 歌词获取与搜索结构体/枚举 (Lyric Fetching & Search Structs/Enums)
//=============================================================================

/// 枚举，用于标识自动歌词搜索的源及其顺序
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutoSearchSource {
    LocalCache, // 本地缓存
    QqMusic,    // QQ音乐
    Kugou,      // 酷狗音乐
    Netease,    // 网易云音乐
    AmllDb,     // AMLL-DB
}

impl AutoSearchSource {
    /// 返回所有可用的搜索源的默认顺序
    pub fn default_order() -> Vec<Self> {
        vec![
            Self::LocalCache, // 本地缓存通常应该最先尝试
            Self::AmllDb,
            Self::QqMusic,
            Self::Kugou,
            Self::Netease,
        ]
    }

    /// 将枚举成员转换为用于存储的字符串标识符
    pub fn to_id_string(self) -> &'static str {
        match self {
            AutoSearchSource::LocalCache => "local",
            AutoSearchSource::QqMusic => "qq",
            AutoSearchSource::Kugou => "kugou",
            AutoSearchSource::Netease => "netease",
            AutoSearchSource::AmllDb => "amll",
        }
    }

    /// 获取用于UI显示的名称
    pub fn display_name(&self) -> &'static str {
        match self {
            AutoSearchSource::LocalCache => "本地歌词",
            AutoSearchSource::QqMusic => "QQ音乐",
            AutoSearchSource::Kugou => "酷狗音乐",
            AutoSearchSource::Netease => "网易云音乐",
            AutoSearchSource::AmllDb => "AMLL-DB",
        }
    }
}

impl FromStr for AutoSearchSource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "local" | "localcache" => Ok(AutoSearchSource::LocalCache),
            "qq" | "qqmusic" => Ok(AutoSearchSource::QqMusic),
            "kugou" => Ok(AutoSearchSource::Kugou),
            "netease" | "ncm" => Ok(AutoSearchSource::Netease),
            "amll" | "amlldb" => Ok(AutoSearchSource::AmllDb),
            _ => Err(format!("未知的自动搜索源标识符: {}", s)),
        }
    }
}

/// 代表本地歌词缓存索引中的一个条目
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalLyricCacheEntry {
    pub smtc_title: String,
    pub smtc_artists: Vec<String>,
    pub ttml_filename: String,
    pub saved_timestamp: u64,                   // UNIX 时间戳 (秒)
    pub original_source_format: Option<String>, // 原始格式的字符串表示
}

impl LocalLyricCacheEntry {
    pub fn new(
        smtc_title: String,
        smtc_artists: Vec<String>,
        ttml_filename: String,
        original_source_format: Option<LyricFormat>,
    ) -> Self {
        Self {
            smtc_title,
            smtc_artists,
            ttml_filename,
            saved_timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            original_source_format: original_source_format.map(|f| f.to_string().to_string()),
        }
    }
}

/// 自动获取歌词结果的枚举 (用于从异步任务传递回主线程)
#[derive(Debug, Clone)]
pub enum AutoFetchResult {
    Success {
        source: AutoSearchSource,
        source_format: LyricFormat,
        main_lyrics: String,
        translation_lrc: Option<String>,
        romanization_qrc: Option<String>,
        romanization_lrc: Option<String>,
        krc_translation_lines: Option<Vec<String>>,
        platform_metadata: HashMap<String, String>,
    },
    NotFound,
    FetchError(String),
}

/// 代表从不同平台获取的原始歌词数据，用于在下载后统一处理。
#[derive(Debug, Clone)]
pub enum PlatformFetchedData {
    Qq(crate::qq_lyrics_fetcher::qqlyricsfetcher::FetchedQqLyrics),
    Kugou(crate::kugou_lyrics_fetcher::FetchedKrcLyrics),
    Netease(crate::netease_lyrics_fetcher::FetchedNeteaseLyrics),
    Amll(crate::amll_lyrics_fetcher::FetchedAmllTtmlLyrics),
}

//=============================================================================
// 7. UI 状态枚举 (UI State Enums)
//=============================================================================

/// QQ音乐下载状态枚举
#[derive(Debug, Clone)]
pub enum QqMusicDownloadState {
    Idle,                                                                // 空闲状态
    Downloading,                                                         // 下载中
    Success(crate::qq_lyrics_fetcher::qqlyricsfetcher::FetchedQqLyrics), // 下载成功，包含获取到的歌词数据
    Error(String),                                                       // 下载失败，包含错误信息
}

/// 酷狗音乐KRC歌词下载状态枚举
#[derive(Debug, Clone)]
pub enum KrcDownloadState {
    Idle,                                                   // 空闲状态
    Downloading,                                            // 下载中
    Success(crate::kugou_lyrics_fetcher::FetchedKrcLyrics), // 下载成功
    Error(String),                                          // 下载失败
}

/// 网易云音乐歌词下载状态枚举
#[derive(Debug, Clone)]
pub enum NeteaseDownloadState {
    Idle,                                                         // 空闲状态
    InitializingClient,                                           // 正在初始化API客户端
    Downloading,                                                  // 下载中
    Success(crate::netease_lyrics_fetcher::FetchedNeteaseLyrics), // 下载成功
    Error(String),                                                // 下载失败
}

/// AMLL-DB 索引下载状态枚举
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmllIndexDownloadState {
    Idle,
    CheckingForUpdate,           // 正在检查更新
    UpdateAvailable(String),     // 有可用更新 (String 是远程新的 HEAD SHA)
    Downloading(Option<String>), // 下载时可以附带期望的远程 HEAD
    Success(String),             // 成功加载，并存储加载时的 HEAD SHA
    Error(String),
}

/// AMLL-DB TTML歌词下载状态枚举
#[derive(Debug, Clone, PartialEq)]
pub enum AmllTtmlDownloadState {
    Idle,                                                       // 空闲
    SearchingIndex,                                             // 正在搜索索引
    DownloadingTtml,                                            // 正在下载TTML
    Success(crate::amll_lyrics_fetcher::FetchedAmllTtmlLyrics), // 成功
    Error(String),                                              // 错误
}

/// 自动歌词搜索状态枚举
#[derive(Debug, Clone, PartialEq, Default)]
pub enum AutoSearchStatus {
    #[default]
    NotAttempted, // 未尝试
    Searching,            // 搜索中
    Success(LyricFormat), // 成功，并指明找到的格式
    NotFound,             // 未找到
    Error(String),        // 发生错误
}

//=============================================================================
// 8. 辅助类型与函数
//=============================================================================

/// 用于存储 ASS 文件中的标记信息 (通常是特定 actor 标记的注释行)。
pub type MarkerInfo = (usize, String); // (行号, 标记文本)

/// 代表UI中显示和配置单个自动搜索源所需的数据。
/// (源枚举, 搜索状态的Arc<Mutex>, 可选的已处理歌词数据的Arc<Mutex<Option<...>>>)
pub type SourceConfigTuple = (
    AutoSearchSource,
    Arc<Mutex<AutoSearchStatus>>,
    Option<Arc<Mutex<Option<ProcessedLyricsSourceData>>>>,
);

/// 将自动搜索源的顺序列表转换为逗号分隔的字符串。
pub fn search_order_to_string(order: &[AutoSearchSource]) -> String {
    order
        .iter()
        .map(|s| s.to_id_string())
        .collect::<Vec<&str>>()
        .join(",")
}

/// 将逗号分隔的字符串转换为自动搜索源的顺序列表。
/// 会处理重复项和未知项，并确保所有默认源都存在于结果中。
pub fn string_to_search_order(order_str: &str) -> Vec<AutoSearchSource> {
    let mut ordered_sources = Vec::new();
    let mut seen_sources = HashSet::new(); // 使用 HashSet 跟踪已添加的源

    for id_str in order_str.split(',') {
        if let Ok(source) = AutoSearchSource::from_str(id_str.trim()) {
            if seen_sources.insert(source) {
                // 仅当源未被添加过时才添加
                ordered_sources.push(source);
            }
        } else {
            log::warn!("[设置] 加载搜索顺序时发现未知源ID: {}", id_str);
        }
    }

    // 确保所有默认源都存在，如果缺失则添加到末尾
    for default_source in AutoSearchSource::default_order() {
        if !seen_sources.contains(&default_source) {
            ordered_sources.push(default_source);
            // 不需要再次插入到 seen_sources，因为它仅用于初始解析阶段的去重
            log::debug!(
                "[设置] 自动搜索顺序中缺失源 {:?}，已追加到末尾。",
                default_source.display_name()
            );
        }
    }
    ordered_sources
}