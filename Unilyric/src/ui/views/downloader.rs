use crate::app_actions::{DownloaderAction, UserAction};
use crate::app_definition::{PreviewState, SearchState, UniLyricApp};
use crate::types::ProviderState;
use crate::ui::constants::TITLE_ALIGNMENT_OFFSET;
use eframe::egui::{self, Align, Button, Color32, Layout, ScrollArea, Spinner, TextEdit};

pub fn draw_downloader_view(app: &mut UniLyricApp, ctx: &egui::Context) {
    if matches!(
        app.lyrics_helper_state.provider_state,
        ProviderState::Uninitialized
    ) {
        app.trigger_provider_loading();
    }

    let mut action_to_send = None;

    egui::SidePanel::left("downloader_left_panel")
        .resizable(true)
        .default_width(300.0)
        .width_range(250.0..=500.0)
        .show(ctx, |left_ui| {
            left_ui.add_space(TITLE_ALIGNMENT_OFFSET);

            left_ui.horizontal(|header_ui| {
                header_ui.heading("搜索");
                header_ui.with_layout(Layout::right_to_left(Align::Center), |btn_ui| {
                    if btn_ui.button("返回").clicked() {
                        action_to_send =
                            Some(UserAction::Downloader(Box::new(DownloaderAction::Close)));
                    }
                });
            });

            left_ui.separator();
            let is_searching = matches!(app.downloader.search_state, SearchState::Searching);

            let mut perform_search = false;

            egui::Grid::new("search_inputs_grid")
                .num_columns(2)
                .show(left_ui, |grid_ui| {
                    grid_ui.label("歌曲名:");
                    let title_edit = grid_ui.add_enabled(
                        !is_searching,
                        TextEdit::singleline(&mut app.downloader.title_input).hint_text("必填"),
                    );
                    if title_edit.lost_focus() && grid_ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        perform_search = true;
                    }
                    grid_ui.end_row();

                    grid_ui.label("艺术家:");
                    let artist_edit = grid_ui.add_enabled(
                        !is_searching,
                        TextEdit::singleline(&mut app.downloader.artist_input).hint_text("可选"),
                    );
                    if artist_edit.lost_focus()
                        && grid_ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        perform_search = true;
                    }
                    grid_ui.end_row();

                    grid_ui.label("专辑:");
                    let album_edit = grid_ui.add_enabled(
                        !is_searching,
                        TextEdit::singleline(&mut app.downloader.album_input).hint_text("可选"),
                    );
                    if album_edit.lost_focus() && grid_ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        perform_search = true;
                    }
                    grid_ui.end_row();

                    grid_ui.label("时长 (ms):");
                    grid_ui.add_enabled(
                        !is_searching,
                        egui::DragValue::new(&mut app.downloader.duration_ms_input).speed(1000.0),
                    );
                    grid_ui.end_row();
                });

            left_ui.horizontal(|h_ui| {
                let providers_ready =
                    matches!(app.lyrics_helper_state.provider_state, ProviderState::Ready);
                let search_enabled =
                    !app.downloader.title_input.is_empty() && !is_searching && providers_ready;

                if h_ui
                    .add_enabled(search_enabled, Button::new("搜索"))
                    .clicked()
                {
                    perform_search = true;
                }

                if h_ui.button("从SMTC填充").clicked() {
                    action_to_send = Some(UserAction::Downloader(Box::new(
                        DownloaderAction::FillFromSmtc,
                    )));
                }

                if is_searching {
                    h_ui.add(Spinner::new());
                }
            });

            if perform_search {
                action_to_send = Some(UserAction::Downloader(Box::new(
                    DownloaderAction::PerformSearch,
                )));
            }

            left_ui.add_space(10.0);
            left_ui.heading("搜索结果");
            left_ui.separator();

            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(left_ui, |s_ui| match &app.downloader.search_state {
                    SearchState::Idle => {
                        s_ui.label("请输入关键词进行搜索。");
                    }
                    SearchState::Searching => {
                        s_ui.label("正在搜索...");
                    }
                    SearchState::Error(err) => {
                        s_ui.colored_label(Color32::RED, "搜索失败:");
                        s_ui.label(err);
                    }
                    SearchState::Success(results) => {
                        if results.is_empty() {
                            s_ui.label("未找到结果。");
                        } else {
                            for result in results {
                                let is_selected =
                                    app.downloader.selected_result_for_preview.as_ref()
                                        == Some(result);

                                let artists_str = result
                                    .artists
                                    .iter()
                                    .map(|a| a.name.as_str())
                                    .collect::<Vec<_>>()
                                    .join("/");

                                let album_str = result.album.as_deref().unwrap_or("未知专辑");

                                let duration_str = result.duration.map_or_else(
                                    || "未知时长".to_string(),
                                    |ms| {
                                        let secs = ms / 1000;
                                        format!("{:02}:{:02}", secs / 60, secs % 60)
                                    },
                                );

                                let display_text = format!(
                                    "{} - {}\n专辑: {}\n时长: {} | 来源: {} | 匹配度: {:?}",
                                    result.title,
                                    artists_str,
                                    album_str,
                                    duration_str,
                                    result.provider_name,
                                    result.match_type
                                );
                                if s_ui.selectable_label(is_selected, display_text).clicked() {
                                    action_to_send = Some(UserAction::Downloader(Box::new(
                                        DownloaderAction::SelectResultForPreview(result.clone()),
                                    )));
                                }
                            }
                        }
                    }
                });
        });

    egui::CentralPanel::default().show(ctx, |right_ui| {
        right_ui.heading("歌词预览");
        right_ui.separator();

        match &app.downloader.preview_state {
            PreviewState::Idle => {}
            PreviewState::Loading => {
                right_ui.centered_and_justified(|cj_ui| {
                    cj_ui.vertical_centered(|vc_ui| {
                        vc_ui.add(Spinner::new());
                    });
                });
            }
            PreviewState::Error(err) => {
                right_ui.centered_and_justified(|cj_ui| {
                    cj_ui.label(format!("预览加载失败:\n{}", err));
                });
            }
            PreviewState::Success(preview_text) => {
                let can_apply = app.downloader.selected_full_lyrics.is_some();
                egui::TopBottomPanel::bottom("preview_actions_panel").show_inside(
                    right_ui,
                    |bottom_ui| {
                        bottom_ui.with_layout(Layout::right_to_left(Align::Center), |btn_ui| {
                            if btn_ui.add_enabled(can_apply, Button::new("应用")).clicked() {
                                action_to_send = Some(UserAction::Downloader(Box::new(
                                    DownloaderAction::ApplyAndClose,
                                )));
                            }
                            if let Some(result) = &app.downloader.selected_result_for_preview {
                                let url_opt = match result.provider_name.as_str() {
                                    "qq" => Some(format!("https://y.qq.com/n/ryqq_v2/songDetail/{}", result.provider_id)),
                                    "netease" => Some(format!("https://music.163.com/#/song?id={}", result.provider_id)),
                                    "kugou" => {
                                        let mut url = format!("https://www.kugou.com/song/#hash={}", result.provider_id);
                                        if let Some(aid) = &result.album_id {
                                            url.push_str(&format!("&album_id={aid}"));
                                        }
                                        Some(url)
                                    },
                                    "amll-ttml-database" => Some(format!("https://github.com/amll-dev/amll-ttml-db/blob/main/raw-lyrics/{}", result.provider_id)),
                                    _ => None,
                                };

                                if let Some(url) = url_opt
                                    && btn_ui.button("打开源网页").clicked() {
                                        ctx.open_url(egui::OpenUrl::new_tab(url));
                                }

                                btn_ui.separator();

                                btn_ui.label(format!("ID: {}", result.provider_id.as_str()));
                            }
                        });
                    },
                );

                egui::CentralPanel::default().show_inside(right_ui, |text_panel_ui| {
                    ScrollArea::vertical().auto_shrink([false, false]).show(
                        text_panel_ui,
                        |s_ui| {
                            s_ui.add(
                                egui::Label::new(egui::RichText::new(preview_text).monospace())
                                    .selectable(true)
                                    .wrap(),
                            );
                        },
                    );
                });
            }
        }
    });

    if let Some(action) = action_to_send {
        app.send_action(action);
    }
}
