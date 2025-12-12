use crate::app_actions::{FileAction, LyricsAction, UserAction};
use crate::app_definition::UniLyricApp;
use crate::types::LrcContentType;
use crate::ui::constants::{BUTTON_STRIP_SPACING, TITLE_ALIGNMENT_OFFSET};
use eframe::egui::{self, Button, ScrollArea};

pub fn draw_input_panel_contents(app: &mut UniLyricApp, ui: &mut egui::Ui) {
    ui.add_space(TITLE_ALIGNMENT_OFFSET);
    ui.horizontal(|title_ui| {
        title_ui.heading("输入歌词");
        title_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |btn_ui| {
            if btn_ui
                .add_enabled(
                    !app.lyrics.input_text.is_empty() || !app.lyrics.output_text.is_empty(),
                    egui::Button::new("清空"),
                )
                .clicked()
            {
                app.send_action(UserAction::Lyrics(Box::new(LyricsAction::ClearAllData)));
            }
            btn_ui.add_space(BUTTON_STRIP_SPACING);
            if btn_ui
                .add_enabled(!app.lyrics.input_text.is_empty(), egui::Button::new("复制"))
                .clicked()
            {
                btn_ui.ctx().copy_text(app.lyrics.input_text.clone());
            }
            btn_ui.add_space(BUTTON_STRIP_SPACING);
            if btn_ui.button("粘贴").clicked() {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    if let Ok(text) = clipboard.get_text() {
                        app.lyrics.input_text = text.clone();
                        app.send_action(UserAction::Lyrics(Box::new(
                            LyricsAction::MainInputChanged(text),
                        )));
                    } else {
                        tracing::error!("无法从剪贴板获取文本");
                    }
                } else {
                    tracing::error!("无法访问剪贴板");
                }
            }
        });
    });
    ui.separator();

    let scroll_area = if app.ui.wrap_text {
        egui::ScrollArea::vertical().id_salt("input_scroll_vertical_only")
    } else {
        egui::ScrollArea::both()
            .id_salt("input_scroll_both")
            .auto_shrink([false, false])
    };

    scroll_area.auto_shrink([false, false]).show(ui, |s_ui| {
        let text_edit_widget = egui::TextEdit::multiline(&mut app.lyrics.input_text)
            .hint_text("在此处粘贴或拖放主歌词文件")
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY);

        let response = if !app.ui.wrap_text {
            let font_id = egui::TextStyle::Monospace.resolve(s_ui.style());
            let text_color = s_ui.visuals().text_color();

            let mut layouter = |ui: &egui::Ui, val: &dyn egui::TextBuffer, _wrap_width: f32| {
                let layout_job = egui::text::LayoutJob::simple(
                    val.as_str().to_owned(),
                    font_id.clone(),
                    text_color,
                    f32::INFINITY,
                );
                ui.fonts_mut(|f| f.layout_job(layout_job))
            };

            s_ui.add(text_edit_widget.layouter(&mut layouter))
        } else {
            s_ui.add(text_edit_widget)
        };

        if response.changed() && !app.lyrics.conversion_in_progress {
            app.send_action(UserAction::Lyrics(Box::new(
                LyricsAction::MainInputChanged(app.lyrics.input_text.clone()),
            )));
        }
    });
}

pub fn draw_translation_lrc_panel_contents(app: &mut UniLyricApp, ui: &mut egui::Ui) {
    let mut text_edited_this_frame = false;

    let title = "翻译 (LRC)";
    let lrc_is_currently_considered_active = app.lyrics.loaded_translation_lrc.is_some()
        || !app.lyrics.display_translation_lrc_output.trim().is_empty();

    ui.add_space(TITLE_ALIGNMENT_OFFSET);
    ui.label(egui::RichText::new(title).heading());
    ui.separator();

    ui.horizontal(|button_strip_ui| {
        let main_lyrics_exist_for_merge = app.lyrics.parsed_lyric_data.as_ref().is_some();
        let import_enabled = main_lyrics_exist_for_merge && !app.lyrics.conversion_in_progress;
        let import_button_widget = egui::Button::new("导入");
        let mut import_button_response =
            button_strip_ui.add_enabled(import_enabled, import_button_widget);
        if !import_enabled {
            import_button_response =
                import_button_response.on_disabled_hover_text("请先加载主歌词文件");
        }
        if import_button_response.clicked() {
            app.send_action(UserAction::File(FileAction::LoadTranslationLrc));
        }

        button_strip_ui.allocate_ui_with_layout(
            button_strip_ui.available_size_before_wrap(),
            egui::Layout::right_to_left(egui::Align::Center),
            |right_aligned_buttons_ui| {
                if right_aligned_buttons_ui
                    .add_enabled(
                        lrc_is_currently_considered_active,
                        egui::Button::new("清除"),
                    )
                    .clicked()
                {
                    app.send_action(UserAction::Lyrics(Box::new(LyricsAction::LrcInputChanged(
                        String::new(),
                        LrcContentType::Translation,
                    ))));
                }
                right_aligned_buttons_ui.add_space(BUTTON_STRIP_SPACING);
                if right_aligned_buttons_ui
                    .add_enabled(
                        !app.lyrics.display_translation_lrc_output.is_empty(),
                        egui::Button::new("复制"),
                    )
                    .clicked()
                {
                    right_aligned_buttons_ui
                        .ctx()
                        .copy_text(app.lyrics.display_translation_lrc_output.clone());
                }
            },
        );
    });

    let scroll_area = if app.ui.wrap_text {
        egui::ScrollArea::vertical().id_salt("translation_lrc_scroll_vertical")
    } else {
        egui::ScrollArea::both()
            .id_salt("translation_lrc_scroll_both")
            .auto_shrink([false, false])
    };

    scroll_area
        .auto_shrink([false, false])
        .show(ui, |s_ui_content| {
            let text_edit_widget =
                egui::TextEdit::multiline(&mut app.lyrics.display_translation_lrc_output)
                    .hint_text("在此处粘贴翻译LRC内容")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(10);

            let response = if !app.ui.wrap_text {
                let font_id = egui::TextStyle::Monospace.resolve(s_ui_content.style());
                let text_color = s_ui_content.visuals().text_color();

                let mut layouter = |ui: &egui::Ui, val: &dyn egui::TextBuffer, _wrap_width: f32| {
                    let layout_job = egui::text::LayoutJob::simple(
                        val.as_str().to_owned(),
                        font_id.clone(),
                        text_color,
                        f32::INFINITY,
                    );
                    ui.fonts_mut(|f| f.layout_job(layout_job))
                };
                s_ui_content.add(text_edit_widget.layouter(&mut layouter))
            } else {
                s_ui_content.add(text_edit_widget)
            };

            if response.changed() {
                text_edited_this_frame = true;
            }
            s_ui_content.allocate_space(s_ui_content.available_size_before_wrap());
        });

    if text_edited_this_frame {
        app.send_action(UserAction::Lyrics(Box::new(LyricsAction::LrcInputChanged(
            app.lyrics.display_translation_lrc_output.clone(),
            LrcContentType::Translation,
        ))));
    }
}

pub fn draw_romanization_lrc_panel_contents(app: &mut UniLyricApp, ui: &mut egui::Ui) {
    let mut text_edited_this_frame = false;

    let title = "罗马音 (LRC)";
    let lrc_is_currently_considered_active = app.lyrics.loaded_romanization_lrc.is_some()
        || !app.lyrics.display_romanization_lrc_output.trim().is_empty();

    ui.add_space(TITLE_ALIGNMENT_OFFSET);
    ui.label(egui::RichText::new(title).heading());
    ui.separator();

    ui.horizontal(|button_strip_ui| {
        let main_lyrics_exist_for_merge = app
            .lyrics
            .parsed_lyric_data
            .as_ref()
            .is_some_and(|p| !p.lines.is_empty());
        let import_enabled = main_lyrics_exist_for_merge && !app.lyrics.conversion_in_progress;
        let import_button_widget = egui::Button::new("导入");
        let mut import_button_response =
            button_strip_ui.add_enabled(import_enabled, import_button_widget);
        if !import_enabled {
            import_button_response =
                import_button_response.on_disabled_hover_text("请先加载主歌词文件");
        }
        if import_button_response.clicked() {
            app.send_action(UserAction::File(FileAction::LoadRomanizationLrc));
        }

        button_strip_ui.allocate_ui_with_layout(
            button_strip_ui.available_size_before_wrap(),
            egui::Layout::right_to_left(egui::Align::Center),
            |right_aligned_buttons_ui| {
                if right_aligned_buttons_ui
                    .add_enabled(
                        lrc_is_currently_considered_active,
                        egui::Button::new("清除"),
                    )
                    .clicked()
                {
                    app.send_action(UserAction::Lyrics(Box::new(LyricsAction::LrcInputChanged(
                        String::new(),
                        LrcContentType::Romanization,
                    ))));
                }
                right_aligned_buttons_ui.add_space(BUTTON_STRIP_SPACING);
                if right_aligned_buttons_ui
                    .add_enabled(
                        !app.lyrics.display_romanization_lrc_output.is_empty(),
                        egui::Button::new("复制"),
                    )
                    .clicked()
                {
                    right_aligned_buttons_ui
                        .ctx()
                        .copy_text(app.lyrics.display_romanization_lrc_output.clone());
                }
            },
        );
    });

    let scroll_area = if app.ui.wrap_text {
        egui::ScrollArea::vertical().id_salt("romanization_lrc_scroll_vertical")
    } else {
        egui::ScrollArea::both()
            .id_salt("romanization_lrc_scroll_both")
            .auto_shrink([false, false])
    };

    scroll_area
        .auto_shrink([false, false])
        .show(ui, |s_ui_content| {
            let text_edit_widget =
                egui::TextEdit::multiline(&mut app.lyrics.display_romanization_lrc_output)
                    .hint_text("在此处粘贴罗马音LRC内容")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(10);

            let response = if !app.ui.wrap_text {
                let font_id = egui::TextStyle::Monospace.resolve(s_ui_content.style());
                let text_color = s_ui_content.visuals().text_color();

                let mut layouter = |ui: &egui::Ui, val: &dyn egui::TextBuffer, _wrap_width: f32| {
                    let layout_job = egui::text::LayoutJob::simple(
                        val.as_str().to_owned(),
                        font_id.clone(),
                        text_color,
                        f32::INFINITY,
                    );
                    ui.fonts_mut(|f| f.layout_job(layout_job))
                };
                s_ui_content.add(text_edit_widget.layouter(&mut layouter))
            } else {
                s_ui_content.add(text_edit_widget)
            };

            if response.changed() {
                text_edited_this_frame = true;
            }
            s_ui_content.allocate_space(s_ui_content.available_size_before_wrap());
        });

    if text_edited_this_frame {
        app.send_action(UserAction::Lyrics(Box::new(LyricsAction::LrcInputChanged(
            app.lyrics.display_romanization_lrc_output.clone(),
            LrcContentType::Romanization,
        ))));
    }
}

pub fn draw_output_panel_contents(app: &mut UniLyricApp, ui: &mut egui::Ui) {
    ui.horizontal(|title_ui| {
        title_ui.heading("输出结果");
        title_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |btn_ui| {
            let send_to_player_enabled;
            {
                let connector_config_guard = app.amll_connector.config.lock().unwrap();
                send_to_player_enabled = connector_config_guard.enabled
                    && app.lyrics.parsed_lyric_data.is_some()
                    && !app.lyrics.conversion_in_progress;
            }

            let send_button = Button::new("发送到AMLL Player");
            let mut send_button_response = btn_ui.add_enabled(send_to_player_enabled, send_button);

            if !send_to_player_enabled {
                send_button_response =
                    send_button_response.on_disabled_hover_text("需要先成功转换出可用的歌词数据");
            }

            if send_button_response.clicked()
                && let (Some(tx), Some(parsed_data)) = (
                    &app.amll_connector.command_tx,
                    app.lyrics.parsed_lyric_data.as_ref(),
                )
            {
                if tx
                    .try_send(crate::amll_connector::ConnectorCommand::SendLyric(
                        parsed_data.clone(),
                    ))
                    .is_err()
                {
                    tracing::error!("[Unilyric UI] 手动发送歌词失败。");
                } else {
                    tracing::info!("[Unilyrc UI] 已从输出面板手动发送歌词。");
                }
            }

            btn_ui.add_space(BUTTON_STRIP_SPACING);

            if btn_ui
                .add_enabled(
                    !app.lyrics.output_text.is_empty() && !app.lyrics.conversion_in_progress,
                    Button::new("复制"),
                )
                .clicked()
            {
                btn_ui.ctx().copy_text(app.lyrics.output_text.clone());
                app.ui.toasts.add(egui_toast::Toast {
                    text: "输出内容已复制到剪贴板".into(),
                    kind: egui_toast::ToastKind::Success,
                    options: egui_toast::ToastOptions::default().duration_in_seconds(2.0),
                    style: Default::default(),
                });
            }
        });
    });
    ui.separator();

    let scroll_area = if app.ui.wrap_text {
        ScrollArea::vertical().id_salt("output_scroll_vertical_label")
    } else {
        ScrollArea::both()
            .id_salt("output_scroll_both_label")
            .auto_shrink([false, false])
    };

    scroll_area.auto_shrink([false, false]).show(ui, |s_ui| {
        let mut label_widget = egui::Label::new(
            egui::RichText::new(&app.lyrics.output_text)
                .monospace()
                .size(13.0),
        )
        .selectable(true);

        if app.ui.wrap_text {
            label_widget = label_widget.wrap();
        } else {
            label_widget = label_widget.extend();
        }
        s_ui.add(label_widget);
    });
}
