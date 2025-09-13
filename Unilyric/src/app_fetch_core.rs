use crate::app_definition::UniLyricApp;
use crate::types::{AutoFetchResult, AutoSearchSource, AutoSearchStatus};
use image_hasher::HasherConfig;
use lyrics_helper_core::model::track::FullLyricsResult;
use lyrics_helper_core::{
    ConversionInput, ConversionOptions, InputFile, LyricFormat, MatchType, RawLyrics, SearchResult,
};
use lyrics_helper_rs::SearchMode;
use smtc_suite::NowPlayingInfo;

use lyrics_helper_core::model::track::{LyricsAndMetadata, Track};
use std::path::PathBuf;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

const COVER_SIMILARITY_THRESHOLD: u32 = 10;

struct SearchContext {
    helper: Arc<tokio::sync::Mutex<lyrics_helper_rs::LyricsHelper>>,
    result_tx: std::sync::mpsc::Sender<AutoFetchResult>,
    app_settings: crate::app_settings::AppSettings,
    target_format: LyricFormat,
    original_track_info: NowPlayingInfo,
    cover_cache_dir: Option<PathBuf>,
    cancellation_token: CancellationToken,
}

fn is_track_match(
    now_playing: &NowPlayingInfo,
    cache_entry: &crate::types::LocalLyricCacheEntry,
) -> bool {
    let title_match = now_playing.title.as_deref().is_some_and(|playing_title| {
        playing_title
            .trim()
            .eq_ignore_ascii_case(cache_entry.smtc_title.trim())
    });

    if !title_match {
        return false;
    }

    let artists_playing = now_playing.artist.as_deref().unwrap_or_default();
    if artists_playing.is_empty() && cache_entry.smtc_artists.is_empty() {
        return true;
    }

    let playing_artists_set: std::collections::HashSet<String> = artists_playing
        .split(['/', '、', ',', ';'])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let cached_artists_set: std::collections::HashSet<String> = cache_entry
        .smtc_artists
        .iter()
        .map(|s| s.trim().to_lowercase())
        .collect();

    if playing_artists_set == cached_artists_set {
        return true;
    }

    if !playing_artists_set.is_empty() && playing_artists_set.is_subset(&cached_artists_set) {
        return true;
    }

    false
}

async fn execute_search_and_process(
    track_to_search: Track<'_>,
    search_mode: &SearchMode,
    context: &SearchContext,
    log_prefix: &str,
) -> Result<Option<LyricsAndMetadata>, lyrics_helper_rs::LyricsHelperError> {
    let search_result = {
        let future_res = {
            let helper_guard = context.helper.lock().await;
            helper_guard.search_lyrics_comprehensive(
                &track_to_search,
                search_mode,
                Some(context.cancellation_token.clone()),
            )
        };
        match future_res {
            Ok(future) => future.await,
            Err(e) => Err(e),
        }
    };

    match search_result {
        Ok(Some(comprehensive_result)) => {
            info!("[{}] 成功找到歌词，正在进行转换...", log_prefix);

            let mut lyrics_and_metadata = comprehensive_result.primary_lyric_result;
            let source: AutoSearchSource = lyrics_and_metadata
                .source_track
                .provider_name
                .clone()
                .into();

            if context.app_settings.auto_apply_metadata_stripper {
                lyrics_helper_rs::converter::processors::metadata_stripper::strip_descriptive_metadata_lines(
                    &mut lyrics_and_metadata.lyrics.parsed.lines,
                    &context.app_settings.metadata_stripper,
                );
            }
            if context.app_settings.auto_apply_agent_recognizer {
                lyrics_helper_rs::converter::processors::agent_recognizer::recognize_agents(
                    &mut lyrics_and_metadata.lyrics.parsed,
                );
            }

            let output_text = match lyrics_helper_rs::LyricsHelper::generate_lyrics_from_parsed::<
                std::hash::RandomState,
            >(
                lyrics_and_metadata.lyrics.parsed.clone(),
                context.target_format,
                Default::default(),
                None,
            )
            .await
            {
                Ok(res) => res.output_lyrics,
                Err(e) => {
                    error!("[{}] 搜索结果转换失败: {}", log_prefix, e);
                    String::new()
                }
            };

            if context.app_settings.auto_cache
                && lyrics_and_metadata.source_track.match_type == MatchType::Perfect
            {
                info!("[AutoCache] 歌词匹配度为 Perfect，请求缓存到本地。");
                if context
                    .result_tx
                    .send(AutoFetchResult::RequestCache)
                    .is_err()
                {
                    error!("[AutoCache] 发送 RequestCache 请求到主线程失败。");
                }
            }

            let smtc_artists_joined = context
                .original_track_info
                .artist
                .clone()
                .unwrap_or_default();
            let smtc_title = context
                .original_track_info
                .title
                .clone()
                .unwrap_or_default();

            let lyrics_ready_result = AutoFetchResult::LyricsReady {
                source,
                lyrics_and_metadata: Box::new(lyrics_and_metadata.clone()),
                output_text,
                title: smtc_title.clone(),
                artist: smtc_artists_joined.clone(),
            };
            if context.result_tx.send(lyrics_ready_result).is_err() {
                error!("[{}] 发送 LyricsReady 结果到主线程失败。", log_prefix);
                return Ok(Some(lyrics_and_metadata));
            }

            let smtc_cover_hash = context.original_track_info.cover_data_hash.unwrap_or(0);
            let final_cover_data = fetch_and_validate_cover(
                context.helper.clone(),
                &comprehensive_result.all_search_candidates,
                context.original_track_info.cover_data.clone(),
                smtc_cover_hash,
                context.cover_cache_dir.clone(),
                log_prefix,
            )
            .await;

            let cover_result = AutoFetchResult::CoverUpdate {
                title: smtc_title,
                artist: smtc_artists_joined,
                cover_data: final_cover_data,
            };
            if context.result_tx.send(cover_result).is_err() {
                error!("[{}] 发送封面更新结果到主线程失败。", log_prefix);
            }

            Ok(Some(lyrics_and_metadata))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(e),
    }
}

pub(super) fn initial_auto_fetch_and_send_lyrics(
    app: &mut UniLyricApp,
    track_info: NowPlayingInfo,
) {
    *app.fetcher.local_cache_status.lock().unwrap() = AutoSearchStatus::Searching;

    let cache_index = app.local_cache.index.lock().unwrap();
    let matched_entry = cache_index
        .iter()
        .find(|entry| is_track_match(&track_info, entry));

    if let Some(entry) = matched_entry {
        info!(
            "[LocalCache] 在本地缓存中找到匹配项: {:?}",
            entry.ttml_filename
        );
        if let Some(cache_dir) = &app.local_cache.dir_path {
            let file_path = cache_dir.join(&entry.ttml_filename);
            match std::fs::read_to_string(&file_path) {
                Ok(ttml_content) => {
                    let helper_clone = Arc::clone(&app.lyrics_helper_state.helper);

                    let result_tx = app.fetcher.result_tx.clone();

                    app.tokio_runtime.spawn(async move {
                        let main_lyric =
                            InputFile::new(ttml_content.clone(), LyricFormat::Ttml, None, None);

                        let input = ConversionInput {
                            main_lyric,
                            translations: vec![],
                            romanizations: vec![],
                            target_format: LyricFormat::Ttml,
                            user_metadata_overrides: None,
                            additional_metadata: None,
                        };

                        let options = ConversionOptions::default();

                        match helper_clone.lock().await.convert_lyrics(&input, &options) {
                            Ok(conversion_result) => {
                                let parsed_data = conversion_result.source_data;

                                let full_lyrics_result = FullLyricsResult {
                                    raw: RawLyrics {
                                        content: ttml_content,
                                        ..Default::default()
                                    },
                                    parsed: parsed_data,
                                };

                                let lyrics_and_metadata = LyricsAndMetadata {
                                    lyrics: full_lyrics_result,
                                    source_track: Default::default(),
                                };

                                let result_to_send = AutoFetchResult::LyricsSuccess {
                                    source: AutoSearchSource::LocalCache,
                                    lyrics_and_metadata: Box::new(lyrics_and_metadata),
                                    title: track_info.title.clone().unwrap_or_default(),
                                    artist: track_info.artist.clone().unwrap_or_default(),
                                };

                                if result_tx.send(result_to_send).is_err() {
                                    error!(
                                        "[LocalCache Task] 发送本地缓存的成功结果到主线程失败。"
                                    );
                                }
                            }
                            Err(e) => {
                                error!("[LocalCache] 解析缓存的TTML文件时发生错误: {}", e);
                            }
                        }
                    });

                    return;
                }
                Err(e) => {
                    error!("[LocalCache] 读取缓存文件 {:?} 失败: {}", file_path, e);
                    *app.fetcher.local_cache_status.lock().unwrap() =
                        AutoSearchStatus::Error(e.to_string());
                }
            }
        }
    } else {
        info!("[LocalCache] 本地缓存未命中。");
        *app.fetcher.local_cache_status.lock().unwrap() = AutoSearchStatus::NotFound;
    }

    let smtc_title = match track_info.title.as_deref() {
        Some(t) if !t.trim().is_empty() && t != "无歌曲" && t != "无活动会话" => {
            t.trim().to_string()
        }
        _ => {
            info!("[AutoFetch] SMTC 无有效歌曲名称，跳过在线搜索。");
            return;
        }
    };

    let smtc_artists: Vec<String> = track_info
        .artist
        .as_deref()
        .map(|s| {
            s.split(['/', '、', ',', ';'])
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let runtime = app.tokio_runtime.clone();
    let result_tx = app.fetcher.result_tx.clone();

    let cancellation_token = CancellationToken::new();
    app.fetcher.current_fetch_cancellation_token = Some(cancellation_token.clone());

    app.fetcher.last_source_format = None;
    *app.fetcher.qqmusic_status.lock().unwrap() = AutoSearchStatus::Searching;
    *app.fetcher.kugou_status.lock().unwrap() = AutoSearchStatus::Searching;
    *app.fetcher.netease_status.lock().unwrap() = AutoSearchStatus::Searching;
    *app.fetcher.amll_db_status.lock().unwrap() = AutoSearchStatus::Searching;

    let context = SearchContext {
        helper: Arc::clone(&app.lyrics_helper_state.helper),
        result_tx: app.fetcher.result_tx.clone(),
        app_settings: app.app_settings.lock().unwrap().clone(),
        target_format: app.lyrics.target_format,
        original_track_info: track_info,
        cover_cache_dir: app.local_cache.cover_cache_dir.clone(),
        cancellation_token,
    };

    runtime.spawn(async move {
        let artists_slices: Vec<&str> = smtc_artists.iter().map(|s| s.as_str()).collect();
        let track_to_search = Track {
            title: Some(&smtc_title),
            artists: if smtc_artists.is_empty() {
                None
            } else {
                Some(&artists_slices)
            },
            album: context.original_track_info.album_title.as_deref(),
            duration: context.original_track_info.duration_ms,
        };

        let search_mode = {
            let mut providers = lyrics_helper_rs::ProviderName::all();

            if context.app_settings.prioritize_amll_db {
                providers.retain(|p| *p != lyrics_helper_rs::ProviderName::AmllTtmlDatabase);
            }

            if context.app_settings.use_provider_subset {
                let user_subset: Vec<_> = context
                    .app_settings
                    .auto_search_provider_subset
                    .iter()
                    .filter_map(|s| s.parse().ok())
                    .collect();
                providers.retain(|p| user_subset.contains(p));
            }

            if context.app_settings.always_search_all_sources {
                SearchMode::Subset(providers)
            } else {
                SearchMode::Ordered
            }
        };

        let result =
            execute_search_and_process(track_to_search, &search_mode, &context, "AutoFetch").await;

        match result {
            Ok(None) => {
                info!("[AutoFetch] 所有源均未找到歌词。");
                if result_tx.send(AutoFetchResult::NotFound).is_err() {
                    error!("[AutoFetch Task] 发送 NotFound 结果到主线程失败。");
                }
            }
            Err(e) => {
                if let lyrics_helper_rs::LyricsHelperError::Cancelled = e {
                    info!("[AutoFetch] 自动搜索任务被取消。");
                    return;
                }
                error!("[AutoFetch] 搜索时发生错误: {}", e);
                if result_tx
                    .send(AutoFetchResult::FetchError(e.into()))
                    .is_err()
                {
                    error!("[AutoFetch Task] 发送 Error 结果到主线程失败。");
                }
            }
            Ok(Some(_)) => {}
        }
    });
}

/// 触发对特定源的手动重新搜索。
pub(super) fn trigger_manual_refetch_for_source(
    app: &mut UniLyricApp,
    source_to_refetch: AutoSearchSource,
) {
    let track_info = match app.player.current_now_playing.clone() {
        info if info.title.is_some() => info,
        _ => {
            warn!("[ManualRefetch] 无SMTC信息，无法重新搜索。");
            return;
        }
    };

    let smtc_title = track_info.title.as_deref().unwrap_or_default().to_string();
    let smtc_artists: Vec<String> = track_info
        .artist
        .as_deref()
        .map(|s| s.split('/').map(|n| n.trim().to_string()).collect())
        .unwrap_or_default();

    let status_arc_to_update = match source_to_refetch {
        AutoSearchSource::QqMusic => Arc::clone(&app.fetcher.qqmusic_status),
        AutoSearchSource::Kugou => Arc::clone(&app.fetcher.kugou_status),
        AutoSearchSource::Netease => Arc::clone(&app.fetcher.netease_status),
        AutoSearchSource::AmllDb => Arc::clone(&app.fetcher.amll_db_status),
        _ => return,
    };
    *status_arc_to_update.lock().unwrap() = AutoSearchStatus::Searching;

    let cancellation_token = CancellationToken::new();
    app.fetcher.current_fetch_cancellation_token = Some(cancellation_token.clone());

    let context = SearchContext {
        helper: Arc::clone(&app.lyrics_helper_state.helper),
        result_tx: app.fetcher.result_tx.clone(),
        app_settings: app.app_settings.lock().unwrap().clone(),
        target_format: app.lyrics.target_format,
        original_track_info: track_info,
        cover_cache_dir: app.local_cache.cover_cache_dir.clone(),
        cancellation_token,
    };

    let runtime = app.tokio_runtime.clone();

    runtime.spawn(async move {
        let artists_slices: Vec<&str> = smtc_artists.iter().map(|s| s.as_str()).collect();
        let track_to_search = Track {
            title: Some(&smtc_title),
            artists: if artists_slices.is_empty() {
                None
            } else {
                Some(&artists_slices)
            },
            album: context.original_track_info.album_title.as_deref(),
            duration: context.original_track_info.duration_ms,
        };

        let provider_enum = match source_to_refetch {
            AutoSearchSource::QqMusic => lyrics_helper_rs::ProviderName::QQMusic,
            AutoSearchSource::Netease => lyrics_helper_rs::ProviderName::Netease,
            AutoSearchSource::Kugou => lyrics_helper_rs::ProviderName::Kugou,
            AutoSearchSource::AmllDb => lyrics_helper_rs::ProviderName::AmllTtmlDatabase,
            _ => {
                *status_arc_to_update.lock().unwrap() =
                    AutoSearchStatus::Error("不支持的重搜源".to_string());
                return;
            }
        };
        let search_mode = SearchMode::Specific(provider_enum);

        let result =
            execute_search_and_process(track_to_search, &search_mode, &context, "ManualRefetch")
                .await;

        match result {
            Ok(Some(lyrics_and_metadata)) => {
                let source_format = lyrics_and_metadata.lyrics.parsed.source_format;
                *status_arc_to_update.lock().unwrap() = AutoSearchStatus::Success(source_format);
            }
            Ok(None) => {
                *status_arc_to_update.lock().unwrap() = AutoSearchStatus::NotFound;
            }
            Err(e) => {
                if let lyrics_helper_rs::LyricsHelperError::Cancelled = e {
                    info!("[ManualRefetch] 手动重搜任务被取消。");
                    *status_arc_to_update.lock().unwrap() = AutoSearchStatus::NotAttempted;
                } else {
                    *status_arc_to_update.lock().unwrap() = AutoSearchStatus::Error(e.to_string());
                }
            }
        }
    });
}

pub(super) fn clear_last_fetch_results(app: &mut UniLyricApp) {
    *app.fetcher.last_qq_result.lock().unwrap() = None;
    *app.fetcher.last_kugou_result.lock().unwrap() = None;
    *app.fetcher.last_netease_result.lock().unwrap() = None;
    *app.fetcher.last_amll_db_result.lock().unwrap() = None;
    app.fetcher.current_ui_populated = false;
}

/// 对比两张图片的感知哈希，判断它们是否相似。
fn are_images_similar(image_data1: &[u8], image_data2: &[u8]) -> bool {
    const PRE_RESIZE_DIM: u32 = 256;
    let check = || -> Result<bool, String> {
        let image1 =
            image::load_from_memory(image_data1).map_err(|e| format!("无法加载图片1: {}", e))?;
        let image2 =
            image::load_from_memory(image_data2).map_err(|e| format!("无法加载图片2: {}", e))?;

        let thumbnail1 = image1.thumbnail(PRE_RESIZE_DIM, PRE_RESIZE_DIM);
        let thumbnail2 = image2.thumbnail(PRE_RESIZE_DIM, PRE_RESIZE_DIM);

        let hasher = HasherConfig::new().to_hasher();

        let hash1 = hasher.hash_image(&thumbnail1);
        let hash2 = hasher.hash_image(&thumbnail2);
        let distance = hash1.dist(&hash2);

        info!(
            "封面相似度距离: {} (阈值: <= {})",
            distance, COVER_SIMILARITY_THRESHOLD
        );

        Ok(distance <= COVER_SIMILARITY_THRESHOLD)
    };

    match check() {
        Ok(is_similar) => is_similar,
        Err(e) => {
            info!("图片相似度对比失败: {}，使用 SMTC 封面", e);
            false
        }
    }
}

/// 从搜索候选中获取最佳封面，并与SMTC封面进行验证和比较。
async fn fetch_and_validate_cover(
    helper: std::sync::Arc<tokio::sync::Mutex<lyrics_helper_rs::LyricsHelper>>,
    candidates: &[SearchResult],
    smtc_cover_data: Option<Vec<u8>>,
    smtc_cover_hash: u64,
    cache_dir: Option<PathBuf>,
    log_prefix: &str,
) -> Option<Vec<u8>> {
    if smtc_cover_hash != 0
        && let Some(dir) = &cache_dir
    {
        let cache_file_path = dir.join(format!("{}.jpg", smtc_cover_hash));
        if cache_file_path.exists()
            && let Ok(data) = std::fs::read(&cache_file_path)
        {
            info!("[CoverCache] 命中封面缓存: {}", cache_file_path.display());

            let path_clone = cache_file_path.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = filetime::set_file_mtime(path_clone, filetime::FileTime::now()) {
                    warn!("[CoverCache] 更新文件时间戳失败: {}", e);
                }
            });

            return Some(data);
        }
    }

    let provider_cover = {
        let helper_guard = helper.lock().await;
        helper_guard.get_best_cover(candidates).await
    };

    if let (Some(provider_bytes), Some(smtc_bytes)) = (provider_cover, smtc_cover_data) {
        let provider_bytes_for_check = provider_bytes.clone();

        let is_similar_result = tokio::task::spawn_blocking(move || {
            are_images_similar(&provider_bytes_for_check, &smtc_bytes)
        })
        .await;

        match is_similar_result {
            Ok(true) => {
                info!("{}: 封面验证成功，使用提供商的封面。", log_prefix);
                if smtc_cover_hash != 0
                    && let Some(dir) = cache_dir
                {
                    let cache_file_path = dir.join(format!("{}.jpg", smtc_cover_hash));
                    if let Err(e) = std::fs::write(&cache_file_path, &provider_bytes) {
                        warn!("[CoverCache] 写入封面缓存失败: {}", e);
                    } else {
                        info!("[CoverCache] 封面缓存至: {}", cache_file_path.display());
                    }
                }
                Some(provider_bytes)
            }
            Ok(false) => {
                info!("{}: 封面验证失败（不匹配）。", log_prefix);
                None
            }
            Err(e) => {
                error!("[CoverCompare] spawn_blocking 任务失败: {}", e);
                None
            }
        }
    } else {
        None
    }
}
