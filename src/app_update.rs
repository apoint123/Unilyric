use eframe::egui;
use tracing::{error, info, warn};

use crate::amll_connector::ConnectorUpdate;
use crate::amll_connector::protocol::ClientMessage;
use crate::amll_connector::protocol_strings::NullString;
use crate::app::TtmlDbUploadUserAction;
use crate::app_definition::UniLyricApp;
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
    while let Ok(update) = app.amll_connector.update_rx.try_recv() {
        match update {
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
                        crate::app_fetch_core::initial_auto_fetch_and_send_lyrics(
                            app,
                            new_info.clone(),
                        );
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
                _ => {}
            },
        }
    }
}

pub(super) fn handle_auto_fetch_results(app: &mut UniLyricApp) {
    match app.fetcher.result_rx.try_recv() {
        Ok(auto_fetch_result) => match auto_fetch_result {
            AutoFetchResult::Success {
                source,
                full_lyrics_result,
            } => {
                info!("[AutoFetch] 自动获取成功，来源: {source:?}");
                // 总是更新结果缓存和状态，无论是自动搜索还是手动重搜
                let result_cache_opt = match source {
                    AutoSearchSource::QqMusic => Some(&app.fetcher.last_qq_result),
                    AutoSearchSource::Kugou => Some(&app.fetcher.last_kugou_result),
                    AutoSearchSource::Netease => Some(&app.fetcher.last_netease_result),
                    AutoSearchSource::AmllDb => Some(&app.fetcher.last_amll_db_result),
                    AutoSearchSource::LocalCache => {
                        // 本地缓存没有“重载”逻辑，所以我们不需要缓存它
                        // 直接跳过缓存操作
                        None
                    }
                };
                if let Some(result_cache) = result_cache_opt {
                    *result_cache.lock().unwrap() = Some(full_lyrics_result.clone());
                }

                let source_format = full_lyrics_result.parsed.source_format;
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
                    app.send_action(crate::app_actions::UserAction::Lyrics(
                        crate::app_actions::LyricsAction::GenerateFromParsed(full_lyrics_result),
                    ));
                }

                let all_search_status_arcs = [
                    &app.fetcher.local_cache_status,
                    &app.fetcher.qqmusic_status,
                    &app.fetcher.kugou_status,
                    &app.fetcher.netease_status,
                    &app.fetcher.amll_db_status,
                ];

                for status_arc in all_search_status_arcs {
                    let mut guard = status_arc.lock().unwrap();
                    if matches!(*guard, AutoSearchStatus::Searching) {
                        *guard = AutoSearchStatus::NotFound;
                    }
                }
            }

            AutoFetchResult::NotFound => {
                info!("[UniLyricApp] 自动获取歌词：所有在线源均未找到。");
                let sources_to_update_on_not_found = [
                    &app.fetcher.qqmusic_status,
                    &app.fetcher.kugou_status,
                    &app.fetcher.netease_status,
                    &app.fetcher.amll_db_status,
                ];
                for status_arc in sources_to_update_on_not_found {
                    let mut guard = status_arc.lock().unwrap();
                    if matches!(*guard, AutoSearchStatus::Searching) {
                        *guard = AutoSearchStatus::NotFound;
                    }
                }
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

pub(super) fn handle_ttml_db_upload_actions(app: &mut UniLyricApp) {
    match app.ttml_db_upload.action_rx.try_recv() {
        Ok(action) => match action {
            TtmlDbUploadUserAction::InProgressUpdate(msg) => {
                info!("[TTML_DB_Upload_UI] 状态更新: {msg}");
                app.ui.toasts.add(Toast {
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
                app.ttml_db_upload.last_paste_url = Some(paste_url.clone());

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
                app.ui.toasts.add(Toast {
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

                app.ui.toasts.add(Toast {
                    text: final_toast_message_with_url.into(),
                    kind: final_toast_kind,
                    options: ToastOptions::default()
                        .duration_in_seconds(final_toast_duration)
                        .show_icon(true)
                        .show_progress(true),
                    style: Default::default(),
                });
                app.ttml_db_upload.in_progress = false;
            }
            TtmlDbUploadUserAction::PreparationError(err_msg) => {
                error!("[TTML_DB_Upload_UI] 准备阶段错误: {err_msg}");
                app.ttml_db_upload.in_progress = false;
            }
            TtmlDbUploadUserAction::Error(err_msg) => {
                error!("[Unilyric] 上传过程中发生错误: {err_msg}");
                app.ui.toasts.add(Toast {
                    text: format!("上传失败: {err_msg}").into(),
                    kind: ToastKind::Error,
                    options: ToastOptions::default()
                        .duration_in_seconds(5.0)
                        .show_icon(true),
                    style: Default::default(),
                });
                app.ttml_db_upload.in_progress = false;
            }
        },
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            error!("[Unilyric] 上传操作消息通道意外断开!");
            if app.ttml_db_upload.in_progress {
                app.ui.toasts.add(Toast {
                    text: "上传处理通道意外断开，操作可能未完成。".into(),
                    kind: ToastKind::Error,
                    options: ToastOptions::default()
                        .duration_in_seconds(4.0)
                        .show_icon(true),
                    style: Default::default(),
                });
                app.ttml_db_upload.in_progress = false;
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
                if let Ok(text_content) = String::from_utf8(bytes.to_vec()) {
                    app.send_action(crate::app_actions::UserAction::Lyrics(
                        crate::app_actions::LyricsAction::ClearAllData,
                    ));
                    app.lyrics.input_text = text_content;
                    app.lyrics.metadata_source_is_download = false;
                    app.send_action(crate::app_actions::UserAction::Lyrics(
                        crate::app_actions::LyricsAction::Convert,
                    ));
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

/// 处理来自歌词转换任务的结果。
pub(super) fn handle_conversion_results(app: &mut UniLyricApp) {
    if app.lyrics.conversion_result_rx.is_some()
        && let Ok(result) = app.lyrics.conversion_result_rx.as_ref().unwrap().try_recv()
    {
        app.lyrics.conversion_result_rx.take();

        let converted_result = result.map_err(|e| e.to_string());
        app.send_action(crate::app_actions::UserAction::Lyrics(
            crate::app_actions::LyricsAction::ConvertCompleted(converted_result),
        ));
    }
}

/// 处理来自异步歌词搜索任务的结果。
pub(super) fn handle_search_results(app: &mut UniLyricApp) {
    if let Some(rx) = &app.lyrics.search_result_rx
        && let Ok(result) = rx.try_recv()
    {
        app.lyrics.search_result_rx = None;

        let converted_result = result.map_err(|e| e.to_string());
        app.send_action(crate::app_actions::UserAction::Lyrics(
            crate::app_actions::LyricsAction::SearchCompleted(converted_result),
        ));
    }
}

/// 处理来自异步歌词下载任务的结果。
pub(super) fn handle_download_results(app: &mut UniLyricApp) {
    if let Some(rx) = &app.lyrics.download_result_rx
        && let Ok(result) = rx.try_recv()
    {
        app.lyrics.download_result_rx = None;

        let converted_result = result.map_err(|e| e.to_string());
        app.send_action(crate::app_actions::UserAction::Lyrics(
            crate::app_actions::LyricsAction::DownloadCompleted(converted_result),
        ));
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
