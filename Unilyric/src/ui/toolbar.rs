use crate::app_actions::{LyricsAction, ProcessorType, UIAction, UserAction};
use crate::app_definition::{AppView, UniLyricApp};
use crate::ui::constants::BUTTON_STRIP_SPACING;
use eframe::egui::{self, Align, Layout};
use lyrics_helper_core::ChineseConversionConfig;

pub fn draw_toolbar(app: &mut UniLyricApp, ui: &mut egui::Ui) {
    egui::MenuBar::new().ui(ui, |ui_bar| {
        ui_bar.menu_button("文件", |file_menu| {
            if file_menu
                .add(egui::Button::new("打开歌词文件..."))
                .clicked()
            {
                app.send_action(crate::app_actions::UserAction::File(
                    crate::app_actions::FileAction::Open,
                ));
            }
            file_menu.separator();
            let main_lyrics_loaded = (app.lyrics.parsed_lyric_data.is_some()
                && app.lyrics.parsed_lyric_data.as_ref().is_some())
                || !app.lyrics.input_text.is_empty();
            let lrc_load_enabled = main_lyrics_loaded && !app.lyrics.conversion_in_progress;
            let disabled_lrc_hover_text = "请先加载主歌词文件或内容";

            let translation_button = egui::Button::new("加载翻译 (LRC)...");
            let mut translation_button_response =
                file_menu.add_enabled(lrc_load_enabled, translation_button);
            if !lrc_load_enabled {
                translation_button_response =
                    translation_button_response.on_disabled_hover_text(disabled_lrc_hover_text);
            }
            if translation_button_response.clicked() {
                app.send_action(crate::app_actions::UserAction::File(
                    crate::app_actions::FileAction::LoadTranslationLrc,
                ));
            }

            let romanization_button = egui::Button::new("加载罗马音 (LRC)...");
            let mut romanization_button_response =
                file_menu.add_enabled(lrc_load_enabled, romanization_button);
            if !lrc_load_enabled {
                romanization_button_response =
                    romanization_button_response.on_disabled_hover_text(disabled_lrc_hover_text);
            }
            if romanization_button_response.clicked() {
                app.send_action(crate::app_actions::UserAction::File(
                    crate::app_actions::FileAction::LoadRomanizationLrc,
                ));
            }
            file_menu.separator();

            file_menu.menu_button("下载歌词...", |download_menu| {
                if download_menu
                    .add(egui::Button::new("搜索歌词..."))
                    .clicked()
                {
                    app.send_action(crate::app_actions::UserAction::UI(
                        crate::app_actions::UIAction::SetView(
                            crate::app_definition::AppView::Downloader,
                        ),
                    ));
                }
            });

            file_menu.menu_button("批量处理...", |batch_menu| {
                if batch_menu.button("批量转换...").clicked() {
                    app.send_action(UserAction::UI(UIAction::SetView(AppView::BatchConverter)));
                }
            });

            file_menu.separator();
            if file_menu
                .add_enabled(
                    !app.lyrics.output_text.is_empty(),
                    egui::Button::new("保存输出为..."),
                )
                .clicked()
            {
                app.send_action(crate::app_actions::UserAction::File(
                    crate::app_actions::FileAction::Save,
                ));
            }
        });

        ui_bar.menu_button("后处理", |postprocess_menu| {
            let lyrics_loaded = app.lyrics.parsed_lyric_data.is_some();

            if postprocess_menu
                .add_enabled(lyrics_loaded, egui::Button::new("清理元数据行"))
                .on_disabled_hover_text("需要先成功解析歌词")
                .clicked()
            {
                app.send_action(UserAction::Lyrics(Box::new(LyricsAction::ApplyProcessor(
                    ProcessorType::MetadataStripper,
                ))));
            }

            if postprocess_menu
                .add_enabled(lyrics_loaded, egui::Button::new("音节平滑"))
                .on_disabled_hover_text("需要先成功解析歌词")
                .clicked()
            {
                app.send_action(UserAction::Lyrics(Box::new(LyricsAction::ApplyProcessor(
                    ProcessorType::SyllableSmoother,
                ))));
            }

            if postprocess_menu
                .add_enabled(lyrics_loaded, egui::Button::new("演唱者识别"))
                .on_disabled_hover_text("需要先成功解析歌词")
                .clicked()
            {
                app.send_action(UserAction::Lyrics(Box::new(LyricsAction::ApplyProcessor(
                    ProcessorType::AgentRecognizer,
                ))));
            }
        });

        ui_bar.menu_button("简繁转换", |tools_menu| {
            let conversion_enabled = !app.lyrics.input_text.is_empty()
                || app
                    .lyrics
                    .parsed_lyric_data
                    .as_ref()
                    .is_some_and(|d| !d.lines.is_empty());

            tools_menu.label(egui::RichText::new("通用简繁转换").strong());
            app.draw_chinese_conversion_menu_item(
                tools_menu,
                ChineseConversionConfig::S2t,
                "简体 → 繁体 (通用)",
                conversion_enabled,
            );
            app.draw_chinese_conversion_menu_item(
                tools_menu,
                ChineseConversionConfig::T2s,
                "繁体 → 简体 (通用)",
                conversion_enabled,
            );
            tools_menu.separator();

            tools_menu.label(egui::RichText::new("地区性转换 (含用语)").strong());
            tools_menu.menu_button("简体 →", |sub_menu| {
                app.draw_chinese_conversion_menu_item(
                    sub_menu,
                    ChineseConversionConfig::S2twp,
                    "台湾正体",
                    conversion_enabled,
                );
                app.draw_chinese_conversion_menu_item(
                    sub_menu,
                    ChineseConversionConfig::S2hk,
                    "香港繁体",
                    conversion_enabled,
                );
            });
            tools_menu.menu_button("繁体 →", |sub_menu| {
                app.draw_chinese_conversion_menu_item(
                    sub_menu,
                    ChineseConversionConfig::Tw2sp,
                    "大陆简体 (含用语)",
                    conversion_enabled,
                );
                app.draw_chinese_conversion_menu_item(
                    sub_menu,
                    ChineseConversionConfig::Tw2s,
                    "大陆简体 (仅文字)",
                    conversion_enabled,
                );
            });
            tools_menu.separator();

            tools_menu.label(egui::RichText::new("仅文字转换").strong());
            tools_menu.menu_button("繁体互转", |sub_menu| {
                app.draw_chinese_conversion_menu_item(
                    sub_menu,
                    ChineseConversionConfig::Tw2t,
                    "台湾繁体 → 香港繁体",
                    conversion_enabled,
                );
                app.draw_chinese_conversion_menu_item(
                    sub_menu,
                    ChineseConversionConfig::Hk2t,
                    "香港繁体 → 台湾繁体",
                    conversion_enabled,
                );
            });
            tools_menu.menu_button("其他转换", |sub_menu| {
                app.draw_chinese_conversion_menu_item(
                    sub_menu,
                    ChineseConversionConfig::S2tw,
                    "简体 → 台湾繁体 (仅文字)",
                    conversion_enabled,
                );
                app.draw_chinese_conversion_menu_item(
                    sub_menu,
                    ChineseConversionConfig::T2tw,
                    "繁体 → 台湾繁体 (异体字)",
                    conversion_enabled,
                );
                app.draw_chinese_conversion_menu_item(
                    sub_menu,
                    ChineseConversionConfig::T2hk,
                    "繁体 → 香港繁体 (异体字)",
                    conversion_enabled,
                );
                app.draw_chinese_conversion_menu_item(
                    sub_menu,
                    ChineseConversionConfig::Hk2s,
                    "香港繁体 → 简体",
                    conversion_enabled,
                );
            });
            tools_menu.separator();

            tools_menu.label(egui::RichText::new("日语汉字转换").strong());
            app.draw_chinese_conversion_menu_item(
                tools_menu,
                ChineseConversionConfig::Jp2t,
                "日语新字体 → 繁体旧字体",
                conversion_enabled,
            );
            app.draw_chinese_conversion_menu_item(
                tools_menu,
                ChineseConversionConfig::T2jp,
                "繁体旧字体 → 日语新字体",
                conversion_enabled,
            );
        });

        ui_bar.add_space(16.0);
        ui_bar.label("源格式:");
        let mut temp_source_format = app.lyrics.source_format;

        egui::ComboBox::from_id_salt("source_format_toolbar")
            .selected_text(app.lyrics.source_format.to_string())
            .show_ui(ui_bar, |ui_combo| {
                for fmt_option in &app.lyrics.available_formats {
                    let display_text = fmt_option.to_string();
                    let is_selectable_source = true;

                    let response = ui_combo
                        .add_enabled_ui(is_selectable_source, |ui_selectable| {
                            ui_selectable.selectable_value(
                                &mut temp_source_format,
                                *fmt_option,
                                display_text,
                            )
                        })
                        .inner;

                    if response.clicked() && is_selectable_source {
                        ui_combo.close();
                    }
                }
            });

        if temp_source_format != app.lyrics.source_format {
            app.send_action(crate::app_actions::UserAction::Lyrics(Box::new(
                crate::app_actions::LyricsAction::SourceFormatChanged(temp_source_format),
            )));
        }

        ui_bar.add_space(8.0);
        ui_bar.label("目标格式:");
        let mut _target_format_changed_this_frame = false;
        let mut temp_target_format = app.lyrics.target_format;

        egui::ComboBox::from_id_salt("target_format_toolbar")
            .selected_text(app.lyrics.target_format.to_string())
            .show_ui(ui_bar, |ui_combo| {
                for fmt_option in &app.lyrics.available_formats {
                    let display_text = fmt_option.to_string();
                    if ui_combo
                        .selectable_value(&mut temp_target_format, *fmt_option, display_text)
                        .clicked()
                    {
                        ui_combo.close();
                    }
                }
            });

        if temp_target_format != app.lyrics.target_format {
            app.send_action(crate::app_actions::UserAction::Lyrics(Box::new(
                crate::app_actions::LyricsAction::TargetFormatChanged(temp_target_format),
            )));
        }

        ui_bar.with_layout(Layout::right_to_left(Align::Center), |ui_right| {
            ui_right.menu_button("视图", |view_menu| {
                let mut show_translation_lrc_panel_copy = app.ui.show_translation_lrc_panel;
                if view_menu
                    .checkbox(&mut show_translation_lrc_panel_copy, "翻译LRC面板")
                    .changed()
                {
                    app.send_action(crate::app_actions::UserAction::UI(
                        crate::app_actions::UIAction::SetPanelVisibility(
                            crate::app_actions::PanelType::Translation,
                            show_translation_lrc_panel_copy,
                        ),
                    ));
                }

                let mut show_romanization_lrc_panel_copy = app.ui.show_romanization_lrc_panel;
                if view_menu
                    .checkbox(&mut show_romanization_lrc_panel_copy, "罗马音LRC面板")
                    .changed()
                {
                    app.send_action(crate::app_actions::UserAction::UI(
                        crate::app_actions::UIAction::SetPanelVisibility(
                            crate::app_actions::PanelType::Romanization,
                            show_romanization_lrc_panel_copy,
                        ),
                    ));
                }

                view_menu.separator();

                let amll_connector_feature_enabled =
                    app.amll_connector.config.lock().unwrap().enabled;
                view_menu
                    .add_enabled_ui(amll_connector_feature_enabled, |ui_enabled_check| {
                        let mut show_amll_sidebar_copy = app.ui.show_amll_connector_sidebar;
                        if ui_enabled_check
                            .checkbox(&mut show_amll_sidebar_copy, "AMLL Connector 侧边栏")
                            .changed()
                        {
                            app.send_action(crate::app_actions::UserAction::UI(
                                crate::app_actions::UIAction::SetPanelVisibility(
                                    crate::app_actions::PanelType::AmllConnector,
                                    show_amll_sidebar_copy,
                                ),
                            ));
                        }
                    })
                    .response
                    .on_disabled_hover_text("请在设置中启用 AMLL Connector 功能");

                view_menu.separator();

                let mut show_log_panel_copy = app.ui.show_bottom_log_panel;
                if view_menu
                    .checkbox(&mut show_log_panel_copy, "日志面板")
                    .changed()
                {
                    app.send_action(crate::app_actions::UserAction::UI(
                        crate::app_actions::UIAction::SetPanelVisibility(
                            crate::app_actions::PanelType::Log,
                            show_log_panel_copy,
                        ),
                    ));
                }
            });
            let mut wrap_text_copy = app.ui.wrap_text;
            if ui_right.checkbox(&mut wrap_text_copy, "自动换行").changed() {
                app.send_action(crate::app_actions::UserAction::UI(
                    crate::app_actions::UIAction::SetWrapText(wrap_text_copy),
                ));
            }
            ui_right.add_space(BUTTON_STRIP_SPACING);
            if ui_right.button("设置").clicked() {
                app.send_action(crate::app_actions::UserAction::UI(
                    crate::app_actions::UIAction::ShowPanel(
                        crate::app_actions::PanelType::Settings,
                    ),
                ));
            }
        });
    });
}
