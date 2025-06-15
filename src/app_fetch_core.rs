use crate::amll_connector::NowPlayingInfo;
use crate::amll_lyrics_fetcher::{self, AmllIndexEntry};
use crate::app_definition::UniLyricApp;
use crate::qq_lyrics_fetcher::qqlyricsfetcher::QQLyricsFetcherError;
use crate::types::{
    AutoFetchResult, AutoSearchSource, AutoSearchStatus, ConvertError, LocalLyricCacheEntry,
    PlatformFetchedData,
};
use crate::types::{LyricFetchError, LyricFormat, ProcessedLyricsSourceData};
use crate::{app_fetch_core, netease_lyrics_fetcher, utils};

use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
type LyricFetchFuture =
    Pin<Box<dyn Future<Output = Result<ProcessedLyricsSourceData, LyricFetchError>> + Send>>;
type SearchSourceEntry = (
    AutoSearchSource,
    Arc<Mutex<AutoSearchStatus>>,
    LyricFetchFuture,
);
type OptionalCachedResult = Option<Arc<Mutex<Option<ProcessedLyricsSourceData>>>>;
type RefetchStatusAndResult = (Arc<Mutex<AutoSearchStatus>>, OptionalCachedResult);
// 辅助函数：预处理QQ音乐的翻译LRC内容
fn preprocess_qq_translation_lrc_content(lrc_content: String) -> String {
    lrc_content
        .lines()
        .map(|line_str| {
            if let Some(text_start_idx) = line_str.rfind(']') {
                let timestamp_part = &line_str[..=text_start_idx];
                let text_part = line_str[text_start_idx + 1..].trim();
                if text_part == "//" {
                    timestamp_part.to_string()
                } else {
                    line_str.to_string()
                }
            } else {
                // 对于非标准LRC行（例如元数据行但没有文本，或完全无效的行），
                // 如果它们不应出现在预处理结果中，则返回空字符串。
                // 或者，如果希望保留它们，则返回 line_str.to_string()。
                // 当前行为是如果行不含']'，则此行在预处理结果中消失。
                String::new() // 或者 line_str.to_string() 如果要保留
            }
        })
        .filter(|line| !line.is_empty()) // 移除处理后完全变为空的行
        .collect::<Vec<String>>()
        .join("\n")
}

pub(crate) async fn fetch_qq_music_lyrics_core(
    http_client: &reqwest::Client,
    query: &str,
) -> Result<ProcessedLyricsSourceData, LyricFetchError> {
    debug!("[CoreFetch/QQ] 开始获取QQ音乐歌词，查询: '{query}'");

    match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        crate::qq_lyrics_fetcher::qqlyricsfetcher::download_lyrics_by_query_first_match(
            http_client,
            query,
        ),
    )
    .await
    {
        Ok(Ok(fetched_data)) => {
            debug!(
                "[CoreFetch/QQ] 原始数据获取成功 '{}'. 歌曲名: {:?}, 艺术家: {:?}",
                query, fetched_data.song_name, fetched_data.artists_name
            );

            if let Some(main_qrc) = fetched_data.main_lyrics_qrc {
                let processed_translation_lrc = fetched_data
                    .translation_lrc
                    .map(|raw_lrc| {
                        if !raw_lrc.trim().is_empty() {
                            preprocess_qq_translation_lrc_content(raw_lrc)
                        } else {
                            String::new()
                        }
                    })
                    .filter(|s| !s.trim().is_empty());

                let mut platform_meta = HashMap::new();
                if let Some(s_name) = fetched_data.song_name {
                    platform_meta.insert("musicName".to_string(), s_name);
                }
                if !fetched_data.artists_name.is_empty() {
                    platform_meta.insert("artist".to_string(), fetched_data.artists_name.join("/"));
                }
                if let Some(s_album) = fetched_data.album_name {
                    platform_meta.insert("album".to_string(), s_album);
                }
                if let Some(s_id) = fetched_data.song_id {
                    platform_meta.insert("qqMusicId".to_string(), s_id);
                }

                debug!("[CoreFetch/QQ] 成功处理QQ音乐歌词 '{query}'");
                Ok(ProcessedLyricsSourceData {
                    format: LyricFormat::Qrc,
                    main_lyrics: main_qrc,
                    translation_lrc: processed_translation_lrc,
                    romanization_qrc: fetched_data.romanization_qrc,
                    romanization_lrc: None,
                    krc_translation_lines: None,
                    platform_metadata: platform_meta,
                })
            } else {
                Err(LyricFetchError::NotFound)
            }
        }
        Ok(Err(fetcher_error)) => {
            warn!("[CoreFetch/QQ] 获取QQ音乐歌词失败: {fetcher_error}");
            match fetcher_error {
                QQLyricsFetcherError::SongInfoMissing
                | QQLyricsFetcherError::LyricNotFoundForSelectedSong => {
                    Err(LyricFetchError::NotFound)
                }
                QQLyricsFetcherError::RequestRejected => Err(LyricFetchError::NetworkError(
                    "请求已拒绝（代码 2001）".to_string(),
                )),
                QQLyricsFetcherError::ApiProcess(uni_lyric_convert_error)
                | QQLyricsFetcherError::ApiError(uni_lyric_convert_error) => {
                    match uni_lyric_convert_error {
                        ConvertError::NetworkRequest(e) => {
                            Err(LyricFetchError::NetworkError(e.to_string()))
                        }
                        ConvertError::JsonParse(e) => {
                            Err(LyricFetchError::ParseError(format!("JSON 处理错误: {e}")))
                        }
                        ConvertError::RequestRejected => {
                            Err(LyricFetchError::NetworkError("请求已拒绝".to_string()))
                        }
                        ConvertError::LyricNotFound => Err(LyricFetchError::NotFound),
                        ConvertError::Io(e) => {
                            Err(LyricFetchError::InternalError(format!("I/O 错误: {e}")))
                        }
                        ConvertError::FromUtf8(e) => {
                            Err(LyricFetchError::ParseError(format!("UTF-8 错误: {e}")))
                        }
                        _ => Err(LyricFetchError::InternalError(format!(
                            "QQ API 错误: {uni_lyric_convert_error}"
                        ))),
                    }
                }
            }
        }
        Err(_timeout_error) => {
            warn!("[CoreFetch/QQ] 获取QQ音乐歌词超时 '{query}'");
            Err(LyricFetchError::Timeout)
        }
    }
}

pub(crate) async fn fetch_kugou_music_lyrics_core(
    http_client: &reqwest::Client,
    query: &str,
) -> Result<ProcessedLyricsSourceData, LyricFetchError> {
    debug!("[CoreFetch/Kugou] 开始获取酷狗音乐歌词 '{query}'");

    match tokio::time::timeout(
        std::time::Duration::from_secs(20),
        crate::kugou_lyrics_fetcher::fetch_lyrics_for_song_async(http_client, query),
    )
    .await
    {
        Ok(Ok(fetched_krc_data)) => {
            if fetched_krc_data.krc_content.is_empty() {
                warn!("[CoreFetch/Kugou] KRC 内容为空 '{query}'");
                return Err(LyricFetchError::NotFound);
            }

            let mut platform_meta = HashMap::new();
            if let Some(s_name) = fetched_krc_data.song_name {
                platform_meta.insert("musicName".to_string(), s_name);
            }
            if !fetched_krc_data.artists_name.is_empty() {
                platform_meta.insert(
                    "artist".to_string(),
                    fetched_krc_data.artists_name.join("/"),
                );
            }
            if let Some(s_album) = fetched_krc_data.album_name {
                platform_meta.insert("album".to_string(), s_album);
            }
            for item in fetched_krc_data.krc_embedded_metadata {
                platform_meta
                    .entry(item.key.clone())
                    .or_insert_with(|| item.value.clone());
            }

            Ok(ProcessedLyricsSourceData {
                format: LyricFormat::Krc,
                main_lyrics: fetched_krc_data.krc_content,
                translation_lrc: None,
                romanization_qrc: None,
                romanization_lrc: None,
                krc_translation_lines: fetched_krc_data.translation_lines,
                platform_metadata: platform_meta,
            })
        }
        Ok(Err(fetcher_error)) => {
            warn!("[CoreFetch/Kugou] 获取酷狗音乐歌词时失败 '{query}': {fetcher_error}");
            match fetcher_error {
                crate::kugou_lyrics_fetcher::error::KugouError::LyricsNotFound(_)
                | crate::kugou_lyrics_fetcher::error::KugouError::NoCandidatesFound
                | crate::kugou_lyrics_fetcher::error::KugouError::EmptyLyricContent => {
                    Err(LyricFetchError::NotFound)
                }
                crate::kugou_lyrics_fetcher::error::KugouError::Network(e) => {
                    Err(LyricFetchError::NetworkError(e.to_string()))
                }
                crate::kugou_lyrics_fetcher::error::KugouError::Json(e) => {
                    Err(LyricFetchError::ParseError(format!("JSON: {e}")))
                }
                crate::kugou_lyrics_fetcher::error::KugouError::InvalidKrcData(s) => {
                    Err(LyricFetchError::ParseError(s))
                }
                crate::kugou_lyrics_fetcher::error::KugouError::Base64(e) => {
                    Err(LyricFetchError::ParseError(e.to_string()))
                }
                _ => Err(LyricFetchError::InternalError(format!(
                    "Kugou fetcher error: {fetcher_error}"
                ))),
            }
        }
        Err(_timeout_error) => {
            warn!("[CoreFetch/Kugou] 获取酷狗音乐歌词超时 '{query}'");
            Err(LyricFetchError::Timeout)
        }
    }
}

pub(crate) async fn fetch_netease_music_lyrics_core(
    _http_client: &reqwest::Client, // 通常网易云的API客户端自己处理HTTP请求
    netease_client_arc: Arc<std::sync::Mutex<Option<netease_lyrics_fetcher::api::NeteaseClient>>>,
    query: &str,
) -> Result<ProcessedLyricsSourceData, LyricFetchError> {
    debug!("[CoreFetch/Netease] 开始获取网易云音乐歌词，查询: '{query}'");

    let client_instance_for_fetch: netease_lyrics_fetcher::api::NeteaseClient;
    {
        let mut client_option_guard = netease_client_arc.lock().map_err(|_| {
            LyricFetchError::InternalError("Netease client mutex poisoned".to_string())
        })?;

        if client_option_guard.is_none() {
            debug!("[CoreFetch/Netease] NeteaseClient 未初始化，尝试创建新的实例...");
            match netease_lyrics_fetcher::api::NeteaseClient::new() {
                Ok(new_client) => {
                    *client_option_guard = Some(new_client);
                }
                Err(e) => {
                    warn!("[CoreFetch/Netease] NeteaseClient 初始化失败: {e}");
                    return Err(LyricFetchError::ApiClientError(format!(
                        "NeteaseClient init failed: {e}"
                    )));
                }
            }
        }
        client_instance_for_fetch = client_option_guard
            .as_ref()
            .ok_or_else(|| {
                LyricFetchError::ApiClientError(
                    "NeteaseClient not available after init attempt".to_string(),
                )
            })?
            .clone();
    }

    match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        netease_lyrics_fetcher::search_and_fetch_first_netease_lyrics(
            &client_instance_for_fetch,
            query,
        ),
    )
    .await
    {
        Ok(Ok(fetched_netease_data)) => {
            debug!(
                "[CoreFetch/Netease] 原始数据获取成功 '{}'. 歌曲名: {:?}, 艺术家: {:?}",
                query, fetched_netease_data.song_name, fetched_netease_data.artists_name
            );

            let mut platform_meta = HashMap::new();
            if let Some(s_name) = &fetched_netease_data.song_name {
                platform_meta.insert("musicName".to_string(), s_name.clone());
            }
            if !fetched_netease_data.artists_name.is_empty() {
                platform_meta.insert(
                    "artist".to_string(),
                    fetched_netease_data.artists_name.join("/"),
                );
            }
            if let Some(s_album) = &fetched_netease_data.album_name {
                platform_meta.insert("album".to_string(), s_album.clone());
            }
            if let Some(s_id) = &fetched_netease_data.song_id {
                platform_meta.insert("ncmMusicId".to_string(), s_id.clone());
            }

            // 检查是否有 YRC (karaoke_lrc)
            if let Some(yrc_content) = fetched_netease_data
                .karaoke_lrc
                .as_ref()
                .filter(|s| !s.is_empty())
            {
                info!("[CoreFetch/Netease] 找到 YRC 歌词 '{query}'.");
                debug!("[CoreFetch/Netease] 成功处理网易云音乐歌词 (YRC) '{query}'");
                Ok(ProcessedLyricsSourceData {
                    format: LyricFormat::Yrc,
                    main_lyrics: yrc_content.clone(),
                    translation_lrc: fetched_netease_data
                        .translation_lrc
                        .filter(|s| !s.trim().is_empty()),
                    romanization_qrc: None,
                    romanization_lrc: fetched_netease_data
                        .romanization_lrc
                        .filter(|s| !s.trim().is_empty()),
                    krc_translation_lines: None,
                    platform_metadata: platform_meta,
                })
            } else if let Some(lrc_content) = fetched_netease_data
                .main_lrc
                .as_ref()
                .filter(|s| !s.is_empty())
            {
                // 如果没有 YRC，但有 LRC，则将其作为 LyricFormat::Lrc 返回
                debug!("[CoreFetch/Netease] 仅找到 LRC 歌词 '{query}', 将使用此LRC。");
                debug!("[CoreFetch/Netease] 成功处理网易云音乐歌词 (LRC) '{query}'");
                Ok(ProcessedLyricsSourceData {
                    format: LyricFormat::Lrc,         // 格式设置为 LRC
                    main_lyrics: lrc_content.clone(), // 使用 LRC 内容
                    translation_lrc: fetched_netease_data
                        .translation_lrc
                        .filter(|s| !s.trim().is_empty()),
                    romanization_qrc: None,
                    romanization_lrc: fetched_netease_data
                        .romanization_lrc
                        .filter(|s| !s.trim().is_empty()),
                    krc_translation_lines: None,
                    platform_metadata: platform_meta,
                })
            } else {
                // 如果 YRC 和 LRC 都没有
                error!("[CoreFetch/Netease] 未找到有效的 YRC 或 LRC 主歌词 '{query}'");
                Err(LyricFetchError::NotFound)
            }
        }
        Ok(Err(fetcher_error)) => {
            error!("[CoreFetch/Netease] 获取网易云音乐歌词时失败 '{query}': {fetcher_error}");
            match fetcher_error {
                netease_lyrics_fetcher::error::NeteaseError::SongNotFound(_) => {
                    Err(LyricFetchError::NotFound)
                }
                netease_lyrics_fetcher::error::NeteaseError::NoLyrics => {
                    Err(LyricFetchError::NotFound)
                }
                netease_lyrics_fetcher::error::NeteaseError::Network(e) => {
                    Err(LyricFetchError::NetworkError(e.to_string()))
                }
                netease_lyrics_fetcher::error::NeteaseError::ApiError { code: _, message } => {
                    Err(LyricFetchError::ApiClientError(format!("{message:?}")))
                }
                netease_lyrics_fetcher::error::NeteaseError::Crypto(s) => {
                    Err(LyricFetchError::ParseError(format!("加密错误: {s}")))
                }
                netease_lyrics_fetcher::error::NeteaseError::Json(e) => {
                    Err(LyricFetchError::ParseError(format!("JSON: {e}")))
                }
                _ => Err(LyricFetchError::InternalError(format!(
                    "Netease fetcher error: {fetcher_error}"
                ))),
            }
        }
        Err(_timeout_error) => {
            warn!("[CoreFetch/Netease] 获取网易云音乐歌词超时 '{query}'");
            Err(LyricFetchError::Timeout)
        }
    }
}

pub(crate) async fn fetch_amll_db_lyrics_for_smtc_match_core(
    http_client: &reqwest::Client,
    amll_index_arc: Arc<std::sync::Mutex<Vec<AmllIndexEntry>>>,
    repo_base_url: &str,
    smtc_title: &str,
    smtc_artists: &[String],
) -> Result<ProcessedLyricsSourceData, LyricFetchError> {
    debug!(
        "[CoreFetch/AMLL Auto] 开始SMTC匹配搜索。SMTC标题: '{smtc_title}', SMTC艺术家: {smtc_artists:?}"
    );

    // 步骤 1: 获取锁，调用同步辅助函数查找匹配项，然后立即释放锁
    let entry_to_download_opt: Option<AmllIndexEntry> = {
        // 显式作用域
        let index_guard = amll_index_arc.lock().map_err(|_| {
            LyricFetchError::InternalError("AMLL index mutex poisoned (auto)".to_string())
        })?;

        // 调用同步函数进行查找和克隆
        find_best_smtc_match_in_index_sync(&index_guard, smtc_title, smtc_artists)
    }; // index_guard 在此作用域结束时被释放

    // 步骤 2: 使用克隆出来的、拥有的数据执行异步操作
    if let Some(entry_to_download) = entry_to_download_opt {
        // entry_to_download 是拥有的 AmllIndexEntry
        debug!(
            "[CoreFetch/AMLL Auto] 最终选择的最新匹配项: 文件 '{}'",
            entry_to_download.raw_lyric_file
        );

        match tokio::time::timeout(
            std::time::Duration::from_secs(15),
            amll_lyrics_fetcher::download_ttml_from_entry(
                http_client,
                repo_base_url,
                &entry_to_download, // 传递对拥有的数据的引用
            ),
        )
        .await
        {
            Ok(Ok(fetched_amll_data)) => {
                info!(
                    "[CoreFetch/AMLL Auto] TTML 文件下载成功: {}",
                    entry_to_download.raw_lyric_file
                );
                if fetched_amll_data.ttml_content.is_empty() {
                    warn!(
                        "[CoreFetch/AMLL Auto] 下载的 TTML 内容为空，文件 '{}'",
                        entry_to_download.raw_lyric_file
                    );
                    return Err(LyricFetchError::NotFound);
                }
                let mut platform_meta = HashMap::new();
                if let Some(s_name) = fetched_amll_data.song_name {
                    platform_meta.insert("musicName".to_string(), s_name);
                }
                if !fetched_amll_data.artists_name.is_empty() {
                    platform_meta.insert(
                        "artist".to_string(),
                        fetched_amll_data.artists_name.join("/"),
                    );
                }
                if let Some(s_album) = fetched_amll_data.album_name {
                    platform_meta.insert("album".to_string(), s_album);
                }
                if let Some(s_id) = fetched_amll_data.source_id {
                    platform_meta.insert("amllDbId".to_string(), s_id);
                }
                for (key, values) in fetched_amll_data.all_metadata_from_index {
                    if !values.is_empty() {
                        platform_meta
                            .entry(key.clone())
                            .or_insert_with(|| values.first().unwrap().clone());
                    }
                }
                debug!(
                    "[CoreFetch/AMLL Auto] 成功处理AMLL-DB歌词，文件 '{}'",
                    entry_to_download.raw_lyric_file
                );
                Ok(ProcessedLyricsSourceData {
                    format: LyricFormat::Ttml,
                    main_lyrics: fetched_amll_data.ttml_content,
                    translation_lrc: None,
                    romanization_qrc: None,
                    romanization_lrc: None,
                    krc_translation_lines: None,
                    platform_metadata: platform_meta,
                })
            }
            Ok(Err(fetcher_error)) => {
                warn!(
                    "[CoreFetch/AMLL Auto] 下载AMLL TTML文件失败，文件 '{}': {}",
                    entry_to_download.raw_lyric_file, fetcher_error
                );
                match fetcher_error {
                    // 确保这里的 ConvertError 是 crate::types::ConvertError
                    ConvertError::NetworkRequest(e) => {
                        Err(LyricFetchError::NetworkError(e.to_string()))
                    }
                    ConvertError::JsonParse(e) => Err(LyricFetchError::ParseError(format!(
                        "JSON (index parsing) error: {e}"
                    ))),
                    ConvertError::Io(e) => Err(LyricFetchError::InternalError(format!(
                        "I/O error (cache): {e}"
                    ))),
                    _ => Err(LyricFetchError::InternalError(format!(
                        "AMLL fetcher error (auto): {fetcher_error}"
                    ))),
                }
            }
            Err(_timeout_error) => {
                warn!(
                    "[CoreFetch/AMLL Auto] 下载AMLL TTML文件超时，文件 '{}'",
                    entry_to_download.raw_lyric_file
                );
                Err(LyricFetchError::Timeout)
            }
        }
    } else {
        debug!(
            "[CoreFetch/AMLL Auto] 未找到SMTC匹配的AMLL条目。SMTC标题: '{smtc_title}', SMTC艺术家: {smtc_artists:?}"
        );
        Err(LyricFetchError::NotFound)
    }
}

// 辅助同步函数，用于在索引中查找匹配项并返回克隆的条目
// 这个函数不包含任何 .await，因此可以在持有 std::sync::MutexGuard 的情况下安全调用
fn find_best_smtc_match_in_index_sync(
    index_entries: &[AmllIndexEntry], // 直接接收对 Vec 内容的引用
    smtc_title: &str,
    smtc_artists: &[String],
) -> Option<AmllIndexEntry> {
    if index_entries.is_empty() {
        warn!("[CoreFetch] AMLL 索引数据为空。");
        return None;
    }

    let lower_smtc_title = smtc_title.to_lowercase();
    let lower_smtc_artists: Vec<String> = smtc_artists.iter().map(|s| s.to_lowercase()).collect();
    let mut newest_match_candidate: Option<AmllIndexEntry> = None;

    // 正向遍历索引，最后一个匹配的即为最新的
    for entry in index_entries.iter() {
        let mut title_matched = false;
        let mut all_artists_matched = true;

        if let Some(music_names_vec) = entry
            .metadata
            .iter()
            .find(|(key, _)| key == "musicName")
            .map(|(_, values)| values)
            && music_names_vec
                .iter()
                .any(|name| name.to_lowercase().contains(&lower_smtc_title))
        {
            title_matched = true;
        }

        if title_matched && !lower_smtc_artists.is_empty() {
            if let Some(entry_artists_vec) = entry
                .metadata
                .iter()
                .find(|(key, _)| key == "artists")
                .map(|(_, values)| values)
            {
                let lower_entry_artists: Vec<String> =
                    entry_artists_vec.iter().map(|s| s.to_lowercase()).collect();
                for smtc_artist in &lower_smtc_artists {
                    if !lower_entry_artists
                        .iter()
                        .any(|entry_art| entry_art.contains(smtc_artist))
                    {
                        all_artists_matched = false;
                        break;
                    }
                }
            } else {
                all_artists_matched = false;
            }
        }

        if title_matched && all_artists_matched {
            debug!(
                "[CoreFetch] 找到潜在匹配项: 文件 '{}'",
                entry.raw_lyric_file
            );
            newest_match_candidate = Some(entry.clone());
        }
    }
    newest_match_candidate
}

pub fn process_platform_lyrics_data(app: &mut UniLyricApp, fetched_data_enum: PlatformFetchedData) {
    app.clear_all_data(); // 调用 app 实例上的方法
    app.metadata_source_is_download = true; // 标记元数据来源于下载

    match fetched_data_enum {
        PlatformFetchedData::Qq(data) => {
            app.input_text = data.main_lyrics_qrc.unwrap_or_default();
            app.source_format = LyricFormat::Qrc;

            if let Some(raw_translation_lrc) = data.translation_lrc {
                if !raw_translation_lrc.trim().is_empty() {
                    log::debug!("[AppFetchCore] 预处理QQ音乐翻译LRC (移除'//'行)...");
                    // 假设 preprocess_qq_translation_lrc_content 已移至 utils 模块
                    app.pending_translation_lrc_from_download = Some(
                        utils::preprocess_qq_translation_lrc_content(raw_translation_lrc),
                    );
                } else {
                    app.pending_translation_lrc_from_download = None;
                }
            } else {
                app.pending_translation_lrc_from_download = None;
            }

            app.pending_romanization_qrc_from_download = data.romanization_qrc;
            app.pending_romanization_lrc_from_download = None; // Qq 通常不提供罗马音LRC
            app.pending_krc_translation_lines = None; // Qq 不是 KRC

            let mut meta = HashMap::new();
            if let Some(s_name) = data.song_name {
                meta.insert("musicName".to_string(), s_name);
            }
            if !data.artists_name.is_empty() {
                meta.insert("artist".to_string(), data.artists_name.join("/"));
            }
            if let Some(s_album) = data.album_name {
                meta.insert("album".to_string(), s_album);
            }
            if let Some(s_id) = data.song_id {
                meta.insert("qqMusicId".to_string(), s_id);
            }
            app.session_platform_metadata = meta;
        }
        PlatformFetchedData::Kugou(data) => {
            app.input_text = data.krc_content;
            app.source_format = LyricFormat::Krc;
            app.pending_krc_translation_lines = data.translation_lines;
            app.pending_translation_lrc_from_download = None;
            app.pending_romanization_qrc_from_download = None;
            app.pending_romanization_lrc_from_download = None;

            let mut meta = HashMap::new();
            if let Some(s_name) = data.song_name {
                meta.insert("musicName".to_string(), s_name);
            }
            if !data.artists_name.is_empty() {
                meta.insert("artist".to_string(), data.artists_name.join("/"));
            }
            if let Some(s_album) = data.album_name {
                meta.insert("album".to_string(), s_album);
            }

            for item in &data.krc_embedded_metadata {
                if item.key.to_lowercase() == "krcinternaltranslation" {
                    log::debug!(
                        "[AppFetchCore] 已跳过添加原始 KrcInternalTranslation 到会话元数据。"
                    );
                } else {
                    meta.entry(item.key.clone()).or_insert(item.value.clone());
                }
            }
            app.session_platform_metadata = meta;
        }
        PlatformFetchedData::Netease(data) => {
            let (main_lyrics_content, format) =
                if let Some(yrc) = data.karaoke_lrc.filter(|s| !s.is_empty()) {
                    (yrc, LyricFormat::Yrc)
                } else if let Some(lrc) = data.main_lrc.filter(|s| !s.is_empty()) {
                    app.direct_netease_main_lrc_content = Some(lrc.clone());
                    (lrc, LyricFormat::Lrc)
                } else {
                    log::warn!("[AppFetchCore] 网易云音乐未找到有效的YRC或LRC主歌词。");
                    ("".to_string(), LyricFormat::Lrc)
                };

            app.input_text = main_lyrics_content;
            app.source_format = format;
            app.pending_translation_lrc_from_download = data.translation_lrc;
            app.pending_romanization_lrc_from_download = data.romanization_lrc;
            app.pending_romanization_qrc_from_download = None;
            app.pending_krc_translation_lines = None;

            let mut meta = HashMap::new();
            if let Some(s_name) = data.song_name {
                meta.insert("musicName".to_string(), s_name);
            }
            if !data.artists_name.is_empty() {
                meta.insert("artist".to_string(), data.artists_name.join("/"));
            }
            if let Some(s_album) = data.album_name {
                meta.insert("album".to_string(), s_album);
            }
            if let Some(s_id) = data.song_id {
                meta.insert("ncmMusicId".to_string(), s_id);
            }
            app.session_platform_metadata = meta;
        }
        PlatformFetchedData::Amll(data) => {
            app.input_text = data.ttml_content;
            app.source_format = LyricFormat::Ttml;
            app.pending_translation_lrc_from_download = None;
            app.pending_romanization_qrc_from_download = None;
            app.pending_romanization_lrc_from_download = None;
            app.pending_krc_translation_lines = None;

            let mut meta = HashMap::new();
            if let Some(s_name) = data.song_name.as_ref() {
                meta.insert("musicName".to_string(), s_name.clone());
            }
            app.session_platform_metadata = meta;
        }
    }

    app.handle_convert();
}

pub(crate) fn update_all_online_search_status(app: &UniLyricApp, status: AutoSearchStatus) {
    *app.qqmusic_auto_search_status.lock().unwrap() = status.clone();
    *app.kugou_auto_search_status.lock().unwrap() = status.clone();
    *app.netease_auto_search_status.lock().unwrap() = status.clone();
    *app.amll_db_auto_search_status.lock().unwrap() = status.clone();
}

pub(crate) fn reset_all_search_statuses_to_not_attempted(app: &UniLyricApp) {
    *app.local_cache_auto_search_status.lock().unwrap() = AutoSearchStatus::NotAttempted;
    *app.qqmusic_auto_search_status.lock().unwrap() = AutoSearchStatus::NotAttempted;
    *app.kugou_auto_search_status.lock().unwrap() = AutoSearchStatus::NotAttempted;
    *app.netease_auto_search_status.lock().unwrap() = AutoSearchStatus::NotAttempted;
    *app.amll_db_auto_search_status.lock().unwrap() = AutoSearchStatus::NotAttempted;
}

pub(crate) fn update_all_search_status(app: &UniLyricApp, status: AutoSearchStatus) {
    if status == AutoSearchStatus::NotAttempted {
        reset_all_search_statuses_to_not_attempted(app);
    } else if status == AutoSearchStatus::NotFound {
        *app.local_cache_auto_search_status.lock().unwrap() = AutoSearchStatus::NotFound;
        update_all_online_search_status(app, AutoSearchStatus::NotFound);
    } else if status == AutoSearchStatus::Searching {
        update_all_online_search_status(app, AutoSearchStatus::Searching);
    }
}

pub(crate) fn set_other_sources_not_attempted(
    app: &UniLyricApp,
    successful_source: AutoSearchSource,
) {
    log::trace!(
        "[AppFetchCore SetNotAttempted] 调用，成功源: {:?}. 将重置其他 'Searching' 状态的源。",
        successful_source.display_name()
    );
    let sources_to_update_config = [
        (
            AutoSearchSource::QqMusic,
            &app.qqmusic_auto_search_status,
            "QQMusicStatus",
        ),
        (
            AutoSearchSource::Kugou,
            &app.kugou_auto_search_status,
            "KugouStatus",
        ),
        (
            AutoSearchSource::Netease,
            &app.netease_auto_search_status,
            "NeteaseStatus",
        ),
        (
            AutoSearchSource::AmllDb,
            &app.amll_db_auto_search_status,
            "AmllDbStatus",
        ),
    ];
    for (source_enum, status_arc, name_for_log) in sources_to_update_config {
        if source_enum != successful_source {
            let mut guard = status_arc.lock().unwrap();
            log::debug!(
                "[AppFetchCore SetNotAttempted] 检查 {}: 当前状态是 {:?}. 成功源是 {:?}.",
                name_for_log,
                *guard,
                successful_source.display_name()
            );
            if matches!(*guard, AutoSearchStatus::Searching) {
                *guard = AutoSearchStatus::NotAttempted;
                log::trace!(
                    "[AppFetchCore SetNotAttempted] {name_for_log} 状态已更新为 NotAttempted。"
                );
            } else {
                log::debug!(
                    "[AppFetchCore SetNotAttempted] {} 状态不是 Searching (当前是 {:?}), 未更新。",
                    name_for_log,
                    *guard
                );
            }
        } else {
            log::debug!("[AppFetchCore SetNotAttempted] 跳过 {name_for_log} 因为它是成功源。");
        }
    }
}

/// 异步任务：自动获取歌词，填充输入框，触发转换，并发送TTML。
pub fn initial_auto_fetch_and_send_lyrics(app: &mut UniLyricApp, track_info: NowPlayingInfo) {
    let smtc_title_option = track_info.title.clone();
    let smtc_artist_string_option = track_info.artist.clone();

    let smtc_title = match smtc_title_option {
        Some(t) if !t.is_empty() && t != "无歌曲" && t != "无活动会话" => t,
        _ => {
            log::info!("[AppFetchCore AutoFetch] SMTC 报告无有效歌曲名称，跳过自动搜索。");
            reset_all_search_statuses_to_not_attempted(app); // 调用本模块的函数
            if app
                .auto_fetch_result_tx
                .send(AutoFetchResult::NotFound)
                .is_err()
            {
                log::error!(
                    "[AppFetchCore AutoFetch] 发送 NotFound (因无效歌曲标题) 到主线程失败。"
                );
            }
            return;
        }
    };
    let smtc_artists_vec: Vec<String> = smtc_artist_string_option
        .map(|s| {
            s.split(['/', '、', ',', ';'])
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default();

    if smtc_title.is_empty() {
        log::info!("[AppFetchCore AutoFetch] SMTC 标题处理后为空，跳过自动搜索。");
        reset_all_search_statuses_to_not_attempted(app); // 调用本模块的函数
        if app
            .auto_fetch_result_tx
            .send(AutoFetchResult::NotFound)
            .is_err()
        {
            log::error!("[AppFetchCore AutoFetch] 发送 NotFound (因SMTC标题为空) 到主线程失败。");
        }
        return;
    }

    app.last_auto_fetch_source_format = None;
    app.current_auto_search_ui_populated = false;

    *app.last_qq_search_result.lock().unwrap() = None;
    *app.last_kugou_search_result.lock().unwrap() = None;
    *app.last_netease_search_result.lock().unwrap() = None;
    *app.last_amll_db_search_result.lock().unwrap() = None;

    let mut local_cache_hit = false;
    let lower_smtc_title_for_cache = smtc_title.to_lowercase();
    let lower_smtc_artists_for_cache: Vec<String> =
        smtc_artists_vec.iter().map(|s| s.to_lowercase()).collect();

    if let Some(cache_dir) = &app.local_lyrics_cache_dir_path {
        let index_guard = app.local_lyrics_cache_index.lock().unwrap();
        let mut found_entry_in_cache: Option<LocalLyricCacheEntry> = None;
        for entry in index_guard.iter().rev() {
            // 保持原有查找逻辑
            let mut title_match = entry
                .smtc_title
                .to_lowercase()
                .contains(&lower_smtc_title_for_cache);
            if !title_match && lower_smtc_title_for_cache.len() < entry.smtc_title.len() {
                title_match = entry
                    .smtc_title
                    .to_lowercase()
                    .starts_with(&lower_smtc_title_for_cache);
            } else if !title_match && lower_smtc_title_for_cache.len() > entry.smtc_title.len() {
                title_match =
                    lower_smtc_title_for_cache.starts_with(&entry.smtc_title.to_lowercase());
            }
            let mut artists_match = true;
            if !lower_smtc_artists_for_cache.is_empty() {
                let lower_cached_artists: Vec<String> = entry
                    .smtc_artists
                    .iter()
                    .map(|s| s.to_lowercase())
                    .collect();
                if lower_cached_artists.is_empty() {
                    artists_match = false;
                } else {
                    for smtc_artist in &lower_smtc_artists_for_cache {
                        if !lower_cached_artists.iter().any(|cached_art| {
                            cached_art.contains(smtc_artist) || smtc_artist.contains(cached_art)
                        }) {
                            artists_match = false;
                            break;
                        }
                    }
                }
            }
            if title_match && artists_match {
                found_entry_in_cache = Some(entry.clone());
                break;
            }
        }
        drop(index_guard);

        if let Some(cached_entry) = found_entry_in_cache {
            let ttml_file_path = cache_dir.join(&cached_entry.ttml_filename);
            match std::fs::read_to_string(&ttml_file_path) {
                Ok(ttml_content) => {
                    local_cache_hit = true;
                    let mut platform_meta = HashMap::new();
                    platform_meta.insert("musicName".to_string(), cached_entry.smtc_title.clone());
                    if !cached_entry.smtc_artists.is_empty() {
                        platform_meta
                            .insert("artist".to_string(), cached_entry.smtc_artists.join("/"));
                    }
                    if let Some(orig_format_str) = &cached_entry.original_source_format {
                        // 使用新的字段名
                        platform_meta
                            .insert("cachedOriginalFormat".to_string(), orig_format_str.clone());
                    }
                    platform_meta.insert(
                        "cachedFromFile".to_string(),
                        cached_entry.ttml_filename.clone(),
                    );

                    if app
                        .auto_fetch_result_tx
                        .send(AutoFetchResult::Success {
                            source: AutoSearchSource::LocalCache,
                            source_format: LyricFormat::Ttml, // 本地缓存总是TTML
                            main_lyrics: ttml_content,
                            translation_lrc: None,
                            romanization_qrc: None,
                            romanization_lrc: None,
                            krc_translation_lines: None,
                            platform_metadata: platform_meta,
                        })
                        .is_err()
                    {
                        local_cache_hit = false; // 发送失败，不算命中
                        log::error!(
                            "[AppFetchCore AutoFetch] 本地缓存命中，但发送结果到主线程失败。"
                        );
                    } else {
                        log::info!(
                            "[AppFetchCore AutoFetch] 本地缓存命中并成功发送结果: {}",
                            cached_entry.smtc_title
                        );
                    }
                }
                Err(e) => {
                    log::error!(
                        "[AppFetchCore AutoFetch] 读取本地缓存文件 {ttml_file_path:?} 失败: {e}"
                    );
                    // 不在这里发送 FetchError，让后续流程继续尝试在线源
                    // 但 local_cache_hit 仍然是 false
                }
            }
        }
    }

    // 更新本地缓存的搜索状态
    let mut local_cache_status_guard = app.local_cache_auto_search_status.lock().unwrap();
    if local_cache_hit {
        *local_cache_status_guard = AutoSearchStatus::Success(LyricFormat::Ttml);
    } else {
        // 如果之前是 Error，不要覆盖为 NotFound，除非我们确定错误已解决或不再相关
        if !matches!(*local_cache_status_guard, AutoSearchStatus::Error(_)) {
            *local_cache_status_guard = AutoSearchStatus::NotFound;
        }
    }
    drop(local_cache_status_guard);

    let app_settings_guard = app.app_settings.lock().unwrap();
    let always_search_all_sources_captured = app_settings_guard.always_search_all_sources;
    let mut configured_search_order = app_settings_guard.auto_search_source_order.clone();
    drop(app_settings_guard);

    if !local_cache_hit || always_search_all_sources_captured {
        update_all_online_search_status(app, AutoSearchStatus::Searching); // 调用本模块的函数
    } else {
        // 本地缓存命中且不要求搜索所有源，则其他在线源设为未尝试
        update_all_online_search_status(app, AutoSearchStatus::NotAttempted);
        log::info!("[AppFetchCore AutoFetch] 本地缓存命中且无需搜索所有源，流程提前结束。");
        return; // 提前结束，因为本地缓存已处理
    }

    let http_client_clone = app.http_client.clone();
    let qq_status_arc_clone = Arc::clone(&app.qqmusic_auto_search_status);
    let kugou_status_arc_clone = Arc::clone(&app.kugou_auto_search_status);
    let netease_status_arc_clone = Arc::clone(&app.netease_auto_search_status);
    let amll_db_status_arc_clone = Arc::clone(&app.amll_db_auto_search_status);

    let netease_client_arc_clone = Arc::clone(&app.netease_client);
    let amll_index_arc_clone = Arc::clone(&app.amll_index);
    let amll_db_repo_url_base_clone = app.amll_db_repo_url_base.clone();
    let auto_fetch_result_tx_clone = app.auto_fetch_result_tx.clone();
    let media_connector_config_for_task = Arc::clone(&app.media_connector_config);
    let media_connector_command_tx_for_task = app.media_connector_command_tx.clone();

    let query_str_for_online_sources = if smtc_artists_vec.is_empty() {
        smtc_title.clone()
    } else {
        format!("{} {}", smtc_title, smtc_artists_vec.join(" "))
    };

    configured_search_order.retain(|s| *s != AutoSearchSource::LocalCache);

    let mut sources_to_try_ordered: Vec<SearchSourceEntry> = Vec::new();

    for source_id_val_ref in &configured_search_order {
        let source_id_val = *source_id_val_ref;
        let status_arc_ref = match source_id_val {
            AutoSearchSource::QqMusic => &qq_status_arc_clone,
            AutoSearchSource::Kugou => &kugou_status_arc_clone,
            AutoSearchSource::Netease => &netease_status_arc_clone,
            AutoSearchSource::AmllDb => &amll_db_status_arc_clone,
            AutoSearchSource::LocalCache => continue, // Should have been filtered
        };
        let future: Pin<
            Box<dyn Future<Output = Result<ProcessedLyricsSourceData, LyricFetchError>> + Send>,
        > = match source_id_val {
            AutoSearchSource::QqMusic => {
                let client_inner = http_client_clone.clone();
                let query_inner = query_str_for_online_sources.clone();
                Box::pin(async move {
                    // 假设 fetch_qq_music_lyrics_core 也在 app_fetch_core.rs 中
                    fetch_qq_music_lyrics_core(&client_inner, &query_inner).await
                })
            }
            AutoSearchSource::Kugou => {
                let client_inner = http_client_clone.clone();
                let query_inner = query_str_for_online_sources.clone();
                Box::pin(
                    async move { fetch_kugou_music_lyrics_core(&client_inner, &query_inner).await },
                )
            }
            AutoSearchSource::Netease => {
                let client_inner = http_client_clone.clone();
                let nc_arc_inner = netease_client_arc_clone.clone();
                let query_inner = query_str_for_online_sources.clone();
                Box::pin(async move {
                    fetch_netease_music_lyrics_core(&client_inner, nc_arc_inner, &query_inner).await
                })
            }
            AutoSearchSource::AmllDb => {
                let client_inner = http_client_clone.clone();
                let index_arc_inner = amll_index_arc_clone.clone();
                let base_url_inner = amll_db_repo_url_base_clone.clone();
                let title_for_amll_task = smtc_title.clone(); // smtc_title is from outer scope
                let artists_for_amll_task = smtc_artists_vec.clone(); // smtc_artists_vec is from outer scope
                Box::pin(async move {
                    fetch_amll_db_lyrics_for_smtc_match_core(
                        &client_inner,
                        index_arc_inner,
                        &base_url_inner,
                        &title_for_amll_task,
                        &artists_for_amll_task,
                    )
                    .await
                })
            }
            AutoSearchSource::LocalCache => unreachable!(),
        };
        sources_to_try_ordered.push((source_id_val, Arc::clone(status_arc_ref), future));
    }

    if sources_to_try_ordered.is_empty() && (!local_cache_hit || always_search_all_sources_captured)
    {
        // 如果没有在线源可尝试，并且本地缓存未命中 (或要求搜索所有源但没有在线源)
        // 则发送 NotFound
        if !local_cache_hit {
            // 只有在本地缓存也未命中的情况下才发送最终的 NotFound
            if app
                .auto_fetch_result_tx
                .send(AutoFetchResult::NotFound)
                .is_err()
            {
                log::error!("[AppFetchCore AutoFetch] 无在线源可尝试，发送 NotFound 失败。");
            }
        }
        return;
    }

    // 使用 app.tokio_runtime 来 spawn 任务
    app.tokio_runtime.spawn(async move {
        let mut first_success_achieved_in_task = false;
        let mut any_success_this_cycle_in_task = false;

        for (current_source_id, current_status_arc, fetch_future) in sources_to_try_ordered.into_iter() {
            let current_source_display_name = current_source_id.display_name();
            log::trace!("[AppFetchCore AutoFetch TASK] 正在尝试源: '{current_source_display_name}'");

            match fetch_future.await {
                Ok(processed_data) => {
                    log::trace!("[AppFetchCore AutoFetch TASK] 源: '{current_source_display_name}' 成功获取数据。");
                    any_success_this_cycle_in_task = true;
                    let result_to_send = AutoFetchResult::Success {
                        source: current_source_id,
                        source_format: processed_data.format,
                        main_lyrics: processed_data.main_lyrics,
                        translation_lrc: processed_data.translation_lrc,
                        romanization_qrc: processed_data.romanization_qrc,
                        romanization_lrc: processed_data.romanization_lrc,
                        krc_translation_lines: processed_data.krc_translation_lines,
                        platform_metadata: processed_data.platform_metadata,
                    };
                    if auto_fetch_result_tx_clone.send(result_to_send).is_ok() {
                        *current_status_arc.lock().unwrap() = AutoSearchStatus::Success(processed_data.format);
                        if !first_success_achieved_in_task {
                            first_success_achieved_in_task = true;
                        }
                        if !always_search_all_sources_captured && first_success_achieved_in_task {
                            log::trace!("[AppFetchCore AutoFetch TASK] 已找到第一个结果且无需搜索所有源，停止后续搜索。");
                            break;
                        }
                    } else {
                        log::error!("[AppFetchCore AutoFetch TASK] 源: '{current_source_display_name}' 发送结果到主线程失败。");
                        *current_status_arc.lock().unwrap() = AutoSearchStatus::Error("发送结果失败".to_string());
                    }
                }
                Err(fetch_error) => {
                    log::info!("[AppFetchCore AutoFetch TASK] 源: '{current_source_display_name}' 下载失败: {fetch_error}");
                    match fetch_error {
                        LyricFetchError::NotFound => {
                            *current_status_arc.lock().unwrap() = AutoSearchStatus::NotFound;
                        }
                        _ => {
                            *current_status_arc.lock().unwrap() = AutoSearchStatus::Error(fetch_error.to_string());
                        }
                    }
                }
            }
        }

        if !any_success_this_cycle_in_task && !local_cache_hit { // 只有在线源和本地缓存都未成功时
            if auto_fetch_result_tx_clone.send(AutoFetchResult::NotFound).is_err() {
                log::error!("[AppFetchCore AutoFetch TASK] 发送最终 AutoFetchResult::NotFound 失败。");
            }
        }

        // AMLL Player 通信逻辑 (与 app.rs 中版本一致)
        let final_config_is_enabled = media_connector_config_for_task.lock().unwrap().enabled;
        if final_config_is_enabled && !any_success_this_cycle_in_task && !local_cache_hit
            && let Some(tx) = &media_connector_command_tx_for_task {
                log::info!("[AppFetchCore AutoFetch TASK] 未找到任何歌词，尝试发送空TTML给AMLL Player。");
                let empty_ttml_body = ws_protocol::Body::SetLyricFromTTML { data: "".into() };
                if tx.send(crate::amll_connector::ConnectorCommand::SendProtocolBody(empty_ttml_body)).is_err() {
                    log::error!("[AppFetchCore AutoFetch TASK] 发送空TTML (SendProtocolBody) 失败。");
                }
            }
    });
}

pub fn trigger_manual_refetch_for_source(
    app: &mut UniLyricApp,
    source_to_refetch: AutoSearchSource,
) {
    let track_info_opt = app
        .tokio_runtime
        .block_on(async { app.current_media_info.lock().await.clone() });

    let track_info = match track_info_opt {
        Some(info) => info,
        None => {
            log::warn!("[手动重搜] 无SMTC信息，无法手动重搜。");
            let status_arc_to_update = match source_to_refetch {
                AutoSearchSource::QqMusic => &app.qqmusic_auto_search_status,
                AutoSearchSource::Kugou => &app.kugou_auto_search_status,
                AutoSearchSource::Netease => &app.netease_auto_search_status,
                AutoSearchSource::AmllDb => &app.amll_db_auto_search_status,
                AutoSearchSource::LocalCache => {
                    log::warn!("[手动重搜] 本地缓存不支持此类型的重搜。");
                    return;
                }
            };
            *status_arc_to_update.lock().unwrap() =
                AutoSearchStatus::Error("无SMTC信息，无法重搜".to_string());
            return;
        }
    };

    let smtc_title = match track_info.title.as_ref() {
        Some(t) if !t.is_empty() && t != "无歌曲" && t != "无活动会话" => t.clone(),
        _ => {
            log::warn!(
                "[手动重搜] 无有效SMTC歌曲 (标题: {:?})，无法手动重搜。",
                track_info.title
            );
            let status_arc_to_update = match source_to_refetch {
                AutoSearchSource::QqMusic => &app.qqmusic_auto_search_status,
                AutoSearchSource::Kugou => &app.kugou_auto_search_status,
                AutoSearchSource::Netease => &app.netease_auto_search_status,
                AutoSearchSource::AmllDb => &app.amll_db_auto_search_status,
                AutoSearchSource::LocalCache => return,
            };
            *status_arc_to_update.lock().unwrap() =
                AutoSearchStatus::Error("SMTC歌曲标题无效".to_string());
            return;
        }
    };

    let smtc_artists_vec: Vec<String> = track_info
        .artist
        .clone()
        .map(|s| {
            s.split(['/', '、', ',', ';'])
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_else(Vec::new);

    // 注意：如果 smtc_title 为空（理论上已被上面检查过），也应提前返回
    if smtc_title.is_empty() {
        log::warn!("[手动重搜] SMTC 标题为空，无法手动重搜。");
        let status_arc_to_update = match source_to_refetch {
            AutoSearchSource::QqMusic => &app.qqmusic_auto_search_status,
            AutoSearchSource::Kugou => &app.kugou_auto_search_status,
            AutoSearchSource::Netease => &app.netease_auto_search_status,
            AutoSearchSource::AmllDb => &app.amll_db_auto_search_status,
            AutoSearchSource::LocalCache => return,
        };
        *status_arc_to_update.lock().unwrap() = AutoSearchStatus::Error("SMTC标题为空".to_string());
        return;
    }

    log::info!(
        "[手动重搜] 为 '{}' 手动重搜 {:?}...",
        if smtc_artists_vec.is_empty() {
            smtc_title.clone()
        } else {
            format!("{} - {}", smtc_title, smtc_artists_vec.join(", "))
        },
        source_to_refetch.display_name()
    );

    let (status_arc_for_refetch, result_arc_for_refetch_opt): RefetchStatusAndResult =
        match source_to_refetch {
            AutoSearchSource::QqMusic => (
                Arc::clone(&app.qqmusic_auto_search_status),
                Some(Arc::clone(&app.last_qq_search_result)),
            ),
            AutoSearchSource::Kugou => (
                Arc::clone(&app.kugou_auto_search_status),
                Some(Arc::clone(&app.last_kugou_search_result)),
            ),
            AutoSearchSource::Netease => (
                Arc::clone(&app.netease_auto_search_status),
                Some(Arc::clone(&app.last_netease_search_result)),
            ),
            AutoSearchSource::AmllDb => (
                Arc::clone(&app.amll_db_auto_search_status),
                Some(Arc::clone(&app.last_amll_db_search_result)),
            ),
            AutoSearchSource::LocalCache => {
                log::warn!("[AppFetchCore ManualRefetch] 本地缓存不支持手动重搜。");
                return;
            }
        };
    *status_arc_for_refetch.lock().unwrap() = AutoSearchStatus::Searching;

    let http_client_clone = app.http_client.clone();
    let netease_client_arc_clone = Arc::clone(&app.netease_client);
    let amll_index_arc_clone = Arc::clone(&app.amll_index);
    let amll_repo_base_clone = app.amll_db_repo_url_base.clone();
    let tokio_runtime_clone: Arc<tokio::runtime::Runtime> = Arc::clone(&app.tokio_runtime);

    let query_str_for_refetch = if smtc_artists_vec.is_empty() {
        smtc_title.clone()
    } else {
        format!("{} {}", smtc_title, smtc_artists_vec.join(" "))
    };

    // 克隆 SMTC 标题和艺术家列表，因为它们将被移动到异步块中
    let smtc_title_for_task = smtc_title.clone();
    let smtc_artists_vec_for_task = smtc_artists_vec.clone();

    tokio_runtime_clone.spawn(async move {
        let fetch_result: Result<ProcessedLyricsSourceData, LyricFetchError> =
            match source_to_refetch {
                AutoSearchSource::QqMusic => {
                    app_fetch_core::fetch_qq_music_lyrics_core(
                        &http_client_clone,
                        &query_str_for_refetch,
                    )
                    .await
                }
                AutoSearchSource::Kugou => {
                    app_fetch_core::fetch_kugou_music_lyrics_core(
                        &http_client_clone,
                        &query_str_for_refetch,
                    )
                    .await
                }
                AutoSearchSource::Netease => {
                    app_fetch_core::fetch_netease_music_lyrics_core(
                        &http_client_clone,
                        netease_client_arc_clone,
                        &query_str_for_refetch,
                    )
                    .await
                }
                AutoSearchSource::AmllDb => {
                    app_fetch_core::fetch_amll_db_lyrics_for_smtc_match_core(
                        &http_client_clone,
                        amll_index_arc_clone,
                        &amll_repo_base_clone,
                        &smtc_title_for_task,
                        &smtc_artists_vec_for_task,
                    )
                    .await
                }
                AutoSearchSource::LocalCache => {
                    // 理论上不应执行，因为上面已经 return
                    log::error!("[ManualRefetchTask] 内部错误：不应为本地缓存触发重搜任务。");
                    return; // 从异步任务中返回
                }
            };

        match fetch_result {
            Ok(processed_data) => {
                *status_arc_for_refetch.lock().unwrap() =
                    AutoSearchStatus::Success(processed_data.format);
                if let Some(result_arc) = result_arc_for_refetch_opt {
                    *result_arc.lock().unwrap() = Some(processed_data);
                }
            }
            Err(fetch_error) => {
                log::error!(
                    "[手动重搜] 源 {:?} 获取歌词失败: {}",
                    source_to_refetch.display_name(),
                    fetch_error
                );
                let final_status = match fetch_error {
                    LyricFetchError::NotFound => AutoSearchStatus::NotFound,
                    _ => AutoSearchStatus::Error(fetch_error.to_string()),
                };
                *status_arc_for_refetch.lock().unwrap() = final_status;
                if let Some(result_arc) = result_arc_for_refetch_opt {
                    *result_arc.lock().unwrap() = None; // 失败时清除旧的缓存结果
                }
            }
        }
    });
}
