//! 定义了整个库通用的、与具体提供商无关的核心数据模型。
//!
//! 这些结构体（如 `Artist`, `Song`, `Album`）是所有 Provider 在获取到
//! 各自平台的数据后，需要转换成的目标标准格式。

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// 代表一位艺术家的通用模型。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Artist {
    /// 艺术家的唯一 ID (通常来自其所在平台)。
    pub id: String,
    /// 艺术家姓名。
    pub name: String,
}

/// 代表一首歌曲的通用模型。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Song {
    /// 歌曲的唯一 ID (通常来自其所在平台)。
    pub id: String,
    /// 歌曲名。
    pub name: String,
    /// 演唱者列表。
    pub artists: Vec<Artist>,
    /// 歌曲时长。
    pub duration: Option<Duration>,
    /// 歌曲所属专辑名。
    pub album: Option<String>,
    /// 歌曲封面图片 URL。
    pub cover_url: Option<String>,
    /// 歌曲在提供商平台的 ID。
    pub provider_id: String,
    /// 专辑在提供商平台的 ID。
    pub album_id: Option<String>,
}

/// 代表一张专辑的通用模型。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Album {
    /// 专辑的唯一 ID。
    pub id: String,
    /// 专辑名。
    pub name: String,
    /// 专辑的艺术家列表。
    pub artists: Option<Vec<Artist>>,
    /// 专辑包含的歌曲列表。
    pub songs: Option<Vec<Song>>,
    /// 专辑描述。
    pub description: Option<String>,
    /// 专辑发行日期。
    pub release_date: Option<String>,
    /// 歌单封面图片 URL。
    pub cover_url: Option<String>,
    /// 专辑在提供商平台的 ID。
    pub provider_id: String,
}

/// 代表通用的封面图片尺寸
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CoverSize {
    /// 缩略图 (通常 < 150px)
    Thumbnail,
    /// 中等尺寸 (通常 300px - 500px)
    Medium,
    /// 大尺寸或原始尺寸 (通常 > 500px)
    Large,
}

/// 代表一个歌单的通用模型。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Playlist {
    /// 歌单的唯一 ID。
    pub id: String,
    /// 歌单名。
    pub name: String,
    /// 歌单封面图片 URL。
    pub cover_url: Option<String>,
    /// 歌单创建者姓名。
    pub creator_name: Option<String>,
    /// 歌单描述。
    pub description: Option<String>,
    /// 歌单包含的歌曲列表。
    pub songs: Option<Vec<Song>>,
}
