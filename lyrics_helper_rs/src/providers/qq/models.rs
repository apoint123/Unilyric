//! 此模块定义了所有用于反序列化 QQ 音乐 API 响应的数据结构。
//! API 来源于 <https://github.com/luren-dc/QQMusicApi>

use serde::Deserialize;

/// 一个通用的结构体，用于捕获所有 musicu.fcg 业务对象中都存在的 `code` 字段。
#[derive(Debug, Deserialize)]
pub struct BusinessCode {
    /// 业务返回码，0 表示成功。
    pub code: i32,
}

// =================================================================
// 搜索接口 ( `music.search.SearchCgiService.DoSearchForQQMusicMobile` ) 的模型
// =================================================================

/// 包含响应代码和实际数据。
#[derive(Debug, Deserialize)]
pub struct Req1 {
    /// 包含响应主体的容器。
    pub data: Option<Req1Data>,
}

/// 响应数据的容器。
#[derive(Debug, Deserialize)]
pub struct Req1Data {
    /// 响应主体。
    pub body: Option<Req1Body>,
}

/// 响应主体，根据搜索类型包含不同的内容。
#[derive(Debug, Deserialize)]
pub struct Req1Body {
    /// 歌曲搜索结果。
    #[serde(default)]
    pub item_song: Vec<Song>,
}

// =================================================================
// 通用的 `Song`, `Singer`, `Album` 模型，在多个接口中复用
// =================================================================

/// 代表一首歌曲的详细信息。
#[derive(Debug, Deserialize, Clone, serde::Serialize)]
pub struct Song {
    /// 歌曲的数字 ID。
    pub id: Option<u64>,
    /// 歌曲的媒体 ID (mid)，是获取歌词和播放链接的关键标识。
    pub mid: String,
    /// 歌曲名。
    pub title: String,
    /// 演唱者列表。
    pub singer: Vec<Singer>,
    /// 所属专辑信息。
    pub album: Album,
    /// 歌曲时长，单位为秒 (s)。
    pub interval: u64,
    /// 同版本的其他曲目（例如，不同混音或版本）。
    #[serde(rename = "grp")]
    pub group: Option<Vec<Self>>,
    /// 语言代码，9 用来指示纯音乐
    pub language: Option<i64>,
}

/// 代表一位演唱者的信息。
#[derive(Debug, Deserialize, Clone, serde::Serialize)]
pub struct Singer {
    /// 演唱者姓名。
    /// 兼容来自搜索结果的 `singerName` 字段。
    #[serde(alias = "singerName")]
    pub name: String,
    /// 演唱者的数字 ID。
    /// 兼容来自搜索结果的 `singerID` 字段。
    #[serde(alias = "singerID")]
    pub id: Option<u64>,
    /// 演唱者的媒体 ID (mid)。
    /// 兼容来自搜索结果的 `singerMID` 字段。
    #[serde(alias = "singerMID")]
    pub mid: Option<String>,
}

/// 代表一张专辑的简要信息。
#[derive(Debug, Deserialize, Clone, serde::Serialize)]
pub struct Album {
    /// 专辑的数字 ID。
    pub id: Option<u64>,
    /// 专辑的媒体 ID (mid)。
    pub mid: Option<String>,
    /// 专辑名。
    pub name: String,
}

/// 代表 QQ 音乐封面支持的常见分辨率。
///
/// 并非所有专辑都支持所有尺寸。
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum QQMusicCoverSize {
    /// 90x90 像素，适用于缩略图。
    Size90,
    /// 150x150 像素。
    Size150,
    /// 300x300 像素，通用尺寸。
    Size300,
    /// 500x500 像素，高清尺寸。
    Size500,
    /// 800x800 像素，超高清尺寸，可能接近原图。
    Size800,
}

impl QQMusicCoverSize {
    /// 将给定封面尺寸变体的像素大小以 `u32` 形式返回。
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::Size90 => 90,
            Self::Size150 => 150,
            Self::Size300 => 300,
            Self::Size500 => 500,
            Self::Size800 => 800,
        }
    }
}

// =================================================================
// 专辑信息接口 (`music.musichallAlbum.AlbumInfoServer.GetAlbumDetail`) 的模型
// =================================================================

/// 用于包装 `GetAlbumDetail` API 响应的顶层容器。
#[derive(Debug, serde::Deserialize)]
pub struct AlbumDetailApiResult {
    /// 包含了核心专辑信息的对象。
    pub data: AlbumInfo,
}

/// 代表一张专辑的详细信息，根据 `GetAlbumDetail` API 的响应格式重构。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumInfo {
    /// 包含专辑核心元数据的 `basicInfo` 对象。
    #[serde(rename = "basicInfo")]
    pub basic_info: AlbumBasicInfo,
    /// 专辑的发行公司信息。
    pub company: CompanyInfo,
    /// 包含专辑歌手列表的 `singer` 对象。
    pub singer: AlbumSingerInfo,
}

/// 专辑详细信息中的 "basicInfo" 部分，包含了核心元数据。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumBasicInfo {
    /// 专辑的媒体 ID (mid)。
    pub album_mid: String,
    /// 专辑名称。
    pub album_name: String,
    /// 发行日期，格式通常为 "YYYY-MM-DD"。
    pub publish_date: String,
    /// 专辑描述或文案。
    pub desc: String,
}

/// 专辑详细信息中的 "company" 部分，代表发行公司信息。
#[derive(Debug, serde::Deserialize)]
pub struct CompanyInfo {
    /// 公司的数字 ID。
    #[serde(rename = "ID")]
    pub id: u64,
    /// 公司名称。
    pub name: String,
}

/// 专辑详细信息中的 "singer" 部分，它本身是一个容器，包含了歌手列表。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumSingerInfo {
    /// 参与该专辑的歌手列表。
    pub singer_list: Vec<AlbumSinger>,
}

/// 专辑详细信息中 "singer.singerList" 数组里的单个歌手条目。
#[derive(Debug, serde::Deserialize)]
pub struct AlbumSinger {
    /// 歌手的媒体 ID (mid)。
    pub mid: String,
    /// 歌手姓名。
    pub name: String,
}

// =================================================================
// 专辑歌曲列表接口 ( `music.musichallAlbum.AlbumSongList.GetAlbumSongList` ) 的模型
// =================================================================

#[derive(Debug, Deserialize, Clone)]
/// 专辑歌曲列表信息的容器。
pub struct AlbumSonglistInfo {
    /// 实际的数据内容。
    pub data: DataInfo,
}

#[derive(Debug, Deserialize, Clone)]
/// 包含歌曲列表和总数的具体数据。
pub struct DataInfo {
    /// 歌曲项目列表。
    #[serde(rename = "songList")]
    pub song_list: Vec<SongItem>,
}

#[derive(Debug, Deserialize, Clone)]
/// 专辑中的单个歌曲项目，主要用于包装 `SongInfo`。
pub struct SongItem {
    /// 详细的歌曲信息。
    #[serde(rename = "songInfo")]
    pub song_info: SongInfo,
}

// =================================================================
// 通用、详细的歌曲信息模型 (在多个接口响应中使用)
// =================================================================

#[derive(Debug, Deserialize, Clone)]
/// 通用的歌曲详细信息结构体，在多个 API 响应中复用。
pub struct SongInfo {
    /// 歌曲的数字 ID。
    pub id: u64,
    /// 歌曲的字符串 ID (songmid)。
    pub mid: String,
    /// 歌曲名。
    pub name: String,
    /// 歌曲标题，通常与 `name` 相同。
    pub title: String,
    /// 歌曲副标题。
    pub subtitle: String,
    /// 演唱者列表。
    pub singer: Vec<Singer>,
    /// 所属专辑信息。
    pub album: Album,
    /// 歌曲时长，单位为秒 (s)。
    pub interval: u64,
    /// 歌曲发行日期，格式通常为 "YYYY-MM-DD"。
    pub time_public: String,
    /// 歌曲的语言信息。
    pub language: Option<i64>,
}

// =================================================================
// 歌手歌曲列表接口 ( `musichall.song_list_server.GetSingerSongList` ) 的模型
// =================================================================

/// 用于包装 `GetSingerSongList` API 响应的顶层容器。
#[derive(Debug, serde::Deserialize)]
pub struct SingerSongListApiResult {
    /// 包含了核心业务数据的对象。
    pub data: SingerSongListResult,
}

/// 包含 `GetSingerSongList` API 核心响应数据的结构体。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SingerSongListResult {
    /// 歌曲列表，每一项都包含了详细的 songInfo。
    pub song_list: Vec<SongItem>,
}

// =================================================================
// 歌单接口 ( `c.y.qq.com/qzone/fcg-bin/fcg_ucc_getcdinfo_byids_cp.fcg` ) 的模型
// =================================================================

/// 用于包装 `get_playlist_detail` API 响应数据的顶层容器。
#[derive(Debug, serde::Deserialize)]
pub struct PlaylistApiResult {
    /// 包含了核心歌单数据的对象。
    pub data: PlaylistDetailData,
}

/// 包含 `get_playlist_detail` API 核心响应数据的结构体。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistDetailData {
    /// 歌单的详细信息。
    #[serde(rename = "dirinfo")]
    pub info: PlaylistDetailInfo,
    /// 歌单内的歌曲列表。
    #[serde(default)]
    pub songlist: Vec<SongInfo>,
    /// 歌单的歌曲总数。
    #[serde(rename = "songlist_size")]
    pub total_song_num: u32,
}

/// 歌单的元数据信息。
#[derive(Debug, serde::Deserialize)]
pub struct PlaylistDetailInfo {
    /// 歌单ID。
    pub id: u64,
    /// 歌单名。
    pub title: String,
    /// 歌单封面 URL。
    #[serde(rename = "picurl")]
    pub cover_url: String,
    /// 创建者昵称。
    pub host_nick: String,
    /// 歌单描述。
    #[serde(rename = "desc")]
    pub description: String,
    /// 播放量。
    pub listennum: u64,
}

// =================================================================
// 歌曲信息接口 ( `music.pf_song_detail_svr.get_song_detail_yqq` ) 的模型
// =================================================================

/// 用于包装 `get_song_detail_yqq` API 响应数据的顶层容器。
#[derive(Debug, serde::Deserialize)]
pub struct SongDetailApiContainer {
    /// 包含了核心歌曲信息的对象。
    pub data: SongDetailApiResult,
}

/// 包含 `get_song_detail_yqq` API 核心响应数据的结构体。
#[derive(Debug, serde::Deserialize)]
pub struct SongDetailApiResult {
    /// 包含了核心歌曲信息的对象。
    pub track_info: SongInfo,
}

// =================================================================
// 歌词接口 ( `music.musichallSong.PlayLyricInfo.GetPlayLyricInfo` ) 的模型
// =================================================================

/// 用于包装 `GetPlayLyricInfo` API 响应的顶层容器。
#[derive(Debug, serde::Deserialize)]
pub struct LyricApiResult {
    /// 包含了核心歌词数据的对象。
    pub data: LyricApiResponse,
}

/// `GetPlayLyricInfo` API 响应的核心数据，包含加密的歌词字符串。
#[derive(Debug, serde::Deserialize)]
pub struct LyricApiResponse {
    /// 加密的主歌词内容（QRC格式，Base64 编码）。
    pub lyric: String,
    /// 加密的翻译歌词内容（LRC格式，Base64 编码）。
    pub trans: String,
    /// 加密的罗马音歌词内容（QRC格式，Base64 编码）。
    pub roma: String,
}

// =================================================================
// 纯 LRC 接口 ( `c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg` ) 的模型
// =================================================================

#[derive(serde::Deserialize)]
pub struct LrcApiResponse {
    pub code: i32,
    pub lyric: Option<String>,
    pub trans: Option<String>,
}
