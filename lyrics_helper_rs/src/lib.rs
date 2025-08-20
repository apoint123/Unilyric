#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::ignore_without_reason)]

//! # Lyrics Helper RS
//!
//! 一个强大的 Rust 库，用于从多个在线音乐服务搜索歌曲、获取歌词，并进行格式转换。
//!
//! ## 主要功能
//!
//! - **歌词获取**: 从 QQ音乐、网易云音乐、酷狗音乐和 AMLL TTML Database 搜索和获取歌词。
//! - **歌词转换**:
//!   - 支持多种输入格式 (LRC, QRC, KRC, YRC, TTML...)。
//!   - 能够合并主歌词、多个翻译文件、多个罗马音文件。
//!   - 支持输出为多种目标格式，如 LRC, KRC, TTML 等。
//!
//! ## 获取歌词
//!
//! ```rust,no_run
//! use lyrics_helper_core::*;
//! use lyrics_helper_rs::{LyricsHelper, SearchMode};
//!
//! async {
//!     let mut helper = LyricsHelper::new();
//!     helper.load_providers().await.unwrap();
//!
//!     let track_to_search = Track {
//!         title: Some("灯火通明"),
//!         artists: Some(&["小蓝背心"]),
//!         album: None,
//!         duration: None,
//!     };
//!     match helper.search_lyrics(&track_to_search, SearchMode::Ordered) {
//!         Ok(future) => match future.await {
//!             Ok(Some(lyrics)) => println!("获取歌词成功！共 {} 行。", lyrics.lyrics.parsed.lines.len()),
//!             Ok(None) => println!("未找到任何可用的歌词。"),
//!             Err(e) => eprintln!("搜索歌词时发生错误: {}", e),
//!         },
//!         Err(e) => eprintln!("创建搜索任务时发生错误: {}", e),
//!     }
//! };
//! ```
//!
//! ## 格式转换
//!
//! ```rust
//! use lyrics_helper_core::*;
//! use lyrics_helper_rs::LyricsHelper;
//!
//! let helper = LyricsHelper::new();
//!
//! let main_lyric = InputFile::new(
//!     "[00:01.00]Hello\n[00:02.00]World".to_string(),
//!     LyricFormat::Lrc, None, None
//! );
//! let translation = InputFile::new(
//!     "[00:01.00]你好\n[00:02.00]世界".to_string(),
//!     LyricFormat::Lrc, Some("zh-Hans".to_string()), None
//! );
//!
//! let input = ConversionInput {
//!     main_lyric,
//!     translations: vec![translation],
//!     romanizations: vec![],
//!     target_format: LyricFormat::Ttml,
//!     user_metadata_overrides: None
//! };
//!
//! let options = ConversionOptions::default();
//!
//! match helper.convert_lyrics(&input, &options) {
//!     Ok(ttml_output) => {
//!         println!("转换成功！TTML 内容:\n{:?}", ttml_output);
//!     }
//!     Err(e) => {
//!         eprintln!("转换失败: {}", e);
//!     }
//! }
//! ```
pub mod config;
pub mod converter;
pub mod error;
pub mod http;
pub mod model;
pub mod providers;
pub mod search;
#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
pub mod wasm;

use std::{
    collections::{HashMap, HashSet},
    hash::BuildHasher,
    pin::Pin,
    sync::Arc,
};

use crate::http::{HttpClient, ReqwestClient};

#[cfg(not(target_arch = "wasm32"))]
use futures::FutureExt;
use futures::{Future, future};
use lyrics_helper_core::{
    ComprehensiveSearchResult, ConversionInput, ConversionOptions, CoverSize, FullConversionResult,
    FullLyricsResult, LyricFormat, LyricsAndMetadata, ParsedSourceData, SearchResult, Track,
};

pub use crate::error::{LyricsHelperError, Result};

/// 支持的歌词提供商枚举
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProviderName {
    /// QQ音乐
    QQMusic,
    /// 网易云音乐
    Netease,
    /// 酷狗音乐
    Kugou,
    /// AMLL TTML 数据库
    AmllTtmlDatabase,
}

impl ProviderName {
    /// 获取提供商的字符串标识符
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::QQMusic => "qq",
            Self::Netease => "netease",
            Self::Kugou => "kugou",
            Self::AmllTtmlDatabase => "amll-ttml-database",
        }
    }

    /// 从字符串标识符创建 `ProviderName`
    #[must_use]
    pub fn try_from_str(s: &str) -> Option<Self> {
        match s {
            "qq" => Some(Self::QQMusic),
            "netease" => Some(Self::Netease),
            "kugou" => Some(Self::Kugou),
            "amll-ttml-database" => Some(Self::AmllTtmlDatabase),
            _ => None,
        }
    }

    /// 获取所有支持的提供商
    #[must_use]
    pub fn all() -> Vec<Self> {
        vec![
            Self::QQMusic,
            Self::Netease,
            Self::Kugou,
            ProviderName::AmllTtmlDatabase,
        ]
    }

    /// 获取提供商的显示名称
    #[must_use]
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::QQMusic => "QQ音乐",
            Self::Netease => "网易云音乐",
            Self::Kugou => "酷狗音乐",
            Self::AmllTtmlDatabase => "AMLL TTML 数据库",
        }
    }
}

impl std::fmt::Display for ProviderName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ProviderName {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::try_from_str(s).ok_or_else(|| format!("不支持的提供商: {s}"))
    }
}
use crate::providers::amll_ttml_database::AmllTtmlDatabase;

use crate::providers::{Provider, kugou::KugouMusic, netease::NeteaseClient, qq::QQMusic};

/// 定义歌词的搜索策略。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchMode {
    /// 按预设顺序依次搜索提供商。
    ///
    /// 按 `providers` 列表的顺序，逐个尝试，
    /// 并返回从第一个找到可用歌词的提供商处获取的结果。
    ///
    /// 不建议使用此模式，因为提供商总是会返回至少一个结果，（AMLL 歌词库除外）
    /// 即使返回的结果毫无关联。因此，这个模式实际上和
    /// `Specific` 模式一样。如果搜索的歌曲在提供商中不存在，
    /// 则基本上会返回一个错误的歌词，或者直接返回错误。
    Ordered,
    /// 并发搜索所有提供商。
    ///
    /// 同时向所有提供商发起搜索请求，聚合所有结果，
    /// 然后为最高的匹配项获取歌词。这通常能找到最准确的匹配，但开销也最大。
    Parallel,
    /// 只搜索一个特定的提供商。
    ///
    /// 参数是提供商的枚举值。
    Specific(ProviderName),
    /// 在指定的一个提供商子集中并行搜索。
    Subset(Vec<ProviderName>),
}

impl SearchMode {
    /// 创建一个搜索特定提供商的模式
    #[must_use]
    pub fn specific(provider: ProviderName) -> Self {
        SearchMode::Specific(provider)
    }

    /// 创建一个搜索提供商子集的模式
    #[must_use]
    pub fn subset(providers: Vec<ProviderName>) -> Self {
        SearchMode::Subset(providers)
    }

    /// 创建一个只搜索网易云音乐的模式
    #[must_use]
    pub const fn netease_only() -> Self {
        SearchMode::Specific(ProviderName::Netease)
    }

    /// 创建一个只搜索QQ音乐的模式
    #[must_use]
    pub const fn qq_only() -> Self {
        SearchMode::Specific(ProviderName::QQMusic)
    }

    /// 创建一个只搜索酷狗音乐的模式
    #[must_use]
    pub const fn kugou_only() -> Self {
        SearchMode::Specific(ProviderName::Kugou)
    }

    /// 创建一个只搜索AMLL TTML数据库的模式
    #[must_use]
    pub const fn amll_only() -> Self {
        SearchMode::Specific(ProviderName::AmllTtmlDatabase)
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// 一个代表歌词搜索结果的 Future。
pub type SearchLyricsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<LyricsAndMetadata>>> + Send + 'a>>;
#[cfg(target_arch = "wasm32")]
/// 一个代表歌词搜索结果的 Future。
pub type SearchLyricsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<LyricsAndMetadata>>> + 'a>>;

#[cfg(not(target_arch = "wasm32"))]
/// 一个代表全面的歌词搜索结果的 Future。
pub type SearchLyricsComprehensiveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<ComprehensiveSearchResult>>> + Send + 'a>>;
#[cfg(target_arch = "wasm32")]
/// 一个代表全面的歌词搜索结果的 Future。
pub type SearchLyricsComprehensiveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<ComprehensiveSearchResult>>> + 'a>>;

/// 顶层歌词助手客户端，封装了所有提供商，为用户提供统一、简单的接口。
///
/// 这是与本库交互的主要入口点。
pub struct LyricsHelper {
    providers: Vec<Arc<dyn Provider + Send + Sync>>,
    http_client: Arc<dyn HttpClient>,
}

impl Default for LyricsHelper {
    fn default() -> Self {
        Self::new()
    }
}

impl LyricsHelper {
    /// 创建一个新的、空的 `LyricsHelper` 实例。
    ///
    /// 此时实例只适用于歌词转换功能。
    /// 若要使用歌词搜索和下载功能，必须先调用 `load_providers()` 方法。
    #[must_use]
    pub fn new() -> Self {
        let default_client =
            Arc::new(ReqwestClient::new().expect("Failed to create default HTTP client"));
        Self::new_with_http_client(default_client)
    }

    /// 创建一个新的 `LyricsHelper` 实例，使用一个自定义的 HTTP 客户端。
    ///
    /// # 参数
    /// * `http_client` - 一个实现了 `HttpClient` trait 的客户端实例。
    #[must_use]
    pub fn new_with_http_client(http_client: Arc<dyn HttpClient>) -> Self {
        Self {
            providers: Vec::new(),
            http_client,
        }
    }

    /// 初始化并加载所有歌词提供商。
    ///
    /// 这个方法会执行网络请求来准备提供商。
    /// 在使用搜索功能之前，须先调用此方法。
    ///
    /// # 返回
    /// 如果所有提供商都成功或部分成功初始化，则返回 `Ok(())`。
    pub async fn load_providers(&mut self) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        type Initializer<'a> = Pin<
            Box<
                dyn Future<Output = (&'static str, Result<Box<dyn Provider + Send + Sync>>)>
                    + Send
                    + 'a,
            >,
        >;
        #[cfg(target_arch = "wasm32")]
        type Initializer<'a> = Pin<
            Box<dyn Future<Output = (&'static str, Result<Box<dyn Provider + Send + Sync>>)> + 'a>,
        >;

        let http_client = self.http_client.clone();

        let initializers: Vec<Initializer<'_>> = vec![
            Box::pin(async {
                (
                    "QQMusic",
                    QQMusic::with_http_client(http_client.clone())
                        .await
                        .map(|p| Box::new(p) as Box<_>),
                )
            }),
            Box::pin(async {
                (
                    "NeteaseClient",
                    NeteaseClient::with_http_client(http_client.clone())
                        .await
                        .map(|p| Box::new(p) as Box<_>),
                )
            }),
            Box::pin(async {
                (
                    "KugouMusic",
                    KugouMusic::with_http_client(http_client.clone())
                        .await
                        .map(|p| Box::new(p) as Box<_>),
                )
            }),
            Box::pin(async {
                (
                    "AmllTtmlDatabase",
                    AmllTtmlDatabase::with_http_client(http_client.clone())
                        .await
                        .map(|p| Box::new(p) as Box<_>),
                )
            }),
        ];

        let results = future::join_all(initializers).await;

        let providers = results
            .into_iter()
            .filter_map(|(name, result)| match result {
                Ok(provider) => {
                    tracing::info!("[Main] Provider '{}' 初始化成功。", name);
                    Some(Arc::from(provider))
                }
                Err(e) => {
                    tracing::error!("[Main] Provider '{}' 初始化失败: {}", name, e);
                    None
                }
            })
            .collect();

        self.providers = providers;
        Ok(())
    }

    /// 在所有支持的音乐平台中并发地搜索歌曲。
    ///
    /// # 参数
    /// * `track_meta` - 一个包含歌曲标题、艺术家等信息的 `Track` 结构体引用。
    ///
    /// # 返回
    /// 一个 `Result`，成功时包含一个 `Vec<SearchResult>`，该向量已按匹配度从高到低排序。
    #[must_use]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn search_track<'a>(
        &self,
        track_meta: &Track<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchResult>>> + Send + 'a>> {
        if self.providers.is_empty() {
            return Box::pin(async { Err(LyricsHelperError::ProvidersNotInitialized) });
        }

        let providers = self.providers.clone();
        let track_meta = track_meta.clone();

        Box::pin(async move {
            let search_futures = providers
                .iter()
                .map(|provider| search::search_track(provider.as_ref(), &track_meta, true));

            let all_results: Vec<SearchResult> = future::join_all(search_futures)
                .await
                .into_iter()
                .filter_map(Result::ok)
                .flatten()
                .collect();

            let mut sorted_results = all_results;
            sorted_results.sort_by(|a, b| b.match_type.cmp(&a.match_type));

            let mut unique_results = Vec::new();
            let mut seen_keys = HashSet::new();

            for result in sorted_results {
                let key = (result.provider_name.clone(), result.provider_id.clone());
                if seen_keys.insert(key) {
                    unique_results.push(result);
                }
            }

            Ok(unique_results)
        })
    }

    #[must_use]
    #[cfg(target_arch = "wasm32")]
    pub fn search_track<'a>(
        &self,
        track_meta: &Track<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchResult>>> + 'a>> {
        if self.providers.is_empty() {
            return Box::pin(async { Err(LyricsHelperError::ProvidersNotInitialized) });
        }

        let providers = self.providers.clone();
        let track_meta = track_meta.clone();

        Box::pin(async move {
            let search_futures = providers
                .iter()
                .map(|provider| search::search_track(provider.as_ref(), &track_meta, true));

            let all_results: Vec<SearchResult> = future::join_all(search_futures)
                .await
                .into_iter()
                .filter_map(Result::ok)
                .flatten()
                .collect();

            let mut sorted_results = all_results;
            sorted_results.sort_by(|a, b| b.match_type.cmp(&a.match_type));

            let mut unique_results = Vec::new();
            let mut seen_keys = HashSet::new();

            for result in sorted_results {
                let key = (result.provider_name.clone(), result.provider_id.clone());
                if seen_keys.insert(key) {
                    unique_results.push(result);
                }
            }

            Ok(unique_results)
        })
    }

    /// 根据提供商名称和歌曲 ID 获取歌词。
    ///
    /// 这些参数通常来自于 `search_track` 方法返回的 `SearchResult` 对象。
    ///
    /// # 参数
    /// * `provider_name` - 提供商的唯一名称, 例如 "qq" 或 "netease"。
    /// * `song_id` - 在该提供商平台上的歌曲ID。
    ///
    /// # 返回
    /// `Result<ParsedSourceData>` - 成功时返回已解析和合并好的歌词数据。
    #[cfg(not(target_arch = "wasm32"))]
    pub fn get_full_lyrics<'a>(
        &self,
        provider_name: &'a str,
        song_id: &'a str,
    ) -> Result<Pin<Box<dyn Future<Output = Result<FullLyricsResult>> + Send + 'a>>> {
        if self.providers.is_empty() {
            return Err(LyricsHelperError::ProvidersNotInitialized);
        }

        let provider = self
            .providers
            .iter()
            .find(|p| p.name() == provider_name)
            .cloned() // 克隆 Arc
            .ok_or_else(|| LyricsHelperError::ProviderNotSupported(provider_name.to_string()))?;

        let song_id = song_id.to_string();

        Ok(Box::pin(
            async move { provider.get_full_lyrics(&song_id).await },
        ))
    }

    #[cfg(target_arch = "wasm32")]
    pub fn get_full_lyrics<'a>(
        &self,
        provider_name: &'a str,
        song_id: &'a str,
    ) -> Result<Pin<Box<dyn Future<Output = Result<FullLyricsResult>> + 'a>>> {
        if self.providers.is_empty() {
            return Err(LyricsHelperError::ProvidersNotInitialized);
        }

        let provider = self
            .providers
            .iter()
            .find(|p| p.name() == provider_name)
            .cloned()
            .ok_or_else(|| LyricsHelperError::ProviderNotSupported(provider_name.to_string()))?;

        let song_id = song_id.to_string();

        Ok(Box::pin(
            async move { provider.get_full_lyrics(&song_id).await },
        ))
    }

    /// 执行一次完整的、多文件的歌词转换。
    ///
    /// # 参数
    /// * `input` - 包含所有源文件内容和格式的 `ConversionInput`。
    /// * `options` - 控制转换过程和输出格式的 `ConversionOptions`。
    ///
    /// # 返回
    /// `Result<String>` - 成功时返回包含目标格式内容的字符串。
    pub fn convert_lyrics(
        &self,
        input: &ConversionInput,
        options: &ConversionOptions,
    ) -> Result<FullConversionResult> {
        let options = options.clone();
        Ok(converter::convert_single_lyric(input, &options)?)
    }

    /// 从已解析的数据生成歌词，跳过解析步骤。
    pub async fn generate_lyrics_from_parsed<S: BuildHasher + Send + 'static>(
        source_data: ParsedSourceData,
        target_format: LyricFormat,
        options: ConversionOptions,
        user_metadata_overrides: Option<HashMap<String, Vec<String>, S>>,
    ) -> Result<FullConversionResult> {
        tokio::task::spawn_blocking(move || {
            converter::generate_from_parsed(
                source_data,
                target_format,
                &options,
                &user_metadata_overrides,
            )
            .map_err(|e| LyricsHelperError::Parser(e.to_string()))
        })
        .await
        .map_err(|e| LyricsHelperError::Internal(e.to_string()))?
    }

    /// 根据指定的策略搜索并获取歌词。
    ///
    /// # 参数
    /// * `track_meta` - 包含歌曲标题、艺术家等信息的 `Track` 结构体引用。
    /// * `mode` - `SearchMode` 枚举，用于定义搜索策略。
    ///
    /// # 返回
    /// * `Ok(Some(ParsedSourceData))` - 如果成功找到并获取了歌词。
    /// * `Ok(None)` - 如果按照指定策略搜索后，未找到任何可用的歌词。
    /// * `Err(LyricsHelperError)` - 如果在过程中发生不可恢复的错误。
    pub fn search_lyrics<'a>(
        &self,
        track_meta: &Track<'a>,
        mode: SearchMode,
    ) -> Result<SearchLyricsFuture<'a>> {
        if self.providers.is_empty() {
            return Err(LyricsHelperError::ProvidersNotInitialized);
        }

        let providers_to_search = get_providers_for_mode(&self.providers, &mode)?;
        if providers_to_search.is_empty() {
            return Ok(Box::pin(async { Ok(None) }));
        }

        tracing::info!(
            "使用 [{:?}] 模式在 {} 个提供商中准备搜索任务...",
            mode,
            providers_to_search.len()
        );

        let track_meta = track_meta.clone();

        Ok(Box::pin(async move {
            match mode {
                SearchMode::Ordered | SearchMode::Specific(_) => {
                    search_ordered(&providers_to_search, &track_meta).await
                }
                SearchMode::Parallel | SearchMode::Subset(_) => {
                    search_lyrics_parallel(&providers_to_search, &track_meta).await
                }
            }
        }))
    }

    /// 搜索歌词并返回完整的搜索结果，包括所有候选项。
    ///
    /// # 参数
    /// * `track_meta` - 要搜索的歌曲元数据。
    /// * `mode` - 搜索模式。
    ///
    /// # 返回
    /// * `Ok(Some(ComprehensiveSearchResult))` - 如果成功找到歌词，包含歌词和所有候选项。
    /// * `Ok(None)` - 如果未找到任何歌词。
    /// * `Err(LyricsHelperError)` - 如果发生错误。
    pub fn search_lyrics_comprehensive<'a>(
        &self,
        track_meta: &Track<'a>,
        mode: &SearchMode,
    ) -> Result<SearchLyricsComprehensiveFuture<'a>> {
        if self.providers.is_empty() {
            return Err(LyricsHelperError::ProvidersNotInitialized);
        }

        let providers_to_search = get_providers_for_mode(&self.providers, mode)?;
        if providers_to_search.is_empty() {
            return Ok(Box::pin(async { Ok(None) }));
        }

        tracing::info!(
            "使用 [{:?}] 模式在 {} 个提供商中准备全面搜索任务...",
            mode,
            providers_to_search.len()
        );

        let track_meta = track_meta.clone();

        Ok(Box::pin(async move {
            search_comprehensive_unified(&providers_to_search, &track_meta).await
        }))
    }

    /// 获取最佳的封面。
    ///
    /// # 参数
    /// * `candidates` - 搜索结果候选项列表
    ///
    /// # 返回
    /// * `Some(Vec<u8>)` - 成功获取到的封面数据
    /// * `None` - 所有候选项都无法提供封面
    #[must_use]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn get_best_cover<'a>(
        &'a self,
        candidates: &'a [SearchResult],
    ) -> Pin<Box<dyn Future<Output = Option<Vec<u8>>> + Send + 'a>> {
        let providers_map: HashMap<_, _> = self
            .providers
            .iter()
            .map(|p| (p.name(), p.clone()))
            .collect();

        let candidates = candidates.to_vec();

        Box::pin(async move {
            let candidates_to_try = candidates.iter().take(5);

            for candidate in candidates_to_try {
                if let Some(album_id) = &candidate.album_id
                    && let Some(provider) = providers_map.get(candidate.provider_name.as_str())
                {
                    tracing::debug!(
                        "尝试从 '{}' 获取专辑 '{}' 的封面",
                        provider.name(),
                        album_id
                    );

                    match provider
                        .get_album_cover_url(album_id, CoverSize::Large)
                        .await
                    {
                        Ok(url) => match self.http_client.get(&url).await {
                            Ok(response) => {
                                if response.status < 300 {
                                    tracing::info!(
                                        "从 '{}' 的 '{}' 成功获取到封面",
                                        provider.name(),
                                        candidate.title
                                    );
                                    return Some(response.body);
                                }
                                tracing::debug!("封面请求返回状态码: {}", response.status);
                            }
                            Err(e) => {
                                tracing::debug!("请求封面URL失败: {}", e);
                            }
                        },
                        Err(e) => {
                            tracing::debug!("获取封面URL失败: {}", e);
                        }
                    }
                }
            }

            tracing::info!("遍历完所有候选项都无法获取到封面");
            None
        })
    }

    #[must_use]
    #[cfg(target_arch = "wasm32")]
    pub fn get_best_cover<'a>(
        &'a self,
        candidates: &'a [SearchResult],
    ) -> Pin<Box<dyn Future<Output = Option<Vec<u8>>> + 'a>> {
        let providers_map: HashMap<_, _> = self
            .providers
            .iter()
            .map(|p| (p.name(), p.clone()))
            .collect();

        let candidates = candidates.to_vec();

        Box::pin(async move {
            let candidates_to_try = candidates.iter().take(5);

            for candidate in candidates_to_try {
                if let Some(album_id) = &candidate.album_id
                    && let Some(provider) = providers_map.get(candidate.provider_name.as_str())
                {
                    tracing::debug!(
                        "尝试从 '{}' 获取专辑 '{}' 的封面",
                        provider.name(),
                        album_id
                    );

                    match provider
                        .get_album_cover_url(album_id, CoverSize::Large)
                        .await
                    {
                        Ok(url) => match self.http_client.get(&url).await {
                            Ok(response) => {
                                if response.status < 300 {
                                    tracing::info!(
                                        "从 '{}' 的 '{}' 成功获取到封面",
                                        provider.name(),
                                        candidate.title
                                    );
                                    return Some(response.body);
                                }
                                tracing::debug!("封面请求返回状态码: {}", response.status);
                            }
                            Err(e) => {
                                tracing::debug!("请求封面URL失败: {}", e);
                            }
                        },

                        Err(e) => {
                            tracing::debug!("获取封面URL失败: {}", e);
                        }
                    }
                }
            }

            tracing::info!("遍历完所有候选项都无法获取到封面");
            None
        })
    }
}

/// 根据 `SearchMode` 筛选出要使用的提供商列表。
fn get_providers_for_mode(
    all_providers: &[Arc<dyn Provider + Send + Sync>],
    mode: &SearchMode,
) -> Result<Vec<Arc<dyn Provider + Send + Sync>>> {
    Ok(match mode {
        SearchMode::Ordered | SearchMode::Parallel => all_providers.to_vec(),
        SearchMode::Specific(provider_name) => {
            let name_str = provider_name.as_str();
            if let Some(provider) = all_providers.iter().find(|p| p.name() == name_str) {
                vec![provider.clone()]
            } else {
                return Err(LyricsHelperError::ProviderNotSupported(
                    name_str.to_string(),
                ));
            }
        }
        SearchMode::Subset(provider_names) => {
            let name_strs: HashSet<_> = provider_names.iter().map(ProviderName::as_str).collect();
            let selected_providers: Vec<_> = all_providers
                .iter()
                .filter(|p| name_strs.contains(p.name()))
                .cloned()
                .collect();

            if selected_providers.is_empty() && !provider_names.is_empty() {
                tracing::warn!("在 Subset 模式下，没有找到任何一个指定的、已初始化的提供商。");
            }
            selected_providers
        }
    })
}

/// 对单个提供商执行搜索并获取操作。
async fn search_and_fetch_from_provider(
    provider: &Arc<dyn Provider + Send + Sync>,
    track_meta: &Track<'_>,
) -> Result<Option<LyricsAndMetadata>> {
    let search_results = search::search_track(provider.as_ref(), track_meta, true).await?;
    if let Some(best_match) = search_results.first() {
        tracing::info!(
            "在提供商 '{}' 中找到匹配项: '{}' (ID: {}), 正在尝试获取歌词...",
            provider.name(),
            best_match.title,
            best_match.provider_id
        );
        return match provider.get_full_lyrics(&best_match.provider_id).await {
            Ok(lyrics_data) => Ok(Some(LyricsAndMetadata {
                lyrics: lyrics_data,
                source_track: best_match.clone(),
            })),
            Err(LyricsHelperError::LyricNotFound) => {
                tracing::info!(
                    "找到歌曲 '{}'，但提供商 '{}' 没有提供歌词。",
                    best_match.title,
                    provider.name()
                );
                Ok(None)
            }
            Err(e) => {
                tracing::warn!(
                    "从提供商 '{}' 获取歌曲 ID '{}' 的歌词时失败: {}",
                    provider.name(),
                    best_match.provider_id,
                    e
                );
                Err(e)
            }
        };
    }
    Ok(None)
}

/// 在提供商上按顺序执行搜索和获取。
async fn search_ordered(
    providers: &[Arc<dyn Provider + Send + Sync>],
    track_meta: &Track<'_>,
) -> Result<Option<LyricsAndMetadata>> {
    for provider in providers {
        tracing::debug!("正在尝试提供商: '{}'", provider.name());
        match search_and_fetch_from_provider(provider, track_meta).await {
            Ok(Some(lyrics_result)) => {
                tracing::info!("在 '{}' 成功获取到歌词，搜索结束。", provider.name());
                return Ok(Some(lyrics_result));
            }
            Ok(None) => {}
            Err(e) => return Err(e), // 发生网络错误等
        }
    }
    tracing::info!("所有指定提供商都未能找到歌词。");
    Ok(None)
}

async fn search_lyrics_parallel(
    providers: &[Arc<dyn Provider + Send + Sync>],
    track_meta: &Track<'_>,
) -> Result<Option<LyricsAndMetadata>> {
    let search_futures = providers
        .iter()
        .map(|provider| search::search_track(provider.as_ref(), track_meta, true));

    let all_results: Vec<SearchResult> = future::join_all(search_futures)
        .await
        .into_iter()
        .filter_map(Result::ok)
        .flatten()
        .collect();
    let mut sorted_results = all_results;
    sorted_results.sort_by(|a, b| b.match_type.cmp(&a.match_type));

    if let Some(best_match) = sorted_results.first() {
        tracing::info!(
            "并行搜索完成。最佳匹配项来自 '{}': '{}' (ID: {}), 正在获取歌词...",
            best_match.provider_name,
            best_match.title,
            best_match.provider_id
        );

        if let Some(provider) = providers
            .iter()
            .find(|p| p.name() == best_match.provider_name)
        {
            match provider.get_full_lyrics(&best_match.provider_id).await {
                Ok(lyrics_data) => Ok(Some(LyricsAndMetadata {
                    lyrics: lyrics_data,
                    source_track: best_match.clone(),
                })),
                Err(LyricsHelperError::LyricNotFound) => {
                    tracing::info!("最佳匹配项 '{}' 无歌词。", best_match.title);
                    Ok(None)
                }
                Err(e) => Err(e),
            }
        } else {
            Err(LyricsHelperError::ProviderNotSupported(
                best_match.provider_name.clone(),
            ))
        }
    } else {
        tracing::info!("并行搜索未找到任何结果。");
        Ok(None)
    }
}

async fn search_comprehensive_unified(
    providers: &[Arc<dyn Provider + Send + Sync>],
    track_meta: &Track<'_>,
) -> Result<Option<ComprehensiveSearchResult>> {
    let search_futures = providers
        .iter()
        .map(|provider| search::search_track(provider.as_ref(), track_meta, true));

    let all_candidates: Vec<SearchResult> = future::join_all(search_futures)
        .await
        .into_iter()
        .filter_map(Result::ok)
        .flatten()
        .collect();

    let mut sorted_candidates = all_candidates;
    sorted_candidates.sort_by(|a, b| b.match_type.cmp(&a.match_type));

    if sorted_candidates.is_empty() {
        tracing::info!("所有提供商都未找到任何搜索结果。");
        return Ok(None);
    }

    for candidate in &sorted_candidates {
        tracing::debug!(
            "尝试从 '{}' 获取歌词: '{}' (ID: {})",
            candidate.provider_name,
            candidate.title,
            candidate.provider_id
        );

        if let Some(provider) = providers
            .iter()
            .find(|p| p.name() == candidate.provider_name)
        {
            match provider.get_full_lyrics(&candidate.provider_id).await {
                Ok(lyrics_data) => {
                    tracing::info!(
                        "成功获取到歌词。最佳匹配项来自 '{}': '{}'",
                        candidate.provider_name,
                        candidate.title
                    );

                    return Ok(Some(ComprehensiveSearchResult {
                        primary_lyric_result: LyricsAndMetadata {
                            lyrics: lyrics_data,
                            source_track: candidate.clone(),
                        },
                        all_search_candidates: sorted_candidates,
                    }));
                }
                Err(LyricsHelperError::LyricNotFound) => {
                    tracing::info!("候选项 '{}' 无歌词，尝试下一个。", candidate.title);
                }
                Err(e) => {
                    tracing::warn!(
                        "从 '{}' 获取歌词失败: {}，尝试下一个。",
                        candidate.provider_name,
                        e
                    );
                }
            }
        }
    }

    tracing::info!("所有候选项都无法提供歌词。");
    Ok(None)
}

#[cfg(test)]
mod integration_tests {
    use lyrics_helper_core::{MetadataStore, TtmlGenerationOptionsBuilder};

    use super::*;
    use ttml_processor::generate_ttml;

    fn init_tracing() {
        use tracing_subscriber::{EnvFilter, FmtSubscriber};
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,lyrics_helper_rs=debug"));
        let _ = FmtSubscriber::builder()
            .with_env_filter(filter)
            .with_test_writer()
            .try_init();
    }

    /// 一个完整的端到端测试用例：
    /// 1. 使用网易云音乐搜索一首包含翻译和罗马音的歌曲。
    /// 2. 获取并打印已合并的歌词数据。
    /// 3. 将解析出的数据转换为 TTML 格式。
    #[tokio::test]
    #[ignore]
    async fn test_netease_full_flow() {
        init_tracing();

        let mut helper = LyricsHelper::new();
        helper.load_providers().await.unwrap();

        let track_to_search = Track {
            title: Some("インドア系ならトラックメイカー"),
            artists: Some(&["Yunomi", "nicamoq"]),
            album: Some("インドア系ならトラックメイカー"),
            duration: None,
        };

        let search_results = helper
            .search_track(&track_to_search)
            .await
            .expect("搜索歌曲失败");

        let netease_match = search_results
            .iter()
            .find(|r| r.provider_name == "netease")
            .expect("在搜索结果中未找到来自网易云音乐的匹配项");

        println!(
            "[INFO] 找到目标匹配: '{}' (Provider: {}, ID: {})",
            netease_match.title, netease_match.provider_name, netease_match.provider_id
        );
        assert_eq!(netease_match.provider_name, "netease");

        let full_lyrics_result = helper
            .get_full_lyrics(&netease_match.provider_name, &netease_match.provider_id)
            .expect("创建获取歌词任务失败")
            .await
            .expect("获取歌词失败");

        let parsed_data = full_lyrics_result.parsed;

        let has_main_lyric = !parsed_data.lines.is_empty();
        let has_translation = parsed_data.lines.iter().any(|l| {
            l.main_track()
                .is_some_and(|track| !track.translations.is_empty())
        });
        let has_romanization = parsed_data.lines.iter().any(|l| {
            l.main_track()
                .is_some_and(|track| !track.romanizations.is_empty())
        });

        assert!(has_main_lyric, "解析后的主歌词行不应为空");
        assert!(
            has_translation || has_romanization,
            "获取到的歌词既没有翻译也没有罗马音"
        );

        for line in parsed_data.lines.iter().take(5) {
            let main_text = line.main_text().unwrap_or_default();
            let translation_text = line
                .main_track()
                .and_then(|track| track.translations.first())
                .map_or("N/A".to_string(), |t| {
                    t.words
                        .iter()
                        .flat_map(|w| &w.syllables)
                        .map(|s| s.text.as_str())
                        .collect::<Vec<_>>()
                        .join("")
                });

            let romanization_text = line
                .main_track()
                .and_then(|track| track.romanizations.first())
                .map_or("N/A".to_string(), |t| {
                    t.words
                        .iter()
                        .flat_map(|w| &w.syllables)
                        .map(|s| s.text.as_str())
                        .collect::<Vec<_>>()
                        .join("")
                });

            println!(
                "  - 主歌词: {main_text}\n    翻译: {translation_text}\n    罗马音: {romanization_text}"
            );
        }

        println!("\n[INFO] 步骤 3: 将解析出的歌词数据转换为 TTML 格式...");
        let metadata_store = MetadataStore::from(&parsed_data);

        let agent_store = MetadataStore::to_agent_store(&metadata_store);

        let ttml_options = TtmlGenerationOptionsBuilder::default()
            .format(false)
            .build()
            .unwrap();

        let ttml_output = generate_ttml(
            &parsed_data.lines,
            &metadata_store,
            &agent_store,
            &ttml_options,
        )
        .expect("生成 TTML 失败");

        assert!(ttml_output.starts_with("<tt"), "输出应为有效的 TTML 字符串");

        // println!("{}", ttml_output);
    }

    #[tokio::test]
    #[ignore]
    async fn test_search_lyrics_ordered_mode() -> Result<()> {
        init_tracing();
        let mut helper = LyricsHelper::new();
        helper.load_providers().await?;

        let track_to_search = Track {
            title: Some("风来的夏天"),
            artists: Some(&["小蓝背心"]),
            album: None,
            duration: None,
        };

        let result = helper
            .search_lyrics(&track_to_search, SearchMode::Ordered)?
            .await?;

        assert!(result.is_some(), "顺序搜索应找到至少一个结果");
        let lyrics_data = result.unwrap();
        assert!(
            !lyrics_data.lyrics.parsed.lines.is_empty(),
            "获取到的歌词不应为空"
        );
        println!("成功获取到歌词！");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_search_lyrics_parallel_mode() -> Result<()> {
        init_tracing();
        let mut helper = LyricsHelper::new();
        helper.load_providers().await?;

        let track_to_search = Track {
            title: Some("人海经过"),
            artists: Some(&["小蓝背心"]),
            album: None,
            duration: None,
        };

        let result = helper
            .search_lyrics(&track_to_search, SearchMode::Parallel)?
            .await?;

        assert!(result.is_some(), "并行搜索应找到至少一个结果");
        let lyrics_data = result.unwrap();
        assert!(
            !lyrics_data.lyrics.parsed.lines.is_empty(),
            "获取到的歌词不应为空"
        );
        println!("成功获取到歌词！");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_search_lyrics_specific_provider_mode() -> Result<()> {
        init_tracing();
        let mut helper = LyricsHelper::new();
        helper.load_providers().await?;

        let track_to_search = Track {
            title: Some("星夏"),
            artists: Some(&["小蓝背心"]),
            album: Some("星夏"),
            duration: None,
        };

        let provider_to_test = ProviderName::AmllTtmlDatabase;

        println!(
            "\n[INFO] 正在测试 [Specific] 模式 (提供商: {})",
            provider_to_test.as_str()
        );
        let result = helper
            .search_lyrics(&track_to_search, SearchMode::Specific(provider_to_test))?
            .await?;

        assert!(result.is_some(), "在 AMLL TTML DB 中应能找到《星夏》");
        let lyrics_data = result.unwrap();
        assert!(
            !lyrics_data.lyrics.parsed.lines.is_empty(),
            "获取到的歌词不应为空"
        );
        println!("成功获取到歌词！");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_search_lyrics_subset_mode() -> Result<()> {
        init_tracing();
        let mut helper = LyricsHelper::new();
        helper.load_providers().await?;

        let track_to_search = Track {
            title: Some("风来的夏天"),
            artists: Some(&["小蓝背心"]),
            album: None,
            duration: None,
        };

        let providers_to_test = vec![ProviderName::Netease, ProviderName::QQMusic];

        println!(
            "\n[INFO] 正在测试 [Subset] 模式 (提供商: {:?})",
            providers_to_test
                .iter()
                .map(ProviderName::as_str)
                .collect::<Vec<_>>()
        );
        let result = helper
            .search_lyrics(&track_to_search, SearchMode::Subset(providers_to_test))?
            .await?;

        assert!(result.is_some(), "在指定的提供商子集中应能找到歌词");
        let lyrics_data = result.unwrap();
        assert!(
            !lyrics_data.lyrics.parsed.lines.is_empty(),
            "获取到的歌词不应为空"
        );
        println!("成功获取到歌词！");
        Ok(())
    }
}
