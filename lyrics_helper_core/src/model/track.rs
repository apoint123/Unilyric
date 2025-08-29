//! 定义了与歌曲搜索功能相关的核心数据结构，包括搜索输入、搜索结果和匹配度量。

use serde::{Deserialize, Serialize};

use crate::{converter::types::ParsedSourceData, model::generic::Artist};

/// 代表搜索结果与原始查询元数据的匹配程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Default, Serialize)]
pub enum MatchType {
    /// 没有匹配或匹配度极低。
    #[default]
    None = -1,
    /// 匹配度非常低。
    VeryLow = 10,
    /// 匹配度低。
    Low = 30,
    /// 匹配度中等。
    Medium = 70,
    /// 匹配度较高。
    PrettyHigh = 90,
    /// 匹配度高。
    High = 95,
    /// 匹配度非常高。
    VeryHigh = 99,
    /// 完美匹配。
    Perfect = 100,
}

/// 代表一个可搜索的歌曲元数据，用作搜索函数的输入参数。
#[derive(Default, Debug, Clone)]
pub struct Track<'a> {
    /// 歌曲标题。
    pub title: Option<&'a str>,
    /// 艺术家列表。
    pub artists: Option<&'a [&'a str]>,
    /// 专辑名。
    pub album: Option<&'a str>,
    /// 歌曲时长（毫秒）。
    pub duration: Option<u64>,
}

/// 代表一个标准化的搜索结果条目。
///
/// 这是所有 Provider 的 `search_songs` 方法需要返回的类型。
#[derive(Debug, Deserialize, Clone, Default, Serialize, PartialEq, Eq)]
pub struct SearchResult {
    /// 搜索结果的歌曲标题。
    pub title: String,
    /// 搜索结果的艺术家列表。
    pub artists: Vec<Artist>,
    /// 搜索结果的专辑名。
    pub album: Option<String>,
    /// 搜索结果的专辑ID。
    pub album_id: Option<String>,
    /// 歌曲时长（毫秒）。
    pub duration: Option<u64>,
    /// 在其所在平台的唯一 ID（可能是字符串，如 hash 或 mid）。
    pub provider_id: String,
    /// 提供商的名称 (例如, "qq", "netease")。
    pub provider_name: String,
    /// 在其所在平台的数字 ID (如果可用)。
    pub provider_id_num: Option<u64>,
    /// 此搜索结果的匹配类型。
    pub match_type: MatchType,
    /// 封面链接。
    pub cover_url: Option<String>,
    /// 语言代码。
    pub language: Option<Language>,
}

/// 代表从 API 获取的、未经解析的原始歌词内容。
///
/// 这个结构体主要用作一个临时的数据容器，将从不同 Provider 获取的
/// 歌词文本和其格式信息打包在一起，以便后续处理。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawLyrics {
    /// 歌词的格式，例如 "lrc", "qrc" 等。
    pub format: String,
    /// 原始的歌词文本内容。
    pub content: String,
    /// 可选的、与主歌词一同获取的翻译歌词。
    pub translation: Option<String>,
}

/// 代表完整的歌词获取结果，包括解析后的数据和原始副本。
#[derive(Debug, Clone, Default, Serialize)]
pub struct FullLyricsResult {
    /// 经过统一解析和合并后的标准歌词数据。
    pub parsed: ParsedSourceData,
    /// 从提供商获取的原始歌词副本。
    pub raw: RawLyrics,
}

/// 代表一个包含歌词和其来源元数据的完整搜索结果。
#[derive(Debug, Clone, Default, Serialize)]
pub struct LyricsAndMetadata {
    /// 获取到的歌词详情，包括解析后和原始数据。
    pub lyrics: FullLyricsResult,
    /// 提供该歌词的的元数据。
    pub source_track: SearchResult,
}

/// 代表一次完整的搜索操作的最终结果。
/// 包含最佳歌词匹配和所有搜索候选项。
#[derive(Debug, Clone, Default)]
pub struct ComprehensiveSearchResult {
    /// 包含最佳歌词及其来源元数据的结果。
    pub primary_lyric_result: LyricsAndMetadata,
    /// 初始搜索返回的所有候选项，按匹配度从高到低排序。
    pub all_search_candidates: Vec<SearchResult>,
}

/// 歌曲的语言
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum Language {
    /// 纯音乐
    Instrumental,
    /// 中文
    Chinese,
    /// 英文
    English,
    /// 日语
    Japanese,
    /// 韩语
    Korean,
    /// 其它语言
    Other,
}
