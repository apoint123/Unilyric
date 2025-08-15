//! QQ音乐提供商模块。
//!
//! 该模块提供了与 QQ 音乐 API 交互的各种功能，
//! 包括搜索、获取歌词、歌曲、专辑、歌手和播放列表信息。
//! API 来源于 <https://github.com/luren-dc/QQMusicApi>

use std::{sync::LazyLock, time::Duration};

use async_trait::async_trait;
use base64::{Engine, prelude::BASE64_STANDARD};
use chrono::{Datelike, Local};
use fancy_regex::Regex;

use lyrics_helper_core::{
    Artist, ConversionInput, ConversionOptions, CoverSize, FullLyricsResult, InputFile, Language,
    LyricFormat, ParsedSourceData, RawLyrics, SearchResult, Track, model::generic,
};
use reqwest::Client;
use serde_json::json;
use tracing::{info, trace, warn};
use uuid::Uuid;

use crate::{
    config::{load_cached_config, save_cached_config},
    converter,
    error::{LyricsHelperError, Result},
    providers::{Provider, qq::models::QQMusicCoverSize},
};

pub mod device;
pub mod models;
pub mod qimei;
pub mod qrc_codec;

const MUSIC_U_FCG_URL: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";

const SEARCH_MODULE: &str = "music.search.SearchCgiService";
const SEARCH_METHOD: &str = "DoSearchForQQMusicMobile";

const GET_ALBUM_SONGS_MODULE: &str = "music.musichallAlbum.AlbumSongList";
const GET_ALBUM_SONGS_METHOD: &str = "GetAlbumSongList";

const GET_SINGER_SONGS_MODULE: &str = "musichall.song_list_server";
const GET_SINGER_SONGS_METHOD: &str = "GetSingerSongList";

const GET_LYRIC_MODULE: &str = "music.musichallSong.PlayLyricInfo";
const GET_LYRIC_METHOD: &str = "GetPlayLyricInfo";

const GET_SONG_URL_MODULE: &str = "music.vkey.GetVkey";
const GET_SONG_URL_METHOD: &str = "UrlGetVkey";

const GET_ALBUM_DETAIL_MODULE: &str = "music.musichallAlbum.AlbumInfoServer";
const GET_ALBUM_DETAIL_METHOD: &str = "GetAlbumDetail";

const GET_SONG_DETAIL_MODULE: &str = "music.pf_song_detail_svr";
const GET_SONG_DETAIL_METHOD: &str = "get_song_detail_yqq";

const GET_PLAYLIST_DETAIL_MODULE: &str = "music.srfDissInfo.DissInfo";
const GET_PLAYLIST_DETAIL_METHOD: &str = "CgiGetDiss";

static QRC_LYRIC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"LyricContent="([^"]*)""#).unwrap());

static AMP_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&(?![a-zA-Z]{2,6};|#[0-9]{2,4};)").unwrap());

static QUOT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?P<attr>\s+[\w:.-]+\s*=\s*")(?P<value>(?:[^"]|"(?!\s+[\w:.-]+\s*=\s*"|\s*(?:/?|\?)>))*)"#)
        .unwrap()
});

static YUE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-zA-Z]+[1-6]").unwrap());

/// QQ 音乐的提供商实现。
pub struct QQMusic {
    http_client: Client,
    qimei: String,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[async_trait]
impl Provider for QQMusic {
    fn name(&self) -> &'static str {
        "qq"
    }

    /// 根据歌曲信息在 QQ 音乐上搜索歌曲。
    ///
    /// # 参数
    ///
    /// * 一个 `track`，包含歌曲标题和艺术家信息的 `Track` 引用。
    ///
    /// # 返回
    ///
    /// * 一个 `Result`，其中包含一个 `Vec<SearchResult>`，每个 `SearchResult` 代表一首匹配的歌曲。
    ///
    async fn search_songs(&self, track: &Track<'_>) -> Result<Vec<SearchResult>> {
        let keyword = format!(
            "{} {}",
            track.title.unwrap_or(""),
            track.artists.unwrap_or(&[]).join(" ")
        )
        .trim()
        .to_string();

        let param = json!({
            "num_per_page": 20,
            "page_num": 1,
            "query": keyword,
            "search_type": 0,
            "grp": 1,
            "highlight": 1
        });

        let response_val = self
            .execute_api_request(SEARCH_MODULE, SEARCH_METHOD, param, &[0])
            .await?;

        let search_response: models::Req1 = serde_json::from_value(response_val)?;

        if let Some(data) = search_response.data
            && let Some(body) = data.body
        {
            let song_list = body.item_song;

            let process_song = |s: &models::Song| -> Option<SearchResult> {
                let mut search_result = SearchResult::from(s);
                search_result.provider_id.clone_from(&s.mid);
                search_result.provider_name = self.name().to_string();
                Some(search_result)
            };

            let search_results: Vec<SearchResult> = song_list
                .iter()
                .flat_map(|song| {
                    let main_result = process_song(song);
                    let group_results = song
                        .group
                        .as_ref()
                        .map_or_else(Vec::new, |g| g.iter().filter_map(process_song).collect());
                    main_result.into_iter().chain(group_results)
                })
                .collect();

            return Ok(search_results);
        }

        Ok(vec![])
    }

    async fn get_full_lyrics(&self, song_id: &str) -> Result<FullLyricsResult> {
        self.try_get_lyrics_internal(song_id).await
    }

    /// 根据专辑 MID 获取专辑的详细信息。
    ///
    /// 注意：这个 API 不包含歌曲列表或歌曲总数，
    /// 这些信息需要通过 `get_album_songs` 接口获取。
    ///
    /// # 参数
    ///
    /// * `album_mid`，专辑的 `mid` 字符串。
    ///
    /// # 返回
    ///
    /// 一个 `Result`，其中包含一个 `generic::Album` 结构。
    ///
    async fn get_album_info(&self, album_mid: &str) -> Result<generic::Album> {
        let param = json!({
            "albumMId": album_mid
        });

        let response_val = self
            .execute_api_request(
                GET_ALBUM_DETAIL_MODULE,
                GET_ALBUM_DETAIL_METHOD,
                param,
                &[0],
            )
            .await?;

        let result_container: models::AlbumDetailApiResult = serde_json::from_value(response_val)?;

        let qq_album_info = result_container.data;

        Ok(qq_album_info.into())
    }

    /// 分页获取指定专辑的歌曲列表。
    ///
    /// # 参数
    ///
    /// * `album_mid` — 专辑的 `mid`。
    /// * `page` — 页码（从 1 开始）。
    /// * `page_size` — 每页的歌曲数量。
    ///
    /// # 返回
    ///
    /// 一个 `Result`，其中包含一个 `Vec<generic::Song>`。
    ///
    async fn get_album_songs(
        &self,
        album_mid: &str,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<generic::Song>> {
        let param = json!({
            "albumMid": album_mid,
            "albumID": 0,
            "begin": (page.saturating_sub(1)) * page_size,
            "num": page_size,
            "order": 2
        });

        let response_val = self
            .execute_api_request(GET_ALBUM_SONGS_MODULE, GET_ALBUM_SONGS_METHOD, param, &[0])
            .await?;

        let album_song_list: models::AlbumSonglistInfo = serde_json::from_value(response_val)?;

        let songs = album_song_list
            .data
            .song_list
            .into_iter()
            .map(|item| generic::Song::from(item.song_info))
            .collect();

        Ok(songs)
    }

    /// 分页获取指定歌手的热门歌曲。
    ///
    /// # 参数
    ///
    /// * `singer_mid` — 歌手的 `mid`。
    /// * `page` — 页码（从 1 开始）。
    /// * `page_size` — 每页的歌曲数量。
    ///
    /// # 返回
    ///
    /// 一个 `Result`，其中包含一个 `Vec<generic::Song>`。
    ///
    async fn get_singer_songs(
        &self,
        singer_mid: &str,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<generic::Song>> {
        let begin = page.saturating_sub(1) * page_size;
        let number = page_size;

        let param = json!({
            "singerMid": singer_mid,
            "order": 1,
            "number": number,
            "begin": begin,
        });

        let response_val = self
            .execute_api_request(
                GET_SINGER_SONGS_MODULE,
                GET_SINGER_SONGS_METHOD,
                param,
                &[0],
            )
            .await?;

        let result_container: models::SingerSongListApiResult =
            serde_json::from_value(response_val)?;

        let songs = result_container
            .data
            .song_list
            .into_iter()
            .take(page_size as usize)
            .map(|item| generic::Song::from(item.song_info))
            .collect();

        Ok(songs)
    }

    /// 根据歌单 ID 获取歌单的详细信息和歌曲列表。
    ///
    /// # 参数
    ///
    /// * `playlist_id` — 歌单的 ID (disstid)。
    ///
    /// # 返回
    ///
    /// 一个 `Result`，其中包含一个通用的 `generic::Playlist` 结构。
    ///
    async fn get_playlist(&self, playlist_id: &str) -> Result<generic::Playlist> {
        let disstid = playlist_id.parse::<u64>().map_err(|_| {
            LyricsHelperError::ApiError(format!(
                "无效的播放列表 ID: '{playlist_id}'，必须是纯数字。"
            ))
        })?;

        let param = json!({
            "disstid": disstid,
            "song_begin": 0,
            "song_num": 300,
            "userinfo": true,
            "tag": true,
        });

        let response_val = self
            .execute_api_request(
                GET_PLAYLIST_DETAIL_MODULE,
                GET_PLAYLIST_DETAIL_METHOD,
                param,
                &[0],
            )
            .await?;

        let result_container: models::PlaylistApiResult = serde_json::from_value(response_val)?;
        let playlist_data = result_container.data;

        Ok(playlist_data.into())
    }

    /// 根据歌曲 ID 或 MID 获取单首歌曲的详细信息。
    ///
    /// # 参数
    ///
    /// * `song_id` — 歌曲的数字 ID 或 `mid` 字符串。
    ///
    /// # 返回
    ///
    /// 一个 `Result`，其中包含一个通用的 `generic::Song` 结构。
    ///
    async fn get_song_info(&self, song_id: &str) -> Result<generic::Song> {
        let param = if let Ok(id) = song_id.parse::<u64>() {
            json!({ "song_id": id })
        } else {
            json!({ "song_mid": song_id })
        };

        let response_val = self
            .execute_api_request(GET_SONG_DETAIL_MODULE, GET_SONG_DETAIL_METHOD, param, &[0])
            .await?;

        let result_container: models::SongDetailApiContainer =
            serde_json::from_value(response_val)?;

        let qq_song_info = result_container.data.track_info;

        let cover_url = if let Some(album_mid) = qq_song_info.album.mid.as_deref() {
            self.get_album_cover_url(album_mid, CoverSize::Large)
                .await
                .ok()
        } else {
            None
        };

        let mut generic_song: generic::Song = qq_song_info.into();
        generic_song.cover_url = cover_url;

        Ok(generic_song)
    }

    ///
    /// 根据歌曲 MID 获取歌曲的播放链接。
    ///
    /// # 注意
    ///
    /// 无法获取 VIP 歌曲或需要付费的歌曲的链接，会返回错误。
    ///
    /// # 参数
    ///
    /// * `song_mid` — 歌曲的 `mid`。
    ///
    /// # 返回
    ///
    /// 一个 `Result`，其中包含一个表示可播放 URL 的 `String`。
    ///
    async fn get_song_link(&self, song_mid: &str) -> Result<String> {
        let mids_slice = [song_mid];

        let url_map = self
            .get_song_urls_internal(&mids_slice, models::SongFileType::Mp3_128)
            .await?;

        url_map.get(song_mid).cloned().ok_or_else(|| {
            LyricsHelperError::ApiError(format!("未找到 song_mid '{song_mid}' 的播放链接"))
        })
    }

    async fn get_album_cover_url(&self, album_id: &str, size: CoverSize) -> Result<String> {
        let qq_size = match size {
            CoverSize::Thumbnail => QQMusicCoverSize::Size150,
            CoverSize::Medium => QQMusicCoverSize::Size300,
            CoverSize::Large => QQMusicCoverSize::Size800,
        };

        let cover_url = get_qq_album_cover_url(album_id, qq_size);

        if album_id.is_empty() {
            Err(LyricsHelperError::ApiError("专辑 ID 不能为空".into()))
        } else {
            Ok(cover_url)
        }
    }
}

impl QQMusic {
    ///
    /// 创建一个新的 `QQMusic` 提供商实例。
    ///
    /// # 返回
    ///
    /// 一个 `Result`，成功时包含 `QQMusic` 的实例。
    ///
    pub async fn new() -> Result<Self> {
        const CACHE_FILENAME: &str = "qq_device.json";
        let http_client = Client::builder().timeout(Duration::from_secs(10)).build()?;

        let device = if let Ok(config) = load_cached_config::<device::Device>(CACHE_FILENAME) {
            info!("已从缓存加载 QQ Device。");
            config.data
        } else {
            info!("QQ Device 配置文件不存在或无效，将创建并保存一个新设备。");
            let new_device = device::Device::new();
            if let Err(e) = save_cached_config(CACHE_FILENAME, &new_device) {
                warn!("保存新的 QQ Device 失败: {}", e);
            }
            new_device
        };

        let api_version = "13.2.5.8";
        let qimei_result = qimei::get_qimei(&device, api_version)
            .await
            .map_err(|e| LyricsHelperError::ApiError(format!("获取 Qimei 失败: {e}")))?;

        Ok(Self {
            http_client,
            qimei: qimei_result.q36,
        })
    }

    fn build_comm(&self) -> serde_json::Value {
        json!({
            "cv": 13_020_508,
            "ct": 11,
            "v": 13_020_508,
            "QIMEI36": &self.qimei,
            "tmeAppID": "qqmusic",
            "inCharset": "utf-8",
            "outCharset": "utf-8"
        })
    }

    async fn execute_api_request(
        &self,
        module: &str,
        method: &str,
        param: serde_json::Value,
        expected_codes: &[i32],
    ) -> Result<serde_json::Value> {
        let url = MUSIC_U_FCG_URL;
        let request_key = format!("{module}.{method}");

        let payload = json!({
            "comm": self.build_comm(),
            &request_key: {
                "module": module,
                "method": method,
                "param": param,
            }
        });

        let response_text = self
            .http_client
            .post(url)
            .json(&payload)
            .send()
            .await?
            .text()
            .await?;

        trace!("原始 JSON 响应 {request_key}: {response_text}");

        let mut response_value: serde_json::Value = serde_json::from_str(&response_text)?;

        if let Some(business_object) = response_value
            .get_mut(&request_key)
            .map(serde_json::Value::take)
        {
            let business_code: models::BusinessCode =
                serde_json::from_value(business_object.clone())?;

            if expected_codes.contains(&business_code.code) {
                Ok(business_object)
            } else {
                Err(LyricsHelperError::ApiError(format!(
                    "QQ 音乐 API 业务错误 ({}): code = {}",
                    request_key, business_code.code
                )))
            }
        } else {
            Err(LyricsHelperError::Parser(format!(
                "响应中未找到键: '{request_key}'"
            )))
        }
    }

    ///
    /// 获取指定排行榜的歌曲列表。
    ///
    /// # 参数
    ///
    /// * `top_id` — 排行榜的 ID。
    /// * `page` — 页码。
    /// * `page_size` — 每页数量。
    /// * `period` — 周期，例如 "2023-10-27"。如果为 `None`，会自动生成默认值。
    ///
    /// # 返回
    ///
    /// 一个元组，包含排行榜信息和歌曲列表。
    ///
    pub async fn get_toplist(
        &self,
        top_id: u32,
        page: u32,
        page_size: u32,
        period: Option<String>,
    ) -> Result<(models::ToplistInfo, Vec<models::ToplistSongData>)> {
        // 如果未提供周期，则根据榜单类型生成默认周期
        let final_period = period.unwrap_or_else(|| {
            let now = Local::now();
            match top_id {
                // 日榜
                4 | 27 | 62 => now.format("%Y-%m-%d").to_string(),
                // 周榜
                _ => {
                    // 计算 ISO 周数
                    let week = now.iso_week().week();
                    format!("{}-{}", now.year(), week)
                }
            }
        });

        let param = json!({
            "topId": top_id,
            "offset": (page.saturating_sub(1)) * page_size,
            "num": page_size,
            "period": final_period,
        });

        let response_val = self
            .execute_api_request(
                "musicToplist.ToplistInfoServer",
                "GetDetail",
                param,
                &[2000],
            )
            .await?;

        let detail_data: models::DetailData = serde_json::from_value(response_val)?;

        let info = detail_data.data.info;
        let songs = info.songs.clone();

        Ok((info, songs))
    }

    /// 按类型进行搜索。
    ///
    /// # 参数
    ///
    /// * `keyword` - 要搜索的关键词。
    /// * `search_type` - 搜索的类型，例如 `models::SearchType::Song`。
    /// * `page` - 结果的页码（从1开始）。
    /// * `page_size` - 每页显示的结果数量。
    ///
    /// # 返回
    ///
    /// `Result<Vec<models::TypedSearchResult>>` - 成功时返回一个包含
    /// `models::TypedSearchResult` 枚举向量的 `Ok` 变体，表示不同类型的搜索结果。
    /// 如果发生错误，则返回 `Err` 变体。
    pub async fn search_by_type(
        &self,
        keyword: &str,
        search_type: models::SearchType,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<models::TypedSearchResult>> {
        let param = json!({
            "query": keyword,
            "search_type": search_type.as_u32(),
            "page_num": page,
            "num_per_page": page_size,
            "grp": 1,
            "highlight": 1,
        });

        let response_val = self
            .execute_api_request(SEARCH_MODULE, SEARCH_METHOD, param, &[0])
            .await?;

        let search_response: models::Req1 = serde_json::from_value(response_val)?;

        let mut results = Vec::new();
        if let Some(data) = search_response.data
            && let Some(body) = data.body
        {
            match search_type {
                models::SearchType::Song => {
                    for song in body.item_song {
                        results.push(models::TypedSearchResult::Song(song));
                    }
                }
                models::SearchType::Album => {
                    for album in body.item_album {
                        results.push(models::TypedSearchResult::Album(album));
                    }
                }
                models::SearchType::Singer => {
                    for singer in body.singer {
                        results.push(models::TypedSearchResult::Singer(singer));
                    }
                }
                // TODO: 添加更多分支
                _ => {
                    // 暂时忽略
                }
            }
        }

        Ok(results)
    }

    /// 从解密后的 QRC 歌词文本中提取核心的 `LyricContent` 内容。
    ///
    /// 这个函数会尝试修复文本中不规范的 XML 特殊字符。
    ///
    /// # 参数
    /// * `decrypted_text` - 已经解密的歌词字符串。
    ///
    /// # 返回
    /// * `String` - 提取出的 `LyricContent` 内容，或在某些情况下的原始文本。
    fn extract_from_qrc_wrapper(decrypted_text: &str) -> String {
        if decrypted_text.is_empty() {
            return String::new();
        }

        if !decrypted_text.starts_with("<?xml") {
            return decrypted_text.to_string();
        }

        // 修复独立的 '&' 符号
        let replaced_amp = AMP_RE.replace_all(decrypted_text, "&amp;");

        // 修复未转义的双引号
        let fixed_text = QUOT_RE.replace_all(&replaced_amp, |caps: &fancy_regex::Captures| {
            let attr = &caps["attr"];
            let value = &caps["value"];
            format!("{}{}\"", attr, value.replace('"', "&quot;"))
        });

        if let Ok(Some(caps)) = QRC_LYRIC_RE.captures(&fixed_text)
            && let Some(content) = caps.get(1)
        {
            return content.as_str().to_string();
        }

        decrypted_text.to_string()
    }

    async fn try_get_lyrics_internal(&self, song_id: &str) -> Result<FullLyricsResult> {
        let mut param_map = serde_json::Map::new();
        if song_id.parse::<u64>().is_ok() {
            param_map.insert("songId".to_string(), json!(song_id.parse::<u64>()?));
        } else {
            param_map.insert("songMid".to_string(), json!(song_id));
        }
        param_map.insert("qrc".to_string(), json!(1));
        param_map.insert("trans".to_string(), json!(1));
        param_map.insert("roma".to_string(), json!(1));
        let param = serde_json::Value::Object(param_map);
        let response_val = self
            .execute_api_request(GET_LYRIC_MODULE, GET_LYRIC_METHOD, param, &[0])
            .await?;
        let lyric_result_container: models::LyricApiResult = serde_json::from_value(response_val)?;
        let lyric_resp = lyric_result_container.data;

        let main_lyrics_decrypted = Self::decrypt_with_fallback(&lyric_resp.lyric)?;
        let trans_lyrics_decrypted = Self::decrypt_with_fallback(&lyric_resp.trans)?;
        let roma_lyrics_decrypted = Self::decrypt_with_fallback(&lyric_resp.roma)?;

        let main_lyric_format = if main_lyrics_decrypted.starts_with("<?xml") {
            LyricFormat::Qrc
        } else if main_lyrics_decrypted.trim().starts_with('[')
            && main_lyrics_decrypted.contains(']')
        {
            LyricFormat::Lrc
        } else {
            let mut raw_metadata = std::collections::HashMap::new();
            raw_metadata.insert(
                "introduction".to_string(),
                vec![main_lyrics_decrypted.clone()],
            );
            let parsed_data = ParsedSourceData {
                source_name: "qq".to_string(),
                raw_metadata,
                ..Default::default()
            };
            let raw_lyrics = RawLyrics {
                format: "txt".to_string(),
                content: main_lyrics_decrypted,
                translation: None,
            };
            return Ok(FullLyricsResult {
                parsed: parsed_data,
                raw: raw_lyrics,
            });
        };

        let main_lyrics_content = if main_lyric_format == LyricFormat::Qrc {
            Self::extract_from_qrc_wrapper(&main_lyrics_decrypted)
        } else {
            // 如果是 LRC 或其他格式，解密后的文本就是最终内容
            main_lyrics_decrypted.clone()
        };

        let mut translations = Vec::new();
        if !trans_lyrics_decrypted.is_empty() {
            translations.push(InputFile {
                content: trans_lyrics_decrypted.clone(),
                format: LyricFormat::Lrc,
                language: Some("zh-Hans".to_string()),
                filename: None,
            });
        }

        let romanizations = Self::create_romanization_input(&roma_lyrics_decrypted)
            .into_iter()
            .collect();

        let main_lyric_input = InputFile {
            content: main_lyrics_content.clone(),
            format: main_lyric_format,
            language: None,
            filename: None,
        };

        let conversion_input = ConversionInput {
            main_lyric: main_lyric_input,
            translations,
            romanizations,
            target_format: LyricFormat::Lrc,
            user_metadata_overrides: None,
        };
        let mut parsed_data =
            converter::parse_and_merge(&conversion_input, &ConversionOptions::default())?;
        parsed_data.source_name = "qq".to_string();

        let raw_lyrics = RawLyrics {
            format: main_lyric_format.to_string(),
            content: main_lyrics_content,
            translation: if trans_lyrics_decrypted.is_empty() {
                None
            } else {
                Some(trans_lyrics_decrypted)
            },
        };

        Ok(FullLyricsResult {
            parsed: parsed_data,
            raw: raw_lyrics,
        })
    }

    async fn get_song_urls_internal(
        &self,
        song_mids: &[&str],
        file_type: models::SongFileType,
    ) -> Result<std::collections::HashMap<String, String>> {
        if song_mids.len() > 100 {
            return Err(LyricsHelperError::ApiError(
                "单次请求的歌曲数量不能超过100".to_string(),
            ));
        }

        let (type_code, extension) = file_type.get_parts();

        let filenames: Vec<String> = song_mids
            .iter()
            .map(|mid| format!("{type_code}{mid}{mid}{extension}"))
            .collect();

        let uuid = Self::generate_guid();

        let param = json!({
            "filename": filenames,
            "guid": uuid,
            "songmid": song_mids,
            "songtype": vec![0; song_mids.len()],
        });

        let response_val = self
            .execute_api_request(GET_SONG_URL_MODULE, GET_SONG_URL_METHOD, param, &[0])
            .await?;

        let result_container: models::SongUrlApiResult = serde_json::from_value(response_val)?;

        let result_data = result_container.data;

        let mut url_map = std::collections::HashMap::new();
        let domain = "https://isure.stream.qqmusic.qq.com/";

        for info in result_data.midurlinfo {
            if info.purl.is_empty() {
                return Err(LyricsHelperError::ApiError(format!(
                    "无法获取 songmid '{}' 的链接 (purl 为空)，可能是 VIP 歌曲。",
                    info.songmid
                )));
            }
            url_map.insert(info.songmid, format!("{}{}", domain, info.purl));
        }

        Ok(url_map)
    }

    /// 生成一个随机的 UUID。
    fn generate_guid() -> String {
        let random_uuid = Uuid::new_v4();
        random_uuid.simple().to_string()
    }

    /// 使用备用方案解密歌词数据。
    ///
    /// 优先尝试 Base64 解码，如果失败或结果不是有效的 UTF-8 字符串，
    /// 则回退到使用 DES 解密。
    fn decrypt_with_fallback(encrypted_str: &str) -> Result<String> {
        if let Ok(decoded_bytes) = BASE64_STANDARD.decode(encrypted_str)
            && let Ok(decoded_str) = String::from_utf8(decoded_bytes)
        {
            info!("成功使用 Base64 解密。");
            return Ok(decoded_str);
        }
        qrc_codec::decrypt_qrc(encrypted_str)
    }

    fn create_romanization_input(roma_lyrics_decrypted: &str) -> Option<InputFile> {
        if roma_lyrics_decrypted.is_empty() {
            return None;
        }

        let language_tag = Self::detect_romanization_language(roma_lyrics_decrypted);

        let content = Self::extract_from_qrc_wrapper(roma_lyrics_decrypted);

        Some(InputFile {
            content,
            format: LyricFormat::Qrc,
            language: Some(language_tag),
            filename: None,
        })
    }

    /// 因为歌词响应中的 `language` 字段总是为 `0`，
    /// 所以需要使用启发式规则检测罗马音的语言。
    ///
    /// 如果音节后带数字声调，则判定为粤语。否则视为日语。
    fn detect_romanization_language(roma_text: &str) -> String {
        if YUE_RE.is_match(roma_text).unwrap_or(false) {
            "yue-Latn".to_string()
        } else {
            "ja-Latn".to_string()
        }
    }
}

/// 根据 QQ 音乐专辑的 MID 构造指定尺寸的封面图片 URL。
///
/// # 参数
/// * `album_mid` - 专辑的 `mid` 字符串。
/// * `size` - 想要的封面图片尺寸，使用 `QQMusicCoverSize` 枚举。
///
/// # 返回
/// 一个包含了完整封面图片链接的 `String`。
fn get_qq_album_cover_url(album_mid: &str, size: QQMusicCoverSize) -> String {
    let size_val = size.as_u32();
    format!("https://y.gtimg.cn/music/photo_new/T002R{size_val}x{size_val}M000{album_mid}.jpg")
}

impl From<models::Singer> for Artist {
    fn from(qq_singer: models::Singer) -> Self {
        Self {
            id: qq_singer.mid.unwrap_or_default(),
            name: qq_singer.name,
        }
    }
}

impl From<models::SongInfo> for generic::Song {
    fn from(qq_song: models::SongInfo) -> Self {
        Self {
            id: qq_song.mid.clone(),
            provider_id: qq_song.mid,
            name: qq_song.name,
            artists: qq_song
                .singer
                .into_iter()
                .map(generic::Artist::from)
                .collect(),
            album: Some(qq_song.album.name),
            album_id: qq_song.album.mid.clone(),
            duration: Some(Duration::from_millis(qq_song.interval * 1000)),
            cover_url: qq_song
                .album
                .mid
                .as_deref()
                .map(|mid| get_qq_album_cover_url(mid, QQMusicCoverSize::Size300)),
        }
    }
}

impl From<models::AlbumInfo> for generic::Album {
    fn from(qq_album: models::AlbumInfo) -> Self {
        let album_mid = qq_album.basic_info.album_mid.clone();

        let artists = Some(
            qq_album
                .singer
                .singer_list
                .into_iter()
                .map(|s| Artist {
                    id: s.mid,
                    name: s.name,
                })
                .collect(),
        );

        let release_date = Some(qq_album.basic_info.publish_date);

        Self {
            id: album_mid.clone(),
            provider_id: album_mid.clone(),
            name: qq_album.basic_info.album_name,
            artists,
            description: Some(qq_album.basic_info.desc),
            release_date,
            // 此 API 响应不包含歌曲列表，因此设为 None。
            // 歌曲列表由 get_album_songs 单独获取。
            songs: None,
            cover_url: Some(get_qq_album_cover_url(
                &album_mid,
                QQMusicCoverSize::Size800,
            )),
        }
    }
}

impl From<&models::Song> for SearchResult {
    fn from(s: &models::Song) -> Self {
        let language = match s.language {
            Some(9) => Some(Language::Instrumental),
            Some(0 | 1) => Some(Language::Chinese),
            Some(3) => Some(Language::Japanese),
            Some(4) => Some(Language::Korean),
            Some(5) => Some(Language::English),
            _ => Some(Language::Other),
        };

        Self {
            title: s.name.clone(),
            artists: s
                .singer
                .iter()
                .map(|singer| Artist {
                    id: singer.mid.clone().unwrap_or_default(),
                    name: singer.name.clone(),
                })
                .collect(),
            album: Some(s.album.name.clone()),
            album_id: s.album.mid.clone(),
            duration: Some(s.interval * 1000),
            provider_id_num: s.id,
            cover_url: s
                .album
                .mid
                .as_deref()
                .map(|mid| get_qq_album_cover_url(mid, QQMusicCoverSize::Size800)),
            language,
            ..Default::default()
        }
    }
}

impl From<models::PlaylistDetailData> for generic::Playlist {
    fn from(qq_playlist: models::PlaylistDetailData) -> Self {
        Self {
            id: qq_playlist.info.id.to_string(),
            name: qq_playlist.info.title,
            cover_url: Some(qq_playlist.info.cover_url),
            creator_name: Some(qq_playlist.info.host_nick),
            description: Some(qq_playlist.info.description),
            songs: Some(
                qq_playlist
                    .songlist
                    .into_iter()
                    .map(generic::Song::from)
                    .collect(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SONG_NAME: &str = "目及皆是你";
    const TEST_SINGER_NAME: &str = "小蓝背心";
    const TEST_SONG_MID: &str = "00126fAV2ZKaOd";
    const TEST_SONG_ID: &str = "312214056";
    const TEST_ALBUM_MID: &str = "003dmKuv4689PG";
    const TEST_SINGER_MID: &str = "000iW1zw4fSVdV";
    const TEST_PLAYLIST_ID: &str = "7256912512"; // QQ音乐官方歌单: 欧美| 流行节奏控
    const TEST_TOPLIST_ID: u32 = 26; // QQ音乐热歌榜
    const INSTRUMENTAL_SONG_ID: &str = "201877085"; // 城南花已开

    // 周杰伦的即兴曲，主歌词包含了纯文本介绍内容
    // const SPECIAL_INSTRUMENTAL_SONG_ID: &str = "582359862";

    fn init_tracing() {
        use tracing_subscriber::{EnvFilter, FmtSubscriber};
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,lyrics_helper_rs=trace"));
        let _ = FmtSubscriber::builder()
            .with_env_filter(filter)
            .with_test_writer()
            .try_init();
    }

    #[tokio::test]
    #[ignore]
    async fn test_search_songs() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();
        let track = Track {
            title: Some(TEST_SONG_NAME),
            artists: Some(&[TEST_SINGER_NAME]),
            album: None,
            duration: None,
        };

        let results = provider.search_songs(&track).await.unwrap();
        assert!(!results.is_empty(), "搜索结果不应为空");
        assert!(results.iter().any(|s| s.title.contains(TEST_SONG_NAME)
            && s.artists.iter().any(|a| a.name == TEST_SINGER_NAME)));
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_lyrics() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();

        // 一首包含了主歌词、翻译和罗马音的歌曲：002DuMJE0E9YSa，可用于测试
        let lyrics = provider.get_lyrics(TEST_SONG_ID).await.unwrap();

        assert!(!lyrics.lines.is_empty(), "歌词解析结果不应为空");
        assert!(
            lyrics.lines[0].main_text().is_some(),
            "歌词第一行应该有文本内容"
        );

        assert!(
            lyrics.lines[10].main_track().map_or(0, |track| {
                track
                    .content
                    .words
                    .iter()
                    .flat_map(|word| &word.syllables)
                    .count()
            }) > 0,
            "QRC 歌词应该有音节信息"
        );

        info!("✅ 成功解析了 {} 行歌词", lyrics.lines.len());
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_lyrics_for_instrumental_song() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();

        let result = provider.get_full_lyrics(INSTRUMENTAL_SONG_ID).await;

        assert!(
            result.is_ok(),
            "对于纯音乐，get_full_lyrics 应该返回 Ok，而不是 Err。收到的错误: {:?}",
            result.err()
        );

        let full_lyrics_result = result.unwrap();

        assert_eq!(full_lyrics_result.parsed.lines.len(), 1, "应解析为一行歌词");

        let instrumental_line = &full_lyrics_result.parsed.lines[0];
        assert_eq!(
            instrumental_line.main_text().as_deref(),
            Some("此歌曲为没有填词的纯音乐，请您欣赏"),
            "歌词行的文本内容不匹配"
        );

        assert!(
            !full_lyrics_result.raw.content.is_empty(),
            "纯音乐的原始歌词内容不应为空"
        );

        info!("✅ 纯音乐已正确解析！");
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_album_info() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();
        let album_info = provider.get_album_info(TEST_ALBUM_MID).await.unwrap();

        assert_eq!(album_info.name, TEST_SONG_NAME);

        let artists = album_info.artists.expect("专辑应有歌手信息");
        assert_eq!(artists[0].name, TEST_SINGER_NAME);

        info!("✅ 成功获取专辑 '{}'", album_info.name);
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_album_songs() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();
        let songs = provider
            .get_album_songs(TEST_ALBUM_MID, 1, 5)
            .await
            .unwrap();
        assert!(!songs.is_empty());
        info!("✅ 在专辑中找到 {} 首歌曲", songs.len());
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_singer_songs() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();
        let songs = provider
            .get_singer_songs(TEST_SINGER_MID, 1, 5)
            .await
            .unwrap();
        assert!(!songs.is_empty());
        info!("✅ 为该歌手找到 {} 首歌曲", songs.len());
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_playlist() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();
        let playlist = provider.get_playlist(TEST_PLAYLIST_ID).await.unwrap();

        assert!(!playlist.name.is_empty(), "歌单名称不应为空");
        assert!(playlist.songs.is_some(), "歌单应包含歌曲列表");
        assert!(!playlist.songs.unwrap().is_empty(), "歌单歌曲列表不应为空");

        info!("✅ 成功获取歌单 '{}'", playlist.name);
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_toplist() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();
        let (info, songs) = provider
            .get_toplist(TEST_TOPLIST_ID, 1, 5, None)
            .await
            .unwrap();
        assert_eq!(info.top_id, TEST_TOPLIST_ID);
        assert!(!songs.is_empty());
        info!("✅ 排行榜 '{}' 包含歌曲", info.title);
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_song_info() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();
        let song = provider.get_song_info(TEST_SONG_MID).await.unwrap();

        assert_eq!(song.name, TEST_SONG_NAME);
        assert_eq!(song.artists[0].name, TEST_SINGER_NAME);
        info!("✅ 成功获取歌曲 '{}'", song.name);
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_song_link() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();

        // 如果想测试 VIP 歌曲：
        // let link_result = provider.get_song_link(TEST_SONG_MID).await;

        let link_result = provider.get_song_link("001xeS8622ntLO").await;

        match link_result {
            Ok(link) => {
                assert!(link.starts_with("http"), "链接应以 http 开头");
                info!("✅ 成功获取链接: {}", link);
            }
            Err(e) => {
                // 如果是 VIP 歌曲，API 会返回空 purl，捕捉这个错误也算测试通过
                let msg = e.to_string();
                assert!(msg.contains("purl 为空"), "错误信息应提示 purl 为空");
                info!("✅ 因 VIP 歌曲而失败，信息: {}", msg);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_album_cover_url() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();
        let album_mid = TEST_ALBUM_MID;

        info!("[QQ音乐测试] 正在获取大尺寸封面...");
        let large_cover_url = provider
            .get_album_cover_url(album_mid, CoverSize::Large)
            .await
            .expect("获取大尺寸封面失败");

        assert!(large_cover_url.contains("T002R800x800M000003dmKuv4689PG.jpg"));
        info!("✅ 大尺寸封面URL正确: {}", large_cover_url);

        let thumb_cover_url = provider
            .get_album_cover_url(album_mid, CoverSize::Thumbnail)
            .await
            .expect("获取缩略图封面失败");

        assert!(thumb_cover_url.contains("T002R150x150M000003dmKuv4689PG.jpg"));
        info!("✅ 缩略图封面URL正确: {}", thumb_cover_url);

        let invalid_id_result = provider
            .get_album_info("999999999999999999999999999999999")
            .await;
        assert!(invalid_id_result.is_err(), "无效的专辑ID应该返回错误");
        if let Err(e) = invalid_id_result {
            info!("✅ 成功捕获到错误: {}", e);
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_search_by_type() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();
        let keyword = "小蓝背心";

        let song_results = provider
            .search_by_type(keyword, models::SearchType::Song, 1, 5)
            .await
            .unwrap();

        assert!(!song_results.is_empty(), "按歌曲类型搜索时，结果不应为空");
        assert!(
            matches!(song_results[0], models::TypedSearchResult::Song(_)),
            "搜索歌曲时应返回 Song 类型的结果"
        );
        info!("✅ 按歌曲类型搜索成功！");

        let album_results = provider
            .search_by_type(keyword, models::SearchType::Album, 1, 5)
            .await
            .unwrap();
        assert!(!album_results.is_empty(), "按专辑类型搜索时，结果不应为空");
        assert!(
            matches!(album_results[0], models::TypedSearchResult::Album(_)),
            "搜索专辑时应返回 Album 类型的结果"
        );
        info!("✅ 按专辑类型搜索成功！");

        let singer_results = provider
            .search_by_type(keyword, models::SearchType::Singer, 1, 5)
            .await
            .unwrap();
        assert!(!singer_results.is_empty(), "按歌手类型搜索时，结果不应为空");
        assert!(
            matches!(singer_results[0], models::TypedSearchResult::Singer(_)),
            "搜索歌手时应返回 Singer 类型的结果"
        );
        info!("✅ 按歌手类型搜索成功！");
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_lyrics_full() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();
        let song_mid = "002DuMJE0E9YSa";

        let result = provider.get_full_lyrics(song_mid).await;

        assert!(result.is_ok(), "获取歌词失败: {:?}", result.err());
        let full_lyrics = result.unwrap();

        assert!(!full_lyrics.parsed.lines.is_empty(), "解析后的行不应为空");

        info!("解析结果: {:#?}", full_lyrics.parsed);
    }
}
