use std::{collections::HashMap, fmt, str::FromStr};

use derive_builder::Builder;
use serde::{Deserialize, Serialize};
use strum_macros::{EnumIter, EnumString};

use crate::ParseCanonicalMetadataKeyError;

/// 枚举：表示支持的歌词格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumString, Serialize, Deserialize, EnumIter)]
#[strum(ascii_case_insensitive)]
#[derive(Default)]
pub enum LyricFormat {
    /// `Advanced SubStation Alpha` 格式。
    Ass,
    /// `Timed Text Markup Language` 格式。
    #[default]
    Ttml,
    /// `Apple Music JSON` 格式 (内嵌TTML)。
    AppleMusicJson,
    /// `Lyricify Syllable` 格式。
    Lys,
    /// 标准 LRC (`LyRiCs`) 格式。
    Lrc,
    /// 增强型 LRC (Enhanced LRC) 格式，支持逐字时间戳。
    EnhancedLrc,
    /// QQ 音乐 QRC 格式。
    Qrc,
    /// 网易云音乐 YRC 格式。
    Yrc,
    /// `Lyricify Lines` 格式。
    Lyl,
    /// `Salt Player Lyrics` 格式。
    Spl,
    /// `Lyricify Quick Export` 格式。
    Lqe,
    /// 酷狗 KRC 格式。
    Krc,
}

impl LyricFormat {
    /// 将歌词格式枚举转换为对应的文件扩展名字符串。
    #[must_use]
    pub fn to_extension_str(self) -> &'static str {
        match self {
            LyricFormat::Ass => "ass",
            LyricFormat::Ttml => "ttml",
            LyricFormat::AppleMusicJson => "json",
            LyricFormat::Lys => "lys",
            LyricFormat::Lrc => "lrc",
            LyricFormat::EnhancedLrc => "elrc",
            LyricFormat::Qrc => "qrc",
            LyricFormat::Yrc => "yrc",
            LyricFormat::Lyl => "lyl",
            LyricFormat::Spl => "spl",
            LyricFormat::Lqe => "lqe",
            LyricFormat::Krc => "krc",
        }
    }

    /// 从字符串（通常是文件扩展名或用户输入）解析歌词格式枚举。
    /// 此方法不区分大小写，并会移除输入字符串中的空格和点。
    pub fn from_string(s: &str) -> Option<Self> {
        let normalized_s = s.to_uppercase().replace([' ', '.'], "");
        match normalized_s.as_str() {
            "ASS" | "SUBSTATIONALPHA" | "SSA" => Some(LyricFormat::Ass),
            "TTML" | "XML" => Some(LyricFormat::Ttml),
            "JSON" => Some(LyricFormat::AppleMusicJson),
            "LYS" | "LYRICIFYSYLLABLE" => Some(LyricFormat::Lys),
            "LRC" => Some(LyricFormat::Lrc),
            "ENHANCEDLRC" | "LRCX" | "ELRC" | "ALRC" => Some(LyricFormat::EnhancedLrc),
            "QRC" => Some(LyricFormat::Qrc),
            "YRC" => Some(LyricFormat::Yrc),
            "LYL" | "LYRICIFYLINES" => Some(LyricFormat::Lyl),
            "SPL" => Some(LyricFormat::Spl),
            "LQE" | "LYRICIFYQUICKEXPORT" => Some(LyricFormat::Lqe),
            "KRC" => Some(LyricFormat::Krc),
            _ => None,
        }
    }
}

impl fmt::Display for LyricFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LyricFormat::Ass => write!(f, "ASS"),
            LyricFormat::Ttml => write!(f, "TTML"),
            LyricFormat::AppleMusicJson => write!(f, "JSON (Apple Music)"),
            LyricFormat::Lys => write!(f, "Lyricify Syllable"),
            LyricFormat::Lrc => write!(f, "LRC"),
            LyricFormat::EnhancedLrc => write!(f, "Enhanced LRC"),
            LyricFormat::Qrc => write!(f, "QRC"),
            LyricFormat::Yrc => write!(f, "YRC"),
            LyricFormat::Lyl => write!(f, "Lyricify Lines"),
            LyricFormat::Spl => write!(f, "SPL"),
            LyricFormat::Lqe => write!(f, "Lyricify Quick Export"),
            LyricFormat::Krc => write!(f, "KRC"),
        }
    }
}

/// 定义可以被注解的内容轨道类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ContentType {
    #[default]
    /// 主歌词
    Main,
    /// 背景人声
    Background,
}

/// 定义轨道元数据的规范化键。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackMetadataKey {
    /// BCP 47 语言代码
    Language,
    /// 罗马音方案名
    Scheme,
    /// 自定义元数据键
    Custom(String),
}

/// 表示振假名中的一个音节。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuriganaSyllable {
    /// 振假名文本内容
    pub text: String,
    /// 可选的时间戳 (`start_ms`, `end_ms`)
    pub timing: Option<(u64, u64)>,
}

/// 表示一个语义上的"单词"或"词组"。
///
/// 主要为了 QRC 的振假名信息服务，其它解析器应该将整行作为一个词组。
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    /// 组成该词的音节列表
    pub syllables: Vec<LyricSyllable>,
    /// 可选的振假名信息
    pub furigana: Option<Vec<FuriganaSyllable>>,
}

/// 一个通用的歌词轨道。
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct LyricTrack {
    /// 组成该轨道的音节列表。
    pub words: Vec<Word>,
    /// 轨道元数据。
    #[serde(default)]
    pub metadata: HashMap<TrackMetadataKey, String>,
}

/// 将一个内容轨道（如主歌词）及其所有注解轨道（如翻译、罗马音）绑定在一起的结构。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AnnotatedTrack {
    /// 该内容轨道的类型。
    pub content_type: ContentType,

    /// 内容轨道本身。
    pub content: LyricTrack,

    /// 依附于该内容轨道的翻译轨道列表。
    #[serde(default)]
    pub translations: Vec<LyricTrack>,

    /// 依附于该内容轨道的罗马音轨道列表。
    #[serde(default)]
    pub romanizations: Vec<LyricTrack>,
}

impl AnnotatedTrack {
    /// 为该轨道添加一个翻译。
    pub fn add_translation(&mut self, text: impl Into<String>, language: &str) {
        let translation_track = LyricTrack {
            words: vec![Word {
                syllables: vec![LyricSyllable {
                    text: text.into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            metadata: std::collections::HashMap::from([(
                TrackMetadataKey::Language,
                language.to_string(),
            )]),
        };
        self.translations.push(translation_track);
    }

    /// 根据语言标签检查翻译是否已存在。
    pub fn has_translation(&self, lang_tag: &str) -> bool {
        self.translations.iter().any(|track| {
            track
                .metadata
                .get(&TrackMetadataKey::Language)
                .is_some_and(|lang| lang == lang_tag)
        })
    }
}

/// 表示一位演唱者的类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AgentType {
    #[default]
    /// 单人演唱。
    Person,
    /// 合唱。
    Group,
    /// 未指定或其它类型。
    Other,
}

/// 表示歌词中的演唱者。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    /// 内部ID, 例如 "v1"
    pub id: String,
    /// 可选的完整名称，例如 "演唱者1号"
    pub name: Option<String>,
    /// Agent 的类型
    pub agent_type: AgentType,
}

/// 用于存储歌词轨道中识别到的所有演唱者。
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStore {
    /// 从 ID 到演唱者结构体的映射。
    pub agents_by_id: HashMap<String, Agent>,
}

impl AgentStore {
    /// 获取所有 Agent 的迭代器
    pub fn all_agents(&self) -> impl Iterator<Item = &Agent> {
        self.agents_by_id.values()
    }

    /// 创建一个新的空 AgentStore
    pub fn new() -> Self {
        Self::default()
    }
}

/// 歌词行结构，作为多个并行带注解轨道的容器。
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize, Builder)]
#[builder(default)]
pub struct LyricLine {
    /// 该行包含的所有带注解的轨道。
    #[builder(setter(each = "track"))]
    pub tracks: Vec<AnnotatedTrack>,
    /// 行的开始时间，相对于歌曲开始的绝对时间（毫秒）。
    pub start_ms: u64,
    /// 行的结束时间，相对于歌曲开始的绝对时间（毫秒）。
    pub end_ms: u64,
    /// 可选的演唱者标识。
    ///
    /// 应该为数字 ID，例如 "v1"，"v1000"。
    #[builder(setter(into, strip_option = false))]
    pub agent: Option<String>,
    /// 可选的歌曲组成部分标记。
    #[builder(setter(into, strip_option = false))]
    pub song_part: Option<String>,
    /// 可选的 iTunes Key (如 "L1", "L2")。
    #[builder(setter(into, strip_option = false))]
    pub itunes_key: Option<String>,
}

impl LyricTrack {
    /// 将轨道内所有音节的文本拼接成一个完整的字符串。
    #[must_use]
    pub fn text(&self) -> String {
        self.words
            .iter()
            .flat_map(|word| &word.syllables)
            .map(|syl| {
                if syl.ends_with_space {
                    format!("{} ", syl.text)
                } else {
                    syl.text.clone()
                }
            })
            .collect::<String>()
            .trim() // 我们的数据结构应该确保了不会出现首尾空格
            .to_string()
    }

    /// 返回轨道中所有音节的不可变迭代器
    pub fn syllables(&self) -> impl Iterator<Item = &LyricSyllable> {
        self.words.iter().flat_map(|w| &w.syllables)
    }

    /// 返回轨道中所有音节的可变迭代器
    pub fn syllables_mut(&mut self) -> impl Iterator<Item = &mut LyricSyllable> {
        self.words.iter_mut().flat_map(|w| &mut w.syllables)
    }

    /// 检查该轨道是否是逐字的
    pub fn is_timed(&self) -> bool {
        let mut syllable_count = 0;
        let mut has_timed_syllable = false;

        for s in self.syllables() {
            syllable_count += 1;
            if !has_timed_syllable && s.end_ms > s.start_ms {
                has_timed_syllable = true;
            }

            if syllable_count > 1 && has_timed_syllable {
                return true;
            }
        }
        syllable_count > 1 && has_timed_syllable
    }

    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|w| w.syllables.is_empty())
    }

    pub fn time_range(&self) -> Option<(u64, u64)> {
        let mut syllables = self.syllables();
        if let Some(first) = syllables.next() {
            let initial = (first.start_ms, first.end_ms);
            let (min_start, max_end) = syllables.fold(initial, |(min_s, max_e), syl| {
                (min_s.min(syl.start_ms), max_e.max(syl.end_ms))
            });
            Some((min_start, max_end))
        } else {
            None
        }
    }
}

impl LyricLine {
    /// 创建一个带有指定时间戳的空 `LyricLine`。
    #[must_use]
    pub fn new(start_ms: u64, end_ms: u64) -> Self {
        Self {
            start_ms,
            end_ms,
            ..Default::default()
        }
    }

    /// 返回一个迭代器，用于遍历所有指定内容类型的带注解轨道。
    pub fn tracks_by_type(
        &self,
        content_type: ContentType,
    ) -> impl Iterator<Item = &AnnotatedTrack> {
        self.tracks
            .iter()
            .filter(move |t| t.content_type == content_type)
    }

    /// 返回一个迭代器，用于遍历所有主歌词轨道 (`ContentType::Main`)。
    pub fn main_tracks(&self) -> impl Iterator<Item = &AnnotatedTrack> {
        self.tracks_by_type(ContentType::Main)
    }

    /// 返回一个迭代器，用于遍历所有背景人声轨道 (`ContentType::Background`)。
    pub fn background_tracks(&self) -> impl Iterator<Item = &AnnotatedTrack> {
        self.tracks_by_type(ContentType::Background)
    }

    /// 获取第一个主歌词轨道（如果存在）。
    #[must_use]
    pub fn main_track(&self) -> Option<&AnnotatedTrack> {
        self.main_tracks().next()
    }

    /// 获取第一个背景人声轨道（如果存在）。
    #[must_use]
    pub fn background_track(&self) -> Option<&AnnotatedTrack> {
        self.background_tracks().next()
    }

    /// 获取第一个主歌词轨道的完整文本（如果存在）。
    #[must_use]
    pub fn main_text(&self) -> Option<String> {
        self.main_track().map(|t| t.content.text())
    }

    /// 获取第一个背景人声轨道的完整文本（如果存在）。
    #[must_use]
    pub fn background_text(&self) -> Option<String> {
        self.background_track().map(|t| t.content.text())
    }

    /// 向该行添加一个预先构建好的带注解轨道。
    pub fn add_track(&mut self, track: AnnotatedTrack) {
        self.tracks.push(track);
    }

    /// 向该行添加一个新的、简单的内容轨道（主歌词或背景）。
    ///
    /// # 参数
    /// * `content_type` - 轨道的类型 (`Main` 或 `Background`)。
    /// * `text` - 该轨道的完整文本。
    pub fn add_content_track(&mut self, content_type: ContentType, text: impl Into<String>) {
        let syllable = LyricSyllable {
            text: text.into(),
            start_ms: self.start_ms,
            end_ms: self.end_ms,
            ..Default::default()
        };
        let track = AnnotatedTrack {
            content_type,
            content: LyricTrack {
                words: vec![Word {
                    syllables: vec![syllable],
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        self.add_track(track);
    }

    /// 为该行中所有指定类型的内容轨道添加一个翻译。
    /// 例如，可用于为所有主歌词轨道添加一个统一的翻译。
    pub fn add_translation(
        &mut self,
        content_type: ContentType,
        text: impl Into<String>,
        language: Option<&str>,
    ) {
        let text = text.into();
        for track in self
            .tracks
            .iter_mut()
            .filter(|t| t.content_type == content_type)
        {
            let mut metadata = HashMap::new();
            if let Some(lang) = language {
                metadata.insert(TrackMetadataKey::Language, lang.to_string());
            }
            let translation_track = LyricTrack {
                words: vec![Word {
                    syllables: vec![LyricSyllable {
                        text: text.clone(),
                        start_ms: self.start_ms,
                        end_ms: self.end_ms,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                metadata,
            };
            track.translations.push(translation_track);
        }
    }

    /// 为该行中所有指定类型的内容轨道添加一个罗马音。
    pub fn add_romanization(
        &mut self,
        content_type: ContentType,
        text: impl Into<String>,
        scheme: Option<&str>,
    ) {
        let text = text.into();
        for track in self
            .tracks
            .iter_mut()
            .filter(|t| t.content_type == content_type)
        {
            let mut metadata = HashMap::new();
            if let Some(s) = scheme {
                metadata.insert(TrackMetadataKey::Scheme, s.to_string());
            }
            let romanization_track = LyricTrack {
                words: vec![Word {
                    syllables: vec![LyricSyllable {
                        text: text.clone(),
                        start_ms: self.start_ms,
                        end_ms: self.end_ms,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                metadata,
            };
            track.romanizations.push(romanization_track);
        }
    }

    /// 移除所有指定类型的内容轨道及其所有注解。
    pub fn clear_tracks(&mut self, content_type: ContentType) {
        self.tracks.retain(|t| t.content_type != content_type);
    }

    /// 根据语言代码获取第一个匹配的翻译轨道。
    #[must_use]
    pub fn get_translation_by_lang(&self, lang_tag: &str) -> Option<&LyricTrack> {
        self.main_tracks()
            .flat_map(|annotated_track| &annotated_track.translations)
            .find(|translation_track| {
                matches!(
                    translation_track.metadata.get(&TrackMetadataKey::Language),
                    Some(lang) if lang == lang_tag
                )
            })
    }

    /// 根据语言代码获取第一个匹配的罗马音轨道。
    #[must_use]
    pub fn get_romanization_by_lang(&self, lang_tag: &str) -> Option<&LyricTrack> {
        self.main_tracks()
            .flat_map(|annotated_track| &annotated_track.romanizations)
            .find(|romanization_track| {
                matches!(
                    romanization_track.metadata.get(&TrackMetadataKey::Language),
                    Some(lang) if lang == lang_tag
                )
            })
    }
}

/// 通用的歌词音节结构，用于表示逐字歌词中的一个音节。
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize, Builder)]
#[builder(default)]
pub struct LyricSyllable {
    /// 音节的文本内容。
    ///
    /// 应确保这里不包含空格。空格使用下面的 `ends_with_space` 来表示。
    #[builder(setter(into))]
    pub text: String,
    /// 音节开始时间，相对于歌曲开始的绝对时间（毫秒）。
    pub start_ms: u64,
    /// 音节结束时间，相对于歌曲开始的绝对时间（毫秒）。
    pub end_ms: u64,
    /// 可选的音节持续时间（毫秒）。
    /// 如果提供，`end_ms` 可以由 `start_ms + duration_ms` 计算得出，反之亦然。
    /// 解析器应确保 `start_ms` 和 `end_ms` 最终被填充。
    #[builder(setter(strip_option))]
    pub duration_ms: Option<u64>,
    /// 指示该音节后是否应有空格。
    ///
    /// **重要**: 必须根据此标志在音节后附加空格。`text` 内容中不会包含空格。
    pub ends_with_space: bool,
}

impl LyricSyllable {
    pub fn duration(&self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }
}

/// 定义元数据的规范化键。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, EnumIter)]
pub enum CanonicalMetadataKey {
    /// 歌曲标题。
    Title,
    /// 艺术家。
    Artist,
    /// 专辑名。
    Album,
    /// 主歌词的语言代码 (BCP 47)。
    Language,
    /// 全局时间偏移量（毫秒）。
    Offset,
    /// 词曲作者。
    Songwriter,
    /// 网易云音乐 ID。
    NcmMusicId,
    /// QQ音乐 ID。
    QqMusicId,
    /// Spotify ID。
    SpotifyId,
    /// Apple Music ID。
    AppleMusicId,
    /// 国际标准音像制品编码 (International Standard Recording Code)。
    Isrc,
    /// 逐词歌词作者 Github ID。
    TtmlAuthorGithub,
    /// 逐词歌词作者 GitHub 用户名。
    TtmlAuthorGithubLogin,

    /// 用于所有其他未明确定义的标准或非标准元数据键。
    #[strum(disabled)]
    Custom(String),
}

impl fmt::Display for CanonicalMetadataKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key_name = match self {
            CanonicalMetadataKey::Title => "Title",
            CanonicalMetadataKey::Artist => "Artist",
            CanonicalMetadataKey::Album => "Album",
            CanonicalMetadataKey::Language => "Language",
            CanonicalMetadataKey::Offset => "Offset",
            CanonicalMetadataKey::Songwriter => "Songwriter",
            CanonicalMetadataKey::NcmMusicId => "NCMMusicId",
            CanonicalMetadataKey::QqMusicId => "QQMusicId",
            CanonicalMetadataKey::SpotifyId => "SpotifyId",
            CanonicalMetadataKey::AppleMusicId => "AppleMusicId",
            CanonicalMetadataKey::Isrc => "ISRC",
            CanonicalMetadataKey::TtmlAuthorGithub => "TtmlAuthorGithub",
            CanonicalMetadataKey::TtmlAuthorGithubLogin => "TtmlAuthorGithubLogin",
            CanonicalMetadataKey::Custom(s) => s.as_str(),
        };
        write!(f, "{key_name}")
    }
}

impl CanonicalMetadataKey {
    /// 定义哪些键应该被显示出来
    #[must_use]
    pub fn is_public(&self) -> bool {
        matches!(
            self,
            Self::Title
                | Self::Artist
                | Self::Album
                | Self::NcmMusicId
                | Self::QqMusicId
                | Self::SpotifyId
                | Self::AppleMusicId
                | Self::Isrc
                | Self::TtmlAuthorGithub
                | Self::TtmlAuthorGithubLogin
        )
    }

    /// 返回一个用于排序的数字权重。
    pub fn get_order_rank(&self) -> i32 {
        match self {
            Self::Title => 0,
            Self::Artist => 1,
            Self::Album => 2,
            Self::Songwriter => 3,
            Self::Language => 4,
            Self::Offset => 5,
            Self::NcmMusicId => 10,
            Self::QqMusicId => 11,
            Self::SpotifyId => 12,
            Self::AppleMusicId => 13,
            Self::Isrc => 14,
            Self::TtmlAuthorGithub => 20,
            Self::TtmlAuthorGithubLogin => 21,
            Self::Custom(_) => 1000,
        }
    }
}

impl FromStr for CanonicalMetadataKey {
    type Err = ParseCanonicalMetadataKeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "ti" | "title" | "musicname" => Ok(Self::Title),
            "ar" | "artist" | "artists" => Ok(Self::Artist),
            "al" | "album" => Ok(Self::Album),
            "by" | "ttmlauthorgithublogin" => Ok(Self::TtmlAuthorGithubLogin),
            "language" | "lang" => Ok(Self::Language),
            "offset" => Ok(Self::Offset),
            "songwriter" | "songwriters" => Ok(Self::Songwriter),
            "ncmmusicid" => Ok(Self::NcmMusicId),
            "qqmusicid" => Ok(Self::QqMusicId),
            "spotifyid" => Ok(Self::SpotifyId),
            "applemusicid" => Ok(Self::AppleMusicId),
            "isrc" => Ok(Self::Isrc),
            "ttmlauthorgithub" => Ok(Self::TtmlAuthorGithub),
            _ if !s.is_empty() => Ok(Self::Custom(s.to_string())),
            _ => Err(ParseCanonicalMetadataKeyError(s.to_string())),
        }
    }
}

/// 存储从源文件解析出的、准备进行进一步处理或转换的歌词数据。
/// 这是解析阶段的主要输出，也是后续处理和生成阶段的主要输入。
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedSourceData {
    /// 解析后的歌词行列表。
    pub lines: Vec<LyricLine>,
    /// 从文件头或特定元数据标签中解析出的原始（未规范化）元数据。
    /// 键是原始元数据标签名，值是该标签对应的所有值（因为某些标签可能出现多次）。
    pub raw_metadata: HashMap<String, Vec<String>>,
    /// 解析的源文件格式。
    pub source_format: LyricFormat,
    /// 从文件中解析出的所有演唱者信息。
    #[serde(default)]
    pub agents: AgentStore,
    /// 指示源文件是否是逐行歌词（例如LRC）。
    pub is_line_timed_source: bool,
    /// 解析过程中产生的警告信息列表。
    pub warnings: Vec<String>,
    /// 指示输入的TTML 是否被格式化。
    /// 这影响空格和换行的处理。
    pub detected_formatted_ttml_input: Option<bool>,
    /// 提供商名称
    pub source_name: String,
}

/// 表示从ASS中提取的标记信息。
/// 元组的第一个元素是原始行号，第二个元素是标记文本。
pub type MarkerInfo = (usize, String);

/// 定义 LYS 格式使用的歌词行属性。
pub mod lys_properties {
    /// 视图：未设置，人声：未设置
    pub const UNSET_UNSET: u8 = 0;
    /// 视图：左，人声：未设置
    pub const UNSET_LEFT: u8 = 1;
    /// 视图：右，人声：未设置
    pub const UNSET_RIGHT: u8 = 2;
    /// 视图：未设置，人声：主歌词
    pub const MAIN_UNSET: u8 = 3;
    /// 视图：左，人声：主歌词
    pub const MAIN_LEFT: u8 = 4;
    /// 视图：右，人声：主歌词
    pub const MAIN_RIGHT: u8 = 5;
    /// 视图：未设置，人声：背景
    pub const BG_UNSET: u8 = 6;
    /// 视图：左，人声：背景
    pub const BG_LEFT: u8 = 7;
    /// 视图：右，人声：背景
    pub const BG_RIGHT: u8 = 8;
}
