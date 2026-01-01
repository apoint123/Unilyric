use crate::amll_connector::WebsocketStatus;
use crate::amll_connector::types::ConnectorMode;
use crate::app_actions::{AmllConnectorAction, LyricsAction, PlayerAction, UserAction};
use crate::app_definition::UniLyricApp;
use crate::types::{AutoSearchSource, AutoSearchStatus};
use crate::ui::constants::TITLE_ALIGNMENT_OFFSET;
use eframe::egui::{self, Align, Button, Layout, Spinner};
use egui::Color32;
use lyrics_helper_core::FullLyricsResult;

pub fn draw_amll_connector_sidebar(app: &mut UniLyricApp, ui: &mut egui::Ui) {
    ui.add_space(TITLE_ALIGNMENT_OFFSET);
    ui.heading("AMLL Connector");
    ui.separator();

    ui.strong("AMLL Player 连接:");

    ui.vertical(|ui| {
        let current_status = app.amll_connector.status.lock().unwrap().clone();

        let (mode, url, port) = {
            let config = app.amll_connector.config.lock().unwrap();
            (
                config.mode,
                config.websocket_url.clone(),
                config.server_port,
            )
        };

        match mode {
            ConnectorMode::Client => {
                ui.label("模式: 客户端".to_string());
                ui.label(format!("目标: {url}"));
            }
            ConnectorMode::Server => {
                ui.label("模式: 服务端".to_string());
                ui.label(format!("监听端口: {port}"));
            }
        }

        match current_status {
            WebsocketStatus::Disconnected => {
                let btn_text = match mode {
                    ConnectorMode::Client => "连接到 AMLL Player",
                    ConnectorMode::Server => "启动 WebSocket 服务器",
                };

                if ui.button(btn_text).clicked() {
                    app.send_action(UserAction::AmllConnector(AmllConnectorAction::Connect));
                }
                ui.weak("状态: 未运行");
            }
            WebsocketStatus::Connecting => {
                ui.horizontal(|h_ui| {
                    h_ui.add(Spinner::new());
                    h_ui.label(match mode {
                        ConnectorMode::Client => "正在连接...",
                        ConnectorMode::Server => "正在启动...",
                    });
                });
            }
            WebsocketStatus::Connected => {
                let btn_text = match mode {
                    ConnectorMode::Client => "断开连接",
                    ConnectorMode::Server => "停止服务器",
                };

                if ui.button(btn_text).clicked() {
                    app.send_action(UserAction::AmllConnector(AmllConnectorAction::Disconnect));
                }

                let status_text = match mode {
                    ConnectorMode::Client => "已连接",
                    ConnectorMode::Server => "运行中 (正在监听)",
                };
                ui.colored_label(Color32::GREEN, status_text);
            }
            WebsocketStatus::Error(err_msg_ref) => {
                if ui.button("重试").clicked() {
                    app.send_action(UserAction::AmllConnector(AmllConnectorAction::Retry));
                }
                ui.colored_label(Color32::RED, "状态: 错误");
                ui.small(err_msg_ref);
            }
        }
    });

    ui.separator();

    ui.strong("SMTC 源应用:");

    let available_sessions = app.player.available_sessions.clone();
    let mut selected_id = app.player.last_requested_session_id.clone();

    let combo_label_text = match selected_id.as_ref() {
        Some(id) => available_sessions
            .iter()
            .find(|s| &s.session_id == id)
            .map_or_else(
                || format!("自动 (之前选择的 '{id}' 已失效)"),
                |s_info| s_info.display_name.clone(),
            ),
        None => "自动 (系统默认)".to_string(),
    };

    let combo_changed = egui::ComboBox::from_id_salt("smtc_source_selector")
        .selected_text(combo_label_text)
        .show_ui(ui, |combo_ui| {
            let mut changed_in_combo = false;
            if combo_ui
                .selectable_label(selected_id.is_none(), "自动 (系统默认)")
                .clicked()
            {
                selected_id = None;
                changed_in_combo = true;
            }
            for session_info in &available_sessions {
                if combo_ui
                    .selectable_label(
                        selected_id.as_ref() == Some(&session_info.session_id),
                        &session_info.display_name,
                    )
                    .clicked()
                {
                    selected_id = Some(session_info.session_id.clone());
                    changed_in_combo = true;
                }
            }
            changed_in_combo
        })
        .inner
        .unwrap_or(false);

    if combo_changed {
        app.send_action(UserAction::Player(PlayerAction::SelectSmtcSession(
            selected_id.unwrap_or_default(),
        )));
    }

    ui.separator();
    ui.strong("当前监听 (SMTC):");

    let now_playing = &app.player.current_now_playing;
    if now_playing.title.is_some() {
        ui.label(format!(
            "歌曲: {}",
            now_playing.title.as_deref().unwrap_or("未知")
        ));
        ui.label(format!(
            "艺术家: {}",
            now_playing.artist.as_deref().unwrap_or("未知")
        ));
        ui.label(format!(
            "专辑: {}",
            now_playing.album_title.as_deref().unwrap_or("未知")
        ));

        if let Some(status) = now_playing.playback_status {
            ui.label(match status {
                smtc_suite::PlaybackStatus::Playing => "状态: 播放中",
                smtc_suite::PlaybackStatus::Paused => "状态: 已暂停",
                smtc_suite::PlaybackStatus::Stopped => "状态: 已停止",
            });
        }

        if let Some(cover_bytes) = &now_playing.cover_data
            && !cover_bytes.is_empty()
        {
            let image_id_cow = now_playing.cover_data_hash.map_or_else(
                || "smtc_cover_no_hash".into(),
                |hash| format!("smtc_cover_hash_{hash}").into(),
            );
            let image_source = egui::ImageSource::Bytes {
                uri: image_id_cow,
                bytes: cover_bytes.clone().into(),
            };
            ui.add_sized(
                egui::vec2(200.0, 200.0),
                egui::Image::new(image_source)
                    .max_size(egui::vec2(200.0, 200.0))
                    .maintain_aspect_ratio(true)
                    .bg_fill(Color32::TRANSPARENT),
            );
        }

        ui.strong("时间轴偏移:");
        let mut offset_action_to_send = None;
        ui.horizontal(|h_ui| {
            h_ui.label("偏移量:");
            let mut current_offset = app.player.smtc_time_offset_ms;
            let response = h_ui.add(
                egui::DragValue::new(&mut current_offset)
                    .speed(10.0)
                    .suffix(" ms"),
            );
            if response.changed() {
                offset_action_to_send = Some(UserAction::Player(PlayerAction::SetSmtcTimeOffset(
                    current_offset,
                )));
            }
        });

        if let Some(action) = offset_action_to_send {
            app.send_action(action);
        }
    } else {
        ui.weak("无SMTC信息 / 未选择特定源");
    }

    ui.separator();

    ui.strong("本地歌词缓存:");
    let can_save_to_local =
        !app.lyrics.output_text.is_empty() && app.player.current_now_playing.title.is_some();

    let save_button_widget = Button::new("💾 缓存输出框歌词到本地");
    let mut response = ui.add_enabled(can_save_to_local, save_button_widget);
    if !can_save_to_local {
        response = response.on_disabled_hover_text("需先有歌词输出和媒体信息才能缓存");
    }
    if response.clicked() {
        app.send_action(UserAction::Player(PlayerAction::SaveToLocalCache));
    }

    ui.separator();

    ui.strong("自动歌词搜索状态:");
    let sources_config = vec![
        (
            AutoSearchSource::LocalCache,
            &app.fetcher.local_cache_status,
            None,
        ),
        (
            AutoSearchSource::QqMusic,
            &app.fetcher.qqmusic_status,
            Some(&app.fetcher.last_qq_result),
        ),
        (
            AutoSearchSource::Kugou,
            &app.fetcher.kugou_status,
            Some(&app.fetcher.last_kugou_result),
        ),
        (
            AutoSearchSource::Netease,
            &app.fetcher.netease_status,
            Some(&app.fetcher.last_netease_result),
        ),
        (
            AutoSearchSource::AmllDb,
            &app.fetcher.amll_db_status,
            Some(&app.fetcher.last_amll_db_result),
        ),
    ];

    let mut action_load_lyrics: Option<(AutoSearchSource, FullLyricsResult)> = None;
    let mut action_refetch: Option<AutoSearchSource> = None;

    for (source_enum, status_arc, opt_result_arc) in sources_config {
        ui.horizontal(|item_ui| {
            item_ui.label(format!("{}:", source_enum.display_name()));
            let status = status_arc.lock().unwrap().clone();

            item_ui.with_layout(Layout::right_to_left(Align::Center), |right_aligned_ui| {
                let mut stored_data_for_load: Option<FullLyricsResult> = None;
                if let Some(result_arc) = opt_result_arc
                    && let Some(ref data) = *result_arc.lock().unwrap()
                {
                    stored_data_for_load = Some(data.clone());
                }

                if let Some(data) = stored_data_for_load {
                    if right_aligned_ui
                        .button("载入")
                        .on_hover_text(format!("使用 {} 找到的歌词", source_enum.display_name()))
                        .clicked()
                    {
                        action_load_lyrics = Some((source_enum, data));
                    }
                    right_aligned_ui.add_space(4.0);
                }

                if source_enum != AutoSearchSource::LocalCache
                    && right_aligned_ui.button("重搜").clicked()
                {
                    action_refetch = Some(source_enum);
                }

                let status_display_text = match status {
                    AutoSearchStatus::NotAttempted => "未尝试".to_string(),
                    AutoSearchStatus::Searching => "正在搜索...".to_string(),
                    AutoSearchStatus::Success(_) => "已找到".to_string(),
                    AutoSearchStatus::NotFound => "未找到".to_string(),
                    AutoSearchStatus::Error(_) => "错误".to_string(),
                };

                if let AutoSearchStatus::Searching = status {
                    right_aligned_ui.spinner();
                }
                right_aligned_ui.label(status_display_text);
            });
        });
    }

    if let Some((_source, result)) = action_load_lyrics {
        let lyrics_and_metadata = Box::new(lyrics_helper_core::model::track::LyricsAndMetadata {
            lyrics: result,
            source_track: Default::default(),
        });
        app.send_action(UserAction::Lyrics(Box::new(
            LyricsAction::ApplyFetchedLyrics(lyrics_and_metadata),
        )));
    }
    if let Some(source) = action_refetch {
        crate::app_fetch_core::trigger_manual_refetch_for_source(app, source);
    }
}
