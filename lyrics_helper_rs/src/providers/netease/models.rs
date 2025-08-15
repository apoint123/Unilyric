//! 此模块定义了所有用于反序列化网易云音乐 API 响应的 `struct` 数据结构。
//! API 来源于 <https://github.com/NeteaseCloudMusicApiReborn/api>

use serde::Deserialize;
use serde_json::Value;

// =================================================================
// 搜索接口 (`/eapi/cloudsearch/pc`) 的模型
// =================================================================

/// 搜索 API 的顶层响应结构。
#[derive(Debug, Deserialize)]
pub struct SearchResult {
    /// API 返回码，通常 `200` 表示成功。
    pub code: i32,
    /// 包含搜索结果的容器。
    pub result: SearchResultData,
}

/// 搜索结果的数据部分。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultData {
    /// 匹配到的歌曲对象列表。
    pub songs: Vec<Song>,
    /// 匹配到的歌曲总数。
    pub song_count: u32,
}

// =================================================================
// 歌词接口 (`/eapi/song/lyric/v1`) 的模型
// =================================================================

/// 歌词接口的顶层响应结构。
#[derive(Debug, Deserialize)]
pub struct LyricResult {
    /// API 返回码，`200` 表示成功。
    pub code: i32,
    /// 标准 LRC 歌词。
    pub lrc: Option<LyricData>,
    /// 翻译 LRC 歌词。
    pub tlyric: Option<LyricData>,
    /// 罗马音 LRC 歌词。
    pub romalrc: Option<LyricData>,
    /// 逐字 YRC 歌词。
    pub yrc: Option<LyricData>,
    /// 另一种格式的翻译歌词（内容通常与 `tlyric` 相同）。
    pub ytlrc: Option<Value>,
    /// 另一种格式的罗马音歌词（内容通常与 `romalrc` 相同）。
    pub yromalrc: Option<Value>,
}

/// 单一歌词内容的数据结构。
#[derive(Debug, Deserialize)]
pub struct LyricData {
    /// 歌词文本内容。
    pub lyric: String,
}

// =================================================================
// 专辑接口 (`/weapi/v1/album/:id`) 的模型
// =================================================================

/// 专辑详情 API 的顶层响应。
#[derive(Debug, Deserialize)]
pub struct AlbumResult {
    /// API 返回码，`200` 表示成功。
    pub code: i32,
    /// 专辑的详细信息。
    pub album: Option<NeteaseAlbum>,
    /// 专辑包含的歌曲列表（有时数据在此处，有时在 `album.songs` 中）。
    #[serde(default)]
    pub songs: Vec<Song>,
}

/// 代表一张专辑的详细信息。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeteaseAlbum {
    /// 专辑包含的歌曲列表。
    pub songs: Vec<Song>,
    /// 专辑的数字 ID。
    pub id: u64,
    /// 专辑名。
    pub name: String,
    /// 专辑的艺术家列表。
    pub artists: Vec<Artist>,
    /// 专辑封面图片 URL。
    pub pic_url: Option<String>,
}

// =================================================================
// 歌单接口 (`/weapi/v6/playlist/detail`) 的模型
// =================================================================

/// 歌单详情 API 的顶层响应。
#[derive(Debug, Deserialize)]
pub struct PlaylistResult {
    /// API 返回码，`200` 表示成功。
    pub code: i32,
    /// 歌单的详细信息。
    pub playlist: Playlist,
}

/// 代表一个歌单的详细信息。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    /// 歌单的数字 ID。
    pub id: u64,
    /// 歌单名。
    pub name: String,
    /// 歌单封面图片 URL。
    pub cover_img_url: String,
    /// 歌单描述。
    pub description: Option<String>,
    /// 创建者信息。
    pub creator: Creator,
    /// 歌单中的歌曲列表。
    pub tracks: Vec<Song>,
}

/// 歌单创建者信息。
#[derive(Debug, Deserialize)]
pub struct Creator {
    /// 创建者昵称。
    pub nickname: String,
}

// =================================================================
// 歌曲详情接口 (`/weapi/v3/song/detail`) 的模型
// =================================================================

/// 歌曲详情 API 的顶层响应。
#[derive(Debug, Deserialize)]
pub struct DetailResult {
    /// API 返回码，`200` 表示成功。
    pub code: i32,
    /// 歌曲对象列表。
    pub songs: Vec<Song>,
}

// =================================================================
// 歌曲播放链接接口 (`/weapi/song/enhance/player/url`) 的模型
// =================================================================

/// 获取歌曲播放链接 API 的顶层响应。
#[derive(Debug, Deserialize)]
pub struct SongUrlResult {
    /// API 返回码，`200` 表示成功。
    pub code: i32,
    /// 包含播放链接信息的数据列表。
    pub data: Vec<SongUrlData>,
}

/// 单个歌曲的播放链接信息。
#[derive(Debug, Deserialize)]
pub struct SongUrlData {
    /// 歌曲的数字 ID。
    pub id: u64,
    /// 歌曲的播放 URL。对于无版权或 VIP 歌曲，此字段可能为 `null`。
    pub url: Option<String>,
    /// 该条目自身的返回码，`200` 表示成功获取链接。
    pub code: i32,
}

// =================================================================
// 通用的歌曲/艺术家/专辑模型 (在多个接口响应中复用)
// =================================================================

/// 代表一首歌曲的详细信息。
#[derive(Debug, Deserialize)]
pub struct Song {
    /// 歌曲的数字 ID。
    pub id: u64,
    /// 歌曲名。
    pub name: String,
    /// 演唱者列表。
    #[serde(rename = "ar")]
    pub artist_info: Vec<Artist>,
    /// 所属专辑信息。
    #[serde(rename = "al")]
    pub album_info: Album,
    /// 歌曲时长，单位为毫秒 (ms)。
    #[serde(rename = "dt")]
    pub duration: u64,
}

/// 代表一位艺术家的简要信息。
#[derive(Debug, Deserialize)]
pub struct Artist {
    /// 艺术家的数字 ID。
    pub id: u64,
    /// 艺术家姓名。
    pub name: String,
}

/// 代表一张专辑的简要信息。
#[derive(Debug, Deserialize)]
pub struct Album {
    /// 专辑的数字 ID。
    pub id: u64,
    /// 专辑名。
    pub name: String,
    /// 专辑封面图片 URL。
    #[serde(rename = "picUrl")]
    pub pic_url: Option<String>,
}

// =================================================================
// 歌手歌曲接口 (`/api/v1/artist/songs`) 的模型
// =================================================================

/// 歌手歌曲 API 的顶层响应结构。
#[derive(Debug, Deserialize)]
pub struct ArtistSongsResult {
    /// API 返回码，`200` 表示成功。
    pub code: i32,
    /// 是否还有更多歌曲。
    pub more: bool,
    /// 歌曲总数。
    pub total: u32,
    /// 本次请求返回的歌曲列表。
    pub songs: Vec<Song>,
}

// =================================================================
// 专辑内容接口 (`/api/v1/album/:id`) 的模型
// =================================================================

/// 专辑内容 API 的顶层响应结构。
#[derive(Debug, Deserialize)]
pub struct AlbumContentResult {
    /// API 返回码，`200` 表示成功。
    pub code: i32,
    /// 专辑的详细信息。
    pub album: Album,
    /// 专辑包含的歌曲列表。
    pub songs: Vec<Song>,
}

// =================================================================
// 歌曲播放链接接口 V1 (`/eapi/song/enhance/player/url/v1`) 的模型
// =================================================================

/// 获取歌曲播放链接 API (EAPI v1) 的顶层响应。
#[derive(Debug, Deserialize)]
pub struct SongUrlResultV1 {
    /// 包含播放链接信息的数据列表。
    pub data: Vec<SongUrlDataV1>,
    /// API 返回码，`200` 表示成功。
    pub code: i32,
}

/// 单个歌曲的播放链接信息 (EAPI v1)。
#[derive(Debug, Deserialize)]
pub struct SongUrlDataV1 {
    /// 歌曲的数字 ID。
    pub id: u64,
    /// 歌曲的播放 URL。对于无版权或 VIP 歌曲，此字段可能为 `null`。
    pub url: Option<String>,
    /// 该条目自身的返回码，`200` 表示成功获取链接。
    pub code: i32,
    /// 音质等级
    pub level: Option<String>,
    /// 文件大小
    pub size: u64,
}
