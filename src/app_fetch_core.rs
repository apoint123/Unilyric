use crate::app_definition::UniLyricApp;
use crate::types::{AutoFetchResult, AutoSearchSource, AutoSearchStatus};
use smtc_suite::NowPlayingInfo;

use lyrics_helper_rs::SearchMode;
use lyrics_helper_rs::model::track::Track;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

pub(super) fn initial_auto_fetch_and_send_lyrics(
    app: &mut UniLyricApp,
    track_info: NowPlayingInfo,
) {
    let smtc_title = match track_info.title.as_deref() {
        Some(t) if !t.trim().is_empty() && t != "无歌曲" && t != "无活动会话" => {
            t.trim().to_string()
        }
        _ => {
            info!("[AutoFetch] SMTC 无有效歌曲名称，跳过搜索。");
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
        tracing::info!("[AutoFetch] 使用有序模式进行快速搜索。");
        SearchMode::Ordered
    };
    // 释放锁
    drop(app_settings);

    app.fetcher.last_source_format = None;
    update_all_search_status(app, AutoSearchStatus::Searching);

    let result_tx = app.fetcher.result_tx.clone();

    app.tokio_runtime.spawn(async move {
        debug!(
            "开始搜索歌词: title='{smtc_title}', artists='{smtc_artists:?}', mode='{search_mode:?}'"
        );

        let artists_slices: Vec<&str> = smtc_artists.iter().map(|s| s.as_str()).collect();
        let track_to_search = Track {
            title: Some(&smtc_title),
            artists: if smtc_artists.is_empty() {
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

pub(super) fn update_all_search_status(app: &UniLyricApp, status: AutoSearchStatus) {
    *app.fetcher.local_cache_status.lock().unwrap() = status.clone();
    *app.fetcher.qqmusic_status.lock().unwrap() = status.clone();
    *app.fetcher.kugou_status.lock().unwrap() = status.clone();
    *app.fetcher.netease_status.lock().unwrap() = status.clone();
    *app.fetcher.amll_db_status.lock().unwrap() = status.clone();
    *app.fetcher.musixmatch_status.lock().unwrap() = status.clone();
}

pub(super) fn clear_last_fetch_results(app: &UniLyricApp) {
    *app.fetcher.last_qq_result.lock().unwrap() = None;
    *app.fetcher.last_kugou_result.lock().unwrap() = None;
    *app.fetcher.last_netease_result.lock().unwrap() = None;
    *app.fetcher.last_amll_db_result.lock().unwrap() = None;
    *app.fetcher.last_musixmatch_result.lock().unwrap() = None;
}
