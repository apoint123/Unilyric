use eframe::egui;
use log::{error, info, trace, warn};

use crate::amll_connector::{ConnectorCommand, ConnectorUpdate, WebsocketStatus};
use crate::app::TtmlDbUploadUserAction; // Make sure this path is correct
use crate::app_definition::UniLyricApp; // Assuming UniLyricApp is in app.rs or lib.rs
use crate::app_fetch_core;
use crate::logger::LogLevel;
use crate::types::{AutoFetchResult, AutoSearchStatus, KrcDownloadState, QqMusicDownloadState};
use crate::websocket_server::{PlaybackInfoPayload, ServerCommand};
use egui_toast::{Toast, ToastKind, ToastOptions};
use ws_protocol::Body as ProtocolBody;

// --- Helper functions for UniLyricApp::update ---

/// Handles processing of log messages received by the UI.
pub(crate) fn process_log_messages(app: &mut UniLyricApp) {
    let mut has_warn_or_higher_this_frame = false;
    let mut first_warn_or_higher_message: Option<String> = None;

    while let Ok(log_entry) = app.ui_log_receiver.try_recv() {
        if app.log_display_buffer.len() >= 200 {
            app.log_display_buffer.remove(0);
        }
        if log_entry.level >= LogLevel::Warn {
            if !has_warn_or_higher_this_frame {
                first_warn_or_higher_message = Some(log_entry.message.clone());
            }
            has_warn_or_higher_this_frame = true;
        }
        app.log_display_buffer.push(log_entry);
    }

    if has_warn_or_higher_this_frame {
        let toast_message =
            first_warn_or_higher_message.unwrap_or_else(|| "收到新的警告/错误日志".to_string());
        app.toasts.add(Toast {
            text: toast_message.into(),
            kind: ToastKind::Warning,
            options: ToastOptions::default()
                .duration_in_seconds(5.0)
                .show_progress(true)
                .show_icon(true),
            style: Default::default(),
        });
        if !app.show_bottom_log_panel {
            app.new_trigger_log_exists = true;
        }
    }
}

/// Handles completion of QQ Music downloads.
pub(crate) fn handle_qq_download_completion_logic(app: &mut UniLyricApp) {
    let qq_download_state_snapshot = app.qq_download_state.lock().unwrap().clone();
    match qq_download_state_snapshot {
        QqMusicDownloadState::Success(_) | QqMusicDownloadState::Error(_) => {
            app.handle_qq_download_completion(); // This method is already in UniLyricApp
        }
        _ => {}
    }
}

/// Handles completion of Kugou Music downloads.
pub(crate) fn handle_kugou_download_completion_logic(app: &mut UniLyricApp) {
    let kugou_download_state_snapshot = app.kugou_download_state.lock().unwrap().clone();
    match kugou_download_state_snapshot {
        KrcDownloadState::Success(_) | KrcDownloadState::Error(_) => {
            app.handle_kugou_download_completion(); // This method is already in UniLyricApp
        }
        _ => {}
    }
}

/// Handles completion of Netease Music downloads.
pub(crate) fn handle_netease_download_completion_logic(app: &mut UniLyricApp) {
    // The original app.rs already calls app.handle_netease_download_completion() directly.
    // We can keep it that way or move the call here. For consistency:
    app.handle_netease_download_completion();
}

/// Handles completion of AMLL TTML Database downloads.
pub(crate) fn handle_amll_ttml_download_completion_logic(app: &mut UniLyricApp) {
    // The original app.rs already calls app.handle_amll_ttml_download_completion() directly.
    app.handle_amll_ttml_download_completion();
}

/// Processes updates received from the AMLL Connector worker.
pub(crate) fn process_connector_updates(app: &mut UniLyricApp) {
    let mut updates_from_worker_this_frame: Vec<ConnectorUpdate> = Vec::new();
    loop {
        match app.media_connector_update_rx.try_recv() {
            Ok(update) => {
                updates_from_worker_this_frame.push(update);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                break;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                error!("[UniLyric] 与 AMLL Connector worker 的更新通道已断开!");
                if app.media_connector_config.lock().unwrap().enabled {
                    *app.media_connector_status.lock().unwrap() =
                        WebsocketStatus::错误("Worker 通道断开".to_string());
                    app.media_connector_worker_handle = None;
                }
                break;
            }
        }
    }

    for update_msg in updates_from_worker_this_frame {
        match update_msg {
            ConnectorUpdate::WebsocketStatusChanged(status) => {
                let mut ws_status_guard = app.media_connector_status.lock().unwrap();
                let old_status = ws_status_guard.clone();
                *ws_status_guard = status.clone();
                drop(ws_status_guard);
                info!("[UniLyric] AMLL Connector WebSocket 状态改变: {old_status:?} -> {status:?}");

                if status == WebsocketStatus::已连接 && old_status != WebsocketStatus::已连接
                {
                    app.start_progress_timer_if_needed();

                    if let Some(tx) = &app.media_connector_command_tx {
                        // 1. 发送初始播放状态
                        // app.is_currently_playing_sensed_by_smtc 是最可靠的状态源
                        let initial_playback_body = if app.is_currently_playing_sensed_by_smtc {
                            log::info!(
                                "[UniLyric] WebSocket 已连接，感知到 SMTC 正在播放，发送 OnPlay。"
                            );
                            ProtocolBody::OnResumed
                        } else {
                            log::info!(
                                "[UniLyric] WebSocket 已连接，感知到 SMTC 已暂停，发送 OnPaused。"
                            );
                            ProtocolBody::OnPaused
                        };

                        if tx
                            .send(ConnectorCommand::SendProtocolBody(initial_playback_body))
                            .is_err()
                        {
                            error!("[UniLyric] 连接成功后发送初始播放状态命令失败。");
                        }

                        // 2. 发送当前歌词
                        if !app.output_text.is_empty() {
                            log::info!("[UniLyric] WebSocket 已连接，正在自动发送当前 TTML 歌词。");
                            if tx
                                .send(ConnectorCommand::SendLyricTtml(app.output_text.clone()))
                                .is_err()
                            {
                                log::error!("[UniLyric] 连接成功后自动发送 TTML 歌词失败。");
                            }
                        } else {
                            log::trace!(
                                "[UniLyric] WebSocket 已连接，但输出框为空，不自动发送歌词。"
                            );
                        }
                    } else {
                        log::warn!(
                            "[UniLyric] WebSocket 已连接，但 command_tx 不可用，无法自动同步状态。"
                        );
                    }
                } else if status != WebsocketStatus::已连接 {
                    // 如果状态不是“已连接”（例如断开、错误），则停止定时器
                    app.stop_progress_timer();
                }
            }
            ConnectorUpdate::NowPlayingTrackChanged(new_info_from_event) => {
                let smtc_event_arrival_time = std::time::Instant::now();
                let mut current_event_data = new_info_from_event.clone();

                let mut parsed_artist_successfully = false;
                let temp_original_artist_str_opt = current_event_data.artist.take();
                let mut temp_final_artist: Option<String> = None;
                let mut temp_parsed_album_from_artist_field: Option<String> = None;

                if let Some(original_artist_album_str_val) = temp_original_artist_str_opt {
                    const SEPARATOR: &str = " — ";
                    let parts: Vec<&str> = original_artist_album_str_val
                        .split(SEPARATOR)
                        .map(|s| s.trim())
                        .collect();

                    if !parts.is_empty() {
                        let artist_candidate = parts[0];
                        if !artist_candidate.is_empty() {
                            temp_final_artist = Some(artist_candidate.to_string());
                            parsed_artist_successfully = true;
                            trace!("[UniLyric] 初始艺术家解析自 SMTC: '{artist_candidate}'");

                            if parts.len() > 1 {
                                let album_candidate_from_artist_parts = parts[1..].join(SEPARATOR);
                                if !album_candidate_from_artist_parts.is_empty() {
                                    temp_parsed_album_from_artist_field =
                                        Some(album_candidate_from_artist_parts);
                                    trace!(
                                        "[UniLyric] 从艺术家字段解析出的专辑候选: '{}'",
                                        temp_parsed_album_from_artist_field
                                            .as_deref()
                                            .unwrap_or("")
                                    );
                                }
                            }
                        } else {
                            temp_final_artist = Some(original_artist_album_str_val.clone());
                        }
                    } else {
                        temp_final_artist = Some(original_artist_album_str_val.clone());
                    }

                    if !parsed_artist_successfully && !original_artist_album_str_val.is_empty() {
                        temp_final_artist = Some(original_artist_album_str_val);
                    }
                }
                current_event_data.artist = temp_final_artist;

                if let Some(parsed_album) = temp_parsed_album_from_artist_field {
                    if current_event_data
                        .album_title
                        .as_ref()
                        .is_none_or(|s| s.is_empty())
                    {
                        current_event_data.album_title = Some(parsed_album.clone());
                        trace!(
                            "[UniLyric] 使用从艺术家字段解析的专辑填充 Album: '{:?}'",
                            current_event_data.album_title
                        );
                    } else {
                        trace!(
                            "[UniLyric] 专辑字段已存在值 '{:?}'，未被从艺术家字段解析出的 '{:?}' 覆盖。",
                            current_event_data.album_title, parsed_album
                        );
                    }
                }

                if parsed_artist_successfully {
                    trace!(
                        "[UniLyric] 艺术家/专辑字段解析尝试完成。最终 Artist: {:?}, Album: {:?}",
                        current_event_data.artist, current_event_data.album_title
                    );
                } else if current_event_data.artist.is_some() {
                    trace!(
                        "[UniLyric] 艺术家/专辑字段未经有效分隔符解析。Artist: {:?}, Album: {:?}",
                        current_event_data.artist, current_event_data.album_title
                    );
                }

                let raw_smtc_position_from_event_for_log = current_event_data.position_ms;

                if let Some(original_pos) = current_event_data.position_ms {
                    let mut adjusted_pos_i64 = original_pos as i64 - app.smtc_time_offset_ms;
                    adjusted_pos_i64 = adjusted_pos_i64.max(0);
                    if let Some(duration) = current_event_data.duration_ms
                        && duration > 0
                    {
                        adjusted_pos_i64 = adjusted_pos_i64.min(duration as i64);
                    }
                    current_event_data.position_ms = Some(adjusted_pos_i64 as u64);
                }

                if current_event_data.position_report_time.is_none() {
                    current_event_data.position_report_time = Some(smtc_event_arrival_time);
                }

                let mut effective_info_for_app_state = current_event_data.clone();
                let mut is_genuinely_new_song_flag = false;
                let mut cover_actually_changed_flag = false;
                let mut music_info_for_player_needs_update_flag = false;

                let last_true_info_arc_clone =
                    std::sync::Arc::clone(&app.last_true_smtc_processed_info);
                let current_media_info_arc_clone = std::sync::Arc::clone(&app.current_media_info);
                let tokio_rt_handle = app.tokio_runtime.handle().clone();

                tokio_rt_handle.block_on(async {
                    let mut last_true_guard = last_true_info_arc_clone.lock().await;
                    let opt_previous_true_info = last_true_guard.clone();
                    let mut candidate_for_last_true = current_event_data.clone();
                    let mut candidate_for_simulator_state = current_event_data.clone();

                    if let Some(ref previous_true_info_val) = opt_previous_true_info {
                        if previous_true_info_val.title != current_event_data.title
                            || previous_true_info_val.artist != current_event_data.artist
                        {
                            is_genuinely_new_song_flag = true;
                        }
                        if previous_true_info_val.cover_data_hash
                            != current_event_data.cover_data_hash
                        {
                            cover_actually_changed_flag = true;
                        }
                        if is_genuinely_new_song_flag
                            || previous_true_info_val.album_title != current_event_data.album_title
                            || previous_true_info_val.duration_ms != current_event_data.duration_ms
                        {
                            music_info_for_player_needs_update_flag = true;
                        }

                        let current_is_playing = current_event_data.is_playing.unwrap_or(false);
                        let previous_was_playing =
                            previous_true_info_val.is_playing.unwrap_or(false);

                        if current_is_playing && !previous_was_playing {
                            candidate_for_simulator_state.position_report_time =
                                current_event_data.position_report_time;
                            candidate_for_last_true.position_report_time =
                                current_event_data.position_report_time;
                        } else if current_is_playing && previous_was_playing {
                            if current_event_data.position_ms.is_some()
                                && current_event_data.position_ms
                                    == previous_true_info_val.position_ms
                            {
                                let stable_rt = previous_true_info_val.position_report_time;
                                candidate_for_simulator_state.position_report_time = stable_rt;
                                candidate_for_last_true.position_report_time = stable_rt;
                            } else {
                                candidate_for_simulator_state.position_report_time =
                                    current_event_data.position_report_time;
                                candidate_for_last_true.position_report_time =
                                    current_event_data.position_report_time;
                            }
                        } else {
                            candidate_for_simulator_state.position_report_time =
                                current_event_data.position_report_time;
                            candidate_for_last_true.position_report_time =
                                current_event_data.position_report_time;
                        }
                    } else {
                        is_genuinely_new_song_flag = true;
                        music_info_for_player_needs_update_flag = true;
                        if current_event_data.cover_data_hash.is_some() {
                            cover_actually_changed_flag = true;
                        }
                    }
                    *last_true_guard = Some(candidate_for_last_true);
                    let mut simulator_final_info = current_event_data.clone();
                    simulator_final_info.position_ms = candidate_for_simulator_state.position_ms;
                    simulator_final_info.position_report_time =
                        candidate_for_simulator_state.position_report_time;
                    let mut current_media_guard = current_media_info_arc_clone.lock().await;
                    *current_media_guard = Some(simulator_final_info.clone());
                    effective_info_for_app_state.position_ms = simulator_final_info.position_ms;
                    effective_info_for_app_state.position_report_time =
                        simulator_final_info.position_report_time;
                });

                app.last_smtc_position_ms = effective_info_for_app_state.position_ms.unwrap_or(0);
                app.last_smtc_position_report_time =
                    effective_info_for_app_state.position_report_time;
                let new_app_is_playing_state = current_event_data.is_playing.unwrap_or(false);
                let previous_app_is_playing_state = app.is_currently_playing_sensed_by_smtc;
                app.is_currently_playing_sensed_by_smtc = new_app_is_playing_state;
                app.current_song_duration_ms = current_event_data.duration_ms.unwrap_or(0);

                info!(
                    "[UniLyric] SMTC 信息更新: 存储位置={}ms (SMTC原始位置: {:?}), 用于计时的存储报告时间={:?}, 播放中={}, 时长={}ms",
                    app.last_smtc_position_ms,
                    raw_smtc_position_from_event_for_log,
                    app.last_smtc_position_report_time,
                    app.is_currently_playing_sensed_by_smtc,
                    app.current_song_duration_ms
                );

                if app.websocket_server_enabled {
                    if is_genuinely_new_song_flag {
                        app.process_smtc_update_for_websocket(&current_event_data);
                    } else if new_app_is_playing_state && app.last_smtc_position_ms > 0 {
                        app.send_time_update_to_websocket(app.last_smtc_position_ms);
                    }
                    if !is_genuinely_new_song_flag
                        && new_app_is_playing_state != previous_app_is_playing_state
                    {
                        trace!("[UniLyric WebSocket] 播放状态改变 (非新歌)，发送 PlaybackInfo。");
                        app.process_smtc_update_for_websocket(&current_event_data);
                    }
                }

                if let Some(command_tx) = &app.media_connector_command_tx {
                    let connector_config_guard = app.media_connector_config.lock().unwrap();
                    if connector_config_guard.enabled {
                        if music_info_for_player_needs_update_flag {
                            let artists_vec =
                                current_event_data
                                    .artist
                                    .as_ref()
                                    .map_or_else(Vec::new, |name| {
                                        vec![ws_protocol::Artist {
                                            id: Default::default(),
                                            name: name.as_str().into(),
                                        }]
                                    });
                            let set_music_info_body = ProtocolBody::SetMusicInfo {
                                music_id: Default::default(),
                                music_name: current_event_data
                                    .title
                                    .clone()
                                    .map_or_else(Default::default, |s| s.as_str().into()),
                                album_id: Default::default(),
                                album_name: current_event_data
                                    .album_title
                                    .clone()
                                    .map_or_else(Default::default, |s| s.as_str().into()),
                                artists: artists_vec,
                                duration: current_event_data.duration_ms.unwrap_or(0),
                            };
                            if command_tx
                                .send(crate::amll_connector::ConnectorCommand::SendProtocolBody(
                                    set_music_info_body,
                                ))
                                .is_err()
                            {
                                error!("[UniLyric] 发送 SetMusicInfo 命令失败。");
                            }
                        }

                        if new_app_is_playing_state != previous_app_is_playing_state {
                            if new_app_is_playing_state {
                                trace!("[UniLyric] 状态从暂停变为播放。");
                                if command_tx
                                    .send(
                                        crate::amll_connector::ConnectorCommand::SendProtocolBody(
                                            ProtocolBody::OnResumed,
                                        ),
                                    )
                                    .is_err()
                                {
                                    error!("[UniLyric] 发送 OnResumed 命令失败。");
                                } else {
                                    trace!("[UniLyric] 已发送 OnResumed 给 AMLL Player。");
                                }
                                let progress_at_resume = ProtocolBody::OnPlayProgress {
                                    progress: app.last_smtc_position_ms,
                                };
                                if command_tx
                                    .send(
                                        crate::amll_connector::ConnectorCommand::SendProtocolBody(
                                            progress_at_resume.clone(),
                                        ),
                                    )
                                    .is_err()
                                {
                                    error!(
                                        "[UniLyric] 发送恢复播放时的 OnPlayProgress ({}ms) 失败。",
                                        app.last_smtc_position_ms
                                    );
                                } else {
                                    trace!(
                                        "[UniLyric] 状态变为播放后，立即发送 OnPlayProgress ({}ms) 给 AMLL Player。",
                                        app.last_smtc_position_ms
                                    );
                                }
                            } else {
                                trace!("[UniLyric] 状态从播放变为暂停。");
                                if command_tx
                                    .send(
                                        crate::amll_connector::ConnectorCommand::SendProtocolBody(
                                            ProtocolBody::OnPaused,
                                        ),
                                    )
                                    .is_err()
                                {
                                    error!("[UniLyric] 发送 OnPaused 命令失败。");
                                } else {
                                    trace!("[UniLyric] 已发送 OnPaused 给 AMLL Player。");
                                }
                                let progress_at_pause = ProtocolBody::OnPlayProgress {
                                    progress: app.last_smtc_position_ms,
                                };
                                if command_tx
                                    .send(
                                        crate::amll_connector::ConnectorCommand::SendProtocolBody(
                                            progress_at_pause.clone(),
                                        ),
                                    )
                                    .is_err()
                                {
                                    error!(
                                        "[UniLyric] 发送暂停时的 OnPlayProgress ({}ms) 失败。",
                                        app.last_smtc_position_ms
                                    );
                                } else {
                                    trace!(
                                        "[UniLyric] 状态变为暂停后，立即发送 OnPlayProgress ({}ms) 给 AMLL Player。",
                                        app.last_smtc_position_ms
                                    );
                                }
                            }
                        }

                        if cover_actually_changed_flag {
                            let cover_data_to_send =
                                current_event_data.cover_data.clone().unwrap_or_default();
                            trace!(
                                "[UniLyric] 封面变化，发送封面数据 (长度: {})",
                                cover_data_to_send.len()
                            );
                            let cover_body = ProtocolBody::SetMusicAlbumCoverImageData {
                                data: cover_data_to_send,
                            };
                            if command_tx
                                .send(crate::amll_connector::ConnectorCommand::SendProtocolBody(
                                    cover_body,
                                ))
                                .is_err()
                            {
                                error!("[UniLyric] 发送封面数据命令失败。");
                            }
                        }
                    }
                }

                if is_genuinely_new_song_flag
                    && !current_event_data
                        .title
                        .as_deref()
                        .unwrap_or("")
                        .trim()
                        .is_empty()
                {
                    let connector_config_guard = app.media_connector_config.lock().unwrap();
                    let connector_is_enabled = connector_config_guard.enabled;
                    drop(connector_config_guard);

                    if connector_is_enabled {
                        trace!(
                            "[UniLyric] 正在自动搜索歌词。 歌曲名: {:?}, 艺术家: {:?}",
                            current_event_data.title, current_event_data.artist
                        );
                        app_fetch_core::update_all_search_status(
                            app,
                            crate::types::AutoSearchStatus::NotAttempted,
                        );
                        app_fetch_core::initial_auto_fetch_and_send_lyrics(app, current_event_data);
                    } else {
                        trace!("[UniLyric] 检测到新歌，但 AMLL Connector 未启用，不触发自动搜索。");
                    }
                }
            }
            ConnectorUpdate::SmtcSessionListChanged(sessions) => {
                trace!(
                    "[UniLyric] 收到 SMTC 会话列表更新，共 {} 个会话。",
                    sessions.len()
                );
                let mut available_sessions_guard = app.available_smtc_sessions.lock().unwrap();
                *available_sessions_guard = sessions.clone();
                drop(available_sessions_guard);

                let mut selected_id_guard = app.selected_smtc_session_id.lock().unwrap();
                if let Some(ref current_selected_id) = *selected_id_guard
                    && !sessions
                        .iter()
                        .any(|s| s.session_id == *current_selected_id)
                {
                    trace!(
                        "[UniLyric] 当前选择的 SMTC 会话 ID '{current_selected_id}' 已不再可用，清除选择。"
                    );
                    *selected_id_guard = None;
                }
            }
            ConnectorUpdate::SelectedSmtcSessionVanished(vanished_session_id) => {
                trace!(
                    "[UniLyric] 收到通知：之前选择的 SMTC 会话 ID '{vanished_session_id}' 已消失。"
                );
                let mut selected_id_guard = app.selected_smtc_session_id.lock().unwrap();
                if selected_id_guard.as_ref() == Some(&vanished_session_id) {
                    *selected_id_guard = None;
                    app.toasts.add(egui_toast::Toast {
                        text: format!(
                            "源应用 \"{}\" 已关闭",
                            vanished_session_id
                                .split('!')
                                .next()
                                .unwrap_or(&vanished_session_id)
                        )
                        .into(),
                        kind: egui_toast::ToastKind::Warning,
                        options: egui_toast::ToastOptions::default()
                            .duration_in_seconds(4.0)
                            .show_icon(true),
                        ..Default::default()
                    });
                }
            }
            ConnectorUpdate::AudioSessionVolumeChanged {
                session_id,
                volume,
                is_muted,
            } => {
                trace!(
                    "[Unilyric] 收到 AudioSessionVolumeChanged: session='{session_id}', vol={volume}, mute={is_muted}"
                );
                let mut current_vol_guard = app.current_smtc_volume.lock().unwrap();
                *current_vol_guard = Some((volume, is_muted));
            }
            ConnectorUpdate::AudioDataPacket(_audio_bytes) => {
                error!("[UniLyric] 逻辑错误：收到一个 AudioDataPacket 更新");
            }
            ConnectorUpdate::SimulatedProgressUpdate(time_ms) => {
                if app.websocket_server_enabled && app.is_currently_playing_sensed_by_smtc {
                    app.send_time_update_to_websocket(time_ms);
                }
            }
        }
    }
    app.start_progress_timer_if_needed();
}

/// Handles results from automatic lyric fetching.
pub(crate) fn handle_auto_fetch_results(app: &mut UniLyricApp) {
    match app.auto_fetch_result_rx.try_recv() {
        Ok(auto_fetch_result) => match auto_fetch_result {
            AutoFetchResult::Success {
                source,
                source_format,
                main_lyrics,
                translation_lrc,
                romanization_qrc,
                romanization_lrc,
                krc_translation_lines,
                platform_metadata,
            } => {
                info!("[UniLyricApp] 自动获取成功，来源: {source:?}, 格式: {source_format:?}");

                let app_settings_guard = app.app_settings.lock().unwrap();
                let always_search_all = app_settings_guard.always_search_all_sources;
                drop(app_settings_guard);

                let processed_main_lyrics = main_lyrics;

                let result_data_for_storage = crate::types::ProcessedLyricsSourceData {
                    format: source_format,
                    main_lyrics: processed_main_lyrics.clone(),
                    translation_lrc: translation_lrc.clone(),
                    romanization_qrc: romanization_qrc.clone(),
                    romanization_lrc: romanization_lrc.clone(),
                    krc_translation_lines: krc_translation_lines.clone(),
                    platform_metadata: platform_metadata.clone(),
                };

                match source {
                    crate::types::AutoSearchSource::QqMusic => {
                        *app.last_qq_search_result.lock().unwrap() = Some(result_data_for_storage);
                    }
                    crate::types::AutoSearchSource::Kugou => {
                        *app.last_kugou_search_result.lock().unwrap() =
                            Some(result_data_for_storage);
                    }
                    crate::types::AutoSearchSource::Netease => {
                        *app.last_netease_search_result.lock().unwrap() =
                            Some(result_data_for_storage);
                    }
                    crate::types::AutoSearchSource::AmllDb => {
                        *app.last_amll_db_search_result.lock().unwrap() =
                            Some(result_data_for_storage);
                    }
                    crate::types::AutoSearchSource::LocalCache => {}
                }

                let status_arc_to_update = match source {
                    crate::types::AutoSearchSource::LocalCache => {
                        &app.local_cache_auto_search_status
                    }
                    crate::types::AutoSearchSource::QqMusic => &app.qqmusic_auto_search_status,
                    crate::types::AutoSearchSource::Kugou => &app.kugou_auto_search_status,
                    crate::types::AutoSearchSource::Netease => &app.netease_auto_search_status,
                    crate::types::AutoSearchSource::AmllDb => &app.amll_db_auto_search_status,
                };
                *status_arc_to_update.lock().unwrap() = AutoSearchStatus::Success(source_format);

                if !app.current_auto_search_ui_populated {
                    app.clear_all_data();

                    app.last_auto_fetch_source_for_stripping_check = Some(source);
                    app.last_auto_fetch_source_format = Some(source_format);

                    app.input_text = processed_main_lyrics;
                    app.source_format = source_format;

                    if source == crate::types::AutoSearchSource::Netease
                        && source_format == crate::types::LyricFormat::Lrc
                    {
                        app.direct_netease_main_lrc_content = Some(app.input_text.clone());
                    }

                    app.pending_translation_lrc_from_download = translation_lrc;
                    app.pending_romanization_qrc_from_download = romanization_qrc;
                    app.pending_romanization_lrc_from_download = romanization_lrc;
                    app.pending_krc_translation_lines = krc_translation_lines;
                    app.session_platform_metadata = platform_metadata;
                    app.metadata_source_is_download = true;

                    app.loaded_translation_lrc = None;
                    app.loaded_romanization_lrc = None;

                    app.handle_convert();

                    app.last_auto_fetch_source_for_stripping_check = None;

                    app.current_auto_search_ui_populated = true;

                    if !always_search_all {
                        app_fetch_core::set_other_sources_not_attempted(app, source);
                    }

                    if app.media_connector_config.lock().unwrap().enabled
                        && let Some(tx) = &app.media_connector_command_tx
                    {
                        if !app.output_text.is_empty() {
                            trace!(
                                "[UniLyricApp] (首次填充后) 已发送 TTML (源: {:?}, 长度: {}) 到播放器。",
                                source,
                                app.output_text.len()
                            );
                            let ttml_body = ws_protocol::Body::SetLyricFromTTML {
                                data: app.output_text.as_str().into(),
                            };
                            if tx
                                .send(crate::amll_connector::ConnectorCommand::SendProtocolBody(
                                    ttml_body,
                                ))
                                .is_err()
                            {
                                error!("[UniLyricApp] (首次填充后)发送 TTML 失败。");
                            }
                        } else {
                            warn!(
                                "[UniLyricApp] (首次填充后) 处理后输出为空，不发送TTML。来源: {source:?}"
                            );
                        }
                    }
                } else {
                    trace!(
                        "[UniLyricApp] UI已填充，来源 {source:?} 的歌词结果已存储 (用于侧边栏)，但不更新主UI或再次发送。"
                    );
                }
            }
            AutoFetchResult::NotFound => {
                info!("[UniLyricApp] 自动获取歌词：所有在线源均未找到。");
                let sources_to_update_on_not_found = [
                    &app.qqmusic_auto_search_status,
                    &app.kugou_auto_search_status,
                    &app.netease_auto_search_status,
                    &app.amll_db_auto_search_status,
                ];
                for status_arc in sources_to_update_on_not_found {
                    let mut guard = status_arc.lock().unwrap();
                    if matches!(*guard, AutoSearchStatus::Searching) {
                        *guard = AutoSearchStatus::NotFound;
                    }
                }
                if !app.current_auto_search_ui_populated
                    && app.media_connector_config.lock().unwrap().enabled
                    && let Some(tx) = &app.media_connector_command_tx
                {
                    info!("[UniLyricApp] 未找到任何歌词，尝试发送空TTML给AMLL Player。");
                    let empty_ttml_body = ws_protocol::Body::SetLyricFromTTML { data: "".into() };
                    if tx
                        .send(crate::amll_connector::ConnectorCommand::SendProtocolBody(
                            empty_ttml_body,
                        ))
                        .is_err()
                    {
                        error!("[UniLyricApp] (未找到歌词) 发送空TTML失败。");
                    }
                }
                if app.websocket_server_enabled && !app.current_auto_search_ui_populated {
                    let mut current_title = None;
                    let mut current_artist = None;
                    if let Ok(media_info_guard) = app.current_media_info.try_lock()
                        && let Some(info) = &*media_info_guard
                    {
                        current_title = info.title.clone();
                        current_artist = info.artist.clone();
                    }
                    let empty_lyrics_payload = PlaybackInfoPayload {
                        title: current_title,
                        artist: current_artist,
                        ttml_lyrics: None,
                    };
                    if let Some(ws_tx) = &app.websocket_server_command_tx
                        && let Err(e) = ws_tx
                            .try_send(ServerCommand::BroadcastPlaybackInfo(empty_lyrics_payload))
                    {
                        warn!("[UniLyricApp] 发送空歌词PlaybackInfo到WebSocket失败: {e}");
                    }
                }
            }
            AutoFetchResult::FetchError(err_msg) => {
                error!("[UniLyricApp] 自动获取歌词时发生错误: {err_msg}");
            }
        },
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            error!("[UniLyricApp] 自动获取结果通道已断开!");
        }
    }
}

/// Handles actions related to TTML DB uploads.
pub(crate) fn handle_ttml_db_upload_actions(app: &mut UniLyricApp) {
    match app.ttml_db_upload_action_rx.try_recv() {
        Ok(action) => match action {
            TtmlDbUploadUserAction::InProgressUpdate(msg) => {
                info!("[TTML_DB_Upload_UI] 状态更新: {msg}");
                app.toasts.add(Toast {
                    text: msg.into(),
                    kind: ToastKind::Info,
                    options: ToastOptions::default()
                        .duration_in_seconds(2.5)
                        .show_progress(true)
                        .show_icon(false),
                    style: Default::default(),
                });
            }
            TtmlDbUploadUserAction::PasteReadyAndCopied {
                paste_url,
                github_issue_url_to_open,
            } => {
                info!(
                    "[TTML_DB_Upload_UI] dpaste链接已就绪: {paste_url}, 将打开Issue页面: {github_issue_url_to_open}"
                );
                app.ttml_db_last_paste_url = Some(paste_url.clone());

                let mut clipboard_ok = false;
                let mut clipboard_toast_message = "dpaste链接已复制到剪贴板!".to_string();
                let mut clipboard_toast_kind = ToastKind::Success;

                match arboard::Clipboard::new() {
                    Ok(mut clipboard) => {
                        if clipboard.set_text(paste_url.clone()).is_ok() {
                            clipboard_ok = true;
                            info!("[TTML_DB_Upload_UI] dpaste链接已成功复制到剪贴板。");
                        } else {
                            clipboard_toast_message =
                                "无法自动复制dpaste链接到剪贴板，请手动复制。".to_string();
                            clipboard_toast_kind = ToastKind::Warning;
                            warn!(
                                "[TTML_DB_Upload_UI] 复制dpaste链接到剪贴板失败 (set_text error)。"
                            );
                        }
                    }
                    Err(e) => {
                        clipboard_toast_message =
                            "无法访问系统剪贴板，请手动从通知中复制dpaste链接。".to_string();
                        clipboard_toast_kind = ToastKind::Error;
                        error!("[TTML_DB_Upload_UI] 初始化剪贴板失败: {e}");
                    }
                }
                app.toasts.add(Toast {
                    text: clipboard_toast_message.clone().into(),
                    kind: clipboard_toast_kind,
                    options: ToastOptions::default()
                        .duration_in_seconds(3.5)
                        .show_icon(true),
                    style: Default::default(),
                });

                let final_toast_message: String;
                let final_toast_kind: ToastKind;
                let final_toast_duration = 30.0;

                if webbrowser::open(&github_issue_url_to_open).is_ok() {
                    info!("[TTML_DB_Upload_UI] GitHub Issue页面已在浏览器中打开。");
                    final_toast_message = format!(
                        "{}\nGitHub Issue页面已打开。\n\n后续操作指引：\n1. (如果自动复制失败) 从本条通知或日志中复制 dpaste 链接。\n2. 返回已打开的GitHub Issue页面，将链接粘贴到“TTML歌词文件下载直链”。\n3. 填写其他部分（如果需要）并提交。",
                        if clipboard_ok {
                            "dpaste链接已复制!"
                        } else {
                            "请手动复制dpaste链接。"
                        }
                    );
                    final_toast_kind = if clipboard_ok {
                        ToastKind::Success
                    } else {
                        ToastKind::Warning
                    };
                } else {
                    error!("[TTML_DB_Upload_UI] 在浏览器中打开GitHub Issue页面失败。");
                    let ttml_db_repo_owner = "Steve-xmh";
                    let ttml_db_repo_name = "amll-ttml-db";
                    let repo_url_for_manual_submission = format!(
                        "https://github.com/{ttml_db_repo_owner}/{ttml_db_repo_name}/issues/new"
                    );
                    final_toast_message = format!(
                        "{}\n但打开Issue页面失败。\n\n请手动：\n1. (如果上面复制失败) 从本条通知或日志中复制 dpaste 链接。\n2. 访问 {}。\n3. 粘贴链接并填写表单提交。",
                        if clipboard_ok {
                            "dpaste链接已复制!"
                        } else {
                            "请手动复制dpaste链接。"
                        },
                        repo_url_for_manual_submission
                    );
                    final_toast_kind = ToastKind::Warning;
                }
                let final_toast_message_with_url =
                    format!("{final_toast_message}\n\ndpaste链接: {paste_url}");

                app.toasts.add(Toast {
                    text: final_toast_message_with_url.into(),
                    kind: final_toast_kind,
                    options: ToastOptions::default()
                        .duration_in_seconds(final_toast_duration)
                        .show_icon(true)
                        .show_progress(true),
                    style: Default::default(),
                });
                app.ttml_db_upload_in_progress = false;
            }
            TtmlDbUploadUserAction::PreparationError(err_msg) => {
                error!("[TTML_DB_Upload_UI] 准备阶段错误: {err_msg}");
                app.ttml_db_upload_in_progress = false;
            }
            TtmlDbUploadUserAction::Error(err_msg) => {
                error!("[Unilyric] 上传过程中发生错误: {err_msg}");
                app.toasts.add(Toast {
                    text: format!("上传失败: {err_msg}").into(),
                    kind: ToastKind::Error,
                    options: ToastOptions::default()
                        .duration_in_seconds(5.0)
                        .show_icon(true),
                    style: Default::default(),
                });
                app.ttml_db_upload_in_progress = false;
            }
        },
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            error!("[Unilyric] 上传操作消息通道意外断开!");
            if app.ttml_db_upload_in_progress {
                app.toasts.add(Toast {
                    text: "上传处理通道意外断开，操作可能未完成。".into(),
                    kind: ToastKind::Error,
                    options: ToastOptions::default()
                        .duration_in_seconds(4.0)
                        .show_icon(true),
                    style: Default::default(),
                });
                app.ttml_db_upload_in_progress = false;
            }
        }
    }
}

/// Draws all UI panels and modal windows.
pub(crate) fn draw_ui_elements(app: &mut UniLyricApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
        app.draw_toolbar(ui); // Assumes draw_toolbar is a method of UniLyricApp
    });
    app.draw_log_panel(ctx); // Assumes draw_log_panel is a method of UniLyricApp

    let amll_connector_feature_is_enabled = app.media_connector_config.lock().unwrap().enabled;

    if !amll_connector_feature_is_enabled {
        app.show_amll_connector_sidebar = false;
    }

    if amll_connector_feature_is_enabled && app.show_amll_connector_sidebar {
        egui::SidePanel::right("amll_connector_sidebar_panel")
            .resizable(false)
            .exact_width(300.0)
            .show(ctx, |ui| {
                app.draw_amll_connector_sidebar(ui); // Assumes method on UniLyricApp
            });
    }

    let available_width = ctx.screen_rect().width();
    let input_panel_width = (available_width * 0.25).clamp(200.0, 400.0);
    let lrc_panel_width = (available_width * 0.20).clamp(150.0, 350.0);
    let markers_panel_width = (available_width * 0.18).clamp(120.0, 300.0);

    egui::SidePanel::left("input_panel")
        .default_width(input_panel_width)
        .show(ctx, |ui| {
            app.draw_input_panel_contents(ui);
        });

    if app.show_markers_panel {
        egui::SidePanel::right("markers_panel")
            .default_width(markers_panel_width)
            .show(ctx, |ui| {
                app.draw_markers_panel_contents(ui, app.wrap_text); // Assumes method
            });
    }
    if app.show_translation_lrc_panel {
        egui::SidePanel::right("translation_lrc_panel")
            .default_width(lrc_panel_width)
            .show(ctx, |ui| {
                app.draw_translation_lrc_panel_contents(ui); // Assumes method
            });
    }
    if app.show_romanization_lrc_panel {
        egui::SidePanel::right("romanization_lrc_panel")
            .default_width(lrc_panel_width)
            .show(ctx, |ui| {
                app.draw_romanization_lrc_panel_contents(ui); // Assumes method
            });
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        app.draw_output_panel_contents(ui); // Assumes method
    });

    if app.show_metadata_panel {
        let mut window_is_actually_open = true;
        let mut should_keep_panel_open_from_internal_logic = app.show_metadata_panel;

        egui::Window::new("编辑元数据")
            .open(&mut window_is_actually_open)
            .default_width(450.0)
            .default_height(400.0)
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
                app.draw_metadata_editor_window_contents(
                    // Assumes method
                    ui,
                    &mut should_keep_panel_open_from_internal_logic,
                );
            });

        if !window_is_actually_open || !should_keep_panel_open_from_internal_logic {
            app.show_metadata_panel = false;
        }
    }

    if app.show_settings_window {
        app.draw_settings_window(ctx); // Assumes method
    }
    app.draw_qqmusic_download_modal_window(ctx); // Assumes method
    app.draw_kugou_download_modal_window(ctx); // Assumes method
    app.draw_netease_download_modal_window(ctx); // Assumes method
    app.draw_amll_download_modal_window(ctx); // Assumes method
}

/// Handles file drop events.
pub(crate) fn handle_file_drops(app: &mut UniLyricApp, ctx: &egui::Context) {
    if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(file) = files.first() {
            if let Some(path) = &file.path {
                crate::io::load_file_and_convert(app, path.clone());
            } else if let Some(bytes) = &file.bytes {
                if let Ok(text_content) = String::from_utf8(bytes.to_vec()) {
                    app.clear_all_data();
                    app.input_text = text_content;
                    app.metadata_source_is_download = false;
                    app.handle_convert();
                } else {
                    warn!("[Unilyric] 拖放的字节数据不是有效的UTF-8文本。");
                }
            }
        }
    } else if !ctx.input(|i| i.raw.hovered_files.is_empty()) {
        egui::Area::new("drag_drop_overlay_area".into())
            .fixed_pos(egui::Pos2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui_overlay| {
                let screen_rect = ui_overlay.ctx().screen_rect();
                ui_overlay.painter().rect_filled(
                    screen_rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(20, 20, 20, 190),
                );
                ui_overlay.painter().text(
                    screen_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "拖放到此处以加载",
                    egui::FontId::proportional(50.0),
                    egui::Color32::WHITE,
                );
            });
    }
}
