//! QQ音乐提供商模块。
//!
//! API 来源于 <https://github.com/luren-dc/QQMusicApi>

use std::{
    collections::HashMap,
    pin::Pin,
    sync::{Arc, LazyLock, RwLock},
    task::{Context, Poll},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use base64::{Engine, prelude::BASE64_STANDARD};
use chrono::{Datelike, Local};
use futures::Sink;
use regex::Regex;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use lyrics_helper_core::{
    Artist, ConversionInput, ConversionOptions, CoverSize, FullLyricsResult, InputFile, Language,
    LyricFormat, ParsedSourceData, RawLyrics, SearchResult, Track, model::generic,
};
use quick_xml::{Reader, events::Event};
use rand::random;
use serde::Deserialize;
use serde_json::json;
use tracing::{debug, error, info, instrument, trace, warn};
use url::Url;
use uuid::Uuid;

use crate::{
    LoginResult, ProviderAuthState, UserProfile,
    config::{load_cached_config, save_cached_config},
    converter,
    error::{LyricsHelperError, Result},
    http::{HttpClient, HttpMethod},
    model::auth::{LoginAction, LoginError, LoginEvent, LoginFlow, LoginMethod},
    providers::{
        Provider,
        login::LoginProvider,
        qq::models::{LrcApiResponse, QQMusicCoverSize, QRCodeInfo, QRCodeStatus},
    },
};

pub mod device;
pub mod models;
pub mod qimei;
pub mod qrc_codec;
pub mod sign;

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

const GET_TOPLIST_MODULE: &str = "musicToplist.ToplistInfoServer";
const GET_TOPLIST_METHOD: &str = "GetDetail";

const APP_VERSION: &str = "14.8.0.8";

const SONG_URL_DOMAIN: &str = "https://isure.stream.qqmusic.qq.com/";
const QQ_MUSIC_REFERER_URL: &str = "https://y.qq.com";
const ALBUM_COVER_BASE_URL: &str = "https://y.gtimg.cn/music/photo_new/";

const LRC_LYRIC_URL: &str = "https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg";

const LYRIC_DOWNLOAD_URL: &str = "https://c.y.qq.com/qqmusic/fcgi-bin/lyric_download.fcg";

static PTUICB_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"ptuiCB\((.*)\)").unwrap());

static QRC_LYRIC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"LyricContent="([^"]*)""#).unwrap());

static AMP_RE: LazyLock<fancy_regex::Regex> =
    LazyLock::new(|| fancy_regex::Regex::new(r"&(?![a-zA-Z]{2,6};|#[0-9]{2,4};)").unwrap());

static YUE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-zA-Z]+[1-6]").unwrap());

/// QQ 音乐的提供商实现。
#[derive(Clone)]
pub struct QQMusic {
    http_client: Arc<dyn HttpClient>,
    qimei: String,
    auth_state: Arc<RwLock<Option<ProviderAuthState>>>,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[async_trait]
impl Provider for QQMusic {
    fn name(&self) -> &'static str {
        "qq"
    }

    #[instrument(skip_all, fields(provider = "qq"))]
    async fn with_http_client(http_client: Arc<dyn HttpClient>) -> Result<Self>
    where
        Self: Sized,
    {
        const CACHE_FILENAME: &str = "qq_device.json";

        let device = if let Ok(config) = load_cached_config::<device::Device>(CACHE_FILENAME) {
            info!(cache_file = CACHE_FILENAME, "已从缓存加载 QQ Device");
            config.data
        } else {
            info!(
                cache_file = CACHE_FILENAME,
                "QQ Device 配置文件不存在或无效，将创建并保存一个新设备"
            );
            let new_device = device::Device::new();
            if let Err(e) = save_cached_config(CACHE_FILENAME, &new_device) {
                warn!(error = %e, filename = CACHE_FILENAME, "保存新的 QQ Device 失败");
            }
            new_device
        };

        let qimei_result = qimei::get_qimei(http_client.as_ref(), &device, APP_VERSION)
            .await
            .map_err(|e| LyricsHelperError::ApiError(format!("获取 Qimei 失败: {e}")))?;

        Ok(Self {
            http_client,
            qimei: qimei_result.q36,
            auth_state: Arc::new(RwLock::new(None)),
        })
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
    #[instrument(skip(self, track), fields(keyword))]
    async fn search_songs(&self, track: &Track<'_>) -> Result<Vec<SearchResult>> {
        let keyword = format!(
            "{} {}",
            track.title.unwrap_or(""),
            track.artists.unwrap_or(&[]).join(" ")
        )
        .trim()
        .to_string();
        tracing::Span::current().record("keyword", tracing::field::display(&keyword));

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

    #[instrument(skip(self), fields(song_id = %song_id))]
    async fn get_full_lyrics(&self, song_id: &str) -> Result<FullLyricsResult> {
        let main_api_result = self.try_get_lyrics_internal(song_id).await;

        match main_api_result {
            Ok(lyrics) => Ok(lyrics),
            Err(e) => {
                if matches!(e, LyricsHelperError::LyricNotFound) {
                    return Err(e);
                }

                warn!(
                    song_id = %song_id,
                    error = ?e,
                    "主接口调用失败，尝试备用接口"
                );

                let numerical_id = match self.get_numerical_song_id(song_id).await {
                    Ok(id) => id,
                    Err(id_err) => {
                        warn!(
                            song_id = %song_id,
                            error = ?id_err,
                            "获取歌曲数字 ID 失败，尝试调用仅 LRC 接口"
                        );
                        return self.try_get_lyrics_lrc_only(song_id).await;
                    }
                };

                // 调用备用接口
                match self.try_get_lyrics_fallback(numerical_id).await {
                    Ok(lyrics) => Ok(lyrics),
                    Err(fallback_err) => {
                        warn!(
                            song_id = %song_id,
                            numerical_id,
                            error = ?fallback_err,
                            "备用接口失败，尝试调用仅 LRC 接口"
                        );
                        self.try_get_lyrics_lrc_only(song_id).await
                    }
                }
            }
        }
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
    #[instrument(skip(self), fields(album_mid = %album_mid))]
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
    #[instrument(skip(self), fields(album_mid = %album_mid, page, page_size))]
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
    #[instrument(skip(self), fields(singer_mid = %singer_mid, page, page_size))]
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
    #[instrument(skip(self), fields(playlist_id = %playlist_id))]
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
    #[instrument(skip(self), fields(song_id = %song_id))]
    async fn get_song_info(&self, song_id: &str) -> Result<generic::Song> {
        let param = song_id.parse::<u64>().map_or_else(
            |_| json!({ "song_mid": song_id }),
            |id| json!({ "song_id": id }),
        );

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
    #[instrument(skip(self), fields(song_mid = %song_mid))]
    async fn get_song_link(&self, song_mid: &str) -> Result<String> {
        let mids_slice = [song_mid];

        let (success_map, failure_map) = self
            .get_song_urls_internal(&mids_slice, models::SongFileType::Mp3_128)
            .await?;

        success_map.get(song_mid).map_or_else(
            || {
                failure_map.get(song_mid).map_or_else(
                    || {
                        Err(LyricsHelperError::ApiError(format!(
                            "未在 API 响应中找到 song_mid '{song_mid}' 的播放链接"
                        )))
                    },
                    |reason| Err(LyricsHelperError::ApiError(reason.clone())),
                )
            },
            |url| Ok(url.clone()),
        )
    }

    #[instrument(skip(self), fields(album_id = %album_id, ?size))]
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

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl LoginProvider for QQMusic {
    #[instrument(skip(self, method), fields(provider = "qq"))]
    fn initiate_login(&self, method: LoginMethod) -> LoginFlow {
        let (events_tx, events_rx) = mpsc::channel(16);
        let (actions_tx, mut actions_rx) = mpsc::channel::<LoginAction>(1);

        let http_client = Arc::clone(&self.http_client);
        let qimei = self.qimei.clone();

        tokio::spawn(async move {
            let provider_clone = Self {
                http_client,
                qimei,
                auth_state: Arc::new(RwLock::new(None)),
            };

            let login_logic = async {
                match method {
                    LoginMethod::QQMusicByCookie { cookies } => {
                        provider_clone.handle_cookie_login(events_tx, cookies).await;
                    }
                    LoginMethod::QQMusicByQRCode => {
                        provider_clone.handle_qr_code_login(events_tx).await;
                    }
                    _ => {
                        let _ = events_tx
                            .send(LoginEvent::Failure(LoginError::ProviderError(
                                "不支持的登录方法".to_string(),
                            )))
                            .await;
                    }
                }
            };

            tokio::select! {
                () = login_logic => {},
                Some(LoginAction::Cancel) = actions_rx.recv() => {
                    info!("[QQ] 用户取消了登录");
                }
            }
        });

        let stream = ReceiverStream::new(events_rx);
        let sink = ActionSink { tx: actions_tx };

        LoginFlow {
            events: Box::pin(stream),
            actions: Box::pin(sink),
        }
    }

    fn set_auth_state(&self, auth_state: &ProviderAuthState) -> Result<()> {
        if let ProviderAuthState::QQMusic { .. } = auth_state {
            self.auth_state.write().map_or_else(
                |_| Err(LyricsHelperError::Internal("无法获取写锁".into())),
                |mut state| {
                    *state = Some(auth_state.clone());
                    Ok(())
                },
            )
        } else {
            Err(LyricsHelperError::ApiError("无效的 AuthState 类型".into()))
        }
    }

    fn get_auth_state(&self) -> Option<ProviderAuthState> {
        self.auth_state.read().ok().and_then(|s| s.clone())
    }

    #[instrument(skip(self), fields(provider = "qq"))]
    async fn verify_session(&self) -> Result<()> {
        if self.get_auth_state().is_none() {
            return Err(LyricsHelperError::LoginFailed(
                LoginError::InvalidCredentials("未设置登录状态".into()),
            ));
        }

        self.execute_api_request(
            "music.UserInfo.userInfoServer",
            "GetLoginUserInfo",
            json!({}),
            &[0],
        )
        .await
        .map_err(|e| {
            LyricsHelperError::LoginFailed(LoginError::InvalidCredentials(format!(
                "会话已失效: {e}"
            )))
        })?;

        Ok(())
    }
}

struct ActionSink {
    tx: mpsc::Sender<LoginAction>,
}

impl Sink<LoginAction> for ActionSink {
    type Error = LoginError;

    fn poll_ready(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: LoginAction) -> std::result::Result<(), Self::Error> {
        self.tx
            .try_send(item)
            .map_err(|e| LoginError::Internal(e.to_string()))
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

impl QQMusic {
    fn build_comm(&self) -> serde_json::Value {
        let mut comm_map = serde_json::Map::from_iter(vec![
            ("cv".to_string(), json!(13_020_508)),
            ("ct".to_string(), json!(11)),
            ("v".to_string(), json!(13_020_508)),
            ("QIMEI36".to_string(), json!(&self.qimei)),
            ("tmeAppID".to_string(), json!("qqmusic")),
            ("inCharset".to_string(), json!("utf-8")),
            ("outCharset".to_string(), json!("utf-8")),
        ]);

        if let Ok(state_guard) = self.auth_state.read()
            && let Some(ProviderAuthState::QQMusic {
                musicid, musickey, ..
            }) = &*state_guard
        {
            comm_map.insert("qq".to_string(), json!(musicid.to_string()));
            comm_map.insert("authst".to_string(), json!(musickey));
            comm_map.insert("tmeLoginType".to_string(), json!(2));
        }

        serde_json::Value::Object(comm_map)
    }

    #[instrument(skip(self, param), fields(module = %module, method = %method))]
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

        let body = serde_json::to_vec(&payload)?;

        let cookie_header: Option<String> = self.auth_state.read().map_or(None, |state_guard| {
            if let Some(ProviderAuthState::QQMusic {
                musicid, musickey, ..
            }) = &*state_guard
            {
                Some(format!(
                    "uin={musicid}; qqmusic_key={musickey}; qm_keyst={musickey};"
                ))
            } else {
                None
            }
        });

        let mut headers: Vec<(&str, &str)> = vec![("Content-Type", "application/json")];
        if let Some(cookie) = &cookie_header {
            headers.push(("Cookie", cookie.as_str()));
        }

        let response = self
            .http_client
            .request_with_headers(HttpMethod::Post, url, &headers, Some(&body))
            .await?;

        let response_text = response.text()?;

        trace!(
            request_key = %request_key,
            response.body = %response_text,
            "收到原始 JSON 响应"
        );

        let mut response_value: serde_json::Value = serde_json::from_str(&response_text)?;

        if let Some(business_object) = response_value
            .get_mut(&request_key)
            .map(serde_json::Value::take)
        {
            let business_code: models::BusinessCode =
                serde_json::from_value(business_object.clone())?;

            if business_code.code == 24001 {
                return Err(LyricsHelperError::LyricNotFound);
            }

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
    #[instrument(skip(self), fields(top_id, page, page_size, ?period))]
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
            .execute_api_request(GET_TOPLIST_MODULE, GET_TOPLIST_METHOD, param, &[2000])
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
    #[instrument(skip(self), fields(keyword = %keyword, ?search_type, page, page_size))]
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
    #[instrument]
    fn extract_from_qrc_wrapper(decrypted_text: &str) -> String {
        if decrypted_text.is_empty() {
            return String::new();
        }

        if !decrypted_text.starts_with("<?xml") {
            return decrypted_text.to_string();
        }

        let try_parse = |text: &str| -> Option<String> {
            let mut reader = Reader::from_str(text);
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Start(e) | Event::Empty(e)) => {
                        if let Ok(Some(attr)) = e.try_get_attribute("LyricContent")
                            && let Ok(value) = attr.decode_and_unescape_value(reader.decoder())
                        {
                            return Some(value.to_string());
                        }
                    }
                    Err(e) => {
                        info!(error = ?e, "quick-xml 解析失败");
                        return None;
                    }
                    Ok(Event::Eof) => return None,
                    _ => (),
                }
                buf.clear();
            }
        };

        if let Some(content) = try_parse(decrypted_text) {
            return content;
        }

        let fixed_amp_text = AMP_RE.replace_all(decrypted_text, "&amp;").to_string();

        let fixed_text = if let Some(start_idx) = fixed_amp_text.find("LyricContent=\"") {
            let content_start = start_idx + "LyricContent=\"".len();
            if let Some(end_idx) = fixed_amp_text[content_start..].rfind('"') {
                let content_end = content_start + end_idx;
                let prefix = &fixed_amp_text[..content_start];
                let content_to_fix = &fixed_amp_text[content_start..content_end];
                let suffix = &fixed_amp_text[content_end..];

                let fixed_content = content_to_fix.replace('"', "&quot;");
                let result = format!("{prefix}{fixed_content}{suffix}");
                result
            } else {
                fixed_amp_text
            }
        } else {
            fixed_amp_text
        };

        if let Some(content) = try_parse(&fixed_text) {
            return content;
        }

        if let Some(caps) = QRC_LYRIC_RE.captures(&fixed_text)
            && let Some(content) = caps.get(1)
        {
            return content
                .as_str()
                .replace("&quot;", "\"")
                .replace("&amp;", "&")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&apos;", "'");
        }

        warn!("所有提取歌词的方法均失败，返回原始文本。");
        decrypted_text.to_string()
    }

    #[instrument(skip(self), fields(song_id = %song_id, method = "primary"))]
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

        if !main_lyrics_decrypted.is_empty() {
            trace!(lyrics.main = %main_lyrics_decrypted, "解密后的主歌词");
        }
        if !trans_lyrics_decrypted.is_empty() {
            trace!(lyrics.translation = %trans_lyrics_decrypted, "解密后的翻译");
        }
        if !roma_lyrics_decrypted.is_empty() {
            trace!(lyrics.romanization = %roma_lyrics_decrypted, "解密后的罗马音");
        }

        Self::build_full_lyrics_result(
            main_lyrics_decrypted,
            trans_lyrics_decrypted,
            &roma_lyrics_decrypted,
            self.name(),
        )
    }

    #[instrument(skip(self, song_mids), fields(mids_count = song_mids.len(), ?file_type))]
    async fn get_song_urls_internal(
        &self,
        song_mids: &[&str],
        file_type: models::SongFileType,
    ) -> Result<(HashMap<String, String>, HashMap<String, String>)> {
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

        let mut success_map = std::collections::HashMap::new();
        let mut failure_map = std::collections::HashMap::new();

        for info in result_data.midurlinfo {
            if info.purl.is_empty() {
                let reason = format!(
                    "无法获取 songmid '{}' 的链接 (purl 为空)，可能是 VIP 歌曲。",
                    info.songmid
                );
                failure_map.insert(info.songmid, reason);
            } else {
                success_map.insert(info.songmid, format!("{}{}", SONG_URL_DOMAIN, info.purl));
            }
        }

        Ok((success_map, failure_map))
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
    #[instrument]
    fn decrypt_with_fallback(encrypted_str: &str) -> Result<String> {
        if let Ok(decoded_bytes) = BASE64_STANDARD.decode(encrypted_str)
            && let Ok(decoded_str) = String::from_utf8(decoded_bytes)
        {
            info!("成功使用 Base64 解密");
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
        if YUE_RE.is_match(roma_text) {
            "yue-Latn".to_string()
        } else {
            "ja-Latn".to_string()
        }
    }

    #[instrument(skip(self), fields(song_id = %song_id))]
    async fn get_numerical_song_id(&self, song_id: &str) -> Result<u64> {
        if let Ok(id) = song_id.parse::<u64>() {
            return Ok(id);
        }

        info!(song_id, "不是数字 ID，尝试通过 API 转换");
        let param = json!({ "song_mid": song_id });

        let response_val = self
            .execute_api_request(GET_SONG_DETAIL_MODULE, GET_SONG_DETAIL_METHOD, param, &[0])
            .await?;

        let result_container: models::SongDetailApiContainer =
            serde_json::from_value(response_val)?;

        Ok(result_container.data.track_info.id)
    }

    /// 备用歌词获取方案，调用 `lyric_download.fcg` 接口。
    ///
    /// # 参数
    ///
    /// * `music_id` - 歌曲的数字 ID。
    #[instrument(skip(self), fields(music_id, method = "fallback"))]
    async fn try_get_lyrics_fallback(&self, music_id: u64) -> Result<FullLyricsResult> {
        let params = &[
            ("version", "15"),
            ("miniversion", "82"),
            ("lrctype", "4"),
            ("musicid", &music_id.to_string()),
        ];

        let response = self
            .http_client
            .post_form(LYRIC_DOWNLOAD_URL, params)
            .await?;
        let response_text = response.text()?;

        trace!(
            response.body = %response_text,
            "备用歌词接口 'lyric_download.fcg' 的原始响应"
        );

        let xml_content = response_text
            .trim()
            .strip_prefix("<!--")
            .unwrap_or(&response_text)
            .strip_suffix("-->")
            .unwrap_or(&response_text)
            .trim();

        if xml_content.is_empty() {
            return Err(LyricsHelperError::LyricNotFound);
        }

        let mut reader = Reader::from_str(xml_content);
        let mut buf = Vec::new();
        let mut main_lyrics_encrypted = String::new();
        let mut trans_lyrics_encrypted = String::new();
        let mut roma_lyrics_encrypted = String::new();
        let mut current_tag = String::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    current_tag = String::from_utf8(e.name().as_ref().to_vec()).unwrap_or_default();
                }
                Ok(Event::CData(e)) => {
                    if let Ok(text) = e.decode()
                        && !text.trim().is_empty()
                    {
                        match current_tag.as_str() {
                            "content" => main_lyrics_encrypted = text.to_string(),
                            "contentts" => trans_lyrics_encrypted = text.to_string(),
                            "contentroma" => roma_lyrics_encrypted = text.to_string(),
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    warn!(error = ?e, xml.content = %xml_content, "XML 解析错误");
                    return Err(LyricsHelperError::Parser(format!(
                        "解析备用接口歌词XML失败: {e}"
                    )));
                }
                Ok(Event::Eof) => break,
                _ => {}
            }
            buf.clear();
        }

        if main_lyrics_encrypted.is_empty() {
            return Err(LyricsHelperError::LyricNotFound);
        }

        let main_lyrics_decrypted = qrc_codec::decrypt_qrc(&main_lyrics_encrypted)?;
        let trans_lyrics_decrypted =
            qrc_codec::decrypt_qrc(&trans_lyrics_encrypted).unwrap_or_default();
        let roma_lyrics_decrypted =
            qrc_codec::decrypt_qrc(&roma_lyrics_encrypted).unwrap_or_default();

        if !main_lyrics_decrypted.is_empty() {
            trace!(lyrics.main = %main_lyrics_decrypted, "解密后的主歌词");
        }
        if !trans_lyrics_decrypted.is_empty() {
            trace!(lyrics.translation = %trans_lyrics_decrypted, "解密后的翻译");
        }
        if !roma_lyrics_decrypted.is_empty() {
            trace!(lyrics.romanization = %roma_lyrics_decrypted, "解密后的罗马音");
        }

        Self::build_full_lyrics_result(
            main_lyrics_decrypted,
            trans_lyrics_decrypted,
            &roma_lyrics_decrypted,
            self.name(),
        )
    }

    fn build_full_lyrics_result(
        main_lyrics_decrypted: String,
        trans_lyrics_decrypted: String,
        roma_lyrics_decrypted: &str,
        source_name: &str,
    ) -> Result<FullLyricsResult> {
        if !(main_lyrics_decrypted.starts_with("<?xml")
            || main_lyrics_decrypted.trim().starts_with('[') && main_lyrics_decrypted.contains(']'))
        {
            let mut raw_metadata = std::collections::HashMap::new();
            raw_metadata.insert(
                "introduction".to_string(),
                vec![main_lyrics_decrypted.clone()],
            );
            let parsed_data = ParsedSourceData {
                source_name: source_name.to_string(),
                raw_metadata,
                ..Default::default()
            };
            let raw_lyrics = RawLyrics {
                format: "txt".to_string(),
                content: main_lyrics_decrypted,
                translation: if trans_lyrics_decrypted.is_empty() {
                    None
                } else {
                    Some(trans_lyrics_decrypted)
                },
            };
            return Ok(FullLyricsResult {
                parsed: parsed_data,
                raw: raw_lyrics,
            });
        }

        let main_lyric_format = if main_lyrics_decrypted.starts_with("<?xml") {
            LyricFormat::Qrc
        } else {
            LyricFormat::Lrc
        };

        let main_lyrics_content = if main_lyric_format == LyricFormat::Qrc {
            Self::extract_from_qrc_wrapper(&main_lyrics_decrypted)
        } else {
            main_lyrics_decrypted
        };

        let mut translations = Vec::new();
        if !trans_lyrics_decrypted.is_empty() {
            translations.push(InputFile {
                content: Self::extract_from_qrc_wrapper(&trans_lyrics_decrypted),
                format: LyricFormat::Lrc,
                language: Some("zh-Hans".to_string()),
                filename: None,
            });
        }

        let romanizations = Self::create_romanization_input(roma_lyrics_decrypted)
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
            additional_metadata: None,
        };
        let mut parsed_data =
            converter::parse_and_merge(&conversion_input, &ConversionOptions::default())?;
        parsed_data.source_name = source_name.to_string();

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

    #[instrument(skip(self), fields(song_mid = %song_mid, method = "lrc_only"))]
    async fn try_get_lyrics_lrc_only(&self, song_mid: &str) -> Result<FullLyricsResult> {
        let headers = [("Referer", QQ_MUSIC_REFERER_URL)];

        let params = &[
            ("songmid", song_mid),
            (
                "pcachetime",
                &chrono::Local::now().timestamp_millis().to_string(),
            ),
            ("g_tk", "5381"),
            ("loginUin", "0"),
            ("hostUin", "0"),
            ("inCharset", "utf8"),
            ("outCharset", "utf-8"),
            ("notice", "0"),
            ("platform", "yqq"),
            ("needNewCode", "0"),
        ];

        let response = self
            .http_client
            .get_with_params_and_headers(LRC_LYRIC_URL, params, &headers)
            .await?;
        let response_text = response.text()?;

        trace!(response.body = %response_text, "LRC 接口原始响应");

        let json_text = response_text
            .trim()
            .strip_prefix("MusicJsonCallback(")
            .unwrap_or(&response_text)
            .strip_suffix(')')
            .unwrap_or(&response_text);

        if json_text.is_empty() {
            return Err(LyricsHelperError::LyricNotFound);
        }

        let api_response: LrcApiResponse = serde_json::from_str(json_text)?;

        if api_response.code != 0 {
            return Err(LyricsHelperError::ApiError(format!(
                "LRC 接口返回业务错误码: {}",
                api_response.code
            )));
        }

        let main_lyrics_b64 = api_response.lyric.unwrap_or_default();
        let trans_lyrics_b64 = api_response.trans.unwrap_or_default();

        if main_lyrics_b64.is_empty() {
            return Err(LyricsHelperError::LyricNotFound);
        }

        let main_lyrics_decrypted_bytes = BASE64_STANDARD
            .decode(main_lyrics_b64)
            .map_err(|e| LyricsHelperError::Decryption(format!("LRC 接口 Base64 解码失败: {e}")))?;
        let main_lyrics_decrypted = String::from_utf8(main_lyrics_decrypted_bytes)?;

        let trans_lyrics_decrypted = if trans_lyrics_b64.is_empty() {
            String::new()
        } else {
            let bytes = BASE64_STANDARD.decode(trans_lyrics_b64)?;
            String::from_utf8(bytes)?
        };

        trace!(lyrics.main = %main_lyrics_decrypted, "解码后的主歌词");
        if !trans_lyrics_decrypted.is_empty() {
            trace!(lyrics.translation = %trans_lyrics_decrypted, "解码后的翻译");
        }

        Self::build_full_lyrics_result(
            main_lyrics_decrypted,
            trans_lyrics_decrypted,
            "",
            self.name(),
        )
    }

    fn parse_cookies(cookies: &str) -> HashMap<String, String> {
        cookies
            .split(';')
            .filter_map(|s| {
                let mut parts = s.trim().splitn(2, '=');
                match (parts.next(), parts.next()) {
                    (Some(key), Some(value)) if !key.is_empty() => {
                        Some((key.to_string(), value.to_string()))
                    }
                    _ => None,
                }
            })
            .collect()
    }

    fn hash33(s: &str) -> i64 {
        let mut hash: i64 = 0;
        for char_code in s.chars().map(|c| c as i64) {
            hash = hash
                .wrapping_shl(5)
                .wrapping_add(hash)
                .wrapping_add(char_code);
        }
        hash & 0x7FFF_FFFF
    }

    pub async fn get_qrcode(&self) -> Result<QRCodeInfo> {
        const QRCODE_URL: &str = "https://ssl.ptlogin2.qq.com/ptqrshow";

        let headers = [("Referer", "https://xui.ptlogin2.qq.com/")];
        let params = [
            ("appid", "716027609"),
            ("e", "2"),
            ("l", "M"),
            ("s", "3"),
            ("d", "72"),
            ("v", "4"),
            ("daid", "383"),
            ("pt_3rd_aid", "100497308"),
        ];

        let t_param = random::<f64>().to_string();

        let mut final_params = params.to_vec();
        final_params.push(("t", &t_param));

        let response = self
            .http_client
            .get_with_params_and_headers(QRCODE_URL, &final_params, &headers)
            .await?;

        let qrsig = response
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
            .and_then(|(_, v)| {
                v.split(';')
                    .find(|s| s.trim().starts_with("qrsig="))
                    .map(|s| s.trim().trim_start_matches("qrsig=").to_string())
            })
            .ok_or_else(|| {
                LyricsHelperError::LoginFailed(LoginError::Network(
                    "未能从响应中获取qrsig".to_string(),
                ))
            })?;

        if qrsig.is_empty() {
            return Err(LyricsHelperError::LoginFailed(LoginError::Network(
                "获取到的qrsig为空".to_string(),
            )));
        }

        Ok(QRCodeInfo {
            image_data: response.body,
            qrsig,
        })
    }

    pub async fn check_qrcode_status(&self, qrsig: &str) -> Result<QRCodeStatus> {
        const POLL_URL: &str = "https://ssl.ptlogin2.qq.com/ptqrlogin";

        let headers = [(
            "Referer".to_string(),
            "https://xui.ptlogin2.qq.com/".to_string(),
        )];

        let headers_slice: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let current_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        let ptqrtoken = Self::hash33(qrsig).to_string();
        let action = format!("0-0-{current_ts}");

        let params = [
            ("u1", "https://graph.qq.com/oauth2.0/login_jump"),
            ("ptqrtoken", &ptqrtoken),
            ("ptredirect", "0"),
            ("h", "1"),
            ("t", "1"),
            ("g", "1"),
            ("from_ui", "1"),
            ("ptlang", "2052"),
            ("action", &action),
            ("js_ver", "20102616"),
            ("js_type", "1"),
            ("pt_uistyle", "40"),
            ("aid", "716027609"),
            ("daid", "383"),
            ("pt_3rd_aid", "100497308"),
            ("has_onekey", "1"),
        ];

        let response = self
            .http_client
            .get_with_params_and_headers(POLL_URL, &params, &headers_slice)
            .await?;

        let text = response.text()?;
        let captures = PTUICB_REGEX
            .captures(&text)
            .and_then(|caps| caps.get(1))
            .ok_or_else(|| {
                LyricsHelperError::LoginFailed(LoginError::Network(
                    "无法解析ptuiCB响应".to_string(),
                ))
            })?;

        let args: Vec<&str> = captures
            .as_str()
            .split(',')
            .map(|s| s.trim_matches('\''))
            .collect();
        let status_code = args.first().ok_or_else(|| {
            LyricsHelperError::LoginFailed(LoginError::Network(
                "ptuiCB响应中缺少状态码".to_string(),
            ))
        })?;

        let status = match *status_code {
            "0" => {
                let url = args.get(2).ok_or_else(|| {
                    LyricsHelperError::LoginFailed(LoginError::Network(
                        "ptuiCB响应中缺少重定向URL".to_string(),
                    ))
                })?;
                QRCodeStatus::Confirmed {
                    url: (*url).to_string(),
                }
            }
            "65" => QRCodeStatus::TimedOut,
            "66" => QRCodeStatus::WaitingForScan,
            "67" => QRCodeStatus::Scanned,
            "68" => QRCodeStatus::Refused,
            code => QRCodeStatus::Error(format!("未知的状态码: {code}")),
        };

        Ok(status)
    }

    fn g_tk_hash(s: &str) -> i64 {
        let mut hash: i64 = 5381;
        for char_code in s.chars().map(|c| c as i64) {
            hash = hash
                .wrapping_shl(5)
                .wrapping_add(hash)
                .wrapping_add(char_code);
        }
        hash & 0x7FFF_FFFF
    }

    #[instrument(skip(self, redirect_url), fields(provider = "qq"))]
    async fn fetch_p_skey_from_redirect(&self, redirect_url: &str) -> Result<String> {
        debug!(url = %redirect_url, "正在向 check_sig URL 发起GET请求以获取 p_skey...");

        let headers = [(
            "Referer".to_string(),
            "https://xui.ptlogin2.qq.com/".to_string(),
        )];

        let headers_slice: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();

        let check_sig_response = self
            .http_client
            .request_with_headers(HttpMethod::Get, redirect_url, &headers_slice, None)
            .await
            .map_err(|e| {
                error!("GET重定向URL时出错: {:?}", e);
                e
            })?;

        check_sig_response
            .headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
            .find_map(|(_, v)| {
                v.split(';')
                    .find(|s| s.trim().starts_with("p_skey="))
                    .map(|s| s.trim().trim_start_matches("p_skey=").to_string())
            })
            .ok_or_else(|| {
                error!("未能从响应头中找到 p_skey cookie");
                LyricsHelperError::LoginFailed(LoginError::Network(
                    "未能从响应头中找到p_skey".to_string(),
                ))
            })
    }

    #[instrument(skip(self, p_skey), fields(provider = "qq"))]
    async fn get_authorization_code(&self, p_skey: &str) -> Result<String> {
        let g_tk = Self::g_tk_hash(p_skey).to_string();
        let auth_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .to_string();
        let ui = Uuid::new_v4().to_string();

        const AUTHORIZE_URL: &str = "https://graph.qq.com/oauth2.0/authorize";
        let form_data = [
            ("response_type", "code"),
            ("client_id", "100497308"),
            (
                "redirect_uri",
                "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/",
            ),
            ("scope", "get_user_info,get_app_friends"),
            ("state", "state"),
            ("switch", ""),
            ("from_ptlogin", "1"),
            ("src", "1"),
            ("update_auth", "1"),
            ("openapi", "1010_1030"),
            ("g_tk", &g_tk),
            ("auth_time", &auth_time),
            ("ui", &ui),
        ];

        let final_redirect_url = self
            .http_client
            .post_form_for_redirect(AUTHORIZE_URL, &form_data)
            .await?;

        let parsed_url = Url::parse(&final_redirect_url).map_err(|_| {
            LyricsHelperError::LoginFailed(LoginError::Network(
                "无法解析最终的重定向URL".to_string(),
            ))
        })?;

        parsed_url
            .query_pairs()
            .find_map(|(key, value)| {
                if key == "code" {
                    Some(value.into_owned())
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                LyricsHelperError::LoginFailed(LoginError::Network(
                    "最终URL中缺少'code'参数".to_string(),
                ))
            })
    }

    #[instrument(skip(self, code), fields(provider = "qq"))]
    async fn exchange_code_for_auth_state(&self, code: &str) -> Result<ProviderAuthState> {
        let login_data = self
            .execute_api_request(
                "QQConnectLogin.LoginServer",
                "QQLogin",
                json!({ "code": code }),
                &[0],
            )
            .await?;

        #[derive(Deserialize)]
        struct LoginData {
            musicid: u64,
            musickey: String,
            #[serde(rename = "encryptUin")]
            encrypt_uin: Option<String>,
            refresh_key: Option<String>,
        }

        #[derive(Deserialize)]
        struct LoginResponse {
            data: LoginData,
        }

        let response: LoginResponse = serde_json::from_value(login_data)?;
        let credentials = response.data;

        Ok(ProviderAuthState::QQMusic {
            musicid: credentials.musicid,
            musickey: credentials.musickey,
            refresh_key: credentials.refresh_key,
            encrypt_uin: credentials.encrypt_uin,
        })
    }

    #[instrument(skip(self, redirect_url), fields(provider = "qq"))]
    async fn finalize_login_with_url(&self, redirect_url: &str) -> Result<ProviderAuthState> {
        let p_skey = self.fetch_p_skey_from_redirect(redirect_url).await?;
        let code = self.get_authorization_code(&p_skey).await?;
        self.exchange_code_for_auth_state(&code).await
    }

    async fn login_with_cookie(&self, cookies: &str) -> Result<LoginResult> {
        let cookie_map = Self::parse_cookies(cookies);
        let musicid_str = cookie_map
            .get("uin")
            .or_else(|| cookie_map.get("musicid"))
            .ok_or_else(|| {
                LoginError::InvalidCredentials("Cookie中缺少 'uin' 或 'musicid'".into())
            })?;
        let musickey = cookie_map
            .get("qqmusic_key")
            .or_else(|| cookie_map.get("musickey"))
            .ok_or_else(|| {
                LoginError::InvalidCredentials("Cookie中缺少 'qqmusic_key' 或 'musickey'".into())
            })?;

        let musicid = musicid_str
            .trim_start_matches('o')
            .parse::<u64>()
            .map_err(|_| LoginError::InvalidCredentials("无法将 'uin' 解析为数字".into()))?;

        let auth_state = ProviderAuthState::QQMusic {
            musicid,
            musickey: musickey.clone(),
            refresh_key: cookie_map.get("refresh_key").cloned(),
            encrypt_uin: cookie_map.get("encrypt_uin").cloned(),
        };

        self.login_with_auth_state(&auth_state).await
    }

    async fn login_with_auth_state(&self, auth_state: &ProviderAuthState) -> Result<LoginResult> {
        self.set_auth_state(auth_state)?;

        #[derive(Deserialize)]
        struct UserInfoResponse {
            #[serde(rename = "uin")]
            user_id: i64,
            nick: String,
            headurl: String,
        }

        let user_info_data = self
            .execute_api_request(
                "music.UserInfo.userInfoServer",
                "GetLoginUserInfo",
                json!({}),
                &[0],
            )
            .await?;

        let user_info: UserInfoResponse = serde_json::from_value(user_info_data)
            .map_err(|e| LoginError::ProviderError(format!("解析用户信息失败: {e}")))?;

        let profile = UserProfile {
            user_id: user_info.user_id,
            nickname: user_info.nick,
            avatar_url: user_info.headurl,
        };

        Ok(LoginResult {
            profile,
            auth_state: auth_state.clone(),
        })
    }

    async fn handle_cookie_login(&self, events_tx: mpsc::Sender<LoginEvent>, cookies: String) {
        let _ = events_tx.send(LoginEvent::Initiating).await;
        match self.login_with_cookie(&cookies).await {
            Ok(result) => {
                let _ = events_tx.send(LoginEvent::Success(result)).await;
            }
            Err(e) => {
                let login_error = match e {
                    LyricsHelperError::LoginFailed(login_err) => login_err,
                    other_err => LoginError::ProviderError(other_err.to_string()),
                };
                let _ = events_tx.send(LoginEvent::Failure(login_error)).await;
            }
        }
    }

    async fn handle_qr_code_login(&self, events_tx: mpsc::Sender<LoginEvent>) {
        if events_tx.send(LoginEvent::Initiating).await.is_err() {
            return;
        }

        let qr_info = match self.get_qrcode().await {
            Ok(info) => info,
            Err(e) => {
                let _ = events_tx
                    .send(LoginEvent::Failure(LoginError::ProviderError(
                        e.to_string(),
                    )))
                    .await;
                return;
            }
        };

        if events_tx
            .send(LoginEvent::QRCodeReady {
                image_data: qr_info.image_data,
            })
            .await
            .is_err()
        {
            return;
        }

        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;

            if events_tx.is_closed() {
                return;
            }

            match self.check_qrcode_status(&qr_info.qrsig).await {
                Ok(status) => match status {
                    QRCodeStatus::Confirmed { url } => {
                        let final_event = match self.finalize_login_with_url(&url).await {
                            Ok(auth_state) => match self.login_with_auth_state(&auth_state).await {
                                Ok(result) => LoginEvent::Success(result),
                                Err(e) => {
                                    let login_error = match e {
                                        LyricsHelperError::LoginFailed(le) => le,
                                        _ => LoginError::ProviderError(e.to_string()),
                                    };
                                    LoginEvent::Failure(login_error)
                                }
                            },
                            Err(e) => LoginEvent::Failure(LoginError::ProviderError(e.to_string())),
                        };
                        let _ = events_tx.send(final_event).await;
                        return;
                    }
                    QRCodeStatus::TimedOut => {
                        let _ = events_tx
                            .send(LoginEvent::Failure(LoginError::TimedOut))
                            .await;
                        return;
                    }
                    QRCodeStatus::Refused => {
                        let _ = events_tx
                            .send(LoginEvent::Failure(LoginError::UserCancelled))
                            .await;
                        return;
                    }
                    QRCodeStatus::Error(msg) => {
                        let _ = events_tx
                            .send(LoginEvent::Failure(LoginError::ProviderError(msg)))
                            .await;
                        return;
                    }
                    QRCodeStatus::WaitingForScan => {
                        if events_tx.send(LoginEvent::WaitingForScan).await.is_err() {
                            return;
                        }
                    }
                    QRCodeStatus::Scanned => {
                        if events_tx
                            .send(LoginEvent::ScannedWaitingForConfirmation)
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                },
                Err(e) => {
                    let _ = events_tx
                        .send(LoginEvent::Failure(LoginError::Network(e.to_string())))
                        .await;
                    return;
                }
            }
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
    format!("{ALBUM_COVER_BASE_URL}T002R{size_val}x{size_val}M000{album_mid}.jpg")
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
    const TEST_SONG_NUMERICAL_ID: u64 = 7_137_425;

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

        // info!("解析结果: {:#?}", full_lyrics.parsed);
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_lyrics_fallback() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();

        let result = provider
            .try_get_lyrics_fallback(TEST_SONG_NUMERICAL_ID)
            .await;

        assert!(result.is_ok(), "备用接口调用失败: {:?}", result.err());

        let full_lyrics = result.unwrap();
        assert!(!full_lyrics.parsed.lines.is_empty(), "歌词行不应为空");

        let has_full_line = full_lyrics.parsed.lines.iter().any(|line| {
            line.get_translation_by_lang("zh-Hans").is_some()
                && line.get_romanization_by_lang("ja-Latn").is_some()
        });

        assert!(has_full_line, "未能成功解析出歌词行");

        info!("✅ 备用歌词接口测试通过！");
    }

    #[tokio::test]
    #[ignore]
    async fn test_try_get_lyrics_lrc_only() {
        init_tracing();
        let provider = QQMusic::new().await.unwrap();

        let result = provider.try_get_lyrics_lrc_only(TEST_SONG_MID).await;

        assert!(result.is_ok(), "LRC 接口调用失败: {:?}", result.err());

        let full_lyrics = result.unwrap();
        assert!(!full_lyrics.parsed.lines.is_empty(), "歌词行不应为空");

        info!("✅ LRC 歌词接口测试通过！");
    }
}
