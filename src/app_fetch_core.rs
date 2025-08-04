use crate::app_definition::UniLyricApp;
use crate::types::{AutoFetchResult, AutoSearchSource, AutoSearchStatus};
use image_hasher::HasherConfig;
use lyrics_helper_rs::model::track::FullLyricsResult;
use smtc_suite::NowPlayingInfo;

use lyrics_helper_rs::{
    SearchMode,
    model::track::{LyricsAndMetadata, Track},
};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

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
                        let main_lyric = lyrics_helper_rs::converter::types::InputFile::new(
                            ttml_content.clone(),
                            lyrics_helper_rs::converter::LyricFormat::Ttml,
                            None,
                            None,
                        );

                        let input = lyrics_helper_rs::converter::types::ConversionInput {
                            main_lyric,
                            translations: vec![],
                            romanizations: vec![],
                            target_format: lyrics_helper_rs::converter::LyricFormat::Ttml,
                            user_metadata_overrides: None,
                        };

                        let options =
                            lyrics_helper_rs::converter::types::ConversionOptions::default();

                        match helper_clone.lock().await.convert_lyrics(input, &options) {
                            Ok(conversion_result) => {
                                let parsed_data = conversion_result.source_data;

                                let full_lyrics_result = FullLyricsResult {
                                    raw: lyrics_helper_rs::model::track::RawLyrics {
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

    let smtc_album = track_info.album_title.clone();
    let smtc_duration = track_info.duration_ms;
    let smtc_cover_data = track_info.cover_data.clone();

    let helper = Arc::clone(&app.lyrics_helper_state.helper);
    let app_settings = app.app_settings.lock().unwrap().clone();
    let result_tx = app.fetcher.result_tx.clone();
    let target_format = app.lyrics.target_format;

    app.fetcher.last_source_format = None;
    *app.fetcher.qqmusic_status.lock().unwrap() = AutoSearchStatus::Searching;
    *app.fetcher.kugou_status.lock().unwrap() = AutoSearchStatus::Searching;
    *app.fetcher.netease_status.lock().unwrap() = AutoSearchStatus::Searching;
    *app.fetcher.amll_db_status.lock().unwrap() = AutoSearchStatus::Searching;

    app.tokio_runtime.spawn(async move {
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

        let search_mode = {
            let mut providers = lyrics_helper_rs::ProviderName::all();
            if app_settings.use_provider_subset {
                let user_subset: Vec<_> = app_settings
                    .auto_search_provider_subset
                    .iter()
                    .filter_map(|s| s.parse().ok())
                    .collect();
                providers.retain(|p| user_subset.contains(p));
            }
            SearchMode::Subset(providers)
        };

        debug!(
            "开始搜索: title='{}', artists='{:?}', mode='{:?}'",
            smtc_title, smtc_artists, search_mode
        );

        let search_result = {
            let search_future_result = {
                let helper_guard = helper.lock().await;
                helper_guard.search_lyrics_comprehensive(track_to_search, search_mode)
            };
            match search_future_result {
                Ok(future) => future.await,
                Err(e) => Err(e),
            }
        };

        match search_result {
            Ok(Some(comprehensive_result)) => {
                let source: AutoSearchSource = comprehensive_result
                    .primary_lyric_result
                    .source_track
                    .provider_name
                    .clone()
                    .into();
                info!("搜索成功，来源: {:?}。正在进行前置转换...", source);

                let lyrics_and_metadata = comprehensive_result.primary_lyric_result.clone();

                let output_text_result = {
                    let helper_guard = helper.lock().await;
                    helper_guard
                        .generate_lyrics_from_parsed(
                            lyrics_and_metadata.lyrics.parsed.clone(),
                            target_format,
                            Default::default(),
                            None,
                        )
                        .await
                };

                let output_text = match output_text_result {
                    Ok(res) => res.output_lyrics,
                    Err(e) => {
                        error!("[AutoFetch] 搜索结果前置转换失败: {}", e);
                        String::new()
                    }
                };

                let lyrics_result = AutoFetchResult::LyricsReady {
                    source,
                    lyrics_and_metadata: Box::new(lyrics_and_metadata),
                    output_text,
                };

                if result_tx.send(lyrics_result).is_err() {
                    error!("[AutoFetch Task] 发送 LyricsReady 结果到主线程失败。");
                    return;
                }

                let final_cover_data = fetch_and_validate_cover(
                    helper.clone(),
                    &comprehensive_result.all_search_candidates,
                    smtc_cover_data,
                    "搜索",
                )
                .await;

                let cover_result = AutoFetchResult::CoverUpdate(final_cover_data);

                if result_tx.send(cover_result).is_err() {
                    error!("[AutoFetch Task] 发送封面更新结果到主线程失败。");
                }
            }
            Ok(None) => {
                info!("未找到任何歌词。");
                if result_tx.send(AutoFetchResult::NotFound).is_err() {
                    error!("[AutoFetch Task] 发送 NotFound 结果到主线程失败。");
                }
            }
            Err(e) => {
                error!("搜索歌词时发生错误: {}", e);
                if result_tx
                    .send(AutoFetchResult::FetchError(e.into()))
                    .is_err()
                {
                    error!("[AutoFetch Task] 发送 Error 结果到主线程失败。");
                }
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
            tracing::warn!("[ManualRefetch] 无SMTC信息，无法重新搜索。");
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

    let target_format = app.lyrics.target_format;

    let status_arc_to_update = match source_to_refetch {
        AutoSearchSource::QqMusic => Arc::clone(&app.fetcher.qqmusic_status),
        AutoSearchSource::Kugou => Arc::clone(&app.fetcher.kugou_status),
        AutoSearchSource::Netease => Arc::clone(&app.fetcher.netease_status),
        AutoSearchSource::AmllDb => Arc::clone(&app.fetcher.amll_db_status),
        _ => return,
    };
    *status_arc_to_update.lock().unwrap() = AutoSearchStatus::Searching;

    let result_tx = app.fetcher.result_tx.clone();

    app.tokio_runtime.spawn(async move {
        let artists_slices: Vec<&str> = smtc_artists.iter().map(|s| s.as_str()).collect();
        let track_to_search = lyrics_helper_rs::model::track::Track {
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
                helper_guard.search_lyrics_comprehensive(track_to_search, search_mode)
            };

            match search_future_result {
                Ok(future) => future.await,
                Err(e) => Err(e),
            }
        };

        match search_result {
            Ok(Some(comprehensive_result)) => {
                info!(
                    "[ManualRefetch] 在 {:?} 中成功找到歌词，正在进行转换...",
                    source_to_refetch
                );

                let lyrics_and_metadata = comprehensive_result.primary_lyric_result.clone();

                let output_text_result = {
                    let helper_guard = helper.lock().await;
                    helper_guard
                        .generate_lyrics_from_parsed(
                            lyrics_and_metadata.lyrics.parsed.clone(),
                            target_format,
                            Default::default(),
                            None,
                        )
                        .await
                };

                let output_text = match output_text_result {
                    Ok(conversion_result) => conversion_result.output_lyrics,
                    Err(e) => {
                        error!("[ManualRefetch] 前置转换失败: {}", e);
                        String::new()
                    }
                };

                let lyrics_ready_result = AutoFetchResult::LyricsReady {
                    source: source_to_refetch,
                    lyrics_and_metadata: Box::new(lyrics_and_metadata),
                    output_text,
                };

                if result_tx.send(lyrics_ready_result).is_err() {
                    error!("[ManualRefetch Task] 发送 LyricsReady 结果到主线程失败。");
                    return;
                }

                let final_cover_data = fetch_and_validate_cover(
                    helper.clone(),
                    &comprehensive_result.all_search_candidates,
                    smtc_cover_data,
                    "手动重搜",
                )
                .await;

                let cover_result = AutoFetchResult::CoverUpdate(final_cover_data);

                if result_tx.send(cover_result).is_err() {
                    error!("[ManualRefetch Task] 发送封面更新结果到主线程失败。");
                }
            }
            Ok(None) => {
                *status_arc_to_update.lock().unwrap() = AutoSearchStatus::NotFound;
            }
            Err(e) => {
                *status_arc_to_update.lock().unwrap() = AutoSearchStatus::Error(e.to_string());
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
    let check = || -> Result<bool, String> {
        let image1 =
            image::load_from_memory(image_data1).map_err(|e| format!("无法加载图片1: {}", e))?;
        let image2 =
            image::load_from_memory(image_data2).map_err(|e| format!("无法加载图片2: {}", e))?;

        let hasher = HasherConfig::new().to_hasher();
        let hash1 = hasher.hash_image(&image1);
        let hash2 = hasher.hash_image(&image2);
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
            warn!("图片相似度对比失败: {}，使用 SMTC 封面", e);
            false
        }
    }
}

/// 从搜索候选中获取最佳封面，并与SMTC封面进行验证和比较。
async fn fetch_and_validate_cover(
    helper: std::sync::Arc<tokio::sync::Mutex<lyrics_helper_rs::LyricsHelper>>,
    candidates: &[lyrics_helper_rs::model::track::SearchResult],
    smtc_cover_data: Option<Vec<u8>>,
    log_prefix: &str,
) -> Option<Vec<u8>> {
    let provider_cover = {
        let helper_guard = helper.lock().await;
        helper_guard.get_best_cover(candidates).await
    };

    match (provider_cover, smtc_cover_data) {
        (Some(provider_bytes), Some(smtc_bytes)) => {
            if are_images_similar(&provider_bytes, &smtc_bytes) {
                info!("{}: 封面验证成功，使用提供商的高清封面。", log_prefix);
                Some(provider_bytes)
            } else {
                warn!(
                    "{}: 封面验证失败（不匹配），回退使用SMTC缩略图。",
                    log_prefix
                );
                Some(smtc_bytes)
            }
        }
        (Some(provider_bytes), None) => {
            info!("{}: SMTC未提供封面数据，使用提供商封面。", log_prefix);
            Some(provider_bytes)
        }
        (None, Some(smtc_bytes)) => {
            info!("{}: 获取提供商封面失败，使用SMTC缩略图。", log_prefix);
            Some(smtc_bytes)
        }
        (None, None) => {
            info!("{}: 无可用封面数据。", log_prefix);
            None
        }
    }
}
