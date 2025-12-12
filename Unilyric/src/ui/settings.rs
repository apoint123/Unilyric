use crate::app_actions::{AmllConnectorAction, SettingsAction, UserAction};
use crate::app_definition::UniLyricApp;
use crate::app_settings::AppAmllMirror;
use crate::types::{AutoSearchSource, SettingsCategory};
use eframe::egui::{self, ComboBox, Layout, ScrollArea, TextEdit};
use log::LevelFilter;

pub fn draw_settings_window(app: &mut UniLyricApp, ctx: &egui::Context) {
    let mut is_settings_window_open = app.ui.show_settings_window;

    egui::Window::new("应用程序设置")
        .open(&mut is_settings_window_open)
        .resizable(true)
        .default_width(700.0)
        .max_height(450.0)
        .show(ctx, |ui| {
            ui.horizontal_top(|h_ui| {
                egui::SidePanel::left("settings_category_panel")
                    .exact_width(140.0)
                    .show_inside(h_ui, |nav_ui| {
                        nav_ui.style_mut().spacing.item_spacing = egui::vec2(4.0, 8.0);
                        nav_ui.heading("设置");
                        nav_ui.separator();

                        let categories = [
                            SettingsCategory::General,
                            SettingsCategory::Interface,
                            SettingsCategory::AutoSearch,
                            SettingsCategory::Connector,
                            SettingsCategory::Postprocessors,
                        ];

                        for category in categories {
                            nav_ui.selectable_value(
                                &mut app.ui.current_settings_category,
                                category,
                                category.display_name(),
                            );
                        }
                    });

                egui::CentralPanel::default().show_inside(h_ui, |content_ui| {
                    ScrollArea::vertical().show(content_ui, |scroll_ui| {
                        match app.ui.current_settings_category {
                            SettingsCategory::General => draw_settings_general(app, scroll_ui),
                            SettingsCategory::Interface => draw_settings_interface(app, scroll_ui),
                            SettingsCategory::AutoSearch => {
                                draw_settings_auto_search(app, scroll_ui)
                            }
                            SettingsCategory::Connector => {
                                draw_settings_amll_connector(app, scroll_ui)
                            }
                            SettingsCategory::Postprocessors => {
                                draw_settings_postprocessors(app, scroll_ui)
                            }
                        }
                    });
                });
            });
            ui.separator();
            ui.with_layout(
                Layout::right_to_left(egui::Align::Center),
                |bottom_buttons_ui| {
                    if bottom_buttons_ui.button("取消").clicked() {
                        app.send_action(crate::app_actions::UserAction::Settings(
                            crate::app_actions::SettingsAction::Cancel,
                        ));
                    }
                    if bottom_buttons_ui
                        .button("重置")
                        .on_hover_text("撤销当前窗口中的所有更改")
                        .clicked()
                    {
                        app.send_action(UserAction::Settings(SettingsAction::Reset));
                    }
                    if bottom_buttons_ui
                        .button("保存并应用")
                        .on_hover_text("保存设置到文件。部分设置将在下次启动或下次自动搜索时生效")
                        .clicked()
                    {
                        app.send_action(crate::app_actions::UserAction::Settings(
                            crate::app_actions::SettingsAction::Save(Box::new(
                                app.ui.temp_edit_settings.clone(),
                            )),
                        ));
                    }
                },
            );
        });

    if !is_settings_window_open {
        app.ui.show_settings_window = false;
    }
}

fn draw_settings_general(app: &mut UniLyricApp, ui: &mut egui::Ui) {
    ui.heading("通用设置");
    ui.add_space(10.0);

    egui::Grid::new("log_settings_grid")
        .num_columns(2)
        .spacing([40.0, 4.0])
        .striped(true)
        .show(ui, |grid_ui| {
            grid_ui.label("启用文件日志:");
            grid_ui.checkbox(
                &mut app.ui.temp_edit_settings.log_settings.enable_file_log,
                "",
            );
            grid_ui.end_row();

            grid_ui.label("文件日志级别:");
            ComboBox::from_id_salt("file_log_level_combo_settings")
                .selected_text(format!(
                    "{:?}",
                    app.ui.temp_edit_settings.log_settings.file_log_level
                ))
                .show_ui(grid_ui, |ui_combo| {
                    ui_combo.selectable_value(
                        &mut app.ui.temp_edit_settings.log_settings.file_log_level,
                        LevelFilter::Off,
                        "Off",
                    );
                    ui_combo.selectable_value(
                        &mut app.ui.temp_edit_settings.log_settings.file_log_level,
                        LevelFilter::Error,
                        "Error",
                    );
                    ui_combo.selectable_value(
                        &mut app.ui.temp_edit_settings.log_settings.file_log_level,
                        LevelFilter::Warn,
                        "Warn",
                    );
                    ui_combo.selectable_value(
                        &mut app.ui.temp_edit_settings.log_settings.file_log_level,
                        LevelFilter::Info,
                        "Info",
                    );
                    ui_combo.selectable_value(
                        &mut app.ui.temp_edit_settings.log_settings.file_log_level,
                        LevelFilter::Debug,
                        "Debug",
                    );
                    ui_combo.selectable_value(
                        &mut app.ui.temp_edit_settings.log_settings.file_log_level,
                        LevelFilter::Trace,
                        "Trace",
                    );
                });
            grid_ui.end_row();

            grid_ui.label("控制台日志级别:");
            ComboBox::from_id_salt("console_log_level_combo_settings")
                .selected_text(format!(
                    "{:?}",
                    app.ui.temp_edit_settings.log_settings.console_log_level
                ))
                .show_ui(grid_ui, |ui_combo| {
                    ui_combo.selectable_value(
                        &mut app.ui.temp_edit_settings.log_settings.console_log_level,
                        LevelFilter::Off,
                        "Off",
                    );
                    ui_combo.selectable_value(
                        &mut app.ui.temp_edit_settings.log_settings.console_log_level,
                        LevelFilter::Error,
                        "Error",
                    );
                    ui_combo.selectable_value(
                        &mut app.ui.temp_edit_settings.log_settings.console_log_level,
                        LevelFilter::Warn,
                        "Warn",
                    );
                    ui_combo.selectable_value(
                        &mut app.ui.temp_edit_settings.log_settings.console_log_level,
                        LevelFilter::Info,
                        "Info",
                    );
                    ui_combo.selectable_value(
                        &mut app.ui.temp_edit_settings.log_settings.console_log_level,
                        LevelFilter::Debug,
                        "Debug",
                    );
                    ui_combo.selectable_value(
                        &mut app.ui.temp_edit_settings.log_settings.console_log_level,
                        LevelFilter::Trace,
                        "Trace",
                    );
                });
            grid_ui.end_row();
        });
}

fn draw_settings_interface(app: &mut UniLyricApp, ui: &mut egui::Ui) {
    ui.heading("界面设置");
    ui.add_space(10.0);

    ui.horizontal(|h_ui| {
        h_ui.label("界面字体:");

        let mut selected = app
            .ui
            .temp_edit_settings
            .selected_font_family
            .clone()
            .unwrap_or_else(|| "默认".to_string());

        egui::ComboBox::from_label("")
            .selected_text(&selected)
            .show_ui(h_ui, |combo_ui| {
                if combo_ui
                    .selectable_value(&mut selected, "默认".to_string(), "默认 (内置字体)")
                    .clicked()
                {
                    app.ui.temp_edit_settings.selected_font_family = None;
                }
                for font_name in &app.ui.available_system_fonts {
                    if combo_ui
                        .selectable_value(&mut selected, font_name.clone(), font_name)
                        .clicked()
                    {
                        app.ui.temp_edit_settings.selected_font_family = Some(font_name.clone());
                    }
                }
            });
    });
}

fn draw_settings_auto_search(app: &mut UniLyricApp, ui: &mut egui::Ui) {
    ui.heading("自动歌词搜索设置");
    ui.add_space(10.0);

    let auto_cache_enabled = app.ui.temp_edit_settings.auto_cache;

    ui.checkbox(&mut app.ui.temp_edit_settings.auto_cache, "自动缓存歌词");

    ui.add_enabled_ui(auto_cache_enabled, |enabled_ui| {
        enabled_ui.horizontal(|h_ui| {
            h_ui.label("最多缓存数量:");
            h_ui.add(
                egui::DragValue::new(&mut app.ui.temp_edit_settings.auto_cache_max_count)
                    .speed(1.0),
            );
        });
    });

    ui.separator();
    ui.checkbox(
        &mut app.ui.temp_edit_settings.prioritize_amll_db,
        "优先搜索 AMLL TTML 数据库 (推荐)",
    );
    ui.checkbox(
        &mut app.ui.temp_edit_settings.enable_t2s_for_auto_search,
        "将繁体 SMTC 信息转为简体再搜索 (推荐)",
    );
    ui.checkbox(
        &mut app.ui.temp_edit_settings.always_search_all_sources,
        "始终搜索所有源 (推荐)",
    );
    ui.add_space(10.0);
    ui.checkbox(
        &mut app.ui.temp_edit_settings.use_provider_subset,
        "只在以下选择的源中搜索:",
    );

    ui.add_enabled_ui(
        app.ui.temp_edit_settings.use_provider_subset,
        |enabled_ui| {
            egui::Frame::group(enabled_ui.style()).show(enabled_ui, |group_ui| {
                group_ui.label("选择要使用的提供商:");
                let all_providers = AutoSearchSource::default_order();
                for provider in all_providers {
                    let provider_name = Into::<&'static str>::into(provider).to_string();
                    let mut is_selected = app
                        .ui
                        .temp_edit_settings
                        .auto_search_provider_subset
                        .contains(&provider_name);
                    if group_ui
                        .checkbox(&mut is_selected, provider.display_name())
                        .changed()
                    {
                        if is_selected {
                            app.ui
                                .temp_edit_settings
                                .auto_search_provider_subset
                                .push(provider_name);
                        } else {
                            app.ui
                                .temp_edit_settings
                                .auto_search_provider_subset
                                .retain(|p| p != &provider_name);
                        }
                    }
                }
            });
        },
    );
}

fn draw_settings_amll_connector(app: &mut UniLyricApp, ui: &mut egui::Ui) {
    ui.heading("AMLL Connector 设置");
    ui.add_space(10.0);

    egui::Grid::new("amll_connector_settings_grid")
        .num_columns(2)
        .spacing([40.0, 4.0])
        .striped(true)
        .show(ui, |grid_ui| {
            grid_ui.label("启用 AMLL Connector 功能:");
            grid_ui
                .checkbox(&mut app.ui.temp_edit_settings.amll_connector_enabled, "")
                .on_hover_text("转发 SMTC 信息到 AMLL Player，让 AMLL Player 也支持其他音乐软件");
            grid_ui.end_row();

            grid_ui.label("WebSocket URL:");
            grid_ui
                .add(
                    TextEdit::singleline(
                        &mut app.ui.temp_edit_settings.amll_connector_websocket_url,
                    )
                    .hint_text("ws://localhost:11444")
                    .desired_width(f32::INFINITY),
                )
                .on_hover_text("需点击“保存并应用”");
            grid_ui.end_row();

            grid_ui.label("将音频数据发送到 AMLL Player");
            grid_ui.checkbox(&mut app.ui.temp_edit_settings.send_audio_data_to_player, "");
            grid_ui.end_row();

            grid_ui
                .label("时间轴偏移量 (毫秒):")
                .on_hover_text("调整SMTC报告的时间戳以匹配歌词");
            grid_ui.add(
                egui::DragValue::new(&mut app.ui.temp_edit_settings.smtc_time_offset_ms)
                    .speed(10.0)
                    .suffix(" ms"),
            );
            grid_ui.end_row();

            // grid_ui
            //     .label("校准时间轴")
            //     .on_hover_text("切歌时立刻跳转到0ms，可能对 Spotify 有奇效");
            // grid_ui.checkbox(
            //     &mut app.ui.temp_edit_settings.calibrate_timeline_on_song_change,
            //     "",
            // );
            // grid_ui.end_row();

            // grid_ui
            //     .label("在新曲目开始时快速暂停/播放")
            //     .on_hover_text("更强力地校准时间轴");
            // grid_ui.checkbox(
            //     &mut app.ui.temp_edit_settings.flicker_play_pause_on_song_change,
            //     "",
            // );
            // grid_ui.end_row();
        });
    ui.add_space(10.0);
    ui.strong("AMLL DB 镜像");

    ui.horizontal(|h_ui| {
        if h_ui.button("立即检查更新").clicked() {
            app.send_action(UserAction::AmllConnector(
                AmllConnectorAction::CheckIndexUpdate,
            ));
        }

        if h_ui.button("重新加载所有提供商").clicked() {
            app.send_action(UserAction::AmllConnector(
                AmllConnectorAction::ReloadProviders,
            ));
        }
    });

    let current_mirror = &mut app.ui.temp_edit_settings.amll_mirror;

    let mirror_name = match current_mirror {
        AppAmllMirror::GitHub => "GitHub",
        AppAmllMirror::Dimeta => "Dimeta",
        AppAmllMirror::Bikonoo => "Bikonoo",
        AppAmllMirror::Custom { .. } => "自定义",
    };

    ComboBox::from_id_salt("amll_mirror_selector")
        .selected_text(mirror_name)
        .show_ui(ui, |combo_ui| {
            combo_ui.selectable_value(current_mirror, AppAmllMirror::Dimeta, "Dimeta");
            combo_ui.selectable_value(current_mirror, AppAmllMirror::Bikonoo, "Bikonoo");
            combo_ui.selectable_value(current_mirror, AppAmllMirror::GitHub, "GitHub (主源)");

            let is_custom = matches!(current_mirror, AppAmllMirror::Custom { .. });
            if combo_ui.selectable_label(is_custom, "自定义").clicked() && !is_custom {
                *current_mirror = AppAmllMirror::Custom {
                    index_url: String::new(),
                    lyrics_url_template: String::new(),
                };
            }
        });

    if let AppAmllMirror::Custom {
        index_url,
        lyrics_url_template,
    } = current_mirror
    {
        ui.add_space(5.0);
        ui.label("索引 URL:");
        ui.text_edit_singleline(index_url)
            .on_hover_text("指向 raw-lyrics-index.jsonl 文件的完整 URL");

        ui.label("歌词模板 URL:");
        ui.text_edit_singleline(lyrics_url_template)
            .on_hover_text("必须包含 {song_id} 占位符，例如：https://my.mirror/lyrics/{song_id}");
    }
}

fn draw_settings_postprocessors(app: &mut UniLyricApp, ui: &mut egui::Ui) {
    ui.heading("后处理器设置");
    ui.separator();

    ui.strong("自动应用");
    ui.label("自动获取歌词后，运行以下后处理器：");
    ui.checkbox(
        &mut app.ui.temp_edit_settings.auto_apply_metadata_stripper,
        "清理元数据行",
    );
    ui.checkbox(
        &mut app.ui.temp_edit_settings.auto_apply_agent_recognizer,
        "识别演唱者",
    );
    ui.separator();

    ui.collapsing("元数据清理器", |stripper_ui| {
        let options = &mut app.ui.temp_edit_settings.metadata_stripper;

        let mut is_enabled = options
            .flags
            .contains(lyrics_helper_core::MetadataStripperFlags::ENABLED);
        if stripper_ui
            .checkbox(&mut is_enabled, "启用元数据清理")
            .changed()
        {
            options.flags.set(
                lyrics_helper_core::MetadataStripperFlags::ENABLED,
                is_enabled,
            );
        }

        let mut regex_enabled = options
            .flags
            .contains(lyrics_helper_core::MetadataStripperFlags::ENABLE_REGEX_STRIPPING);
        if stripper_ui
            .checkbox(&mut regex_enabled, "启用正则表达式清理")
            .changed()
        {
            options.flags.set(
                lyrics_helper_core::MetadataStripperFlags::ENABLE_REGEX_STRIPPING,
                regex_enabled,
            );
        }

        stripper_ui.label("关键词 (每行一个):");
        let mut keywords_text = options.keywords.join("\n");
        if stripper_ui
            .add(TextEdit::multiline(&mut keywords_text).desired_rows(3))
            .changed()
        {
            options.keywords = keywords_text.lines().map(String::from).collect();
        }

        stripper_ui.label("正则表达式 (每行一个):");
        let mut regex_text = options.regex_patterns.join("\n");
        if stripper_ui
            .add(TextEdit::multiline(&mut regex_text).desired_rows(3))
            .changed()
        {
            options.regex_patterns = regex_text.lines().map(String::from).collect();
        }
    });

    ui.collapsing("音节平滑", |smoothing_ui| {
        let options = &mut app.ui.temp_edit_settings.syllable_smoothing;

        smoothing_ui.horizontal(|h_ui| {
            h_ui.label("平滑因子 (0.0-0.5):");
            h_ui.add(egui::Slider::new(&mut options.factor, 0.0..=0.5));
        });
        smoothing_ui.horizontal(|h_ui| {
            h_ui.label("平滑迭代次数:");
            h_ui.add(egui::DragValue::new(&mut options.smoothing_iterations).speed(1.0));
        });
        smoothing_ui.horizontal(|h_ui| {
            h_ui.label("时长差异阈值 (ms):");
            h_ui.add(egui::DragValue::new(&mut options.duration_threshold_ms).speed(1.0));
        });
        smoothing_ui.horizontal(|h_ui| {
            h_ui.label("间隔阈值 (ms):");
            h_ui.add(egui::DragValue::new(&mut options.gap_threshold_ms).speed(1.0));
        });
    });
}
