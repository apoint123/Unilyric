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

pub(super) fn initial_auto_fetch_and_send_lyrics(
    app: &mut UniLyricApp,
    track_info: NowPlayingInfo,
) {
    info!(
        "[initial_auto_fetch_and_send_lyrics] 封面哈希: {:?}",
        track_info.cover_data_hash
    );

    let mut lyrics_found_in_cache = false;
    let mut cover_found_in_cache = false;

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
                    lyrics_found_in_cache = true;
                    *app.fetcher.local_cache_status.lock().unwrap() =
                        AutoSearchStatus::Success(LyricFormat::Ttml);

                    let helper_clone = Arc::clone(&app.lyrics_helper_state.helper);
                    let result_tx = app.fetcher.result_tx.clone();
                    let track_info_clone_for_lyrics = track_info.clone();

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
                                    title: track_info_clone_for_lyrics.title.unwrap_or_default(),
                                    artist: track_info_clone_for_lyrics.artist.unwrap_or_default(),
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

                    if let (Some(hash), Some(cover_cache_dir)) =
                        (track_info.cover_data_hash, &app.local_cache.cover_cache_dir)
                        && hash != 0
                    {
                        let cover_path = cover_cache_dir.join(format!("{}.jpg", hash));
                        if cover_path.exists() {
                            info!(
                                "[CoverCache] 在本地缓存中找到匹配的封面: {}",
                                cover_path.display()
                            );
                            match std::fs::read(&cover_path) {
                                Ok(cover_data) => {
                                    cover_found_in_cache = true;
                                    let cover_result = AutoFetchResult::CoverUpdate {
                                        title: track_info.title.clone().unwrap_or_default(),
                                        artist: track_info.artist.clone().unwrap_or_default(),
                                        cover_data: Some(cover_data),
                                    };
                                    if app.fetcher.result_tx.send(cover_result).is_err() {
                                        error!(
                                            "[CoverCache Task] 发送本地缓存的封面结果到主线程失败。"
                                        );
                                    }
                                }
                                Err(e) => {
                                    error!("[CoverCache] 读取缓存的封面文件失败: {e}");
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("[LocalCache] 读取缓存文件 {:?} 失败: {}", file_path, e);
                    *app.fetcher.local_cache_status.lock().unwrap() =
                        AutoSearchStatus::Error(e.to_string());
                }
            }
        }
    }

    if lyrics_found_in_cache && cover_found_in_cache {
        *app.fetcher.qqmusic_status.lock().unwrap() = AutoSearchStatus::NotAttempted;
        *app.fetcher.kugou_status.lock().unwrap() = AutoSearchStatus::NotAttempted;
        *app.fetcher.netease_status.lock().unwrap() = AutoSearchStatus::NotAttempted;
        *app.fetcher.amll_db_status.lock().unwrap() = AutoSearchStatus::NotAttempted;
        return;
    }

    if !lyrics_found_in_cache {
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

    let smtc_album = track_info.album_title.clone();
    let smtc_duration = track_info.duration_ms;
    let smtc_cover_data = track_info.cover_data.clone();

    let runtime = app.tokio_runtime.clone();
    let helper = Arc::clone(&app.lyrics_helper_state.helper);
    let app_settings = app.app_settings.lock().unwrap().clone();
    let result_tx = app.fetcher.result_tx.clone();
    let cover_cache_dir = app.local_cache.cover_cache_dir.clone();

    let cancellation_token = CancellationToken::new();
    app.fetcher.current_fetch_cancellation_token = Some(cancellation_token.clone());

    app.fetcher.last_source_format = None;
    *app.fetcher.qqmusic_status.lock().unwrap() = AutoSearchStatus::Searching;
    *app.fetcher.kugou_status.lock().unwrap() = AutoSearchStatus::Searching;
    *app.fetcher.netease_status.lock().unwrap() = AutoSearchStatus::Searching;
    *app.fetcher.amll_db_status.lock().unwrap() = AutoSearchStatus::Searching;

    runtime.spawn(async move {
        let artists_slices: Vec<&str> = smtc_artists.iter().map(|s| s.as_str()).collect();
        let track_to_search = Track {
            title: Some(&smtc_title),
            artists: if smtc_artists.is_empty() {
                None
            } else {
                Some(&artists_slices)
            },
            album: smtc_album.as_deref(),
            duration: smtc_duration,
        };

        let mut final_lyrics: Option<LyricsAndMetadata> = None;
        let mut final_candidates: Vec<SearchResult> = Vec::new();

        if !lyrics_found_in_cache {
            if app_settings.prioritize_amll_db {
                let amll_mode =
                    SearchMode::specific(lyrics_helper_rs::ProviderName::AmllTtmlDatabase);
                let amll_search_result = {
                    let future_res = {
                        let helper_guard = helper.lock().await;
                        helper_guard.search_lyrics_comprehensive(
                            &track_to_search,
                            &amll_mode,
                            Some(cancellation_token.clone()),
                        )
                    };
                    match future_res {
                        Ok(future) => future.await,
                        Err(e) => Err(e),
                    }
                };

                if let Ok(Some(comprehensive_result)) = amll_search_result
                    && comprehensive_result
                        .primary_lyric_result
                        .source_track
                        .match_type
                        >= MatchType::PrettyHigh
                {
                    final_lyrics = Some(comprehensive_result.primary_lyric_result);
                    final_candidates = comprehensive_result.all_search_candidates;
                }
            }

            let regular_search_mode = {
                let mut providers = lyrics_helper_rs::ProviderName::all();
                providers.retain(|p| *p != lyrics_helper_rs::ProviderName::AmllTtmlDatabase);

                if app_settings.use_provider_subset {
                    let user_subset: Vec<_> = app_settings
                        .auto_search_provider_subset
                        .iter()
                        .filter_map(|s| s.parse().ok())
                        .collect();
                    providers.retain(|p| user_subset.contains(p));
                }

                if app_settings.always_search_all_sources {
                    SearchMode::Subset(providers)
                } else {
                    SearchMode::Ordered
                }
            };

            let regular_search_result = {
                let future_res = {
                    let helper_guard = helper.lock().await;
                    helper_guard.search_lyrics_comprehensive(
                        &track_to_search,
                        &regular_search_mode,
                        Some(cancellation_token.clone()),
                    )
                };
                match future_res {
                    Ok(future) => future.await,
                    Err(e) => Err(e),
                }
            };

            match regular_search_result {
                Ok(Some(comprehensive_result)) => {
                    final_candidates = comprehensive_result.all_search_candidates;
                    if final_lyrics.is_none() {
                        final_lyrics = Some(comprehensive_result.primary_lyric_result);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    let lyrics_helper_rs::LyricsHelperError::Cancelled = e else {
                        return;
                    };

                    error!("[AutoFetch] 常规搜索时发生错误: {}", e);
                    if final_lyrics.is_none() {
                        if result_tx
                            .send(AutoFetchResult::FetchError(e.into()))
                            .is_err()
                        {
                            error!("[AutoFetch Task] 发送 Error 结果到主线程失败。");
                        }
                        return;
                    }
                }
            }
        } else {
            let all_providers_mode = SearchMode::Parallel;

            let search_result = {
                let future_res = {
                    let helper_guard = helper.lock().await;
                    helper_guard.search_lyrics_comprehensive(
                        &track_to_search,
                        &all_providers_mode,
                        Some(cancellation_token.clone()),
                    )
                };
                match future_res {
                    Ok(future) => future.await,
                    Err(e) => Err(e),
                }
            };

            match search_result {
                Ok(Some(comprehensive_result)) => {
                    final_candidates = comprehensive_result.all_search_candidates;
                }
                Ok(None) => {}
                Err(e) => {
                    error!("[AutoFetch] 为获取封面候选进行搜索时发生错误: {e}");
                }
            }
        }

        let mut lyrics_processed_online = false;

        if let Some(lyrics_and_metadata) = final_lyrics {
            lyrics_processed_online = true;
            let source: AutoSearchSource = lyrics_and_metadata
                .source_track
                .provider_name
                .clone()
                .into();

            if app_settings.auto_cache
                && lyrics_and_metadata.source_track.match_type
                    == lyrics_helper_core::MatchType::Perfect
            {
                info!("[AutoCache] 歌词匹配度为 Perfect，缓存到本地。");
                if result_tx.send(AutoFetchResult::RequestCache).is_err() {
                    error!("[AutoCache] 发送 RequestCache 请求到主线程失败。");
                }
            }

            let lyrics_result = AutoFetchResult::LyricsReady {
                source,
                lyrics_and_metadata: Box::new(lyrics_and_metadata),
                title: smtc_title.clone(),
                artist: smtc_artists.join("/"),
            };

            if result_tx.send(lyrics_result).is_err() {
                error!("[AutoFetch Task] 发送 LyricsReady 结果到主线程失败。");
            }
        }

        if !cover_found_in_cache && !final_candidates.is_empty() {
            let smtc_cover_hash = track_info.cover_data_hash.unwrap_or(0);

            let final_cover_data = fetch_and_validate_cover(
                helper.clone(),
                &final_candidates,
                smtc_cover_data,
                smtc_cover_hash,
                cover_cache_dir,
                "混合搜索",
            )
            .await;

            let cover_result = AutoFetchResult::CoverUpdate {
                title: smtc_title.clone(),
                artist: smtc_artists.join("/"),
                cover_data: final_cover_data,
            };

            if result_tx.send(cover_result).is_err() {
                error!("[AutoFetch Task] 发送封面更新结果到主线程失败。");
            }
        }
        if !lyrics_found_in_cache && !lyrics_processed_online {
            info!("[AutoFetch] 所有在线源均未找到歌词。");
            if result_tx.send(AutoFetchResult::NotFound).is_err() {
                error!("[AutoFetch Task] 发送 NotFound 结果到主线程失败。");
            }
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

    let helper = Arc::clone(&app.lyrics_helper_state.helper);

    let smtc_title = if let Some(t) = track_info.title {
        t
    } else {
        return;
    };
    let smtc_artists: Vec<String> = track_info
        .artist
        .map(|s| s.split('/').map(|n| n.trim().to_string()).collect())
        .unwrap_or_default();

    let app_settings = app.app_settings.lock().unwrap().clone();
    let cover_cache_dir = app.local_cache.cover_cache_dir.clone();

    let runtime = app.tokio_runtime.clone();

    let status_arc_to_update = match source_to_refetch {
        AutoSearchSource::QqMusic => Arc::clone(&app.fetcher.qqmusic_status),
        AutoSearchSource::Kugou => Arc::clone(&app.fetcher.kugou_status),
        AutoSearchSource::Netease => Arc::clone(&app.fetcher.netease_status),
        AutoSearchSource::AmllDb => Arc::clone(&app.fetcher.amll_db_status),
        _ => return,
    };
    *status_arc_to_update.lock().unwrap() = AutoSearchStatus::Searching;

    let result_tx = app.fetcher.result_tx.clone();
    let cancellation_token = CancellationToken::new();
    app.fetcher.current_fetch_cancellation_token = Some(cancellation_token.clone());

    runtime.spawn(async move {
        let artists_slices: Vec<&str> = smtc_artists.iter().map(|s| s.as_str()).collect();
        let track_to_search = Track {
            title: Some(&smtc_title),
            artists: if artists_slices.is_empty() {
                None
            } else {
                Some(&artists_slices)
            },
            album: track_info.album_title.as_deref(),
            duration: track_info.duration_ms,
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
        let smtc_cover_data = track_info.cover_data.clone();

        let search_mode = SearchMode::Specific(provider_enum);

        let search_result = {
            let search_future_result = {
                let helper_guard = helper.lock().await;
                helper_guard.search_lyrics_comprehensive(
                    &track_to_search,
                    &search_mode,
                    Some(cancellation_token),
                )
            };
            match search_future_result {
                Ok(future) => future.await,
                Err(e) => Err(e),
            }
        };

        match search_result {
            Ok(Some(comprehensive_result)) => {
                info!(
                    "[ManualRefetch] 在 {:?} 中成功找到歌词...",
                    source_to_refetch
                );

                let lyrics_and_metadata = comprehensive_result.primary_lyric_result.clone();

                if app_settings.auto_cache
                    && comprehensive_result
                        .primary_lyric_result
                        .source_track
                        .match_type
                        == lyrics_helper_core::MatchType::Perfect
                {
                    info!("[AutoCache] 歌词匹配度为 Perfect，缓存到本地。");
                    if result_tx.send(AutoFetchResult::RequestCache).is_err() {
                        error!("[AutoCache] 发送 RequestCache 请求到主线程失败。");
                    }
                }

                let lyrics_ready_result = AutoFetchResult::LyricsReady {
                    source: source_to_refetch,
                    lyrics_and_metadata: Box::new(lyrics_and_metadata),
                    title: smtc_title.clone(),
                    artist: smtc_artists.join("/"),
                };

                if result_tx.send(lyrics_ready_result).is_err() {
                    error!("[ManualRefetch Task] 发送 LyricsReady 结果到主线程失败。");
                    return;
                }

                let smtc_cover_hash = track_info.cover_data_hash.unwrap_or(0);

                let final_cover_data = fetch_and_validate_cover(
                    helper.clone(),
                    &comprehensive_result.all_search_candidates,
                    smtc_cover_data,
                    smtc_cover_hash,
                    cover_cache_dir,
                    "手动重搜",
                )
                .await;

                let cover_result = AutoFetchResult::CoverUpdate {
                    title: smtc_title.clone(),
                    artist: smtc_artists.join("/"),
                    cover_data: final_cover_data,
                };

                if result_tx.send(cover_result).is_err() {
                    error!("[ManualRefetch Task] 发送封面更新结果到主线程失败。");
                }
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
