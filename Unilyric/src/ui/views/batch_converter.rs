use crate::app_actions::{BatchConverterAction, UIAction, UserAction};
use crate::app_definition::{AppView, BatchConverterStatus, UniLyricApp};
use eframe::egui::{self};

pub fn draw_batch_converter_view(app: &mut UniLyricApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("batch_converter_toolbar").show(ctx, |ui| {
        egui::MenuBar::new().ui(ui, |bar_ui| {
            if bar_ui.button("返回").clicked() {
                app.send_action(UserAction::UI(UIAction::SetView(AppView::Editor)));
            }
        });
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("批量歌词转换器");
        ui.separator();

        ui.horizontal(|h_ui| {
            h_ui.strong("输入目录:");
            if let Some(path) = &app.batch_converter.input_dir {
                h_ui.monospace(path.to_string_lossy());
            } else {
                h_ui.weak("未选择");
            }
            if h_ui.button("选择...").clicked() {
                app.send_action(UserAction::BatchConverter(
                    BatchConverterAction::SelectInputDir,
                ));
            }
        });

        ui.horizontal(|h_ui| {
            h_ui.strong("输出目录:");
            if let Some(path) = &app.batch_converter.output_dir {
                h_ui.monospace(path.to_string_lossy());
            } else {
                h_ui.weak("未选择");
            }
            if h_ui.button("选择...").clicked() {
                app.send_action(UserAction::BatchConverter(
                    BatchConverterAction::SelectOutputDir,
                ));
            }
        });

        ui.add_space(10.0);

        let can_scan = app.batch_converter.input_dir.is_some()
            && app.batch_converter.output_dir.is_some()
            && app.batch_converter.status != BatchConverterStatus::Converting;

        let scan_button = ui.add_enabled(can_scan, egui::Button::new("扫描任务"));
        if scan_button.clicked() {
            app.send_action(UserAction::BatchConverter(BatchConverterAction::ScanTasks));
        }
        if !can_scan && app.batch_converter.input_dir.is_none() {
            scan_button.on_disabled_hover_text("请先选择输入目录");
        } else if !can_scan && app.batch_converter.output_dir.is_none() {
            scan_button.on_disabled_hover_text("请先选择输出目录");
        }

        ui.separator();

        ui.heading("转换任务");

        let status_text = match app.batch_converter.status {
            BatchConverterStatus::Idle => "等待扫描...".to_string(),
            BatchConverterStatus::Ready => format!(
                "已扫描 {} 个任务, 等待开始。",
                app.batch_converter.tasks.len()
            ),
            BatchConverterStatus::Converting => "正在转换...".to_string(),
            BatchConverterStatus::Completed => {
                format!("已完成所有 {} 个任务。", app.batch_converter.tasks.len())
            }
            BatchConverterStatus::Failed(ref err) => format!("失败: {}", err),
        };
        ui.label(status_text);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |scroll_ui| {
                egui::Grid::new("batch_tasks_grid")
                    .num_columns(3)
                    .striped(true)
                    .show(scroll_ui, |grid_ui| {
                        grid_ui.strong("主文件");
                        grid_ui.strong("状态");
                        grid_ui.strong("详情");
                        grid_ui.end_row();

                        for task in &app.batch_converter.tasks {
                            if let Some(main_file) =
                                app.batch_converter.file_lookup.get(&task.main_lyric_id)
                            {
                                grid_ui.label(&main_file.filename);
                            } else {
                                grid_ui.label("未知文件");
                            }

                            match &task.status {
                                lyrics_helper_core::BatchEntryStatus::Pending => {
                                    grid_ui.label("等待中");
                                }
                                lyrics_helper_core::BatchEntryStatus::ReadyToConvert => {
                                    grid_ui.label("准备就绪");
                                }
                                lyrics_helper_core::BatchEntryStatus::Converting => {
                                    grid_ui.horizontal(|h| {
                                        h.add(egui::Spinner::new());
                                        h.label("转换中...");
                                    });
                                }
                                lyrics_helper_core::BatchEntryStatus::Completed { .. } => {
                                    grid_ui.colored_label(egui::Color32::GREEN, "完成");
                                }
                                lyrics_helper_core::BatchEntryStatus::Failed(_) => {
                                    grid_ui.colored_label(egui::Color32::RED, "失败");
                                }
                                lyrics_helper_core::BatchEntryStatus::SkippedNoMatch => {
                                    grid_ui.label("已跳过");
                                }
                            };

                            if let lyrics_helper_core::BatchEntryStatus::Failed(err_msg) =
                                &task.status
                            {
                                grid_ui.label(err_msg);
                            } else if let lyrics_helper_core::BatchEntryStatus::Completed {
                                output_path,
                                ..
                            } = &task.status
                            {
                                grid_ui.label(output_path.to_string_lossy());
                            } else {
                                grid_ui.label("");
                            }
                            grid_ui.end_row();
                        }
                    });
            });

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |bottom_ui| {
            bottom_ui.add_space(10.0);
            bottom_ui.horizontal(|h_ui| {
                let can_start_conversion =
                    app.batch_converter.status == BatchConverterStatus::Ready;
                if h_ui
                    .add_enabled(can_start_conversion, egui::Button::new("开始转换"))
                    .clicked()
                {
                    app.send_action(UserAction::BatchConverter(
                        BatchConverterAction::StartConversion,
                    ));
                }
                if h_ui.button("重置").clicked() {
                    app.send_action(UserAction::BatchConverter(BatchConverterAction::Reset));
                }
            });
        });
    });
}
