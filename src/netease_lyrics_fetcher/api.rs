// 导入同级模块 crypto 中的参数准备函数
use crate::netease_lyrics_fetcher::crypto::prepare_eapi_params;
// 导入同级模块 error 中定义的错误类型和 Result 别名
use crate::netease_lyrics_fetcher::error::{NeteaseError, Result};
// 导入同级模块 neteasemodel 中定义的 API 响应数据结构
use crate::netease_lyrics_fetcher::neteasemodel::{LyricResponse, SearchResponse};
// 导入 reqwest 库的相关组件，用于发送 HTTP 请求和处理头部信息
use reqwest::Client;
use reqwest::header::{CONTENT_TYPE, COOKIE, REFERER, USER_AGENT};
// 导入 serde::Serialize 特征，用于将请求体序列化为 JSON
use serde::Serialize;
// 导入标准库的 HashMap 用于构建请求表单数据和 Cookie
use std::collections::HashMap;
// 导入 SystemTime 和 UNIX_EPOCH 用于生成时间戳 (EAPI Cookie 可能需要)
use std::time::{SystemTime, UNIX_EPOCH};
// 导入 rand::Rng 用于生成随机数 (EAPI Cookie 可能需要)
use rand::Rng;

// 网易云音乐 API 的基础 URL
const BASE_URL_NETEASE: &str = "https://music.163.com";
// 默认的 User-Agent，用于模拟浏览器行为 (主要用于 WEAPI 或旧的 GET API)
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36";
// EAPI 请求专用的 User-Agent，模拟移动端或特定客户端
const EAPI_USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 9; PCT-AL10) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/70.0.3538.64 HuaweiBrowser/10.0.3.311 Mobile Safari/537.36";

// WEAPI 加密中使用的 RSA 公钥指数 (通常是 "010001" 即 65537)
const PUBKEY_STR_API: &str = "010001";
// WEAPI 加密中使用的 RSA 公钥模数 (一个很长的十六进制字符串)
const MODULUS_STR_API: &str = "00e0b509f6259df8642dbc35662901477df22677ec152b5ff68ace615bb7b725152b3ab17a876aea8a5aa76d2e417629ec4ee341f56135fccf695280104e0312ecbda92557c93870114af6c9d05c4f7f0c3685b7a46bee255932575cce10b424d813cfe4875d3e82047b97ddef52741d546b8e289dc6935b3ece0462db0a22b8e7";

/// `NeteaseClient` 结构体，封装了与网易云音乐 API 交互所需的状态和方法。
#[derive(Debug, Clone)]
pub struct NeteaseClient {
    http_client: Client,       // reqwest HTTP 客户端实例
    weapi_secret_key: String,  // WEAPI 请求用的对称密钥 (AES key, 16字节, 随机生成)
    weapi_enc_sec_key: String, // WEAPI 请求用的 `encSecKey` (weapi_secret_key 经过 RSA 加密后的值)
}

/// 枚举，定义了网易云音乐搜索的类型 (目前只有歌曲搜索)。
#[derive(Debug, Clone, Copy)]
pub enum NeteaseSearchType {
    Song, // 搜索歌曲
}

impl NeteaseSearchType {
    /// 返回搜索类型对应的 API 参数字符串。
    fn api_type_str(&self) -> &'static str {
        match self {
            NeteaseSearchType::Song => "1",
        }
    }
}

impl NeteaseClient {
    /// 创建一个新的 `NeteaseClient` 实例。
    /// 在创建过程中，会生成 WEAPI 所需的随机对称密钥和 RSA 加密的 `encSecKey`。
    pub fn new() -> Result<Self> {
        // 构建 reqwest HTTP 客户端，设置 User-Agent 和启用 Cookie 存储
        let http_client = Client::builder()
            .user_agent(DEFAULT_USER_AGENT)
            .cookie_store(true) // 启用 cookie 支持，某些 API 可能需要
            .build()?; // 如果构建失败，返回 reqwest::Error，会被转换为 NeteaseError::Network

        // 生成 WEAPI 使用的16字节随机密钥 (AES-128-CBC 的密钥)
        let weapi_secret_key = crate::netease_lyrics_fetcher::crypto::create_secret_key(16);
        // 使用 RSA 公钥 (PUBKEY_STR_API, MODULUS_STR_API) 加密上面生成的随机密钥，得到 encSecKey
        let weapi_enc_sec_key = crate::netease_lyrics_fetcher::crypto::rsa_encode(
            &weapi_secret_key,
            PUBKEY_STR_API,
            MODULUS_STR_API,
        )
        .map_err(|e| NeteaseError::Crypto(format!("生成encSecKey失败: {}", e)))?;

        Ok(Self {
            http_client,
            weapi_secret_key,
            weapi_enc_sec_key,
        })
    }

    /// 异步发送一个 WEAPI POST 请求。
    /// WEAPI 请求参数需要经过两轮 AES CBC 加密。
    ///
    /// # Arguments
    /// * `url` - 目标 WEAPI 端点的完整 URL。
    /// * `payload` - 需要发送的原始请求数据 (会被序列化为 JSON 并加密)。
    ///
    /// # Returns
    /// `Result<reqwest::Response>` - HTTP 响应。
    async fn post_weapi_async<T: Serialize>(
        &self,
        url: &str,
        payload: &T,
    ) -> Result<reqwest::Response> {
        // 1. 将原始请求数据序列化为 JSON 字符串
        let raw_json_data = serde_json::to_string(payload)?;

        // 2. 第一轮 AES CBC 加密：使用固定的密钥 (NONCE_STR) 和 IV (VI_STR)
        let params_first_pass = crate::netease_lyrics_fetcher::crypto::aes_cbc_encrypt(
            &raw_json_data,
            crate::netease_lyrics_fetcher::crypto::NONCE_STR, // crypto 模块中定义的固定密钥
            crate::netease_lyrics_fetcher::crypto::VI_STR,    // crypto 模块中定义的固定 IV
        )?;

        // 3. 第二轮 AES CBC 加密：使用客户端实例中存储的随机密钥 (self.weapi_secret_key) 和固定 IV
        let final_params = crate::netease_lyrics_fetcher::crypto::aes_cbc_encrypt(
            &params_first_pass,
            &self.weapi_secret_key,
            crate::netease_lyrics_fetcher::crypto::VI_STR,
        )?;

        // 4. 构建表单数据
        let mut form_data = HashMap::new();
        form_data.insert("params", final_params); // 加密后的请求参数
        form_data.insert("encSecKey", self.weapi_enc_sec_key.clone()); // RSA 加密的对称密钥

        // 5. 发送 POST 请求
        let response = self
            .http_client
            .post(url)
            .header(REFERER, BASE_URL_NETEASE) // 设置 Referer 头部
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded") // 设置 Content-Type
            .form(&form_data) // 设置表单数据
            .send()
            .await?;

        Ok(response)
    }

    /// 异步发送一个 EAPI POST 请求。
    /// EAPI 请求参数需要经过特定的加密流程（MD5, AES ECB）。
    ///
    /// # Arguments
    /// * `url_path_segment` - API 的路径部分 (例如 "/api/song/lyric/v1")，用于加密参数。
    /// * `full_url` - 目标 EAPI 端点的完整 URL。
    /// * `payload` - 需要发送的原始请求数据。
    ///
    /// # Returns
    /// `Result<reqwest::Response>` - HTTP 响应。
    async fn post_eapi_async<T: Serialize>(
        &self,
        url_path_segment: &str, // API 路径，如 "/api/cloudsearch/pc"
        full_url: &str,         // 完整的请求 URL
        payload: &T,            // 原始请求参数
    ) -> Result<reqwest::Response> {
        // 1. 准备 EAPI 参数：调用 crypto 模块的函数进行加密
        //    这个函数会处理 JSON 序列化、拼接特定字符串、MD5 哈希、再拼接，最后进行 AES ECB 加密
        let encrypted_params_hex = prepare_eapi_params(url_path_segment, payload)?;

        // 2. 构建表单数据
        let mut form_data = HashMap::new();
        form_data.insert("params", encrypted_params_hex); // 加密后的参数

        // 3. 构造 EAPI 请求所需的 Cookie
        //    这些 Cookie 字段模拟了官方客户端的行为
        let current_time_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let random_suffix: u32 = rand::rng().random_range(0..1000);

        let mut cookie_header_map = HashMap::new();
        cookie_header_map.insert("__csrf", "".to_string()); // CSRF token (通常为空)
        cookie_header_map.insert("os", "pc".to_string()); // 操作系统
        cookie_header_map.insert("appver", "8.0.0".to_string()); // 应用版本
        cookie_header_map.insert("buildver", (current_time_ms / 1000).to_string()); // 构建版本 (秒级时间戳)
        cookie_header_map.insert("channel", "".to_string()); //渠道
        cookie_header_map.insert(
            "requestId",
            format!("{}_{:04}", current_time_ms, random_suffix),
        ); // 请求ID
        // MUSIC_U 是登录凭证，如果需要登录才能访问的接口，这里需要填入有效值
        cookie_header_map.insert("MUSIC_U", "".to_string());
        cookie_header_map.insert("resolution", "1920x1080".to_string()); // 分辨率
        cookie_header_map.insert("versioncode", "140".to_string()); // 版本代码

        // 将 Cookie Map 转换为字符串
        let cookie_str = cookie_header_map
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("; ");

        // 4. 发送 POST 请求
        let response = self
            .http_client
            .post(full_url)
            .header(REFERER, BASE_URL_NETEASE) // 设置 Referer
            .header(USER_AGENT, EAPI_USER_AGENT) // 使用 EAPI 特定的 User-Agent
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded") // Content-Type
            .header(COOKIE, cookie_str) // 设置 Cookie
            .form(&form_data) // 设置表单数据
            .send()
            .await?;
        Ok(response)
    }

    /// 使用 WEAPI 接口获取指定歌曲 ID 的歌词。
    pub async fn fetch_lyrics_weapi(&self, song_id: i64) -> Result<LyricResponse> {
        let url = format!("{}/weapi/song/lyric?csrf_token=", BASE_URL_NETEASE); // WEAPI 歌词接口 URL
        log::info!("[NeteaseAPI WEAPI] 正在获取歌词，歌曲ID: {}", song_id);

        // 构建 WEAPI 请求的 payload
        let mut payload = HashMap::new();
        payload.insert("id", song_id.to_string());
        payload.insert("os", "pc".to_string());
        payload.insert("lv", "-1".to_string());
        payload.insert("kv", "-1".to_string());
        payload.insert("tv", "-1".to_string());
        payload.insert("rv", "-1".to_string());
        payload.insert("csrf_token", "".to_string()); // CSRF token (通常为空)

        // 发送 WEAPI 请求并获取响应
        let response = self.post_weapi_async(&url, &payload).await?;
        // 将 JSON 响应反序列化为 LyricResponse 结构体
        let lyric_result: LyricResponse = response.json().await?;

        // 检查 API 返回的状态码
        if lyric_result.code != 200 {
            // 200 表示成功
            log::warn!(
                "[NeteaseAPI WEAPI] API错误，歌曲ID {}: 代码 {}",
                song_id,
                lyric_result.code
            );
            return Err(NeteaseError::ApiError {
                code: lyric_result.code,
                message: Some(format!("获取歌词失败，歌曲ID: {}", song_id)),
            });
        }
        // 检查是否明确无歌词
        if lyric_result.nolyric {
            log::info!("[NeteaseAPI WEAPI] 歌曲无歌词，歌曲ID: {}", song_id);
            return Err(NeteaseError::NoLyrics);
        }

        log::info!("[NeteaseAPI WEAPI] 成功获取歌词，歌曲ID: {}.", song_id);
        // 记录获取到的歌词类型
        if lyric_result
            .lrc
            .as_ref()
            .and_then(|l| l.lyric.as_ref())
            .is_some()
        {
            log::info!("[NeteaseAPI WEAPI] 找到LRC歌词");
        }
        if lyric_result
            .klyric
            .as_ref()
            .and_then(|k| k.lyric.as_ref())
            .is_some()
        {
            log::info!("[NeteaseAPI WEAPI] 找到逐字歌词 (klyric)");
        }
        if lyric_result.yrc.is_some() {
            // yrc 字段通常包含 YRC 格式的逐字歌词
            if let Some(yrc_val) = &lyric_result.yrc {
                // yrc 字段的值是一个 JSON 对象，其 "lyric" 字段包含 YRC 字符串
                if yrc_val
                    .as_object()
                    .and_then(|o| o.get("lyric")?.as_str())
                    .is_some_and(|s| !s.is_empty())
                {
                    log::info!("[NeteaseAPI WEAPI] 找到YRC歌词");
                } else {
                    log::info!("[NeteaseAPI WEAPI] 找到YRC歌词但内容为空");
                }
            }
        }
        Ok(lyric_result)
    }

    /// 使用 EAPI 接口获取指定歌曲 ID 的歌词。
    /// EAPI 通常能返回更丰富的歌词数据，包括 YRC 格式的逐字歌词。
    pub async fn fetch_lyrics_eapi(&self, song_id: i64) -> Result<LyricResponse> {
        let url_path = "/api/song/lyric/v1"; // EAPI 歌词接口的路径部分
        // EAPI 的主机名可能与 WEAPI 不同，这里使用 interface3.music.163.com
        let full_url = "https://interface3.music.163.com/eapi/song/lyric/v1".to_string();
        log::info!("[NeteaseAPI EAPI] 正在获取歌词，歌曲ID: {}", song_id);

        // 构建 EAPI 请求的 payload
        let mut payload = HashMap::new();
        payload.insert("id", song_id.to_string());
        payload.insert("cp", "false".to_string());
        payload.insert("lv", "0".to_string());
        payload.insert("kv", "0".to_string());
        payload.insert("tv", "0".to_string());
        payload.insert("rv", "0".to_string());
        payload.insert("yv", "0".to_string());
        payload.insert("ytv", "0".to_string());
        payload.insert("yrv", "0".to_string());
        payload.insert("csrf_token", "".to_string());

        // 发送 EAPI 请求并获取响应
        let response = self.post_eapi_async(url_path, &full_url, &payload).await?;
        let lyric_result: LyricResponse = response.json().await?;

        if lyric_result.code != 200 {
            log::warn!(
                "[NeteaseAPI EAPI] API错误，歌曲ID {}: 代码 {}",
                song_id,
                lyric_result.code
            );
            return Err(NeteaseError::ApiError {
                code: lyric_result.code,
                message: Some(format!("获取歌词失败，歌曲ID: {}", song_id)),
            });
        }
        if lyric_result.nolyric {
            log::info!("[NeteaseAPI EAPI] 歌曲无歌词，歌曲ID: {}", song_id);
            return Err(NeteaseError::NoLyrics);
        }

        log::info!(
            "[NeteaseAPI EAPI] 成功获取歌曲ID {} 的歌词。正在检查逐字歌词...",
            song_id
        );
        let mut found_word_by_word = false;
        // 检查 yrc 字段
        if let Some(yrc_val) = &lyric_result.yrc {
            if yrc_val
                .as_object()
                .and_then(|o| o.get("lyric")?.as_str())
                .is_some_and(|s| !s.is_empty())
            {
                log::info!("[NeteaseAPI EAPI] 找到逐字歌词 (YRC)");
                found_word_by_word = true;
            } else {
                log::info!("[NeteaseAPI EAPI] 找到逐字歌词 (YRC) 但为空。");
            }
        }
        // 检查 klyric 字段 (另一种逐字歌词)
        if lyric_result
            .klyric
            .as_ref()
            .and_then(|k| k.lyric.as_ref())
            .is_some_and(|s| !s.is_empty())
        {
            log::info!("[NeteaseAPI EAPI] 找到逐字歌词 (klyric)");
            found_word_by_word = true;
        }

        if !found_word_by_word
            && lyric_result
                .lrc
                .as_ref()
                .and_then(|l| l.lyric.as_ref())
                .is_some()
        {
            log::info!("[NeteaseAPI EAPI] 只找到LRC歌词");
        } else if !found_word_by_word {
            log::info!("[NeteaseAPI EAPI] API响应成功，但未找到歌词内容");
        }

        Ok(lyric_result)
    }

    /// 使用 EAPI 接口搜索歌曲。
    pub async fn search_songs_eapi(
        &self,
        keywords: &str,
        limit: u32,
        offset: u32,
    ) -> Result<SearchResponse> {
        let url_path = "/api/cloudsearch/pc"; // EAPI 搜索接口路径
        let full_url = "https://interface.music.163.com/eapi/cloudsearch/pc"; // EAPI 搜索接口完整 URL

        // EAPI 请求通常需要在 payload 中包含一个 "header" 字段，其内容也是一个 JSON 对象，
        // 包含了与 Cookie 中类似的设备和应用信息。
        let current_time_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let random_suffix: u32 = rand::rng().random_range(0..1000);

        let mut eapi_payload_header = HashMap::new();
        eapi_payload_header.insert("__csrf", "".to_string());
        eapi_payload_header.insert("os", "pc".to_string());
        eapi_payload_header.insert("appver", "8.0.0".to_string());
        eapi_payload_header.insert("buildver", (current_time_ms / 1000).to_string());
        eapi_payload_header.insert("channel", "".to_string());
        eapi_payload_header.insert("resolution", "1920x1080".to_string());
        eapi_payload_header.insert(
            "requestId",
            format!("{}_{:04}", current_time_ms, random_suffix),
        );
        eapi_payload_header.insert("versioncode", "140".to_string());
        eapi_payload_header.insert("MUSIC_U", "".to_string());

        // 构建 EAPI 搜索请求的 payload
        let mut payload = HashMap::new();
        payload.insert("s", keywords.to_string());
        payload.insert("type", "1".to_string());
        payload.insert("limit", limit.to_string());
        payload.insert("offset", offset.to_string());
        payload.insert("total", "true".to_string());
        payload.insert("header", serde_json::to_string(&eapi_payload_header)?);

        // 发送 EAPI 请求
        let response = self.post_eapi_async(url_path, full_url, &payload).await?;
        let search_result: SearchResponse = response.json().await?;

        if search_result.code != 200 {
            log::warn!(
                "[NeteaseAPI EAPI] 搜索API错误，: 代码 {}",
                search_result.code
            );
            return Err(NeteaseError::ApiError {
                code: search_result.code,
                message: Some("歌曲搜索失败".to_string()),
            });
        }
        log::info!(
            "[NeteaseAPI EAPI] 搜索成功。找到 {} 首歌",
            search_result.result.as_ref().map_or(0, |r| r.song_count)
        );
        Ok(search_result)
    }

    /// 使用旧的 Web API (GET 请求) 搜索歌曲。
    /// 这个接口可能参数较简单，不需要复杂的加密。
    pub async fn search_songs_weapi(
        &self,
        keywords: &str,
        search_type: NeteaseSearchType, // 使用枚举定义搜索类型
        limit: u32,
        offset: u32,
        total: bool, // 是否返回总数
    ) -> Result<SearchResponse> {
        // 直接发送 GET 请求
        let response = self
            .http_client
            .get(format!("{}/api/search/get/web", BASE_URL_NETEASE)) // 旧的 Web 搜索 API
            .query(&[
                // 将参数作为 URL query string
                ("s", keywords),
                ("type", search_type.api_type_str()),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
                ("total", &total.to_string()),
            ])
            .header(REFERER, BASE_URL_NETEASE) // 设置 Referer
            .send()
            .await?;

        let search_result: SearchResponse = response.json().await?;

        if search_result.code != 200 {
            let mut message_str = "歌曲搜索失败".to_string();
            if search_result.code == -460 {
                message_str.push_str(" (错误代码 -460, 可能是地区限制问题)");
            }
            return Err(NeteaseError::ApiError {
                code: search_result.code,
                message: Some(message_str),
            });
        }
        log::info!(
            "[NeteaseAPI WEAPI-GET] 搜索成功。找到 {} 首歌曲",
            search_result.result.as_ref().map_or(0, |r| r.song_count)
        );
        Ok(search_result)
    }

    /// 统一的歌曲搜索函数。
    /// 优先尝试使用 EAPI 进行搜索，如果失败，则回退到使用 WEAPI (GET) 进行搜索。
    pub async fn search_songs_unified(
        &self,
        keywords: &str,
        limit: u32,
        offset: u32,
    ) -> Result<SearchResponse> {
        // 首先尝试 EAPI 搜索
        match self.search_songs_eapi(keywords, limit, offset).await {
            Ok(result) => {
                // EAPI 成功
                Ok(result)
            }
            Err(eapi_err) => {
                // EAPI 失败
                // 尝试 WEAPI (GET) 搜索
                match self
                    .search_songs_weapi(keywords, NeteaseSearchType::Song, limit, offset, true)
                    .await
                {
                    Ok(weapi_result) => {
                        // WEAPI 成功
                        Ok(weapi_result)
                    }
                    Err(weapi_err) => {
                        // WEAPI 也失败
                        log::warn!("[NeteaseAPI] WEAPI 后备尝试也失败: {:?}", weapi_err);
                        Err(eapi_err)
                    }
                }
            }
        }
    }

    /// 统一的歌词获取函数。
    /// 优先尝试使用 EAPI 获取歌词，如果失败，则回退到使用 WEAPI。
    pub async fn fetch_lyrics_unified(&self, song_id: i64) -> Result<LyricResponse> {
        // 首先尝试 EAPI 获取歌词
        match self.fetch_lyrics_eapi(song_id).await {
            Ok(lyrics) => {
                // EAPI 成功
                Ok(lyrics)
            }
            Err(eapi_err) => {
                // EAPI 失败
                // 尝试 WEAPI 获取歌词
                match self.fetch_lyrics_weapi(song_id).await {
                    Ok(lyrics_weapi) => {
                        // WEAPI 成功
                        Ok(lyrics_weapi)
                    }
                    Err(weapi_err) => {
                        // WEAPI 也失败
                        log::warn!("[NeteaseAPI] WEAPI 后备尝试也失败: {:?}", weapi_err);
                        Err(eapi_err)
                    }
                }
            }
        }
    }
}
