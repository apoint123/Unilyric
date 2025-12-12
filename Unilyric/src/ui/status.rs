use crate::app_actions::{PanelType, UIAction, UserAction};
use crate::app_definition::UniLyricApp;
use eframe::egui;

pub fn draw_log_panel(app: &mut UniLyricApp, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("log_panel_id")
        .resizable(true)
        .default_height(150.0)
        .min_height(60.0)
        .max_height(ctx.available_rect().height() * 0.7)
        .show_animated(ctx, app.ui.show_bottom_log_panel, |ui| {
            ui.vertical_centered_justified(|ui_header| {
                ui_header.horizontal(|h_ui| {
                    h_ui.label(egui::RichText::new("日志").strong());
                    h_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |btn_ui| {
                        if btn_ui.button("关闭").clicked() {
                            app.send_action(UserAction::UI(UIAction::HidePanel(
                                crate::app_actions::PanelType::Log,
                            )));
                        }
                        if btn_ui.button("清空").clicked() {
                            app.send_action(UserAction::UI(UIAction::ClearLogs));
                        }
                    });
                });
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |scroll_ui| {
                    if app.ui.log_display_buffer.is_empty() {
                        scroll_ui.add_space(5.0);
                        scroll_ui.label(egui::RichText::new("暂无日志。").weak().italics());
                        scroll_ui.add_space(5.0);
                    } else {
                        for entry in &app.ui.log_display_buffer {
                            scroll_ui.horizontal_wrapped(|line_ui| {
                                line_ui.label(
                                    egui::RichText::new(
                                        entry.timestamp.format("[%H:%M:%S.%3f]").to_string(),
                                    )
                                    .monospace(),
                                );
                                line_ui.add_space(4.0);
                                line_ui.label(
                                    egui::RichText::new(format!("[{}]", entry.level.as_str()))
                                        .monospace()
                                        .color(entry.level.color())
                                        .strong(),
                                );
                                line_ui.add_space(4.0);
                                line_ui.label(egui::RichText::new(&entry.message).monospace());
                            });
                        }
                    }
                    scroll_ui.allocate_space(scroll_ui.available_size_before_wrap());
                });
        });
}

pub fn draw_status_bar(app: &mut UniLyricApp, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("app_status_bar").show(ctx, |ui| {
        ui.horizontal_centered(|h_ui| {
            h_ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |right_ui| {
                    let warnings_count = app.lyrics.current_warnings.len();
                    if warnings_count > 0 {
                        let button_text = format!("⚠️ {}", warnings_count);
                        let button = right_ui.button(button_text);
                        if button.clicked() {
                            app.send_action(UserAction::UI(UIAction::ShowPanel(
                                PanelType::Warnings,
                            )));
                        }
                    }
                },
            );
        });
    });
}

pub fn draw_warnings_panel(app: &mut UniLyricApp, ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("warnings_panel_id")
        .resizable(true)
        .default_height(150.0)
        .min_height(60.0)
        .show_animated(ctx, app.ui.show_warnings_panel, |ui| {
            ui.vertical_centered_justified(|ui_header| {
                ui_header.horizontal(|h_ui| {
                    h_ui.label(egui::RichText::new("解析警告").strong());
                    h_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |btn_ui| {
                        if btn_ui.button("关闭").clicked() {
                            app.send_action(UserAction::UI(UIAction::HidePanel(
                                PanelType::Warnings,
                            )));
                        }
                    });
                });
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |scroll_ui| {
                    if app.lyrics.current_warnings.is_empty() {
                        scroll_ui.label(egui::RichText::new("暂无警告。").weak().italics());
                    } else {
                        for warning in &app.lyrics.current_warnings {
                            scroll_ui.horizontal_wrapped(|line_ui| {
                                line_ui.label("⚠️");
                                line_ui.label(warning);
                            });
                        }
                    }
                });
        });
}
