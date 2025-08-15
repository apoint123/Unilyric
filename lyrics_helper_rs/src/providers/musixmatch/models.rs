//! 此模块定义了所有用于反序列化 Musixmatch API 响应的数据结构。
//!
//! API 来源于 https://github.com/Strvm/musicxmatch-api

use serde::{Deserialize, Serialize};

// =================================================================
// 通用的 Musixmatch 响应包装结构
// =================================================================

/// Musixmatch API 响应的顶层通用结构。
///
/// # 泛型参数
/// - `T`: 响应主体（body）部分的数据类型。
#[derive(Debug, Deserialize, Serialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct ApiResponse<T> {
    /// 响应消息，包含头部和主体数据。
    pub message: Message<T>,
}

/// 响应消息的容器，包含头部和主体。
///
/// # 泛型参数
/// - `T`: 响应主体（body）部分的数据类型。
#[derive(Debug, Deserialize, Serialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct Message<T> {
    /// 响应头部，包含状态码、执行时间等信息。
    pub header: Header,
    /// 响应主体，包含具体的业务数据。
    #[serde(default)]
    pub body: Option<T>,
}

/// 通用的响应头部。
///
/// 包含状态码、执行时间和提示信息等。
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct Header {
    /// API 返回的状态码，`200` 表示成功。
    #[serde(default)]
    pub status_code: i32,
    /// API 服务器执行请求所花费的时间（秒）。
    #[serde(default)]
    pub execute_time: f64,
    /// API 返回的提示信息，例如在令牌过期时会返回 "renew"。
    pub hint: Option<String>,
}
// =================================================================
// `track.get` 接口的模型
// =================================================================

/// `track.get` 接口响应的 `body` 部分。
///
/// 包含匹配到的歌曲信息。
#[derive(Debug, Deserialize, Default, Serialize)]
pub struct GetTrackBody {
    /// 匹配到的歌曲信息。
    pub track: Option<Track>,
}

/// 代表一首歌曲的详细信息。
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Track {
    /// 歌曲在 Musixmatch 的内部 ID。
    pub track_id: i64,
    /// 歌曲名。
    pub track_name: String,
    /// 艺术家的内部 ID。
    pub artist_id: i64,
    /// 艺术家姓名。
    pub artist_name: String,
    /// 专辑的内部 ID。
    pub album_id: i64,
    /// 专辑名。
    pub album_name: String,
    /// 歌曲时长，单位为秒 (s)。
    pub track_length: i32,
    /// 通用的歌曲 ID，是获取歌词等信息的关键。
    pub commontrack_id: i64,
    /// 是否有 RichSync (逐字) 歌词，1 为是，0 为否。
    pub has_richsync: i32,
    /// 歌曲封面信息，100x100 分辨率。
    #[serde(default)]
    pub album_coverart_100x100: String,
    /// 歌曲封面信息，350x350分辨率。
    #[serde(default)]
    pub album_coverart_350x350: String,
    /// 歌曲封面信息，500x500分辨率。
    #[serde(default)]
    pub album_coverart_500x500: String,
    /// 歌曲封面信息，800x800分辨率。
    #[serde(default)]
    pub album_coverart_800x800: String,
}

// =================================================================
// `track.search` 接口的模型
// =================================================================

/// `track.search` 接口响应的 `body` 部分。
///
/// 包含歌曲列表。
#[derive(Debug, Deserialize, Default, Serialize)]
pub struct SearchTrackBody {
    /// 歌曲列表。
    #[serde(default)]
    pub track_list: Vec<TrackListItem>,
}

/// 歌曲列表中的一个条目，包装了一个 `Track` 对象。
#[derive(Debug, Deserialize, Serialize)]
pub struct TrackListItem {
    /// 歌曲详细信息。
    pub track: Track,
}

// =================================================================
// `album.get` 接口的模型
// =================================================================

/// `album.get` 接口响应的 `body` 部分。
///
/// 包含专辑信息。
#[derive(Debug, Deserialize, Default, Serialize)]
pub struct GetAlbumBody {
    /// 匹配到的专辑信息。
    pub album: Option<Album>,
}

/// 代表一个专辑的详细信息。
#[derive(Debug, Deserialize, Serialize)]
pub struct Album {
    /// 专辑的内部 ID。
    pub album_id: i64,
    /// 专辑名。
    pub album_name: String,
    /// 艺术家的内部 ID。
    pub artist_id: i64,
    /// 艺术家姓名。
    pub artist_name: String,
    /// 专辑发行日期。
    #[serde(default)]
    pub album_release_date: String,
    /// 专辑封面信息，100x100 分辨率。
    #[serde(default)]
    pub album_coverart_100x100: String,
    /// 专辑封面信息，350x350 分辨率。
    #[serde(default)]
    pub album_coverart_350x350: String,
    /// 专辑封面信息，500x500 分辨率。
    #[serde(default)]
    pub album_coverart_500x500: String,
    /// 专辑封面信息，800x800 分辨率。
    #[serde(default)]
    pub album_coverart_800x800: String,
}

// =================================================================
// `album.tracks.get` 接口的模型
// =================================================================

/// `album.tracks.get` 接口响应的 `body` 部分。
///
/// 包含专辑下的歌曲列表。
#[derive(Debug, Deserialize, Default, Serialize)]
pub struct GetAlbumTracksBody {
    /// 歌曲列表。
    #[serde(default)]
    pub track_list: Vec<TrackListItem>,
}

// =================================================================
// `macro.subtitles.get` (获取 LRC 歌词) 的模型
// =================================================================

/// `macro.subtitles.get` 接口响应的 `body` 部分。
///
/// 包含宏调用结构体。
#[derive(Debug, Deserialize, Default, Serialize)]
pub struct GetSubtitlesBody {
    /// Musixmatch 的宏调用结构，响应内容被嵌套在里面。
    #[serde(default)]
    pub macro_calls: MacroCalls,
}

/// 宏调用容器。
///
/// 包含实际的字幕获取响应。
#[derive(Debug, Deserialize, Serialize)]
pub struct MacroCalls {
    /// 实际的字幕获取响应被嵌套在此字段下。
    #[serde(rename = "track.subtitles.get")]
    pub track_subtitles_get: ApiResponse<SubtitleGetBody>,
}

/// 为 MacroCalls 手动实现 Default，因为 `ApiResponse` 默认需要 T 是 Default 的。
impl Default for MacroCalls {
    fn default() -> Self {
        Self {
            track_subtitles_get: ApiResponse {
                message: Message {
                    header: Default::default(),
                    body: Default::default(),
                },
            },
        }
    }
}

/// 字幕获取响应的 `body` 部分。
///
/// 包含字幕列表。
#[derive(Debug, Deserialize, Default, Serialize)]
pub struct SubtitleGetBody {
    /// 字幕对象列表，通常只关心第一个。
    #[serde(default)]
    pub subtitle_list: Vec<SubtitleListItem>,
}

/// 字幕列表项，包装了一个 `Subtitle` 对象。
#[derive(Debug, Deserialize, Serialize)]
pub struct SubtitleListItem {
    /// 字幕对象，包含 LRC 格式的歌词文本。
    pub subtitle: Subtitle,
}

/// 包含 LRC 歌词文本的结构。
#[derive(Debug, Deserialize, Default, Serialize)]
pub struct Subtitle {
    /// LRC 格式的歌词文本内容。
    #[serde(default)]
    pub subtitle_body: String,
}

// =================================================================
// `crowd.track.translations.get` (获取翻译) 的模型
// =================================================================

/// 获取翻译接口响应的 `body` 部分。
///
/// 包含所有可用的翻译条目。
#[derive(Debug, Deserialize, Default, Serialize)]
pub struct GetTranslationsBody {
    /// 翻译列表，包含所有可用的翻译条目。
    #[serde(default)]
    pub translations_list: Vec<TranslationsListItem>,
}

/// 翻译列表项，包装了一个 `Translation` 对象。
#[derive(Debug, Deserialize, Serialize)]
pub struct TranslationsListItem {
    /// 翻译对象，包含翻译文本和语言信息。
    pub translation: Translation,
}

/// 包含翻译文本的结构。
#[derive(Debug, Deserialize, Default, Serialize)]
pub struct Translation {
    /// 翻译文本内容。
    #[serde(default)]
    pub description: String,
    /// 翻译的语言代码 (BCP-47)。
    #[serde(default)]
    pub language: String,
}
// =================================================================
// `track.richsync.get` (获取逐字歌词) 的模型
// =================================================================

/// `track.richsync.get` 接口响应的 `body` 部分。
///
/// 包含 RichSync 歌词数据。
#[derive(Debug, Deserialize, Default, Serialize)]
pub struct GetRichSyncBody {
    /// RichSync 歌词数据，包含逐字时间戳信息。
    #[serde(default)]
    pub richsync: RichSync,
}

/// 包含 RichSync 歌词的结构。
///
/// 其中 `richsync_body` 是一个内嵌的 JSON 字符串，需要二次解析。
#[derive(Debug, Deserialize, Default, Serialize)]
pub struct RichSync {
    /// 这是一个内嵌的 JSON 字符串，需要进行二次解析。
    #[serde(default)]
    pub richsync_body: String,
}

// =================================================================
// 用于解析 RichSync 内嵌 JSON 的模型
// =================================================================

/// 代表 RichSync 中的一行歌词。
#[derive(Debug, Deserialize, Serialize)]
pub struct RichSyncLine {
    /// 行开始时间（秒）。
    #[serde(rename = "ts")]
    pub line_start_ms: f64,
    /// 行结束时间（秒）。
    #[serde(rename = "te")]
    pub line_end_ms: f64,
    /// 该行包含的音节列表。
    #[serde(rename = "l")]
    pub syllables: Vec<RichSyncSyllable>,
    /// 整行歌词的文本。
    #[serde(rename = "x")]
    pub line_text: String,
}

/// 代表 RichSync 中的一个音节（单词）。
#[derive(Debug, Deserialize, Serialize)]
pub struct RichSyncSyllable {
    /// 音节的文本内容。
    #[serde(rename = "c")]
    pub text: String,
    /// 音节开始时间相对于该行开始时间的偏移量（秒）。
    #[serde(rename = "o")]
    pub offset: f64,
}
