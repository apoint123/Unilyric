//! 此模块定义了所有用于反序列化酷狗音乐 API 响应的数据结构。
//! API 来源于 <https://github.com/MakcRe/KuGouMusicApi>

use serde::{Deserialize, Serialize};

// =================================================================
// 歌词下载接口 (`/download`) 的模型
// =================================================================

/// 歌词下载接口的响应结构。
#[derive(serde::Deserialize)]
pub struct LyricDownloadResponse {
    /// Base64 编码的加密歌词内容。
    pub content: String,
}

// =================================================================
// 歌曲搜索接口 (`/api/v3/search/song`) 的模型
// =================================================================

/// 歌曲搜索 API 的顶层响应结构。
#[derive(Debug, Deserialize)]
pub struct SearchSongResponse {
    /// API 状态码，`1` 通常表示成功。
    pub status: i32,

    /// API 错误码，`0` 通常表示成功。
    #[serde(rename = "error_code")]
    pub err_code: Option<i32>,

    /// 错误的具体信息。
    #[serde(rename = "error_msg")]
    pub error: Option<String>,

    /// 包含实际搜索结果的数据容器。
    pub data: Option<SearchSongData>,
}

/// 歌曲搜索结果的数据部分。
#[derive(Debug, Deserialize)]
pub struct SearchSongData {
    /// 包含歌曲详细信息的列表。
    #[serde(rename = "lists")]
    pub info: Vec<SongInfo>,
}

/// 代表一首歌曲的详细信息。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SongInfo {
    /// 歌曲的唯一文件哈希。这是最重要的ID。
    pub file_hash: String,

    /// 歌曲名。
    #[serde(rename = "OriSongName")]
    pub song_name: String,

    /// 歌曲后缀，例如 "(DJ版)"。
    #[serde(rename = "Suffix")]
    pub suffix: String,

    /// 专辑名。
    pub album_name: String,

    /// 歌曲时长，单位为秒 (s)。
    pub duration: u64,

    /// 专辑的唯一ID。
    #[serde(rename = "AlbumID")]
    pub album_id: String,

    /// 歌曲的数字ID。
    #[serde(rename = "Audioid")]
    pub audio_id: u64,

    /// 包含 `{size}` 占位符的专辑封面图片 URL 模板。
    pub image: String,

    /// 歌手信息列表。
    pub singers: Vec<SingerInfo>,

    #[serde(rename = "trans_param")]
    /// 包含语言等额外参数的结构。
    pub trans_param: Option<TransParam>,
}

/// 歌手信息结构。
#[derive(Debug, Deserialize)]
pub struct SingerInfo {
    /// 歌手的数字ID。
    pub id: u64,
    /// 歌手名。
    pub name: String,
}

/// 包含语言等额外参数的结构。
#[derive(Debug, Deserialize)]
pub struct TransParam {
    /// 语言，例如 "国语"。
    pub language: Option<String>,
}
// =================================================================
// 歌词搜索接口 (`/search`) 的模型
// =================================================================

/// 歌词搜索 API 的顶层响应结构。
/// 这个接口用于根据 `hash` 获取歌词的下载凭证。
#[derive(Debug, Deserialize)]
pub struct SearchLyricsResponse {
    /// API 状态码，`200` 通常表示成功。
    pub status: i32,
    /// 候选歌词列表。通常只关心第一个匹配项。
    pub candidates: Vec<Candidate>,
}

/// 代表一个可供下载的歌词候选版本。
#[derive(Debug, Deserialize)]
pub struct Candidate {
    /// 该歌词版本的唯一 ID，用于下载。
    pub id: String,
    /// 下载歌词所需的访问密钥 (access key)。
    pub accesskey: String,
}

// =================================================================
// 专辑详情接口 (`/kmr/v2/albums`) 的模型
// =================================================================

// --- 请求模型 ---

/// 用于请求专辑详情的 POST 请求体负载结构。
#[derive(Serialize)]
pub struct AlbumDetailRequestPayload<'a> {
    /// 包含专辑 ID 的数据数组，酷狗 API 设计为支持批量请求。
    pub data: [AlbumId<'a>; 1],
    /// 可能与购买状态相关的标志，通常为 0。
    pub is_buy: u8,
}

/// 在 `AlbumDetailRequestPayload` 中用于包装专辑 ID 的结构。
#[derive(Serialize)]
pub struct AlbumId<'a> {
    /// 专辑的唯一标识符。
    pub album_id: &'a str,
}

// --- 响应模型 ---

/// 专辑详情 API 的顶层响应结构。
#[derive(Debug, Deserialize)]
pub struct AlbumDetailResponse {
    /// API 状态码, 1 表示成功。
    pub status: i32,
    /// API 错误码, 0 表示成功。
    pub error_code: i32,
    /// 包含专辑详情的 `data` 字段，酷狗 API 将其设计为一个数组。
    pub data: Option<Vec<AlbumDetailData>>,
}

/// 代表专辑具体信息的数据结构。
#[derive(Debug, Deserialize, Default)]
pub struct AlbumDetailData {
    /// 专辑的唯一 ID (字符串类型)。
    #[serde(default)]
    #[serde(rename = "album_id")]
    pub album_id: Option<String>,

    /// 专辑名称。
    #[serde(default)]
    #[serde(rename = "album_name")]
    pub album_name: Option<String>,

    /// 专辑介绍。
    #[serde(default)]
    pub intro: Option<String>,

    /// 专辑发行时间，格式如 "YYYY-MM-DD"。
    #[serde(default)]
    #[serde(rename = "publish_date")]
    pub publish_time: Option<String>,

    /// 包含 `{size}` 占位符的专辑封面图片 URL 模板。
    /// 留空可以获取最大尺寸的图片。
    #[serde(default)]
    #[serde(rename = "sizable_cover")]
    pub img_url: Option<String>,

    /// 演唱者姓名，多位演唱者通常用 `、` 分隔。
    #[serde(default)]
    #[serde(rename = "author_name")]
    pub singer_name: Option<String>,
}

// =================================================================
// 专辑歌曲列表接口 (`/v1/album_audio/lite`) 的模型
// =================================================================

// --- 请求模型 ---

/// 用于请求专辑歌曲列表的 POST 请求体负载结构。
#[derive(Serialize)]
pub struct AlbumSongsRequestPayload<'a> {
    /// 目标专辑的唯一 ID。
    pub album_id: &'a str,
    /// 请求的页码，从 1 开始。
    pub page: u32,
    /// 每页返回的歌曲数量。
    pub pagesize: u32,
    /// 一个附加参数，通常为空字符串。
    pub is_buy: &'static str,
}

// --- 响应模型 ---

/// 专辑歌曲列表 API 的顶层响应结构。
#[derive(Debug, Deserialize)]
pub struct AlbumSongsResponse {
    /// API 状态码, 1 表示成功。
    pub status: i32,
    /// API 错误码, 0 表示成功。
    pub error_code: i32,
    /// 包含歌曲列表和总数的数据容器。
    pub data: Option<AlbumSongsData>,
}

/// 专辑歌曲列表的数据部分。
#[derive(Debug, Deserialize)]
pub struct AlbumSongsData {
    /// 包含歌曲详细信息的列表。
    pub songs: Vec<AlbumSongInfo>,
}

/// 为专辑歌曲列表接口设计的歌曲信息模型。
#[derive(Debug, Deserialize)]
pub struct AlbumSongInfo {
    /// 包含歌曲名和歌手名的基础信息对象。
    pub base: AlbumSongBaseInfo,
    /// 包含 hash 和时长等音频相关信息的对象。
    pub audio_info: AlbumSongAudioInfo,
}

/// `AlbumSongInfo` 的基础信息部分。
#[derive(Debug, Deserialize)]
pub struct AlbumSongBaseInfo {
    /// 歌曲的完整名称。
    #[serde(rename = "audio_name")]
    pub song_name: String,
    /// 演唱者姓名。
    #[serde(rename = "author_name")]
    pub singer_name: String,
}

/// `AlbumSongInfo` 的音频信息部分。
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct AlbumSongAudioInfo {
    /// 标准音质的歌曲文件 hash，是获取播放链接的关键。
    #[serde(default)]
    pub hash: Option<String>,
    /// 歌曲时长，单位为毫秒 (ms)。
    #[serde(default)]
    pub duration: Option<u64>,
    /// 128kbps 音质的歌曲文件 hash。
    #[serde(default)]
    pub hash_128: Option<String>,
    /// 320kbps 音质的歌曲文件 hash。
    #[serde(default)]
    pub hash_320: Option<String>,
    /// FLAC 无损音质的歌曲文件 hash。
    #[serde(default)]
    pub hash_flac: Option<String>,
    /// Hi-Res 高解析度音质的歌曲文件 hash。
    #[serde(default)]
    pub hash_high: Option<String>,
}

// =================================================================
// 歌手歌曲列表接口 (`/kmr/v1/audio_group/author`) 的模型
// =================================================================

// --- 请求模型 ---

/// 用于请求歌手歌曲列表的 POST 请求体负载结构 (`kmr/v1/audio_group/author`)。
#[derive(Serialize)]
pub struct KmrSingerSongsRequestPayload<'a> {
    /// 应用 ID。
    pub appid: &'a str,
    /// 客户端版本。
    pub clientver: &'a str,
    /// 根据 dfid 计算的 MD5 值。
    pub mid: &'a str,
    /// 秒级 Unix 时间戳。
    pub clienttime: &'a str,
    /// 由 `sign_params_key` 算法生成的签名。
    pub key: String,
    /// 歌手 ID。
    pub author_id: &'a str,
    /// 每页返回的数量。
    pub pagesize: u32,
    /// 请求的页码。
    pub page: u32,
    /// 排序方式，1：最热，2：最新。
    pub sort: u8,
    /// 地区代码，通常为 "all"。
    pub area_code: &'static str,
}

// --- 响应模型 ---

/// 歌手歌曲列表 API 的顶层响应结构 (`kmr/v1/audio_group/author`)。
#[derive(Debug, Deserialize)]
pub struct KmrSingerSongsResponse {
    /// API 状态码, 1 表示成功。
    pub status: i32,
    /// API 错误码, 0 表示成功。
    pub error_code: i32,
    /// 包含歌曲详细信息的列表。
    #[serde(default)]
    pub data: Vec<KmrSongInfo>,
}

/// 代表一首歌曲的详细信息 (`kmr/v1/audio_group/author`)。
#[derive(Debug, Deserialize)]
pub struct KmrSongInfo {
    /// 歌曲名。
    pub audio_name: String,
    /// 歌手名 (API 返回的是单个字符串)。
    pub author_name: String,
    /// 歌曲的哈希。
    #[serde(rename = "hash")]
    pub hash: String,
    /// 歌曲时长，单位为秒 (s)。
    pub timelength: u64,
}

// =================================================================
// 歌单详情元数据接口 (`/v3/get_list_info`) 的模型
// =================================================================

// --- 请求模型 ---

/// 用于请求歌单详情元数据的 POST 请求体负载结构。
#[derive(Serialize)]
pub struct PlaylistDetailRequestPayload<'a> {
    /// 包含歌单 ID 的数据数组。
    pub data: Vec<PlaylistIdObject<'a>>,
    /// 用户ID，未登录时为 "0"。
    pub userid: &'a str,
    /// 用户令牌，未登录时为空。
    pub token: &'static str,
}

/// 在 `PlaylistDetailRequestPayload` 中用于包装歌单 ID 的结构。
#[derive(Serialize)]
pub struct PlaylistIdObject<'a> {
    /// 歌单的全局唯一 ID (gid)。
    pub global_collection_id: &'a str,
}

// --- 响应模型 ---

/// 歌单详情元数据 API 的顶层响应结构。
#[derive(Debug, Deserialize)]
pub struct PlaylistDetailResponse {
    /// 包含歌单详情的数据数组。
    #[serde(default)]
    pub data: Vec<PlaylistDetailData>,
}

/// 代表歌单的具体元数据信息。
#[derive(Debug, Deserialize)]
pub struct PlaylistDetailData {
    /// 歌单的全局唯一 ID。
    pub global_collection_id: String,
    /// 歌单名。
    pub name: String,
    /// 歌单介绍。
    #[serde(default)]
    pub intro: String,
    /// 包含 `{size}` 占位符的封面 URL。
    /// 留空可以获取最大尺寸的图片。
    pub pic: String,
    /// 歌单创建者用户名。
    pub list_create_username: String,
}

// =================================================================
// 歌单歌曲列表接口 (`/pubsongs/v2/get_other_list_file_nofilt`) 的模型
// =================================================================

/// 歌单歌曲列表 API 的顶层响应结构。
#[derive(Debug, Deserialize)]
pub struct PlaylistSongsResponse {
    /// 包含歌曲列表的数据容器。
    pub data: Option<PlaylistSongsData>,
}

/// 歌单歌曲列表的数据部分。
#[derive(Debug, Deserialize)]
pub struct PlaylistSongsData {
    /// 包含歌曲详细信息的列表。
    #[serde(default)]
    pub songs: Vec<PlaylistSongInfo>,
}

/// 歌单中歌曲的信息模型。
#[derive(Debug, Deserialize)]
pub struct PlaylistSongInfo {
    /// 歌曲名，通常格式为 "歌手 - 歌名"。
    pub name: String,
    /// 歌曲 hash。
    pub hash: String,
    /// 时长，单位是毫秒 (ms)。
    pub timelen: u64,
    /// 歌手信息列表。
    #[serde(default)]
    pub singerinfo: Vec<PlaylistSongAuthor>,
}

/// 歌单中歌曲的作者信息模型。
#[derive(Debug, Deserialize, Default)]
pub struct PlaylistSongAuthor {
    /// 歌手名。
    pub name: String,
    /// 歌手 ID。
    pub id: u64,
}

// =================================================================
// 歌曲详情接口 (`/v2/get_res_privilege/lite`) 的模型
// =================================================================

// --- 请求模型 ---

/// 用于请求歌曲详情的 POST 请求体负载结构。
#[derive(Serialize)]
pub struct SongDetailRequestPayload<'a> {
    /// 应用 ID。
    pub appid: &'a str,
    /// 客户端版本。
    pub clientver: &'a str,
    /// 地区代码。
    pub area_code: &'static str,
    /// 用户行为。
    pub behavior: &'static str,
    /// 是否需要 hash 偏移信息。
    pub need_hash_offset: u8,
    /// 是否需要关联信息。
    pub relate: u8,
    /// 是否支持验证。
    pub support_verify: u8,
    /// 请求的资源列表。
    pub resource: Vec<SongDetailResource<'a>>,
    /// 请求的音质列表。
    pub qualities: [&'static str; 7],
}

/// 在 `SongDetailRequestPayload` 中用于包装歌曲 hash 的资源对象。
#[derive(Serialize)]
pub struct SongDetailResource<'a> {
    /// 资源类型，固定为 "audio"。
    #[serde(rename = "type")]
    pub resource_type: &'static str,
    /// 页面 ID。
    pub page_id: u32,
    /// 歌曲的唯一文件哈希。
    pub hash: &'a str,
    /// 专辑 ID，通常为 0。
    pub album_id: u64,
}

// --- 响应模型 ---

/// 歌曲详情 API 的顶层响应结构。
#[derive(Debug, Deserialize)]
pub struct SongDetailResponse {
    /// API 状态码, 1 表示成功。
    pub status: i32,
    /// API 错误码, 0 表示成功。
    pub error_code: i32,
    /// API 返回一个包含多种音质信息的对象数组。
    #[serde(default)]
    pub data: Vec<SongDetailData>,
}

/// 代表歌曲具体信息的数据结构 (来自详情接口)。
#[derive(Debug, Deserialize)]
pub struct SongDetailData {
    /// 歌曲的唯一文件哈希。
    pub hash: String,
    /// 歌曲名，通常格式为 "歌手 - 歌名"。
    pub name: String,
    /// 单独的歌手名字段。
    pub singername: String,
    /// 包含时长和封面等信息的嵌套对象。
    pub info: SongDetailInfo,
}

/// `SongDetailData` 中包含时长和封面等信息的嵌套对象。
#[derive(Debug, Deserialize)]
pub struct SongDetailInfo {
    /// 时长，单位是毫秒 (ms)。
    pub duration: u64,
    /// 封面图 URL 模板。
    pub image: String,
}
