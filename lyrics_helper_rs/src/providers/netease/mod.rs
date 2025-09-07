//! 此模块实现了与网易云音乐平台进行交互的 `Provider`。
//! API 来源于 <https://github.com/NeteaseCloudMusicApiReborn/api>

use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STD};
use chrono::Utc;
use const_format::concatcp;
use cookie_store::serde::json;
use lyrics_helper_core::{
    ConversionInput, ConversionOptions, CoverSize, FullLyricsResult, InputFile, LyricFormat,
    RawLyrics, SearchResult, Track, model::generic,
};
use md5::{Digest, Md5};
use parking_lot::Mutex;
use rand::{Rng, SeedableRng, seq::SliceRandom};
use serde::Serialize;
use serde_json::json;
use tracing::trace;
use wreq::header::{CONTENT_TYPE, REFERER, USER_AGENT};

use crate::{
    converter,
    error::{LyricsHelperError, Result},
    http::HttpClient,
    model::auth::{LoginCredentials, LoginResult, ProviderAuthState, UserProfile},
    providers::{LoginProvider, Provider},
};

mod crypto;
pub mod models;

const ID_XOR_KEY_1: &[u8] = b"3go8&$8*3*3h0k(2)2";

const API_BASE_URL: &str = "https://music.163.com";
const API_INTERFACE_BASE_URL: &str = "https://interface.music.163.com";
const API_INTERFACE3_BASE_URL: &str = "https://interface3.music.163.com";

const ACCOUNT_GET_URL: &str = concatcp!(API_BASE_URL, "/api/nuser/account/get");
const REGISTER_ANONIMOUS_URL: &str = concatcp!(API_BASE_URL, "/api/register/anonimous");

const SONG_ENHANCE_PLAYER_URL_V1_PATH: &str = "/api/song/enhance/player/url/v1";
const SONG_ENHANCE_PLAYER_URL_V1_URL: &str =
    concatcp!(API_INTERFACE3_BASE_URL, "/eapi/song/enhance/player/url/v1");

const SEARCH_CLOUDSEARCH_PC_PATH: &str = "/api/cloudsearch/pc";
const SEARCH_CLOUDSEARCH_PC_URL: &str = concatcp!(API_INTERFACE_BASE_URL, "/eapi/cloudsearch/pc");

const SONG_LYRIC_V1_PATH: &str = "/api/song/lyric/v1";
const SONG_LYRIC_V1_URL: &str = concatcp!(API_INTERFACE3_BASE_URL, "/eapi/song/lyric/v1");

const ALBUM_GET_V1_URL: &str = concatcp!(API_BASE_URL, "/weapi/v1/album/");
const ARTIST_SONGS_V1_URL: &str = concatcp!(API_BASE_URL, "/weapi/v1/artist/songs");
const PLAYLIST_DETAIL_V6_URL: &str = concatcp!(API_BASE_URL, "/weapi/v6/playlist/detail");
const SONG_DETAIL_V3_URL: &str = concatcp!(API_BASE_URL, "/weapi/v3/song/detail");
const SONG_ENHANCE_PLAYER_URL_URL: &str = concatcp!(API_BASE_URL, "/weapi/song/enhance/player/url");

const SELECTED_DEVICE_IDS: &str = include_str!(concat!(env!("OUT_DIR"), "/selected_deviceids.txt"));

// TODO: 允许选择设备类型
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientType {
    PC,
    Android,
    IPhone,
}

#[derive(Debug, Clone)]
struct ClientConfig {
    client_type: ClientType,
    device_id: String,
    os_version: Option<String>,
    app_version: Option<String>,
    version_code: Option<String>,
    mobile_name: Option<String>,
    channel: Option<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            client_type: ClientType::PC,
            device_id: uuid::Uuid::new_v4().to_string(),
            os_version: Some("10".to_string()),
            app_version: Some("3.0.18".to_string()),
            version_code: None,
            mobile_name: None,
            channel: Some("netease".to_string()),
        }
    }
}

#[derive(Serialize)]
struct PayloadHeader<'a> {
    os: &'a str,
    appver: &'a str,
    osver: &'a str,
    #[serde(rename = "deviceId")]
    device_id: &'a str,
    #[serde(rename = "requestId")]
    request_id: String,
}

#[derive(Serialize)]
struct EapiPayload<'a> {
    ids: Vec<&'a str>,
    level: &'a str,
    #[serde(rename = "encodeType")]
    encode_type: &'a str,
    header: String,
}

/// 网易云音乐的客户端实现。
///
/// 内部持有了为 WEAPI 请求生成的、一次性的随机密钥。
#[derive(Debug, Clone)]
pub struct NeteaseClient {
    /// 为 WEAPI 生成的 16 位随机对称密钥。
    weapi_secret_key: String,
    /// 使用 RSA 公钥加密 `weapi_secret_key` 后得到的结果，作为请求的一部分发送。
    weapi_enc_sec_key: String,
    http_client: Arc<dyn HttpClient>,
    config: ClientConfig,
    user_profile: Arc<Mutex<Option<UserProfile>>>,
}

impl NeteaseClient {
    async fn fetch_user_profile(&self) -> Result<UserProfile> {
        let payload = json!({});
        let resp: models::AccountProfileResult = self.post_weapi(ACCOUNT_GET_URL, &payload).await?;

        if let Some(profile) = resp.profile {
            Ok(UserProfile {
                user_id: profile.user_id,
                nickname: profile.nickname,
                avatar_url: profile.avatar_url,
            })
        } else {
            Err(LyricsHelperError::LoginFailed(
                "Cookie 无效或已过期".to_string(),
            ))
        }
    }

    async fn register_anonimous(&self) -> Result<()> {
        let device_ids: Vec<&str> = SELECTED_DEVICE_IDS.lines().collect();
        if device_ids.is_empty() {
            tracing::warn!("[Netease] 未能获取到任何 Device ID，跳过游客登录。");
            return Ok(());
        }

        let mut rng = rand::rngs::StdRng::from_entropy();

        let device_id = device_ids.choose(&mut rng).unwrap_or(&"");

        let signature = cloudmusic_dll_encode_id(device_id);
        let encoded_username = BASE64_STD.encode(format!("{device_id} {signature}"));

        let payload = json!({ "username": encoded_username });

        let resp: serde_json::Value = self.post_weapi(REGISTER_ANONIMOUS_URL, &payload).await?;

        // 虽然 weapi 在这里返回 400 错误，但是 cookie 确实返回给我们了
        if resp.get("code").and_then(serde_json::Value::as_i64) == Some(400) {
            tracing::info!("[Netease] 游客登录成功！");
        } else {
            let msg = resp
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("未知错误");
            return Err(LyricsHelperError::ApiError(format!("游客登录失败: {msg}")));
        }
        Ok(())
    }

    /// 辅助函数，用于发送加密的 WEAPI 请求。
    async fn post_weapi<T: Serialize, R: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        payload: &T,
    ) -> Result<R> {
        let raw_json_data = serde_json::to_string(payload)?;

        // 第一轮 AES-CBC 加密
        let params_first_pass_base64 =
            crypto::aes_cbc_encrypt_base64(&raw_json_data, crypto::NONCE_STR, crypto::VI_STR)?;

        // 第二轮 AES-CBC 加密
        let final_params_base64 = crypto::aes_cbc_encrypt_base64(
            &params_first_pass_base64,
            &self.weapi_secret_key,
            crypto::VI_STR,
        )?;

        // 构建请求表单
        let form_data = [
            ("params", final_params_base64),
            ("encSecKey", self.weapi_enc_sec_key.clone()),
        ];

        let user_agent = get_user_agent(self.config.client_type);

        let headers = [
            (USER_AGENT.as_str(), user_agent),
            (REFERER.as_str(), API_BASE_URL),
            (CONTENT_TYPE.as_str(), "application/x-www-form-urlencoded"),
        ];

        let body_bytes = serde_urlencoded::to_string(&form_data)
            .map_err(|e| LyricsHelperError::Internal(e.to_string()))?
            .into_bytes();

        let response = self
            .http_client
            .request_with_headers(
                crate::http::HttpMethod::Post,
                url,
                &headers,
                Some(&body_bytes),
            )
            .await?;

        let response_text = response.text()?;

        trace!("weapi 原始响应内容: {}", &response_text);

        if response_text.is_empty() {
            return Err(LyricsHelperError::ApiError(
                "WEAPI 接口返回了空响应。".to_string(),
            ));
        }

        match serde_json::from_str::<R>(&response_text) {
            Ok(data) => Ok(data),
            Err(e) => {
                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&response_text)
                    && json_val.get("code").and_then(serde_json::Value::as_i64) == Some(400)
                {
                    return Err(LyricsHelperError::ApiError(
                        "网易云 API 返回 '参数错误' (code 400)。".to_string(),
                    ));
                }
                Err(LyricsHelperError::from(e))
            }
        }
    }

    /// 辅助函数，用于发送加密的 EAPI 请求。
    async fn post_eapi<T: Serialize, R: serde::de::DeserializeOwned>(
        &self,
        url_path_segment: &str,
        full_url: &str,
        payload: &T,
    ) -> Result<R> {
        let encrypted_params_hex = crypto::prepare_eapi_params(url_path_segment, payload)?;
        let form_data = [("params", encrypted_params_hex)];

        let user_agent = get_user_agent(self.config.client_type);

        let headers = [
            (USER_AGENT.as_str(), user_agent),
            (REFERER.as_str(), API_BASE_URL),
            (CONTENT_TYPE.as_str(), "application/x-www-form-urlencoded"),
        ];

        let body_bytes = serde_urlencoded::to_string(&form_data)
            .map_err(|e| LyricsHelperError::Internal(e.to_string()))?
            .into_bytes();

        let response = self
            .http_client
            .request_with_headers(
                crate::http::HttpMethod::Post,
                full_url,
                &headers,
                Some(&body_bytes),
            )
            .await?;

        let response_text = response.text()?;

        trace!("eapi 原始响应内容: {}", &response_text);
        serde_json::from_str::<R>(&response_text).map_err(LyricsHelperError::from)
    }

    fn build_eapi_header(&self) -> serde_json::Value {
        let current_time_ms = Utc::now().timestamp_millis();
        let config = &self.config;

        json!({
            "os": match config.client_type {
                ClientType::PC => "pc",
                ClientType::Android => "android",
                ClientType::IPhone => "iPhone OS",
            },
            "appver": config.app_version.as_deref().unwrap_or("8.0.0"),
            "versioncode": config.version_code.as_deref().unwrap_or("140"),
            "osver": config.os_version.as_deref().unwrap_or(""),
            "deviceId": &config.device_id,
            "mobilename": config.mobile_name.as_deref().unwrap_or(""),
            "buildver": Utc::now().timestamp().to_string(),
            "resolution": "1920x1080",
            "channel": config.channel.as_deref().unwrap_or(""),
            "requestId": format!("{}_{:04}", current_time_ms, rand::random::<u16>() % 1000),
            "__csrf": "",
        })
    }

    /// 使用 EAPI 获取歌曲链接，可用于 VIP 歌曲。
    pub async fn get_song_link_v1(&self, song_id: &str) -> Result<String> {
        let request_id = rand::thread_rng()
            .gen_range(10_000_000..100_000_000)
            .to_string();
        let header_struct = PayloadHeader {
            os: "pc",
            appver: "",
            osver: "",
            device_id: "pyncm!",
            request_id,
        };
        let header_str = serde_json::to_string(&header_struct).map_err(|e| {
            LyricsHelperError::Internal(format!("序列化 EAPI payload header 失败: {e}"))
        })?;

        let payload_struct = EapiPayload {
            ids: vec![song_id],
            level: "standard",
            encode_type: "flac",
            header: header_str,
        };

        let resp: models::SongUrlResultV1 = self
            .post_eapi(
                SONG_ENHANCE_PLAYER_URL_V1_PATH,
                SONG_ENHANCE_PLAYER_URL_V1_URL,
                &payload_struct,
            )
            .await?;

        if resp.code != 200 {
            return Err(LyricsHelperError::ApiError(format!(
                "获取播放链接失败，接口返回状态码: {}",
                resp.code
            )));
        }

        let song_data = resp
            .data
            .into_iter()
            .find(|d| d.id.to_string() == song_id)
            .ok_or(LyricsHelperError::LyricNotFound)?;

        if song_data.code == 200 {
            song_data.url.ok_or_else(|| {
                LyricsHelperError::ApiError(
                    "获取播放链接失败，可能因 VIP 或版权问题无链接。".into(),
                )
            })
        } else {
            Err(LyricsHelperError::ApiError(format!(
                "获取播放链接失败，歌曲接口返回状态码: {}",
                song_data.code
            )))
        }
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[async_trait]
impl Provider for NeteaseClient {
    fn name(&self) -> &'static str {
        "netease"
    }

    fn as_login_provider(&self) -> Option<&dyn LoginProvider> {
        Some(self)
    }

    async fn with_http_client(http_client: Arc<dyn HttpClient>) -> Result<Self>
    where
        Self: Sized,
    {
        let weapi_secret_key = crypto::create_secret_key(16);
        let weapi_enc_sec_key = crypto::rsa_encode(
            &weapi_secret_key,
            crypto::PUBKEY_STR_API,
            crypto::MODULUS_STR_API,
        )?;

        let client = Self {
            weapi_secret_key,
            weapi_enc_sec_key,
            http_client,
            config: ClientConfig::default(),
            user_profile: Arc::new(Mutex::new(None)),
        };

        if let Err(e) = client.register_anonimous().await {
            tracing::warn!("[Netease] 游客登录失败: {}", e);
        }

        Ok(client)
    }

    async fn search_songs(&self, track: &Track<'_>) -> Result<Vec<SearchResult>> {
        let title = track.title.unwrap_or_default();
        if title.is_empty() {
            return Ok(vec![]);
        }

        let keyword = track
            .artists
            .and_then(|a| a.first())
            .map_or_else(|| title.to_string(), |artists| format!("{title} {artists}"));

        // 使用 EAPI 搜索接口
        let payload = json!({
            "s": keyword,
            "type": "1", // 1 代表搜索单曲
            "limit": 30,
            "offset": 0,
            "total": true
        });

        let resp: models::SearchResult = self
            .post_eapi(
                SEARCH_CLOUDSEARCH_PC_PATH,
                SEARCH_CLOUDSEARCH_PC_URL,
                &payload,
            )
            .await?;

        let search_results = resp
            .result
            .songs
            .into_iter()
            .map(|song| SearchResult {
                title: song.name,
                artists: song
                    .artist_info
                    .into_iter()
                    .map(|a| generic::Artist {
                        id: a.id.to_string(),
                        name: a.name,
                    })
                    .collect(),
                album: Some(song.album_info.name),
                album_id: Some(song.album_info.id.to_string()),
                cover_url: song.album_info.pic_url,
                duration: Some(song.duration),
                provider_id: song.id.to_string(),
                provider_name: self.name().to_string(),
                provider_id_num: Some(song.id),
                language: None,
                ..Default::default()
            })
            .collect();

        Ok(search_results)
    }

    async fn get_full_lyrics(&self, id: &str) -> Result<FullLyricsResult> {
        let header = self.build_eapi_header();

        let mut payload = json!({ "id": id, "cp": "false", "lv": "0", "kv": "0", "tv": "0", "rv": "0", "yv": "0", "ytv": "0", "yrv": "0", "csrf_token": "" });
        payload["header"] = header;

        let resp: models::LyricResult = self
            .post_eapi(SONG_LYRIC_V1_PATH, SONG_LYRIC_V1_URL, &payload)
            .await?;

        if resp.code != 200 {
            return Err(LyricsHelperError::ApiError(format!(
                "网易云歌词接口返回非 200 状态码: {}",
                resp.code
            )));
        }

        let yrc_content = resp.yrc.as_ref().and_then(|d| {
            if d.lyric.is_empty() {
                None
            } else {
                Some(d.lyric.clone())
            }
        });
        let lrc_content = resp.lrc.as_ref().and_then(|d| {
            if d.lyric.is_empty() {
                None
            } else {
                Some(d.lyric.clone())
            }
        });
        let tlyric_content = resp.tlyric.as_ref().and_then(|d| {
            if d.lyric.is_empty() {
                None
            } else {
                Some(d.lyric.clone())
            }
        });
        let romalrc_content = resp.romalrc.as_ref().and_then(|d| {
            if d.lyric.is_empty() {
                None
            } else {
                Some(d.lyric.clone())
            }
        });

        let (main_format, main_content) = if let Some(content) = yrc_content {
            (LyricFormat::Yrc, content)
        } else if let Some(content) = lrc_content {
            (LyricFormat::Lrc, content)
        } else {
            return Err(LyricsHelperError::LyricNotFound);
        };

        let main_lyric_input = InputFile {
            content: main_content.clone(),
            format: main_format,
            language: None,
            filename: None,
        };

        let mut translations = Vec::new();
        if let Some(content) = &tlyric_content {
            translations.push(InputFile {
                content: content.clone(),
                format: LyricFormat::Lrc,
                language: Some("zh-Hans".to_string()),
                filename: None,
            });
        }

        let mut romanizations = Vec::new();
        if let Some(content) = romalrc_content {
            romanizations.push(InputFile {
                content,
                format: LyricFormat::Lrc,
                language: Some("ja-Latn".to_string()),
                filename: None,
            });
        }

        let conversion_input = ConversionInput {
            main_lyric: main_lyric_input,
            translations,
            romanizations,
            target_format: LyricFormat::Lrc,
            user_metadata_overrides: None,
        };

        let mut parsed_data =
            converter::parse_and_merge(&conversion_input, &ConversionOptions::default())?;
        parsed_data.source_name = self.name().to_string();

        let raw_lyrics = RawLyrics {
            format: main_format.to_string(),
            content: main_content,
            translation: tlyric_content,
        };

        Ok(FullLyricsResult {
            parsed: parsed_data,
            raw: raw_lyrics,
        })
    }

    async fn get_album_info(&self, album_id: &str) -> Result<generic::Album> {
        let url = format!("{ALBUM_GET_V1_URL}{album_id}?csrf_token=");
        let payload = json!({ "csrf_token": "" });

        let response: models::AlbumResult = self.post_weapi(&url, &payload).await?;

        if response.code != 200 {
            let error_message = match response.code {
                -460 => "请求被拒绝 (错误码-460)，可能为 IP 限制".to_string(),
                _ => format!("获取专辑信息失败，接口返回错误码: {}", response.code),
            };
            return Err(LyricsHelperError::ApiError(error_message));
        }

        let netease_album = response.album.ok_or_else(|| {
            LyricsHelperError::ApiError(format!("未能找到ID为 '{album_id}' 的专辑信息。"))
        })?;

        let songs_to_map = if response.songs.is_empty() {
            netease_album.songs
        } else {
            response.songs
        };

        let tracks = songs_to_map.into_iter().map(Into::into).collect();

        let album_info = generic::Album {
            id: netease_album.id.to_string(),
            name: netease_album.name,
            artists: Some(netease_album.artists.into_iter().map(Into::into).collect()),
            songs: Some(tracks),
            description: None,
            release_date: None,
            cover_url: netease_album.pic_url,
            provider_id: netease_album.id.to_string(),
        };

        Ok(album_info)
    }

    async fn get_album_songs(
        &self,
        album_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<generic::Song>> {
        let url = format!("{ALBUM_GET_V1_URL}{album_id}");
        let payload = json!({ "csrf_token": "" });

        let resp: models::AlbumContentResult = self.post_weapi(&url, &payload).await?;

        if resp.code != 200 {
            return Err(LyricsHelperError::ApiError(format!(
                "获取专辑歌曲失败，接口返回错误码: {}",
                resp.code
            )));
        }

        // 这个接口不支持服务端分页
        let songs = resp
            .songs
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .map(Into::into)
            .collect();

        Ok(songs)
    }

    async fn get_singer_songs(
        &self,
        singer_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<generic::Song>> {
        let payload = json!({
            "id": singer_id,
            "private_cloud": "true",
            "work_type": 1,
            "order": "hot", // 默认为热门排序，可根据需求设为"time"
            "offset": offset,
            "limit": limit,
            "csrf_token": ""
        });

        let resp: models::ArtistSongsResult =
            self.post_weapi(ARTIST_SONGS_V1_URL, &payload).await?;

        if resp.code != 200 {
            return Err(LyricsHelperError::ApiError(format!(
                "获取歌手歌曲失败，接口返回错误码: {}",
                resp.code
            )));
        }

        let songs = resp.songs.into_iter().map(Into::into).collect();

        Ok(songs)
    }

    async fn get_playlist(&self, playlist_id: &str) -> Result<generic::Playlist> {
        let payload = json!({ "id": playlist_id, "n": 1000, "s": "8", "csrf_token": "" });

        let resp: models::PlaylistResult =
            self.post_weapi(PLAYLIST_DETAIL_V6_URL, &payload).await?;

        let netease_playlist = resp.playlist;
        let generic_playlist = generic::Playlist {
            id: netease_playlist.id.to_string(),
            name: netease_playlist.name,
            cover_url: Some(netease_playlist.cover_img_url),
            creator_name: Some(netease_playlist.creator.nickname),
            description: netease_playlist.description,
            songs: Some(
                netease_playlist
                    .tracks
                    .into_iter()
                    .map(Into::into)
                    .collect(),
            ),
        };

        Ok(generic_playlist)
    }

    async fn get_song_info(&self, song_id: &str) -> Result<generic::Song> {
        let c_field = json!([{"id": song_id}]).to_string();
        let payload = json!({ "c": c_field, "csrf_token": "" });

        let resp: models::DetailResult = self.post_weapi(SONG_DETAIL_V3_URL, &payload).await?;

        let netease_song = resp
            .songs
            .into_iter()
            .next()
            .ok_or(LyricsHelperError::LyricNotFound)?;

        Ok(netease_song.into())
    }

    async fn get_song_link(&self, song_id: &str) -> Result<String> {
        let payload = json!({ "ids": format!("[{}]", song_id), "br": 999_000, "csrf_token": "" });

        let resp: models::SongUrlResult = self
            .post_weapi(SONG_ENHANCE_PLAYER_URL_URL, &payload)
            .await?;

        let song_url_data = resp
            .data
            .into_iter()
            .find(|d| d.id.to_string() == song_id)
            .ok_or(LyricsHelperError::LyricNotFound)?;

        if song_url_data.code == 200 {
            song_url_data.url.ok_or_else(|| {
                LyricsHelperError::ApiError(
                    "获取播放链接失败，可能因 VIP 或版权问题无链接。".into(),
                )
            })
        } else {
            Err(LyricsHelperError::ApiError(format!(
                "获取播放链接失败，接口返回状态码: {}",
                song_url_data.code
            )))
        }
    }

    /// 根据专辑 ID 获取专辑封面的 URL。
    ///
    /// # 参数
    /// * `album_id` - 网易云音乐的专辑 ID。
    /// * `size` - 期望的封面尺寸。
    ///
    /// # 返回
    /// 一个 `Result`，成功时包含一个封面图片的 URL 字符串。
    async fn get_album_cover_url(&self, album_id: &str, size: CoverSize) -> Result<String> {
        let album_info = self.get_album_info(album_id).await?;

        let base_cover_url = album_info.cover_url.ok_or_else(|| {
            LyricsHelperError::ApiError(format!("专辑 (ID: {album_id}) 信息中未找到封面URL。"))
        })?;

        let size_param = match size {
            CoverSize::Thumbnail => "150y150",
            CoverSize::Medium => "400y400",
            CoverSize::Large => "800y800",
        };

        Ok(format!("{base_cover_url}?param={size_param}"))
    }
}

impl From<models::Song> for generic::Song {
    fn from(song: models::Song) -> Self {
        Self {
            id: song.id.to_string(),
            name: song.name,
            artists: song
                .artist_info
                .into_iter()
                .map(generic::Artist::from)
                .collect(),
            duration: Some(std::time::Duration::from_millis(song.duration)),
            album: Some(song.album_info.name),
            cover_url: song.album_info.pic_url,
            provider_id: song.id.to_string(),
            album_id: Some(song.album_info.id.to_string()),
        }
    }
}

impl From<models::Artist> for generic::Artist {
    fn from(artist: models::Artist) -> Self {
        Self {
            id: artist.id.to_string(),
            name: artist.name,
        }
    }
}

fn cloudmusic_dll_encode_id(device_id: &str) -> String {
    let device_id_bytes = device_id.as_bytes();
    let mut xored_bytes = Vec::with_capacity(device_id_bytes.len());

    for (i, byte) in device_id_bytes.iter().enumerate() {
        xored_bytes.push(byte ^ ID_XOR_KEY_1[i % ID_XOR_KEY_1.len()]);
    }

    let mut hasher = Md5::new();
    hasher.update(&xored_bytes);
    let digest = hasher.finalize();

    BASE64_STD.encode(digest)
}

const fn get_user_agent(client_type: ClientType) -> &'static str {
    match client_type {
        ClientType::PC => {
            "Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Safari/537.36 Chrome/91.0.4472.164 NeteaseMusicDesktop/3.0.18.203152"
        }
        ClientType::Android => {
            "NeteaseMusic/9.1.65.240927161425(9001065);Dalvik/2.1.0 (Linux; U; Android 14; 23013RK75C Build/UKQ1.230804.001)"
        }
        ClientType::IPhone => "NeteaseMusic 9.0.90/5038 (iPhone; iOS 16.2; zh_CN)",
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl LoginProvider for NeteaseClient {
    async fn login(&self, credentials: &LoginCredentials<'_>) -> Result<LoginResult> {
        let LoginCredentials::NeteaseByCookie { music_u } = credentials else {
            return Err(LyricsHelperError::LoginFailed("无效的凭据类型".to_string()));
        };

        let mut temp_store = cookie_store::CookieStore::default();
        let cookie_str = format!("MUSIC_U={music_u}");
        let url = API_BASE_URL
            .parse()
            .map_err(|e| LyricsHelperError::Internal(format!("无法解析API基础URL: {e}")))?;

        let raw_cookie = cookie_store::RawCookie::parse(cookie_str)
            .map_err(|e| LyricsHelperError::Internal(format!("Cookie解析失败: {e}")))?;
        temp_store.store_response_cookies(std::iter::once(raw_cookie), &url);

        let mut writer = Vec::new();
        json::save_incl_expired_and_nonpersistent(&temp_store, &mut writer)
            .map_err(|e| LyricsHelperError::Internal(format!("Cookie序列化失败: {e}")))?;

        let cookie_json = String::from_utf8(writer)
            .map_err(|e| LyricsHelperError::Internal(format!("Cookie JSON转换UTF-8失败: {e}")))?;

        self.http_client.set_cookies(&cookie_json)?;

        let user_profile = self.fetch_user_profile().await?;
        *self.user_profile.lock() = Some(user_profile.clone());

        Ok(LoginResult {
            profile: user_profile,
            auth_state: ProviderAuthState::Netease,
        })
    }

    async fn verify_session(&self) -> Result<()> {
        match self.fetch_user_profile().await {
            Ok(profile) => {
                *self.user_profile.lock() = Some(profile);
                Ok(())
            }
            Err(e) => {
                *self.user_profile.lock() = None;
                Err(e)
            }
        }
    }

    fn set_auth_state(&self, auth_state: &ProviderAuthState) -> Result<()> {
        if matches!(auth_state, ProviderAuthState::Netease) {
            *self.user_profile.lock() = None;
            Ok(())
        } else {
            Err(LyricsHelperError::Internal(
                "无效的认证状态类型".to_string(),
            ))
        }
    }

    fn get_auth_state(&self) -> Option<ProviderAuthState> {
        if self.user_profile.lock().is_some() {
            Some(ProviderAuthState::Netease)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    const TEST_SONG_NAME: &str = "明天见";
    const TEST_SINGER_NAME: &str = "小蓝背心";
    const TEST_SONG_ID: &str = "2116402049";
    const TEST_ALBUM_ID: &str = "182985259";

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
        let provider = NeteaseClient::new().await.unwrap();

        let search_track = Track {
            title: Some(TEST_SONG_NAME),
            artists: Some(&[TEST_SINGER_NAME]),
            album: None,
            duration: None,
        };
        let results = provider.search_songs(&search_track).await.unwrap();

        println!(
            "为 '{}({})' 找到 {} 首歌曲",
            TEST_SONG_NAME,
            TEST_SINGER_NAME,
            results.len()
        );
        assert!(!results.is_empty(), "搜索结果不应为空");

        let target_found = results.iter().any(|s| {
            s.title == TEST_SONG_NAME && s.artists.iter().any(|a| a.name == TEST_SINGER_NAME)
        });
        assert!(
            target_found,
            "在搜索结果中未找到目标歌曲 '{TEST_SONG_NAME} - {TEST_SINGER_NAME}'"
        );
        println!("✅ 测试 search_songs 通过");
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_lyrics() {
        init_tracing();

        let provider = NeteaseClient::new().await.unwrap();
        let lyrics = provider.get_lyrics("33894312").await.unwrap();

        assert!(!lyrics.lines.is_empty(), "解析后的歌词行列表不应为空");
        println!("✅ 测试 get_lyrics 通过，格式: {:?}", lyrics.source_format);
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_album_info() {
        init_tracing();

        let provider = NeteaseClient::new().await.unwrap();
        let album_info = provider.get_album_info(TEST_ALBUM_ID).await.unwrap();

        assert_eq!(album_info.name, "明天见");
        let artists = album_info.artists.expect("专辑应有歌手信息");
        assert_eq!(artists[0].name, TEST_SINGER_NAME);
        println!("✅ 测试 get_album_info 通过: 专辑为 '{}'", album_info.name);
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_playlist() {
        init_tracing();

        const NEW_SONGS_PLAYLIST_ID: &str = "3779629";
        let provider = NeteaseClient::new().await.unwrap();
        let playlist = provider.get_playlist(NEW_SONGS_PLAYLIST_ID).await.unwrap();

        let songs = playlist.songs.as_ref().expect("歌单应包含歌曲列表");
        println!(
            "✅ 测试 get_playlist 通过: 歌单 '{}' 包含 {} 首歌曲。",
            playlist.name,
            songs.len()
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_song_info() {
        init_tracing();

        let provider = NeteaseClient::new().await.unwrap();
        let song = provider.get_song_info(TEST_SONG_ID).await.unwrap();

        assert_eq!(song.name, TEST_SONG_NAME);
        assert_eq!(song.artists[0].name, TEST_SINGER_NAME);
        println!(
            "✅ 测试 get_song_info 通过: 歌曲为 '{}' - '{}'",
            song.name, song.artists[0].name
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_song_link() {
        init_tracing();

        let provider = NeteaseClient::new().await.unwrap();
        let link_result = provider.get_song_link(TEST_SONG_ID).await;

        match link_result {
            Ok(link) => {
                assert!(link.starts_with("http"), "返回的链接应以 http/https 开头");
                println!("✅ 测试 get_song_link 通过: 获取到链接: {link}");
            }
            Err(e) => {
                if let LyricsHelperError::ApiError(msg) = e {
                    println!(
                        "✅ 测试 get_song_link 通过: 已正确处理接口错误 (如VIP或版权问题): {msg}"
                    );
                } else {
                    panic!("测试 get_song_link 因意外的非接口错误而失败: {e:?}");
                }
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_album_cover_url() {
        init_tracing();

        let provider = NeteaseClient::new().await.unwrap();
        let album_id = "182985259";

        let medium_cover_url = provider
            .get_album_cover_url(album_id, CoverSize::Medium)
            .await
            .expect("获取中等尺寸封面失败");

        assert!(medium_cover_url.contains("?param=400y400"));
        println!("✅ 中等尺寸封面URL正确: {medium_cover_url}");

        let large_cover_url = provider
            .get_album_cover_url(album_id, CoverSize::Large)
            .await
            .expect("获取大尺寸封面失败");

        assert!(large_cover_url.contains("?param=800y800"));
        println!("✅ 大尺寸封面URL正确: {large_cover_url}");

        let invalid_id_result = provider
            .get_album_cover_url("99999999999999999999999999999", CoverSize::Medium)
            .await;
        assert!(invalid_id_result.is_err(), "无效的专辑ID应该返回错误");
        if let Err(e) = invalid_id_result {
            // 网易云音乐在这里似乎会返回一个两个JSON拼接在一起的畸形响应
            assert!(
                matches!(
                    e,
                    LyricsHelperError::ApiError(_) | LyricsHelperError::JsonParse(_)
                ),
                "错误类型不是预期的 ApiError 或 JsonParse，而是: {e:?}"
            );
            println!("✅ 成功捕获到预期的 ApiError 或 JsonParse 错误: {e}");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_singer_songs() {
        init_tracing();

        let provider = NeteaseClient::new().await.unwrap();

        let singer_id = "12138269";
        let limit = 5;
        let offset = 0;

        let songs_result = provider.get_singer_songs(singer_id, limit, offset).await;

        assert!(songs_result.is_ok(), "获取歌手歌曲列表不应失败");
        let songs = songs_result.unwrap();

        println!(
            "✅ 测试 get_singer_songs 通过: 为歌手ID '{}' 获取到 {} 首歌曲。",
            singer_id,
            songs.len()
        );

        assert_eq!(
            songs.len(),
            limit as usize,
            "返回的歌曲数量应与请求的limit一致"
        );

        // 验证第一页和第二页的内容不同
        let offset_page2 = 5;
        let songs_page2_result = provider
            .get_singer_songs(singer_id, limit, offset_page2)
            .await;

        assert!(songs_page2_result.is_ok(), "获取歌手歌曲列表第二页不应失败");
        let songs_page2 = songs_page2_result.unwrap();

        assert!(!songs_page2.is_empty(), "第二页应有歌曲");
        assert_ne!(
            songs.first().unwrap().id,
            songs_page2.first().unwrap().id,
            "第一页和第二页的歌曲内容应该不同"
        );
        println!("✅ 歌手歌曲分页(offset)测试通过。");
    }

    #[tokio::test]
    #[ignore]
    async fn test_get_album_songs() {
        init_tracing();

        let provider = NeteaseClient::new().await.unwrap();

        let album_id = "182985259";

        let limit1 = 2;
        let offset1 = 0;
        let songs1_result = provider.get_album_songs(album_id, limit1, offset1).await;

        assert!(songs1_result.is_ok(), "获取专辑歌曲第一页不应失败");
        let songs1 = songs1_result.unwrap();

        assert_eq!(songs1.len(), limit1 as usize, "第一页应返回2首歌曲");
        println!(
            "✅ 测试 get_album_songs 通过: 成功获取专辑 '{}' 的前 {} 首歌曲。",
            album_id,
            songs1.len()
        );

        let limit2 = 2;
        let offset2 = 2;
        let songs2_result = provider.get_album_songs(album_id, limit2, offset2).await;

        assert!(songs2_result.is_ok(), "获取专辑歌曲第二页不应失败");
        let songs2 = songs2_result.unwrap();

        assert_eq!(songs2.len(), 1, "第二页应返回剩下的1首歌曲");

        assert_ne!(
            songs1.first().unwrap().id,
            songs2.first().unwrap().id,
            "分页功能验证失败"
        );
        println!("✅ 专辑分页测试通过。");
    }

    // 没开 VIP，不测了
    // #[tokio::test]
    // #[ignore]
    // async fn test_get_song_link_v1_vip() {
    //     init_tracing();
    //     const VIP_SONG_ID: &str = "1847975477";

    //     let cookie =
    //         std::env::var("NETEASE_COOKIE").expect("请设置 NETEASE_COOKIE 环境变量");

    //     let provider = NeteaseClient::new_with_cookie(cookie).unwrap();

    //     let link_result = provider.get_song_link_v1(VIP_SONG_ID).await;

    //     match link_result {
    //         Ok(link) => {
    //             assert!(link.starts_with("http"), "返回的链接应以 http/https 开头");
    //             println!(
    //                 "✅ 测试 get_song_link_v1 (VIP) 通过: 获取到链接: {}",
    //                 &link
    //             );
    //         }
    //         Err(e) => {
    //             panic!("获取 VIP 歌曲链接失败: {e:?}");
    //         }
    //     }
    // }

    #[tokio::test]
    #[ignore]
    async fn test_anonymous_login_on_creation() {
        init_tracing();

        let provider_result = NeteaseClient::new().await;
        let provider = provider_result.unwrap();

        let cookies_json_result = provider.http_client.get_cookies();
        let cookies_json = cookies_json_result.unwrap();

        println!("获取到的 Cookies: {cookies_json}");

        assert!(
            !cookies_json.is_empty() && cookies_json != "[]" && cookies_json != "{}",
            "游客登录后，Cookie 罐不应该是空的"
        );

        assert!(cookies_json.contains("NMTID"), "应该包含游客令牌 'NMTID'");
    }

    #[tokio::test]
    #[ignore]
    async fn test_login_by_cookie() {
        init_tracing();
        let music_u_cookie = env::var("NETEASE_MUSIC_U");
        if music_u_cookie.is_err() {
            println!("未找到环境变量 NETEASE_MUSIC_U。");
            return;
        }

        let music_u_cookie = music_u_cookie.unwrap();

        let provider = NeteaseClient::new().await.expect("创建 NeteaseClient 失败");

        let credentials = LoginCredentials::NeteaseByCookie {
            music_u: &music_u_cookie,
        };

        let login_result = provider.login(&credentials).await;

        assert!(
            login_result.is_ok(),
            "Cookie 登录不应该失败！错误: {:?}",
            login_result.err()
        );
        let profile = login_result.unwrap();

        assert!(
            !profile.profile.nickname.is_empty(),
            "登录成功后，返回的用户信息里应该有昵称！"
        );

        println!("✅ 成功以 '{}' 的身份登录！", profile.profile.nickname);
    }
}
