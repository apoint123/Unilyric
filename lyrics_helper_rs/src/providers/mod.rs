//! 提供商模块
//!
//! 该模块定义了与 Providers 进行交互的核心抽象。

pub mod amll_ttml_database;
pub mod kugou;
pub mod login;
// pub mod musixmatch;
pub mod netease;
pub mod qq;

use std::sync::Arc;

use crate::error::Result;
use crate::http::{HttpClient, WreqClient};
pub use login::LoginProvider;

use async_trait::async_trait;
use lyrics_helper_core::{
    CoverSize, FullLyricsResult, ParsedSourceData, SearchResult, Track, model::generic,
};

/// 定义了所有音乐平台提供商需要实现的通用接口。
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait Provider: Send + Sync {
    /// 使用默认的 `WreqClient` HTTP 客户端创建一个新的 Provider 实例。
    async fn new() -> Result<Self>
    where
        Self: Sized,
    {
        let http_client = Arc::new(WreqClient::new()?);
        Self::with_http_client(http_client).await
    }

    /// 使用自定义的 HTTP 客户端创建一个新的 Provider 实例。
    async fn with_http_client(http_client: Arc<dyn HttpClient>) -> Result<Self>
    where
        Self: Sized;

    ///
    /// 返回提供商的唯一名称。
    ///
    /// 一个全小写的静态字符串，例如 `"qq"`, `"netease"`。
    ///
    fn name(&self) -> &'static str;

    ///
    /// 尝试将此 Provider 转换为 `LoginProvider`。
    ///
    /// 如果 Provider 不支持登录，则返回 None。
    /// 默认返回 None，需要登录的 Provider 应重写此方法。
    ///
    fn as_login_provider(&self) -> Option<&dyn LoginProvider> {
        None
    }

    /// 根据歌曲信息（如歌曲标题、艺术家）搜索歌曲。
    ///
    /// # 参数
    /// * `track` - 一个包含搜索关键词的 `Track` 引用。
    ///
    /// # 返回
    /// 一个 `Result`，成功时包含一个 `Vec<SearchResult>`，代表搜索到的歌曲列表。
    ///
    async fn search_songs(&self, track: &Track<'_>) -> Result<Vec<SearchResult>>;

    ///
    /// 根据歌曲 ID 获取已解析的的歌词，包括 `ParsedSourceData` 数据和原始副本。
    ///
    /// 这是获取歌词的主要方法。
    ///
    /// # 参数
    /// * `song_id` - 特定于该提供商的歌曲 ID。
    ///
    /// # 返回
    /// 一个 `Result`，成功时包含解析后的 `ParsedSourceData` 数据和原始副本。
    ///
    async fn get_full_lyrics(&self, song_id: &str) -> Result<FullLyricsResult>;

    ///
    /// 根据歌曲 ID 获取已解析的的歌词。
    ///
    /// 一般在只需要 `ParsedSourceData` 数据的场景下使用。
    ///
    /// # 参数
    /// * `song_id` - 特定于该提供商的歌曲 ID。
    ///
    /// # 返回
    /// 一个 `Result`，成功时包含解析和合并后的 `ParsedSourceData`。
    ///
    async fn get_lyrics(&self, song_id: &str) -> Result<ParsedSourceData> {
        Ok(self.get_full_lyrics(song_id).await?.parsed)
    }

    ///
    /// 根据专辑 ID 获取专辑的详细信息，包括专辑封面、发行日期和歌曲列表等。
    ///
    /// # 参数
    /// * `album_id` - 特定于该提供商的专辑 ID。
    ///
    /// # 返回
    /// 一个 `Result`，成功时包含一个通用的 `generic::Album` 结构。
    ///
    async fn get_album_info(&self, album_id: &str) -> Result<generic::Album>;

    ///
    /// 分页获取指定专辑的歌曲列表。
    ///
    /// # 参数
    /// * `album_id` - 专辑 ID。
    /// * `page` - 页码（通常从 1 开始）。
    /// * `page_size` - 每页的歌曲数量。
    ///
    /// # 返回
    /// 一个 `Result`，成功时包含一个 `Vec<generic::Song>`。
    ///
    async fn get_album_songs(
        &self,
        album_id: &str,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<generic::Song>>;

    ///
    /// 分页获取指定歌手的热门歌曲列表。
    ///
    /// # 参数
    /// * `singer_id` - 歌手 ID。
    /// * `page` - 页码（通常从 1 开始）。
    /// * `page_size` - 每页的歌曲数量。
    ///
    /// # 返回
    /// 一个 `Result`，成功时包含一个 `Vec<generic::Song>`。
    ///
    async fn get_singer_songs(
        &self,
        singer_id: &str,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<generic::Song>>;

    ///
    /// 根据歌单 ID 获取歌单的详细信息，包括歌曲列表。
    ///
    /// # 参数
    /// * `playlist_id` - 歌单 ID。
    ///
    /// # 返回
    /// 一个 `Result`，成功时包含一个通用的 `generic::Playlist` 结构。
    ///
    async fn get_playlist(&self, playlist_id: &str) -> Result<generic::Playlist>;

    ///
    /// 根据歌曲 ID 获取单首歌曲的详细信息。
    ///
    /// # 参数
    /// * `song_id` - 歌曲 ID。
    ///
    /// # 返回
    /// 一个 `Result`，成功时包含一个通用的 `generic::Song` 结构。
    ///
    async fn get_song_info(&self, song_id: &str) -> Result<generic::Song>;

    ///
    /// 根据歌曲 ID 获取可播放的音频文件链接。
    ///
    /// # 注意
    /// 大概率因版权、地区限制或 VIP 而失败。
    /// 建议登陆后再调用此接口。
    ///
    /// # 参数
    /// * `song_id` - 歌曲 ID。
    ///
    /// # 返回
    /// 一个 `Result`，成功时包含一个代表可播放 URL 的 `String`。
    ///
    async fn get_song_link(&self, song_id: &str) -> Result<String>;

    /// 根据专辑 ID 获取专辑封面的 URL。
    ///
    /// # 参数
    /// * `album_id` - 特定于该提供商的专辑 ID。
    /// * `size` - 期望的封面尺寸。
    ///
    /// # 返回
    /// 一个 `Result`，成功时包含一个封面图片的 URL 字符串。
    /// 如果提供商不支持此功能或找不到封面，返回错误。
    async fn get_album_cover_url(&self, album_id: &str, size: CoverSize) -> Result<String>;
}
