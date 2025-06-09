// Copyright (c) 2025 [WXRIW]
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// 导入 serde::Deserialize 特征，用于将 JSON 数据转换为 Rust 结构体
use serde::Deserialize;

// --- 歌词响应相关结构 ---

/// 表示从歌词接口 (例如 /weapi/song/lyric 或 /eapi/song/lyric/v1) 返回的 JSON 对象的顶层结构。
#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)] // 自动派生 Deserialize, Debug, Clone 特征
pub struct LyricResponse {
    #[serde(default)] // 如果 JSON 中缺少此字段，则使用 bool 的默认值 (false)
    pub sgc: bool, // 未知含义
    #[serde(default)]
    pub sfy: bool, // 未知含义
    #[serde(default)]
    pub qfy: bool, // 未知含义

    #[serde(default)]
    pub nolyric: bool, // 指示是否无歌词 (纯音乐)
    #[serde(default)]
    pub uncollected: bool, // 未知含义

    // 未知含义
    // `rename` 用于处理 JSON 字段名与 Rust 字段名不一致的情况
    // `alias` 提供了备选的 JSON 字段名
    // `default` 表示如果 JSON 中不存在这些字段，则此 Option<LyricUser> 为 None
    #[serde(rename = "transUser", alias = "tUser", default)]
    pub trans_user: Option<LyricUser>,

    // 未知含义
    #[serde(rename = "lyricUser", alias = "lUser", default)]
    pub lyric_user: Option<LyricUser>,

    // 各种类型的歌词内容
    #[serde(default)]
    pub lrc: Option<LyricsContent>, // 标准 LRC 歌词
    #[serde(default)]
    pub klyric: Option<LyricsContent>, // 卡拉OK歌词 (通常是逐字，但格式可能与YRC不同或为早期版本)
    #[serde(default)]
    pub tlyric: Option<LyricsContent>, // 翻译歌词 (通常是LRC格式)
    #[serde(default)]
    pub romalrc: Option<LyricsContent>, // 罗马音歌词 (通常是LRC格式)

    // YRC (网易云音乐逐字歌词) 相关字段。
    // 使用 serde_json::Value 是因为这些字段的内部结构可能比较复杂或不固定，
    // 或者只需要提取其中的特定子字段 (例如 yrc.lyric)。
    #[serde(default)]
    pub yrc: Option<serde_json::Value>, // YRC 逐字歌词 (通常是一个包含 "lyric" 字段的 JSON 对象)
    #[serde(default)]
    pub ytlrc: Option<serde_json::Value>, // YRC 翻译歌词 (结构类似 yrc)
    #[serde(default)]
    pub yromalrc: Option<serde_json::Value>, // YRC 罗马音歌词 (结构类似 yrc)

    pub code: i64, // API 响应状态码 (例如 200 表示成功)
}

/// 表示歌词贡献者的用户信息。
#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct LyricUser {
    pub id: i64, // 用户ID
    #[serde(default)]
    pub status: i64, // 未知含义
    #[serde(default)]
    pub demand: i64, // 未知含义
    pub userid: i64, // 用户ID?
    pub nickname: String, // 用户昵称?
    #[serde(default)]
    pub uptime: i64, // 上传/更新时间戳?
}

/// 表示特定类型的歌词内容 (例如 LRC, KLyric, TLyric)。
#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct LyricsContent {
    #[serde(default)]
    pub version: i64, // 歌词版本
    pub lyric: Option<String>, // 歌词文本内容 (字符串形式)
}

// --- 歌曲搜索响应相关结构 ---

/// 表示从歌曲搜索接口 (例如 /api/cloudsearch/pc) 返回的 JSON 对象的顶层结构。
#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct SearchResponse {
    #[serde(rename = "needLogin", default)] // 未知含义
    pub need_login: bool,
    pub result: Option<SearchResultData>, // 包含实际搜索结果的数据部分
    pub code: i64,                        // API 响应状态码
}

/// 包含搜索结果列表和总数。
#[derive(Deserialize, Debug, Clone)]
pub struct SearchResultData {
    #[serde(default)]
    pub songs: Vec<Song>, // 歌曲列表
    #[serde(rename = "songCount", default)]
    pub song_count: i64, // 搜索到的歌曲总数
}

/// 表示单个歌曲的详细信息。
#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct Song {
    pub id: i64,      // 歌曲 ID
    pub name: String, // 歌曲名称
    // 艺术家列表，`alias` 用于兼容 API 可能使用的不同字段名 ("artists" 或 "ar")
    #[serde(alias = "artists", alias = "ar", default)]
    pub artists: Vec<ArtistSimple>,
    // 专辑信息，`alias` 用于兼容 "album" 或 "al"
    #[serde(alias = "album", alias = "al", default)]
    pub album: Option<AlbumSimple>,
    // 歌曲时长 (毫秒)，`alias` 用于兼容 "duration" 或 "dt"
    #[serde(alias = "duration", alias = "dt", default)]
    pub duration: i64,
    #[serde(rename = "publishTime", default)] // 发行时间戳
    pub publish_time: i64,
    #[serde(default)]
    pub alias: Vec<String>, // 歌曲别名列表
    #[serde(default)]
    pub privilege: Option<Privilege>, // 未知含义
}

/// 表示艺术家的简化信息。
#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct ArtistSimple {
    pub id: i64,      // 艺术家 ID
    pub name: String, // 艺术家名称
    #[serde(default)]
    pub tns: Vec<String>, // 未知含义
    #[serde(default)]
    pub alias: Vec<String>, // 艺术家别名列表
}

/// 表示专辑的简化信息。
#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct AlbumSimple {
    pub id: i64,      // 专辑 ID
    pub name: String, // 专辑名称
    #[serde(rename = "picUrl", default)] // 专辑封面图片的 URL
    pub pic_url: Option<String>,
    #[serde(default)]
    pub tns: Vec<String>, // 未知含义
    #[serde(default)]
    pub pic: i64, // 未知含义
}

/// 表示歌曲的权限信息。
#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct Privilege {
    pub id: i64,     // 未知含义
    pub fee: i32,    // 未知含义
    pub payed: i32,  // 未知含义
    pub st: i32,     // 未知含义
    pub pl: i32,     // 未知含义
    pub dl: i32,     // 未知含义
    pub sp: i32,     // 未知含义
    pub cp: i32,     // 未知含义
    pub subp: i32,   // 未知含义
    pub cs: bool,    // 未知含义
    pub maxbr: i32,  // 未知含义
    pub fl: i32,     // 未知含义
    pub toast: bool, // 未知含义
    pub flag: i32,   // 未知含义
}
