use eframe::egui;
use tracing::{debug, error, info, warn};

use crate::amll_connector::ConnectorUpdate;
use crate::amll_connector::protocol::ClientMessage;
use crate::amll_connector::protocol_strings::NullString;
use crate::app_actions::{PlayerAction, UIAction, UserAction};
use crate::app_definition::UniLyricApp;
use crate::error::AppError;
use crate::types::{AutoFetchResult, AutoSearchSource, AutoSearchStatus, LogLevel, ProviderState};
use egui_toast::{Toast, ToastKind, ToastOptions};
use smtc_suite::MediaUpdate;

pub(super) fn process_log_messages(app: &mut UniLyricApp) {
    let mut has_warn_or_higher_this_frame = false;
    let mut first_warn_or_higher_message: Option<String> = None;

    while let Ok(log_entry) = app.ui_log_receiver.try_recv() {
        if app.ui.log_display_buffer.len() >= 200 {
            app.ui.log_display_buffer.remove(0);
        }
        if log_entry.level >= LogLevel::Warn {
            if !has_warn_or_higher_this_frame {
                first_warn_or_higher_message = Some(log_entry.message.clone());
            }
            has_warn_or_higher_this_frame = true;
        }
        app.ui.log_display_buffer.push(log_entry);
    }

    if has_warn_or_higher_this_frame {
        let toast_message =
            first_warn_or_higher_message.unwrap_or_else(|| "收到新的警告/错误日志".to_string());
        app.ui.toasts.add(Toast {
            text: toast_message.into(),
            kind: ToastKind::Warning,
            options: ToastOptions::default()
                .duration_in_seconds(5.0)
                .show_progress(true)
                .show_icon(true),
            style: Default::default(),
        });
        if !app.ui.show_bottom_log_panel {
            app.ui.new_trigger_log_exists = true;
        }
    }
}

pub(super) fn process_connector_updates(app: &mut UniLyricApp) {
    while let Ok(ui_update) = app.amll_connector.update_rx.try_recv() {
        match ui_update.payload {
            ConnectorUpdate::WebsocketStatusChanged(status) => {
                tracing::info!("[App Update] 收到 AMLL Connector 状态更新: {:?}", status);
                *app.amll_connector.status.lock().unwrap() = status;
            }
            ConnectorUpdate::SmtcUpdate(media_update) => match media_update {
                MediaUpdate::TrackChanged(new_info) => {
                    if new_info
                        .title
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                    {
                        continue;
                    }

                    let is_new_song = app.player.current_now_playing.title != new_info.title
                        || app.player.current_now_playing.artist != new_info.artist;

                    if is_new_song {
                        app.player.current_now_playing = new_info.clone();
                        crate::app_fetch_core::clear_last_fetch_results(app);
                        app.auto_fetch_trigger_time =
                            Some(std::time::Instant::now() + std::time::Duration::from_millis(200));
                    } else {
                        let current_info = &mut app.player.current_now_playing;

                        if let Some(pos) = new_info.position_ms {
                            current_info.position_ms = Some(pos);
                        }
                        if let Some(time) = new_info.position_report_time {
                            current_info.position_report_time = Some(time);
                        }
                        if let Some(playing) = new_info.is_playing {
                            current_info.is_playing = Some(playing);
                        }

                        if let Some(cover) = new_info.cover_data {
                            current_info.cover_data = Some(cover);
                            current_info.cover_data_hash = new_info.cover_data_hash;
                        }

                        if let Some(duration) = new_info.duration_ms {
                            current_info.duration_ms = Some(duration);
                        }
                        if let Some(shuffle) = new_info.is_shuffle_active {
                            current_info.is_shuffle_active = Some(shuffle);
                        }
                        if let Some(repeat) = new_info.repeat_mode {
                            current_info.repeat_mode = Some(repeat);
                        }
                    }
                }
                MediaUpdate::SessionsChanged(sessions) => {
                    tracing::info!(
                        "[SMTC Update] 可用会话列表已更新，共 {} 个。",
                        sessions.len()
                    );
                    app.player.available_sessions = sessions;
                }
                MediaUpdate::SelectedSessionVanished(session_id) => {
                    tracing::warn!("[SMTC Update] 选中的会话 '{session_id}' 已消失。");
                    app.ui.toasts.add(egui_toast::Toast {
                        text: "当前媒体源已关闭".into(),
                        kind: egui_toast::ToastKind::Warning,
                        options: egui_toast::ToastOptions::default().duration_in_seconds(3.0),
                        style: Default::default(),
                    });
                }
                MediaUpdate::Error(e) => {
                    tracing::error!("[SMTC Update] smtc-suite 报告了一个错误: {e}");
                }
                MediaUpdate::TrackChangedForced(_now_playing_info) => {}
                MediaUpdate::AudioData(_items) => {}
                MediaUpdate::Diagnostic(_diagnostic_info) => {}
                MediaUpdate::VolumeChanged {
                    session_id: _,
                    volume: _,
                    is_muted: _,
                } => {}
            },
        }
        if ui_update.repaint_needed {
            app.egui_ctx.request_repaint();
        }
    }
}

pub(super) fn handle_auto_fetch_results(app: &mut UniLyricApp) {
    while let Ok(auto_fetch_result) = app.fetcher.result_rx.try_recv() {
        match auto_fetch_result {
            AutoFetchResult::LyricsReady {
                source,
                lyrics_and_metadata,
                output_text,
                title,
                artist,
            } => {
                let now_playing = &app.player.current_now_playing;
                let current_title = now_playing.title.as_deref().unwrap_or_default();
                let current_artist = now_playing.artist.as_deref().unwrap_or_default();

                if current_title != title || current_artist != artist {
                    debug!(
                        "[AutoFetch] 收到过时的歌词 (当前歌曲: '{} - {}', 歌词所属: '{} - {}')，已丢弃。",
                        current_title, current_artist, title, artist
                    );
                    return;
                }

                info!("[AutoFetch] 歌词已就绪，来源: {:?}，正在更新UI。", source);

                let result_cache_opt = match source {
                    AutoSearchSource::QqMusic => Some(&app.fetcher.last_qq_result),
                    AutoSearchSource::Kugou => Some(&app.fetcher.last_kugou_result),
                    AutoSearchSource::Netease => Some(&app.fetcher.last_netease_result),
                    AutoSearchSource::AmllDb => Some(&app.fetcher.last_amll_db_result),
                    AutoSearchSource::LocalCache => None,
                };
                if let Some(result_cache) = result_cache_opt {
                    *result_cache.lock().unwrap() = Some(lyrics_and_metadata.lyrics.clone());
                }

                let source_format = lyrics_and_metadata.lyrics.parsed.source_format;
                let status_to_update = match source {
                    AutoSearchSource::QqMusic => Some(&app.fetcher.qqmusic_status),
                    AutoSearchSource::Kugou => Some(&app.fetcher.kugou_status),
                    AutoSearchSource::Netease => Some(&app.fetcher.netease_status),
                    AutoSearchSource::AmllDb => Some(&app.fetcher.amll_db_status),
                    AutoSearchSource::LocalCache => Some(&app.fetcher.local_cache_status),
                };
                if let Some(status_arc) = status_to_update {
                    *status_arc.lock().unwrap() = AutoSearchStatus::Success(source_format);
                }

                if !app.fetcher.current_ui_populated {
                    app.clear_lyrics_state_for_new_song_internal();

                    app.lyrics.source_format = source_format;
                    app.lyrics.input_text = lyrics_and_metadata.lyrics.raw.content.clone();
                    app.lyrics.output_text = output_text;
                    app.lyrics.parsed_lyric_data = Some(lyrics_and_metadata.lyrics.parsed.clone());
                    app.lyrics.metadata_source_is_download = true;
                    app.fetcher.last_source_format = Some(source_format);
                    app.fetcher.current_ui_populated = true;

                    app.lyrics
                        .metadata_manager
                        .load_from_parsed_data(&lyrics_and_metadata.lyrics.parsed);
                }

                if app.amll_connector.config.lock().unwrap().enabled {
                    if let Some(tx) = &app.amll_connector.command_tx {
                        info!("[AMLL] 自动获取完成，正在发送 TTML 歌词到 Player。");
                        let data_to_send = lyrics_and_metadata.lyrics.parsed.clone();
                        if tx
                            .try_send(crate::amll_connector::ConnectorCommand::SendLyric(
                                data_to_send,
                            ))
                            .is_err()
                        {
                            tracing::error!(
                                "[AMLL] (自动获取完成时) 发送 TTML 歌词失败 (通道已满或关闭)。"
                            );
                        }
                    } else {
                        tracing::warn!("[AMLL] AMLL Connector 已启用但 command_tx 不可用。");
                    }
                }

                app.send_action(UserAction::UI(UIAction::StopOtherSearches));
            }

            AutoFetchResult::LyricsSuccess {
                source,
                lyrics_and_metadata,
                title,
                artist,
            } => {
                let now_playing = &app.player.current_now_playing;
                let current_title = now_playing.title.as_deref().unwrap_or_default();
                let current_artist = now_playing.artist.as_deref().unwrap_or_default();

                if current_title != title || current_artist != artist {
                    debug!(
                        "[AutoFetch] 收到过时的歌词 (当前歌曲: '{} - {}', 歌词所属: '{} - {}')，已丢弃。",
                        current_title, current_artist, title, artist
                    );
                    return;
                }

                let result_cache_opt = match source {
                    AutoSearchSource::QqMusic => Some(&app.fetcher.last_qq_result),
                    AutoSearchSource::Kugou => Some(&app.fetcher.last_kugou_result),
                    AutoSearchSource::Netease => Some(&app.fetcher.last_netease_result),
                    AutoSearchSource::AmllDb => Some(&app.fetcher.last_amll_db_result),
                    AutoSearchSource::LocalCache => None,
                };
                if let Some(result_cache) = result_cache_opt {
                    *result_cache.lock().unwrap() = Some(lyrics_and_metadata.lyrics.clone());
                }

                let source_format = lyrics_and_metadata.lyrics.parsed.source_format;
                let status_to_update = match source {
                    AutoSearchSource::QqMusic => Some(&app.fetcher.qqmusic_status),
                    AutoSearchSource::Kugou => Some(&app.fetcher.kugou_status),
                    AutoSearchSource::Netease => Some(&app.fetcher.netease_status),
                    AutoSearchSource::AmllDb => Some(&app.fetcher.amll_db_status),
                    AutoSearchSource::LocalCache => Some(&app.fetcher.local_cache_status),
                };
                if let Some(status_arc) = status_to_update {
                    *status_arc.lock().unwrap() = AutoSearchStatus::Success(source_format);
                }

                if !app.fetcher.current_ui_populated {
                    app.send_action(crate::app_actions::UserAction::Lyrics(Box::new(
                        crate::app_actions::LyricsAction::ApplyFetchedLyrics(
                            lyrics_and_metadata.clone(),
                        ),
                    )));
                }

                app.send_action(UserAction::UI(UIAction::StopOtherSearches));
            }

            AutoFetchResult::CoverUpdate {
                title,
                artist,
                cover_data,
            } => {
                let now_playing = &app.player.current_now_playing;
                let current_title = now_playing.title.as_deref().unwrap_or_default();
                let current_artist = now_playing.artist.as_deref().unwrap_or_default();

                if current_title == title && current_artist == artist {
                    app.player.current_now_playing.cover_data = cover_data.clone();

                    if let Some(cover_bytes) = cover_data
                        && let Some(command_tx) = &app.amll_connector.command_tx
                    {
                        let send_result = command_tx.try_send(
                            crate::amll_connector::types::ConnectorCommand::SendCover(cover_bytes),
                        );
                        if let Err(e) = send_result {
                            warn!("[AutoFetch] 发送封面到 WebSocket 失败: {}", e);
                        } else {
                            debug!("[AutoFetch] 已发送封面到 WebSocket");
                        }
                    }
                    app.egui_ctx.request_repaint();
                } else {
                    debug!(
                        "[CoverUpdate] 封面已过时 (当前歌曲: '{} - {}', 封面所属: '{} - {}')，已丢弃。",
                        current_title, current_artist, title, artist
                    );
                }
            }

            AutoFetchResult::NotFound => {
                info!("[UniLyricApp] 自动获取歌词：所有在线源均未找到。");
                app.send_action(UserAction::UI(UIAction::StopOtherSearches));
                if !app.fetcher.current_ui_populated
                    && app.amll_connector.config.lock().unwrap().enabled
                    && let Some(tx) = &app.amll_connector.command_tx
                {
                    let empty_ttml_message = ClientMessage::SetLyricFromTTML {
                        data: NullString("".to_string()),
                    };
                    if tx
                        .try_send(crate::amll_connector::ConnectorCommand::SendClientMessage(
                            empty_ttml_message,
                        ))
                        .is_err()
                    {
                        error!("[UniLyricApp] (未找到歌词) 发送空TTML失败。");
                    }
                }
            }
            AutoFetchResult::FetchError(err) => {
                error!("[UniLyricApp] 自动获取歌词时发生错误: {}", err.to_string());
                app.send_action(UserAction::UI(UIAction::StopOtherSearches));
            }
            AutoFetchResult::RequestCache => {
                app.send_action(UserAction::Player(PlayerAction::SaveToLocalCache));
            }
        }
    }
}

pub(super) fn draw_ui_elements(app: &mut UniLyricApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
        app.draw_toolbar(ui);
    });
    app.draw_log_panel(ctx);

    let available_width = ctx.screen_rect().width();
    let input_panel_width = (available_width * 0.25).clamp(200.0, 400.0);

    egui::SidePanel::left("input_panel")
        .default_width(input_panel_width)
        .show(ctx, |ui| {
            app.draw_input_panel_contents(ui);
        });

    let amll_connector_feature_is_enabled = app.amll_connector.config.lock().unwrap().enabled;

    if !amll_connector_feature_is_enabled {
        app.ui.show_amll_connector_sidebar = false;
    }

    if amll_connector_feature_is_enabled && app.ui.show_amll_connector_sidebar {
        egui::SidePanel::right("amll_connector_sidebar_panel")
            .resizable(false)
            .exact_width(300.0)
            .show(ctx, |ui| {
                app.draw_amll_connector_sidebar(ui);
            });
    }

    let lrc_panel_width = (available_width * 0.20).clamp(150.0, 350.0);
    let markers_panel_width = (available_width * 0.18).clamp(120.0, 300.0);

    if app.ui.show_markers_panel {
        egui::SidePanel::right("markers_panel")
            .default_width(markers_panel_width)
            .show(ctx, |ui| {
                app.draw_markers_panel_contents(ui, app.ui.wrap_text);
            });
    }
    if app.ui.show_translation_lrc_panel {
        egui::SidePanel::right("translation_lrc_panel")
            .default_width(lrc_panel_width)
            .show(ctx, |ui| {
                app.draw_translation_lrc_panel_contents(ui);
            });
    }
    if app.ui.show_romanization_lrc_panel {
        egui::SidePanel::right("romanization_lrc_panel")
            .default_width(lrc_panel_width)
            .show(ctx, |ui| {
                app.draw_romanization_lrc_panel_contents(ui);
            });
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        app.draw_output_panel_contents(ui);
    });

    if app.ui.show_metadata_panel {
        let mut window_is_actually_open = true;
        let mut should_keep_panel_open_from_internal_logic = app.ui.show_metadata_panel;

        egui::Window::new("编辑元数据")
            .open(&mut window_is_actually_open)
            .default_width(450.0)
            .default_height(400.0)
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
                app.draw_metadata_editor_window_contents(
                    ui,
                    &mut should_keep_panel_open_from_internal_logic,
                );
            });

        if !window_is_actually_open || !should_keep_panel_open_from_internal_logic {
            app.ui.show_metadata_panel = false;
        }
    }

    if app.ui.show_settings_window {
        app.draw_settings_window(ctx);
    }
}

pub(super) fn handle_file_drops(app: &mut UniLyricApp, ctx: &egui::Context) {
    if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(file) = files.first() {
            if let Some(path) = &file.path {
                crate::io::load_file_and_convert(app, path.clone());
            } else if let Some(bytes) = &file.bytes {
                warn!(
                    "[FileDrop] 文件路径不存在，但检测到字节数据 ({} bytes)。",
                    bytes.len()
                );
                if let Ok(text_content) = String::from_utf8(bytes.to_vec()) {
                    app.send_action(crate::app_actions::UserAction::Lyrics(Box::new(
                        crate::app_actions::LyricsAction::ClearAllData,
                    )));
                    app.lyrics.input_text = text_content;
                    app.lyrics.metadata_source_is_download = false;
                    app.send_action(crate::app_actions::UserAction::Lyrics(Box::new(
                        crate::app_actions::LyricsAction::Convert,
                    )));
                } else {
                    warn!("[FileDrop] 拖放的字节数据不是有效的UTF-8文本。");
                }
            } else {
                warn!("[FileDrop] 文件既没有路径也没有字节数据。");
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

/// 处理来自歌词转换任务的结果。
pub(super) fn handle_conversion_results(app: &mut UniLyricApp) {
    if let Some(rx) = &app.lyrics.conversion_result_rx
        && let Ok(result) = rx.try_recv()
    {
        app.lyrics.conversion_result_rx.take();

        let converted_result = result.map_err(AppError::from);
        app.send_action(crate::app_actions::UserAction::Lyrics(Box::new(
            crate::app_actions::LyricsAction::ConvertCompleted(converted_result),
        )));
    }
}

/// 处理来自异步歌词搜索任务的结果。
pub(super) fn handle_search_results(app: &mut UniLyricApp) {
    if let Some(rx) = app.lyrics.search_result_rx.take() {
        while let Ok(result) = rx.try_recv() {
            let converted_result = result.map_err(AppError::from);
            app.send_action(crate::app_actions::UserAction::Lyrics(Box::new(
                crate::app_actions::LyricsAction::SearchCompleted(converted_result),
            )));
        }
    }
}
/// 处理来自异步歌词下载任务的结果。
pub(super) fn handle_download_results(app: &mut UniLyricApp) {
    if let Some(rx) = &app.lyrics.download_result_rx
        && let Ok(result) = rx.try_recv()
    {
        app.lyrics.download_result_rx = None;

        let converted_result = result.map_err(AppError::from);
        app.send_action(crate::app_actions::UserAction::Lyrics(Box::new(
            crate::app_actions::LyricsAction::DownloadCompleted(converted_result),
        )));
    }
}

/// 并处理来自提供商加载任务的结果。
pub(super) fn handle_provider_load_results(app: &mut UniLyricApp) {
    if let Some(rx) = &app.lyrics_helper_state.provider_load_result_rx
        && let Ok(result) = rx.try_recv()
    {
        match result {
            Ok(_) => {
                info!("[LyricsHelper] 提供商加载成功，下载功能已就绪。");
                app.lyrics_helper_state.provider_state = ProviderState::Ready;
            }
            Err(e) => {
                error!("[LyricsHelper] 提供商加载失败: {}", e);
                app.lyrics_helper_state.provider_state = ProviderState::Failed(e);
            }
        }
        app.lyrics_helper_state.provider_load_result_rx = None;
    }
}
