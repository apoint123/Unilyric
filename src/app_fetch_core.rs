use crate::app_definition::UniLyricApp;
use crate::types::{AutoFetchResult, AutoSearchSource, AutoSearchStatus};
use smtc_suite::NowPlayingInfo;

use lyrics_helper_rs::SearchMode;
use lyrics_helper_rs::model::track::{FullLyricsResult, Track};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

fn is_track_match(
    now_playing: &NowPlayingInfo,
    cache_entry: &crate::types::LocalLyricCacheEntry,
) -> bool {
    let title_match = now_playing.title.as_deref().map_or(false, |playing_title| {
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
                    if let Some(helper) = app.lyrics_helper.as_ref() {
                        let result_tx = app.fetcher.result_tx.clone();
                        let helper_clone = Arc::clone(helper);

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

                            match helper_clone.convert_lyrics(input, &options).await {
                                Ok(conversion_result) => {
                                    let parsed_data = conversion_result.source_data;

                                    let full_lyrics_result = FullLyricsResult {
                                        raw: lyrics_helper_rs::model::track::RawLyrics {
                                            content: ttml_content,
                                            ..Default::default()
                                        },
                                        parsed: parsed_data,
                                    };

                                    let result_to_send = AutoFetchResult::Success {
                                        source: AutoSearchSource::LocalCache,
                                        full_lyrics_result,
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
        .map(|s| {
            s.split(['/', '、', ',', ';'])
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let helper = match app.lyrics_helper.as_ref() {
        Some(h) => Arc::clone(h),
        None => {
            warn!("[AutoFetch] LyricsHelper 尚未初始化，无法开始搜索。");
            return;
        }
    };

    let app_settings = app.app_settings.lock().unwrap();
    let search_mode = if app_settings.use_provider_subset
        && !app_settings.auto_search_provider_subset.is_empty()
    {
        tracing::info!("[AutoFetch] 使用子集模式进行搜索。");
        SearchMode::Subset(app_settings.auto_search_provider_subset.clone())
    } else if app_settings.always_search_all_sources {
        tracing::info!("[AutoFetch] 使用并行模式进行全面搜索。");
        SearchMode::Parallel
    } else {
        tracing::info!("[AutoFetch] 使用顺序模式进行快速搜索。");
        SearchMode::Ordered
    };

    let perform_t2s_conversion = app_settings.enable_t2s_for_auto_search;
    let t2s_converter: Option<_> = if perform_t2s_conversion {
        app.t2s_converter.clone()
    } else {
        None
    };

    // 释放锁
    drop(app_settings);

    app.fetcher.last_source_format = None;
    *app.fetcher.qqmusic_status.lock().unwrap() = AutoSearchStatus::Searching;
    *app.fetcher.kugou_status.lock().unwrap() = AutoSearchStatus::Searching;
    *app.fetcher.netease_status.lock().unwrap() = AutoSearchStatus::Searching;
    *app.fetcher.amll_db_status.lock().unwrap() = AutoSearchStatus::Searching;
    *app.fetcher.musixmatch_status.lock().unwrap() = AutoSearchStatus::Searching;

    let result_tx = app.fetcher.result_tx.clone();

    app.tokio_runtime.spawn(async move {
        let (title_to_search, artists_to_search);

        let mut temp_converted_artists: Vec<String> = Vec::new();
        if perform_t2s_conversion && t2s_converter.is_some() {
            let converter = t2s_converter.as_ref().unwrap();

            title_to_search = converter.convert(&smtc_title);

            for artist in &smtc_artists {
                temp_converted_artists.push(converter.convert(artist));
            }
            artists_to_search = temp_converted_artists;
        } else {
            title_to_search = smtc_title.clone();
            artists_to_search = smtc_artists.clone();
        }

        debug!(
            "开始搜索歌词: title='{}', artists='{:?}', mode='{:?}'",
            title_to_search, artists_to_search, search_mode
        );

        let artists_slices: Vec<&str> = artists_to_search.iter().map(|s| s.as_str()).collect();
        let track_to_search = Track {
            title: Some(&title_to_search),
            artists: if artists_to_search.is_empty() {
                None
            } else {
                Some(&artists_slices)
            },
            album: None,
        };

        match helper.search_lyrics(&track_to_search, search_mode).await {
            Ok(Some(full_lyrics_result)) => {
                let source_name = full_lyrics_result.parsed.source_name.clone();
                info!(
                    "搜索成功，来源: {:?}, 格式: {:?}",
                    source_name, full_lyrics_result.parsed.source_format
                );

                let result = AutoFetchResult::Success {
                    source: source_name.into(),
                    full_lyrics_result,
                };

                if result_tx.send(result).is_err() {
                    error!("[AutoFetch Task] 发送成功结果到主线程失败。");
                }
            }
            Ok(None) => {
                info!("未找到任何歌词。");
                if result_tx.send(AutoFetchResult::NotFound).is_err() {
                    error!("[AutoFetch Task] 发送 NotFound 结果到主线程失败。");
                }
            }
            Err(e) => {
                error!("搜索歌词时发生错误: {e}");
                if result_tx
                    .send(AutoFetchResult::FetchError(e.to_string()))
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

    let helper = if let Some(h) = app.lyrics_helper.as_ref() {
        Arc::clone(h)
    } else {
        return;
    };

    let smtc_title = if let Some(t) = track_info.title {
        t
    } else {
        return;
    };
    let smtc_artists: Vec<String> = track_info
        .artist
        .map(|s| s.split('/').map(|n| n.trim().to_string()).collect())
        .unwrap_or_default();

    let status_arc_to_update = match source_to_refetch {
        AutoSearchSource::QqMusic => Arc::clone(&app.fetcher.qqmusic_status),
        AutoSearchSource::Kugou => Arc::clone(&app.fetcher.kugou_status),
        AutoSearchSource::Netease => Arc::clone(&app.fetcher.netease_status),
        AutoSearchSource::AmllDb => Arc::clone(&app.fetcher.amll_db_status),
        AutoSearchSource::Musixmatch => Arc::clone(&app.fetcher.musixmatch_status),
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
            album: None,
        };

        let search_mode =
            SearchMode::Specific(Into::<&'static str>::into(source_to_refetch).to_string());

        match helper.search_lyrics(&track_to_search, search_mode).await {
            Ok(Some(result)) => {
                let _ = result_tx.send(AutoFetchResult::Success {
                    source: source_to_refetch,
                    full_lyrics_result: result,
                });
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
    *app.fetcher.last_musixmatch_result.lock().unwrap() = None;
    app.fetcher.current_ui_populated = false;
}
