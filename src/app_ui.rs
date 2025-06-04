use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::amll_connector::{
    AMLLConnectorConfig, ConnectorCommand, WebsocketStatus, amll_connector_manager,
};
use crate::amll_lyrics_fetcher::AmllSearchField;
use crate::app_definition::UniLyricApp;

use crate::types::{
    AmllIndexDownloadState, AmllTtmlDownloadState, AutoSearchSource, AutoSearchStatus,
    CanonicalMetadataKey, DisplayLrcLine, EditableMetadataEntry, KrcDownloadState, LrcContentType,
    LyricFormat, NeteaseDownloadState, ProcessedLyricsSourceData, QqMusicDownloadState,
    SourceConfigTuple,
};

use eframe::egui::{
    self, Align, Button, Color32, ComboBox, Layout, ScrollArea, Spinner, TextEdit, Window,
};
use egui::TextWrapMode;
use log::LevelFilter;
use rand::Rng;
use std::fmt::Write;

const TITLE_ALIGNMENT_OFFSET: f32 = 6.0;
const BUTTON_STRIP_SPACING: f32 = 4.0;

// 为 UniLyricApp 实现UI绘制相关的方法
impl UniLyricApp {
    /// 绘制应用顶部的工具栏。
    /// 工具栏包含文件菜单、源格式和目标格式选择下拉框，以及其他控制按钮。
    pub fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        // 使用 egui::menu::bar 创建一个菜单栏容器
        egui::menu::bar(ui, |ui_bar| {
            // --- 文件菜单 ---
            ui_bar.menu_button("文件", |file_menu| {
                // "打开歌词文件..." 按钮
                // add_enabled 控制按钮是否可用 (当没有转换正在进行时可用)
                if file_menu
                    .add_enabled(
                        !self.conversion_in_progress,
                        egui::Button::new("打开歌词文件..."),
                    )
                    .clicked()
                {
                    crate::io::handle_open_file(self);
                }
                file_menu.separator(); // 添加分割线

                // 判断主歌词是否已加载，用于启用/禁用加载LRC翻译/罗马音的按钮
                // 主歌词已加载的条件：
                // 1. parsed_ttml_paragraphs (内部TTML表示) 非空且包含段落
                // 2. 或者 input_text (原始输入文本框) 非空
                // 3. 或者 direct_netease_main_lrc_content (从网易云直接获取的LRC主歌词) 非空
                let main_lyrics_loaded = (self.parsed_ttml_paragraphs.is_some()
                    && self
                        .parsed_ttml_paragraphs
                        .as_ref()
                        .is_some_and(|p| !p.is_empty()))
                    || !self.input_text.is_empty()
                    || self.direct_netease_main_lrc_content.is_some();
                let lrc_load_enabled = main_lyrics_loaded && !self.conversion_in_progress;
                let disabled_lrc_hover_text = "请先加载主歌词文件或内容"; // 按钮禁用时的提示文本

                // "加载翻译 (LRC)..." 按钮
                let translation_button = egui::Button::new("加载翻译 (LRC)...");
                let mut translation_button_response =
                    file_menu.add_enabled(lrc_load_enabled, translation_button);
                if !lrc_load_enabled {
                    // 如果禁用，添加悬停提示
                    translation_button_response =
                        translation_button_response.on_disabled_hover_text(disabled_lrc_hover_text);
                }
                if translation_button_response.clicked() {
                    crate::io::handle_open_lrc_file(self, LrcContentType::Translation); // 加载翻译LRC
                }

                // "加载罗马音 (LRC)..." 按钮
                let romanization_button = egui::Button::new("加载罗马音 (LRC)...");
                let mut romanization_button_response =
                    file_menu.add_enabled(lrc_load_enabled, romanization_button);
                if !lrc_load_enabled {
                    romanization_button_response = romanization_button_response
                        .on_disabled_hover_text(disabled_lrc_hover_text);
                }
                if romanization_button_response.clicked() {
                    crate::io::handle_open_lrc_file(self, LrcContentType::Romanization); // 加载罗马音LRC
                }
                file_menu.separator();

                // "下载歌词..." 子菜单
                let download_enabled = !self.conversion_in_progress; // 下载功能在无转换进行时可用
                file_menu.menu_button("下载歌词...", |download_menu| {
                    if download_menu
                        .add_enabled(download_enabled, egui::Button::new("从QQ音乐获取..."))
                        .clicked()
                    {
                        self.qqmusic_query.clear(); // 清空之前的查询词
                        self.show_qqmusic_download_window = true; // 显示QQ音乐下载窗口
                    }
                    if download_menu
                        .add_enabled(download_enabled, egui::Button::new("从酷狗音乐获取..."))
                        .clicked()
                    {
                        self.kugou_query.clear();
                        self.show_kugou_download_window = true; // 显示酷狗音乐下载窗口
                    }
                    if download_menu
                        .add_enabled(download_enabled, egui::Button::new("从网易云音乐获取..."))
                        .clicked()
                    {
                        self.netease_query.clear();
                        self.show_netease_download_window = true; // 显示网易云音乐下载窗口
                    }
                    if download_menu
                        .add_enabled(
                            download_enabled,
                            Button::new("从 AMLL TTML Database 获取..."),
                        )
                        .clicked()
                    {
                        self.amll_search_query.clear();
                        self.show_amll_download_window = true;
                    }
                });

                file_menu.separator();
                // "保存输出为..." 按钮
                // 当输出文本非空且无转换进行时可用
                if file_menu
                    .add_enabled(
                        !self.output_text.is_empty() && !self.conversion_in_progress,
                        egui::Button::new("保存输出为..."),
                    )
                    .clicked()
                {
                    crate::io::handle_save_file(self); // 调用处理文件保存的函数
                }
            });

            // --- 源格式选择 ---
            ui_bar.add_space(16.0); // 添加一些间距
            ui_bar.label("源格式:"); // 标签
            let mut source_format_changed_this_frame = false; // 标记源格式本帧是否改变
            let mut temp_source_format = self.source_format; // 临时变量存储当前选择，以便检测变化

            // 使用 ComboBox (下拉选择框)
            egui::ComboBox::from_id_salt("source_format_toolbar") // 为ComboBox提供唯一ID
                .selected_text(self.source_format.to_string()) // 显示当前选中的格式名称
                .show_ui(ui_bar, |ui_combo| {
                    // 构建下拉列表内容
                    for fmt_option in &self.available_formats {
                        // 遍历所有可用格式
                        let display_text = fmt_option.to_string();
                        // 所有在 available_formats 中的格式都可以被选择为源格式
                        let is_selectable_source = true;

                        let response = ui_combo
                            .add_enabled_ui(is_selectable_source, |ui_selectable| {
                                // 创建可选条目
                                ui_selectable.selectable_value(
                                    &mut temp_source_format,
                                    *fmt_option,
                                    display_text,
                                )
                            })
                            .inner; // 获取内部响应

                        if !is_selectable_source {
                            // response = response.on_disabled_hover_text("此格式不能作为主转换源"); // 如果将来需要禁用某些源
                        }
                        if response.clicked() && is_selectable_source {
                            ui_combo.close_menu(); // 点击后关闭下拉菜单
                        }
                    }
                });

            // 如果选择的源格式发生变化
            if temp_source_format != self.source_format {
                self.source_format = temp_source_format; // 更新应用状态中的源格式
                source_format_changed_this_frame = true; // 标记已改变
            }

            // --- 目标格式选择 ---
            ui_bar.add_space(8.0);
            ui_bar.label("目标格式:");
            let mut target_format_changed_this_frame = false;
            let mut temp_target_format = self.target_format;

            // 当源格式为LRC时，限制可选的目标格式 (这是一个重要的业务逻辑)
            let source_is_lrc_for_target_restriction = self.source_format == LyricFormat::Lrc;

            // 如果源是LRC，且当前目标不是LQE, SPL, LRC之一，则自动切换到LRC (或LQE)
            if source_is_lrc_for_target_restriction
                && !matches!(
                    self.target_format,
                    LyricFormat::Lqe | LyricFormat::Spl | LyricFormat::Lrc | LyricFormat::Ttml
                )
            {
                self.target_format = LyricFormat::Lrc; // 默认切换到LRC自身
                temp_target_format = self.target_format;
            }

            // 判断源格式是否为逐行歌词 (LRC, LYL)，或者虽然是TTML/JSON/SPL但其内容是逐行歌词
            let restrict_target_to_line_based =
                Self::source_format_is_line_timed(self.source_format)
                    || (matches!(
                        self.source_format,
                        LyricFormat::Ttml | LyricFormat::Json | LyricFormat::Spl
                    ) && self.source_is_line_timed);
            // 定义哪些格式是严格需要逐字时间信息的 (不能从纯逐行格式转换而来)
            let truly_word_based_formats_requiring_syllables = [
                LyricFormat::Ass,
                LyricFormat::Qrc,
                LyricFormat::Yrc,
                LyricFormat::Lys,
                LyricFormat::Krc,
            ];

            egui::ComboBox::from_id_salt("target_format_toolbar")
                .selected_text(self.target_format.to_string())
                .show_ui(ui_bar, |ui_combo| {
                    for fmt_option in &self.available_formats {
                        let mut enabled = true; // 默认可选
                        let mut hover_text_for_disabled: Option<String> = None; // 禁用时的提示

                        // 规则1: 如果源是LRC，目标只能是 LQE, SPL, LRC
                        if source_is_lrc_for_target_restriction {
                            if !matches!(
                                *fmt_option,
                                LyricFormat::Lqe
                                    | LyricFormat::Spl
                                    | LyricFormat::Lrc
                                    | LyricFormat::Ttml
                            ) {
                                enabled = false;
                                hover_text_for_disabled =
                                    Some("LRC源格式只能输出为LQE, SPL, TTML 或 LRC".to_string());
                            }
                        }
                        // 规则2: 如果源是逐行歌词，目标不能是严格的逐字歌词
                        else if restrict_target_to_line_based
                            && truly_word_based_formats_requiring_syllables.contains(fmt_option)
                        {
                            enabled = false;
                            hover_text_for_disabled = Some(format!(
                                "{:?} 为逐行格式，无法转换为逐字格式 {:?}",
                                self.source_format.to_string(), // 使用 to_string() 获取显示名称
                                fmt_option.to_string()
                            ));
                        }

                        let display_text = fmt_option.to_string();
                        let mut response = ui_combo
                            .add_enabled_ui(enabled, |ui_inner| {
                                ui_inner.selectable_value(
                                    &mut temp_target_format,
                                    *fmt_option,
                                    display_text,
                                )
                            })
                            .inner;
                        if !enabled {
                            // 如果禁用，添加提示
                            if let Some(text_to_show_on_disabled_hover) = hover_text_for_disabled {
                                response =
                                    response.on_disabled_hover_text(text_to_show_on_disabled_hover);
                            }
                        }
                        if response.clicked() && enabled {
                            ui_combo.close_menu();
                        }
                    }
                });

            // 如果选择的目标格式发生变化
            if temp_target_format != self.target_format {
                self.target_format = temp_target_format;
                target_format_changed_this_frame = true;
            }

            // --- 格式更改后的处理逻辑 ---
            if source_format_changed_this_frame || target_format_changed_this_frame {
                {
                    let mut app_settings_guard = self.app_settings.lock().unwrap();
                    app_settings_guard.last_source_format = self.source_format;
                    app_settings_guard.last_target_format = self.target_format;
                    if let Err(e) = app_settings_guard.save() {
                        log::error!("[UniLyricApp] 自动保存源/目标格式到设置失败: {}", e);
                    } else {
                        log::trace!(
                            "[UniLyricApp] 已自动保存源格式 ({:?}) 和目标格式 ({:?}) 到设置。",
                            self.source_format,
                            self.target_format
                        );
                    }
                }

                // 再次检查并自动切换目标格式的逻辑 (作为保险)
                if (Self::source_format_is_line_timed(self.source_format)
                    || (matches!(
                        self.source_format,
                        LyricFormat::Ttml | LyricFormat::Json | LyricFormat::Spl
                    ) && self.source_is_line_timed))
                    && truly_word_based_formats_requiring_syllables.contains(&self.target_format)
                    && self.source_format != LyricFormat::Lrc
                {
                    log::info!(
                        "[Unilyric] 源格式为逐行（非LRC），但目标格式为逐字，已自动切换为LRC"
                    );
                    self.target_format = LyricFormat::Lrc;
                }

                if !self.input_text.trim().is_empty() {
                    log::trace!(
                        "[UniLyric Toolbar] 格式更改 (源: {:?}, 目标: {:?})，输入非空，调用 handle_convert。",
                        self.source_format,
                        self.target_format
                    );
                    self.handle_convert();
                } else {
                    log::trace!(
                        "[UniLyric Toolbar] 格式更改 (源: {:?}, 目标: {:?})，输入为空，清理并尝试生成空输出。",
                        self.source_format,
                        self.target_format
                    );
                    self.clear_derived_data();
                    self.output_text.clear();
                    if self.target_format == LyricFormat::Lrc
                        && self.metadata_store.lock().unwrap().is_empty()
                        && self.parsed_ttml_paragraphs.is_none()
                    {
                        // 如果目标是LRC，且没有元数据和歌词内容，输出就是空的
                        // self.output_text 已经被 clear()
                    } else {
                        // 对于其他格式或LRC有元数据的情况，尝试生成
                        self.generate_target_format_output();
                    }
                }
            }

            // --- 工具栏右侧按钮 ---
            ui_bar.with_layout(Layout::right_to_left(Align::Center), |ui_right| {
                ui_right.menu_button("视图", |view_menu| {
                    view_menu.checkbox(&mut self.show_markers_panel, "标记面板");
                    view_menu.checkbox(&mut self.show_translation_lrc_panel, "翻译LRC面板");
                    view_menu.checkbox(&mut self.show_romanization_lrc_panel, "罗马音LRC面板");
                    view_menu.separator();

                    let amll_connector_feature_enabled = self.media_connector_config.lock().unwrap().enabled;

                    view_menu.add_enabled_ui(amll_connector_feature_enabled, |ui_enabled_check| {
                        ui_enabled_check.checkbox(&mut self.show_amll_connector_sidebar, "AMLL Connector侧边栏");
                    }).response.on_disabled_hover_text("请在设置中启用 AMLL Connector 功能");
                    view_menu.separator();
                    view_menu.checkbox(&mut self.show_bottom_log_panel, "日志面板");
                     if self.show_bottom_log_panel && self.new_trigger_log_exists {
                        self.new_trigger_log_exists = false;
                    }
                });
                ui_right.add_space(BUTTON_STRIP_SPACING);
                if ui_right.button("元数据").clicked() { self.show_metadata_panel = true; }
                ui_right.add_space(BUTTON_STRIP_SPACING);
                if ui_right.checkbox(&mut self.wrap_text, "自动换行").changed() { /* UI重绘会自动处理 */ }
                ui_right.add_space(BUTTON_STRIP_SPACING);
                if ui_right.button("设置").clicked() { 
                    self.temp_edit_settings = self.app_settings.lock().unwrap().clone();
                    self.show_settings_window = true;
                }
            });
        });
    }

    /// 绘制应用设置窗口。
    pub fn draw_settings_window(&mut self, ctx: &egui::Context) {
        let mut is_settings_window_open = self.show_settings_window;

        egui::Window::new("应用程序设置")
            .open(&mut is_settings_window_open)
            .resizable(true)
            .default_width(500.0)
            .scroll([false, true])
            .show(ctx, |ui| {
                 egui::Grid::new("log_settings_grid") 
                    .num_columns(2)
                    .spacing([40.0, 4.0])
                    .striped(true)
                    .show(ui, |grid_ui| {
                        grid_ui.heading("日志设置");
                        grid_ui.end_row();

                        grid_ui.label("启用文件日志:");
                        grid_ui.checkbox(&mut self.temp_edit_settings.log_settings.enable_file_log, "");
                        grid_ui.end_row();

                        grid_ui.label("文件日志级别:");
                        ComboBox::from_id_salt("file_log_level_combo_settings")
                            .selected_text(format!("{:?}", self.temp_edit_settings.log_settings.file_log_level))
                            .show_ui(grid_ui, |ui_combo| {
                                ui_combo.selectable_value(&mut self.temp_edit_settings.log_settings.file_log_level, LevelFilter::Off, "Off");
                                ui_combo.selectable_value(&mut self.temp_edit_settings.log_settings.file_log_level, LevelFilter::Error, "Error");
                                ui_combo.selectable_value(&mut self.temp_edit_settings.log_settings.file_log_level, LevelFilter::Warn, "Warn");
                                ui_combo.selectable_value(&mut self.temp_edit_settings.log_settings.file_log_level, LevelFilter::Info, "Info");
                                ui_combo.selectable_value(&mut self.temp_edit_settings.log_settings.file_log_level, LevelFilter::Debug, "Debug");
                                ui_combo.selectable_value(&mut self.temp_edit_settings.log_settings.file_log_level, LevelFilter::Trace, "Trace");
                            });
                        grid_ui.end_row();

                        grid_ui.label("控制台日志级别:");
                        ComboBox::from_id_salt("console_log_level_combo_settings")
                            .selected_text(format!("{:?}", self.temp_edit_settings.log_settings.console_log_level))
                            .show_ui(grid_ui, |ui_combo| {
                                ui_combo.selectable_value(&mut self.temp_edit_settings.log_settings.console_log_level, LevelFilter::Off, "Off");
                                ui_combo.selectable_value(&mut self.temp_edit_settings.log_settings.console_log_level, LevelFilter::Error, "Error");
                                ui_combo.selectable_value(&mut self.temp_edit_settings.log_settings.console_log_level, LevelFilter::Warn, "Warn");
                                ui_combo.selectable_value(&mut self.temp_edit_settings.log_settings.console_log_level, LevelFilter::Info, "Info");
                                ui_combo.selectable_value(&mut self.temp_edit_settings.log_settings.console_log_level, LevelFilter::Debug, "Debug");
                                ui_combo.selectable_value(&mut self.temp_edit_settings.log_settings.console_log_level, LevelFilter::Trace, "Trace");
                            });
                        grid_ui.end_row();
                    });
                ui.add_space(10.0);

                egui::Grid::new("amll_connector_settings_grid")
                    .num_columns(2)
                    .spacing([40.0, 4.0])
                    .striped(true)
                    .show(ui, |grid_ui| {
                        grid_ui.heading("AMLL Connector 设置");
                        grid_ui.end_row();

                        grid_ui.label("启用 AMLL Connector 功能:");
                        grid_ui.checkbox(&mut self.temp_edit_settings.amll_connector_enabled, "")
                        .on_hover_text("转发 SMTC 信息到 AMLL Player，让 AMLL Player 也支持其他音乐软件");
                        grid_ui.end_row();

                        grid_ui.label("WebSocket URL:");
                        grid_ui.add(
                            TextEdit::singleline(&mut self.temp_edit_settings.amll_connector_websocket_url)
                                .hint_text("ws://localhost:11444")
                                .desired_width(f32::INFINITY)
                        ).on_hover_text("需点击“保存并应用”");
                        grid_ui.end_row();

                        grid_ui.label("将音频数据发送到 AMLL Player");
                        grid_ui.checkbox(&mut self.temp_edit_settings.send_audio_data_to_player, "");
                        grid_ui.end_row();


                        grid_ui.heading("SMTC 偏移");
                        grid_ui.end_row();

                        grid_ui.label("时间轴偏移量 (毫秒):");
                        grid_ui.add(
                            egui::DragValue::new(&mut self.temp_edit_settings.smtc_time_offset_ms)
                                .speed(10.0)
                                .suffix(" ms"),
                        );
                        grid_ui.end_row();
                    });

                ui.add_space(10.0);
                ui.strong("自动歌词搜索顺序:");

                let current_order = &mut self.temp_edit_settings.auto_search_source_order;
                let num_sources = current_order.len();

                for i in 0..num_sources {
                    ui.horizontal(|row_ui| {
                        row_ui.label(format!("{}. {}", i + 1, current_order[i].display_name()));

                        row_ui.with_layout(Layout::right_to_left(Align::Center), |btn_ui| {
                            // 向下按钮
                            if btn_ui.add_enabled(i < num_sources - 1, Button::new("🔽")).clicked() {
                                current_order.swap(i, i + 1);
                            }
                            // 向上按钮
                            if btn_ui.add_enabled(i > 0, Button::new("🔼")).clicked() {
                                current_order.swap(i, i - 1);
                            }
                        });
                    });
                    if i < num_sources - 1 {
                        ui.separator();
                    }
                }
                ui.add_space(10.0);

                ui.checkbox(&mut self.temp_edit_settings.always_search_all_sources, "始终自动搜索所有源");

                ui.add_space(10.0);

                ui.separator();
                ui.add_space(10.0);
                ui.strong("自动删除元数据行设置");
                ui.checkbox(&mut self.temp_edit_settings.enable_online_lyric_stripping, "基于关键词的移除");

                // 关键词移除的详细配置 (只有当总开关启用时才显示)
                ui.add_enabled_ui(self.temp_edit_settings.enable_online_lyric_stripping, |enabled_ui| {
                    enabled_ui.collapsing("关键词移除规则设置", |rule_ui| { // 使用可折叠区域
                        rule_ui.add_space(5.0);
                        rule_ui.label("要移除的开头关键词（冒号已自动添加）：");

                        let mut keywords_multiline_edit = self.temp_edit_settings.stripping_keywords.join("\n");
                        egui::ScrollArea::vertical().id_salt("stripping_keywords_scroll_area").max_height(80.0).show(rule_ui, |scroll_ui| {
                            if scroll_ui.add(
                                TextEdit::multiline(&mut keywords_multiline_edit)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("例如：\n作曲\n作词\n编曲")
                            ).changed() {
                                self.temp_edit_settings.stripping_keywords = keywords_multiline_edit
                                    .lines()
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                            }
                        });
                        rule_ui.checkbox(&mut self.temp_edit_settings.stripping_keyword_case_sensitive, "区分大小写");

                    });
                });
                ui.add_space(5.0);

                ui.checkbox(&mut self.temp_edit_settings.enable_ttml_regex_stripping, "基于正则表达式的移除")
                    .on_hover_text("如果某一行的内容匹配任何一个正则表达式，该行将被移除。");

                ui.add_enabled_ui(self.temp_edit_settings.enable_ttml_regex_stripping, |enabled_regex_ui| {
                     enabled_regex_ui.collapsing("正则表达式移除规则设置", |regex_rule_ui| { // 使用可折叠区域
                        regex_rule_ui.add_space(5.0);
                        regex_rule_ui.label("要移除的行匹配的正则表达式（每行一个）：");
                        let mut regexes_multiline_edit = self.temp_edit_settings.ttml_stripping_regexes.join("\n");
                        egui::ScrollArea::vertical().id_salt("stripping_regexes_scroll_area").max_height(80.0).show(regex_rule_ui, |scroll_ui| {
                            if scroll_ui.add(
                                TextEdit::multiline(&mut regexes_multiline_edit)
                                    .desired_width(f32::INFINITY)
                            ).changed() {
                                self.temp_edit_settings.ttml_stripping_regexes = regexes_multiline_edit
                                    .lines()
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                            }
                        });
                    });
                });

                ui.separator();
                ui.add_space(10.0);

                ui.horizontal(|bottom_buttons_ui| {
                    if bottom_buttons_ui.button("保存并应用").on_hover_text("保存设置到文件。日志和搜索顺序设置将在下次启动或下次自动搜索时生效").clicked() {
                        let old_send_audio_data_setting = self.app_settings.lock().unwrap().send_audio_data_to_player;
                        let new_send_audio_data_setting = self.temp_edit_settings.send_audio_data_to_player;

                        if self.temp_edit_settings.save().is_ok() {
                        let new_settings_clone = self.temp_edit_settings.clone();
                        let mut app_settings_guard = self.app_settings.lock().unwrap();
                        *app_settings_guard = new_settings_clone;
                        self.smtc_time_offset_ms = app_settings_guard.smtc_time_offset_ms;

                        let new_mc_config_from_settings = AMLLConnectorConfig {
                            enabled: app_settings_guard.amll_connector_enabled,
                            websocket_url: app_settings_guard.amll_connector_websocket_url.clone(),
                        };
                        let connector_enabled_runtime = new_mc_config_from_settings.enabled;
                        drop(app_settings_guard);

                        let mut current_mc_config_guard = self.media_connector_config.lock().unwrap();
                        let old_mc_config = current_mc_config_guard.clone();
                        *current_mc_config_guard = new_mc_config_from_settings.clone();
                        drop(current_mc_config_guard);

                        log::debug!("[Unilyric UI] 设置已保存。新 AMLL Connector配置: {:?}", new_mc_config_from_settings);

                        if new_mc_config_from_settings.enabled {
                            amll_connector_manager::ensure_running(self);
                            if let Some(tx) = &self.media_connector_command_tx {
                                if old_mc_config != new_mc_config_from_settings {
                                    log::debug!("[Unilyric UI] 发送 UpdateConfig 命令给AMLL Connector worker。");
                                    if tx.send(crate::amll_connector::ConnectorCommand::UpdateConfig(new_mc_config_from_settings.clone())).is_err() {
                                        log::error!("[Unilyric UI] 发送 UpdateConfig 命令给AMLL Connector worker 失败。");
                                    }
                                }
                            }
                        } else {
                            amll_connector_manager::ensure_running(self); // 确保如果禁用了，worker会停止
                        }

                        if connector_enabled_runtime && old_send_audio_data_setting != new_send_audio_data_setting {
                            self.audio_visualization_enabled_by_ui = new_send_audio_data_setting;
                            if let Some(tx) = &self.media_connector_command_tx {
                                let command = if new_send_audio_data_setting {
                                    log::info!("[Unilyric UI] 设置更改：启动音频数据转发。");
                                    ConnectorCommand::StartAudioVisualization
                                } else {
                                    log::info!("[Unilyric UI] 设置更改：停止音频数据转发。");
                                    ConnectorCommand::StopAudioVisualization
                                };
                                if tx.send(command).is_err() {
                                    log::error!("[Unilyric UI] 应用设置更改时，发送音频可视化控制命令失败。");
                                }
                            }
                        }

                        self.show_settings_window = false;
                        } else {
                            log::error!("保存应用设置失败。");
                            self.show_settings_window = false;
                        }
                    }
                    if bottom_buttons_ui.button("取消").clicked() {
                        self.show_settings_window = false;
                    }
                });
            });

        if !is_settings_window_open {
            self.show_settings_window = false;
        }
    }
    /// 绘制元数据编辑器窗口的内容。
    ///
    /// # Arguments
    /// * `ui` - `egui::Ui` 的可变引用，用于绘制UI元素。
    /// * `_open` - (当前未使用) 通常用于 `egui::Window` 的打开状态，但这里窗口的打开状态由 `self.show_metadata_panel` 控制。
    pub fn draw_metadata_editor_window_contents(&mut self, ui: &mut egui::Ui, _open: &mut bool) {
        let mut metadata_changed_this_frame = false; // 标记元数据在本帧是否被修改
        let mut entry_to_delete_idx: Option<usize> = None; // 存储要删除的条目的索引

        // 使用可滚动的区域来显示元数据列表
        egui::ScrollArea::vertical().show(ui, |scroll_ui| {
            if self.editable_metadata.is_empty() {
                // 如果没有元数据可编辑
                scroll_ui.label(
                    egui::RichText::new("无元数据可编辑。\n可从文件加载，或手动添加。").weak(),
                );
            }

            // 克隆 editable_metadata 以允许在迭代时修改 (例如删除条目)
            let mut temp_editable_metadata = self.editable_metadata.clone();

            // 遍历可编辑的元数据条目
            for (index, entry) in temp_editable_metadata.iter_mut().enumerate() {
                let item_id = entry.id; // 每个条目有唯一的 egui::Id，用于区分UI控件状态

                scroll_ui.horizontal(|row_ui| {
                    // 每条元数据占一行
                    // "固定" 复选框，用于标记该元数据是否在加载新文件时保留
                    if row_ui.checkbox(&mut entry.is_pinned, "").changed() {
                        metadata_changed_this_frame = true;
                    }
                    row_ui
                        .label("固定")
                        .on_hover_text("勾选后，此条元数据在加载新歌词时将尝试保留其值");

                    row_ui.add_space(5.0);
                    row_ui.label("键:");
                    // 元数据键的文本编辑框
                    if row_ui
                        .add_sized(
                            [row_ui.available_width() * 0.3, 0.0], // 占据可用宽度的30%
                            egui::TextEdit::singleline(&mut entry.key)
                                .id_salt(item_id.with("key_edit")) // 控件ID
                                .hint_text("元数据键"), // 输入提示
                        )
                        .changed()
                    {
                        metadata_changed_this_frame = true;
                        entry.is_from_file = false;
                    } // 如果改变，标记已修改且不再是来自文件

                    row_ui.add_space(5.0);
                    row_ui.label("值:");
                    // 元数据值的文本编辑框
                    if row_ui
                        .add(
                            egui::TextEdit::singleline(&mut entry.value)
                                .id_salt(item_id.with("value_edit"))
                                .hint_text("元数据值"),
                        )
                        .changed()
                    {
                        metadata_changed_this_frame = true;
                        entry.is_from_file = false;
                    }

                    // 删除按钮
                    if row_ui.button("🗑").on_hover_text("删除此条元数据").clicked() {
                        entry_to_delete_idx = Some(index); // 标记要删除的条目的索引 (基于 temp_editable_metadata)
                        metadata_changed_this_frame = true;
                    }
                });
                scroll_ui.separator(); // 每行后的分割线
            }
            // 将可能修改过的元数据列表写回 self.editable_metadata
            self.editable_metadata = temp_editable_metadata;

            // "添加新元数据" 按钮
            if scroll_ui.button("添加新元数据").clicked() {
                // 为新条目生成一个相对唯一的ID
                let new_entry_id_num =
                    self.editable_metadata.len() as u32 + rand::rng().random::<u32>();
                let new_id = egui::Id::new(format!("new_editable_meta_entry_{}", new_entry_id_num));
                self.editable_metadata.push(EditableMetadataEntry {
                    key: format!("新键_{}", new_entry_id_num % 100), // 默认键名
                    value: "".to_string(),                           // 默认空值
                    is_pinned: false,                                // 默认不固定
                    is_from_file: false,                             // 新添加的不是来自文件
                    id: new_id,                                      // UI ID
                });
                metadata_changed_this_frame = true;
            }
        }); // ScrollArea 结束

        // 如果有条目被标记为删除，则从 self.editable_metadata 中移除
        if let Some(idx_del) = entry_to_delete_idx {
            if idx_del < self.editable_metadata.len() {
                // 再次确认索引有效
                self.editable_metadata.remove(idx_del);
            }
        }

        // 如果元数据在本帧内发生任何变化（编辑、添加、删除、更改固定状态）
        if metadata_changed_this_frame {
            // 调用函数将UI中的可编辑列表同步回内部的 MetadataStore，并触发一次转换以更新输出
            self.sync_store_from_editable_list_and_trigger_conversion();
        }

        // 窗口底部的关闭按钮
    }

    /// 绘制底部日志面板。
    pub fn draw_log_panel(&mut self, ctx: &egui::Context) {
        // 使用 TopBottomPanel 创建一个可调整大小的底部面板
        egui::TopBottomPanel::bottom("log_panel_id")
            .resizable(true) // 允许用户拖动调整面板高度
            .default_height(150.0) // 默认高度
            .min_height(60.0) // 最小高度
            .max_height(ctx.available_rect().height() * 0.7) // 最大高度不超过屏幕的70%
            .show_animated(ctx, self.show_bottom_log_panel, |ui| {
                // 面板的显示/隐藏受 self.show_bottom_log_panel 控制
                // 面板头部：标题和按钮
                ui.vertical_centered_justified(|ui_header| {
                    // 使标题和按钮在水平方向上两端对齐
                    ui_header.horizontal(|h_ui| {
                        h_ui.label(egui::RichText::new("日志").strong()); // 标题
                        h_ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |btn_ui| {
                                if btn_ui.button("关闭").clicked() {
                                    // 关闭按钮
                                    self.show_bottom_log_panel = false;
                                    self.new_trigger_log_exists = false; // 关闭时清除新日志提示
                                }
                                if btn_ui.button("清空").clicked() {
                                    // 清空按钮
                                    self.log_display_buffer.clear(); // 清空日志缓冲区
                                }
                            },
                        );
                    });
                });
                ui.separator(); // 头部和内容区分割线

                // 使用可滚动区域显示日志条目
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false]) // 不自动缩小，保持填充可用空间
                    .stick_to_bottom(true) // 自动滚动到底部以显示最新日志
                    .show(ui, |scroll_ui| {
                        if self.log_display_buffer.is_empty() {
                            // 如果没有日志
                            scroll_ui.add_space(5.0);
                            scroll_ui.label(egui::RichText::new("暂无日志。").weak().italics());
                            scroll_ui.add_space(5.0);
                        } else {
                            // 遍历并显示日志缓冲区中的每条日志
                            for entry in &self.log_display_buffer {
                                scroll_ui.horizontal_wrapped(|line_ui| {
                                    // 每条日志一行，自动换行
                                    // 时间戳
                                    line_ui.label(
                                        egui::RichText::new(
                                            entry.timestamp.format("[%H:%M:%S.%3f]").to_string(),
                                        )
                                        .monospace()
                                        .color(egui::Color32::DARK_GRAY), // 等宽字体，深灰色
                                    );
                                    line_ui.add_space(4.0);
                                    // 日志级别 (带颜色)
                                    line_ui.label(
                                        egui::RichText::new(format!("[{}]", entry.level.as_str()))
                                            .monospace()
                                            .color(entry.level.color())
                                            .strong(), // 等宽，特定颜色，加粗
                                    );
                                    line_ui.add_space(4.0);
                                    // 日志消息
                                    line_ui.label(
                                        egui::RichText::new(&entry.message).monospace().weak(),
                                    ); // 等宽，弱化显示
                                });
                            }
                        }
                        // 确保滚动区域至少有其声明的大小，即使内容不足
                        scroll_ui.allocate_space(scroll_ui.available_size_before_wrap());
                    });
            });
    }

    /// 绘制主歌词输入面板的内容。
    pub fn draw_input_panel_contents(&mut self, ui: &mut egui::Ui) {
        ui.add_space(TITLE_ALIGNMENT_OFFSET);
        ui.horizontal(|title_ui| {
            title_ui.heading("输入歌词");
            title_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |btn_ui| {
                if btn_ui
                    .add_enabled(
                        !self.input_text.is_empty() || !self.output_text.is_empty(),
                        egui::Button::new("清空"),
                    )
                    .clicked()
                {
                    self.clear_all_data();
                }
                btn_ui.add_space(BUTTON_STRIP_SPACING);
                if btn_ui
                    .add_enabled(!self.input_text.is_empty(), egui::Button::new("复制"))
                    .clicked()
                {
                    btn_ui.ctx().copy_text(self.input_text.clone());
                }
                btn_ui.add_space(BUTTON_STRIP_SPACING);
                if btn_ui.button("粘贴").clicked() {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        if let Ok(text) = clipboard.get_text() {
                            self.input_text = text;
                            self.handle_convert();
                        } else {
                            log::error!("无法从剪贴板获取文本");
                        }
                    } else {
                        log::error!("无法访问剪贴板");
                    }
                }
            });
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .id_salt("input_scroll_vertical_only")
            .auto_shrink([false, false])
            .show(ui, |s_ui| {
                let text_edit_widget = egui::TextEdit::multiline(&mut self.input_text)
                    .hint_text("在此处粘贴或拖放主歌词文件")
                    .font(egui::TextStyle::Monospace)
                    .interactive(!self.conversion_in_progress)
                    .desired_width(f32::INFINITY);

                let response = s_ui.add(text_edit_widget);
                if response.changed() && !self.conversion_in_progress {
                    self.handle_convert();
                }
            });
    }

    /// 绘制翻译LRC面板的内容。
    pub fn draw_translation_lrc_panel_contents(&mut self, ui: &mut egui::Ui) {
        let mut clear_action_triggered = false;
        let mut import_action_triggered = false;
        let mut text_edited_this_frame = false;

        let title = "翻译 (LRC)";
        let lrc_is_currently_considered_active = self.loaded_translation_lrc.is_some()
            || !self.display_translation_lrc_output.trim().is_empty();

        ui.add_space(TITLE_ALIGNMENT_OFFSET);
        ui.label(egui::RichText::new(title).heading());
        ui.separator();

        ui.horizontal(|button_strip_ui| {
            let main_lyrics_exist_for_merge = self
                .parsed_ttml_paragraphs
                .as_ref()
                .is_some_and(|p| !p.is_empty());
            let import_enabled = main_lyrics_exist_for_merge && !self.conversion_in_progress;
            let import_button_widget = egui::Button::new("导入");
            let mut import_button_response =
                button_strip_ui.add_enabled(import_enabled, import_button_widget);
            if !import_enabled {
                import_button_response =
                    import_button_response.on_disabled_hover_text("请先加载主歌词文件");
            }
            if import_button_response.clicked() {
                import_action_triggered = true;
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
                        clear_action_triggered = true;
                    }
                    right_aligned_buttons_ui.add_space(BUTTON_STRIP_SPACING);
                    if right_aligned_buttons_ui
                        .add_enabled(
                            !self.display_translation_lrc_output.is_empty(),
                            egui::Button::new("复制"),
                        )
                        .clicked()
                    {
                        right_aligned_buttons_ui
                            .ctx()
                            .copy_text(self.display_translation_lrc_output.clone());
                    }
                },
            );
        });

        // TextEdit 总是使用垂直滚动条
        egui::ScrollArea::vertical()
            .id_salt("translation_lrc_scroll_vertical")
            .auto_shrink([false, false])
            .show(ui, |s_ui_content| {
                let text_edit_widget =
                    egui::TextEdit::multiline(&mut self.display_translation_lrc_output)
                        .hint_text("在此处粘贴翻译LRC内容")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(10);

                let response = s_ui_content.add(text_edit_widget);
                if response.changed() {
                    text_edited_this_frame = true;
                }
                s_ui_content.allocate_space(s_ui_content.available_size_before_wrap());
            });

        if import_action_triggered {
            crate::io::handle_open_lrc_file(self, LrcContentType::Translation);
            let mut reconstructed_display_text = String::new();
            if let Some(display_lines) = &self.loaded_translation_lrc {
                for line_entry in display_lines {
                    match line_entry {
                        DisplayLrcLine::Parsed(lrc_line) => {
                            let _ = writeln!(
                                reconstructed_display_text,
                                "{}{}",
                                crate::utils::format_lrc_time_ms(lrc_line.timestamp_ms),
                                lrc_line.text
                            );
                        }
                        DisplayLrcLine::Raw { original_text } => {
                            let _ = writeln!(reconstructed_display_text, "{}", original_text);
                        }
                    }
                }
            }
            self.display_translation_lrc_output = reconstructed_display_text
                .trim_end_matches('\n')
                .to_string();
            if !self.display_translation_lrc_output.is_empty() {
                self.display_translation_lrc_output.push('\n');
            }

            if self
                .parsed_ttml_paragraphs
                .as_ref()
                .is_some_and(|p| !p.is_empty())
            {
                self.handle_convert();
            }
        }

        if clear_action_triggered {
            self.loaded_translation_lrc = None;
            self.display_translation_lrc_output.clear();
            log::info!("已清除翻译 LRC (通过UI按钮)。");
            if self
                .parsed_ttml_paragraphs
                .as_ref()
                .is_some_and(|p| !p.is_empty())
            {
                self.handle_convert();
            }
        }

        if text_edited_this_frame {
            match crate::lrc_parser::parse_lrc_text_to_lines(&self.display_translation_lrc_output) {
                Ok((parsed_display_lines, _bilingual_translations, _parsed_meta)) => {
                    // 接收三个值
                    self.loaded_translation_lrc = Some(parsed_display_lines.clone());
                    let mut reconstructed_text = String::new();
                    for line_entry in parsed_display_lines {
                        match line_entry {
                            DisplayLrcLine::Parsed(lrc_line) => {
                                let _ = writeln!(
                                    reconstructed_text,
                                    "{}{}",
                                    crate::utils::format_lrc_time_ms(lrc_line.timestamp_ms),
                                    lrc_line.text
                                );
                            }
                            DisplayLrcLine::Raw { original_text } => {
                                let _ = writeln!(reconstructed_text, "{}", original_text);
                            }
                        }
                    }
                    self.display_translation_lrc_output =
                        reconstructed_text.trim_end_matches('\n').to_string();
                    if !self.display_translation_lrc_output.is_empty() {
                        self.display_translation_lrc_output.push('\n');
                    }
                    log::debug!(
                        "[UI Edit] 翻译LRC文本已编辑. Parsed into: {:?}. Triggering convert.",
                        self.loaded_translation_lrc
                    );
                }
                Err(e) => {
                    self.loaded_translation_lrc = None;
                    log::warn!(
                        "[UI Edit] 编辑的翻译LRC文本解析器返回错误: {}. 关联的LRC数据已清除.",
                        e
                    );
                    self.toasts.add(egui_toast::Toast {
                        text: format!("翻译LRC内容解析错误: {}", e).into(),
                        kind: egui_toast::ToastKind::Error,
                        options: egui_toast::ToastOptions::default()
                            .duration_in_seconds(4.0)
                            .show_icon(true),
                        style: Default::default(),
                    });
                }
            }
            if self
                .parsed_ttml_paragraphs
                .as_ref()
                .is_some_and(|p| !p.is_empty())
            {
                log::debug!("[UI Edit] 翻译LRC编辑后，触发 handle_convert");
                self.handle_convert();
            }
        }
    }

    /// 绘制罗马音LRC面板的内容。
    pub fn draw_romanization_lrc_panel_contents(&mut self, ui: &mut egui::Ui) {
        let mut clear_action_triggered = false;
        let mut import_action_triggered = false;
        let mut text_edited_this_frame = false;

        let title = "罗马音 (LRC)";
        let lrc_is_currently_considered_active = self.loaded_romanization_lrc.is_some()
            || !self.display_romanization_lrc_output.trim().is_empty();

        ui.add_space(TITLE_ALIGNMENT_OFFSET);
        ui.label(egui::RichText::new(title).heading());
        ui.separator();

        ui.horizontal(|button_strip_ui| {
            let main_lyrics_exist_for_merge = self
                .parsed_ttml_paragraphs
                .as_ref()
                .is_some_and(|p| !p.is_empty());
            let import_enabled = main_lyrics_exist_for_merge && !self.conversion_in_progress;
            let import_button_widget = egui::Button::new("导入");
            let mut import_button_response =
                button_strip_ui.add_enabled(import_enabled, import_button_widget);
            if !import_enabled {
                import_button_response =
                    import_button_response.on_disabled_hover_text("请先加载主歌词文件");
            }
            if import_button_response.clicked() {
                import_action_triggered = true;
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
                        clear_action_triggered = true;
                    }
                    right_aligned_buttons_ui.add_space(BUTTON_STRIP_SPACING);
                    if right_aligned_buttons_ui
                        .add_enabled(
                            !self.display_romanization_lrc_output.is_empty(),
                            egui::Button::new("复制"),
                        )
                        .clicked()
                    {
                        right_aligned_buttons_ui
                            .ctx()
                            .copy_text(self.display_romanization_lrc_output.clone());
                    }
                },
            );
        });

        // TextEdit 总是使用垂直滚动条
        egui::ScrollArea::vertical()
            .id_salt("romanization_lrc_scroll_vertical_v4") // 更新 ID
            .auto_shrink([false, false])
            .show(ui, |s_ui_content| {
                let text_edit_widget =
                    egui::TextEdit::multiline(&mut self.display_romanization_lrc_output)
                        .hint_text("在此处粘贴罗马音LRC内容")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(10);

                let response = s_ui_content.add(text_edit_widget);
                if response.changed() {
                    text_edited_this_frame = true;
                }
                s_ui_content.allocate_space(s_ui_content.available_size_before_wrap());
            });

        if import_action_triggered {
            crate::io::handle_open_lrc_file(self, LrcContentType::Romanization);
            let mut reconstructed_display_text = String::new();
            if let Some(display_lines) = &self.loaded_romanization_lrc {
                for line_entry in display_lines {
                    match line_entry {
                        DisplayLrcLine::Parsed(lrc_line) => {
                            let _ = writeln!(
                                reconstructed_display_text,
                                "{}{}",
                                crate::utils::format_lrc_time_ms(lrc_line.timestamp_ms),
                                lrc_line.text
                            );
                        }
                        DisplayLrcLine::Raw { original_text } => {
                            let _ = writeln!(reconstructed_display_text, "{}", original_text);
                        }
                    }
                }
            }
            self.display_romanization_lrc_output = reconstructed_display_text
                .trim_end_matches('\n')
                .to_string();
            if !self.display_romanization_lrc_output.is_empty() {
                self.display_romanization_lrc_output.push('\n');
            }

            if self
                .parsed_ttml_paragraphs
                .as_ref()
                .is_some_and(|p| !p.is_empty())
            {
                self.handle_convert();
            }
        }

        if clear_action_triggered {
            self.loaded_romanization_lrc = None;
            self.display_romanization_lrc_output.clear();
            log::info!("已清除罗马音 LRC (通过UI按钮)。");
            if self
                .parsed_ttml_paragraphs
                .as_ref()
                .is_some_and(|p| !p.is_empty())
            {
                self.handle_convert();
            }
        }

        if text_edited_this_frame {
            match crate::lrc_parser::parse_lrc_text_to_lines(&self.display_romanization_lrc_output)
            {
                Ok((parsed_display_lines, _bilingual_translations, _parsed_meta)) => {
                    self.loaded_romanization_lrc = Some(parsed_display_lines.clone());

                    let mut reconstructed_text = String::new();
                    for line_entry in parsed_display_lines {
                        match line_entry {
                            DisplayLrcLine::Parsed(lrc_line) => {
                                let _ = writeln!(
                                    reconstructed_text,
                                    "{}{}",
                                    crate::utils::format_lrc_time_ms(lrc_line.timestamp_ms),
                                    lrc_line.text
                                );
                            }
                            DisplayLrcLine::Raw { original_text } => {
                                let _ = writeln!(reconstructed_text, "{}", original_text);
                            }
                        }
                    }
                    self.display_romanization_lrc_output =
                        reconstructed_text.trim_end_matches('\n').to_string();
                    if !self.display_romanization_lrc_output.is_empty() {
                        self.display_romanization_lrc_output.push('\n');
                    }

                    log::debug!(
                        "[UI Edit] 罗马音LRC文本已编辑. Parsed into: {:?}. Triggering convert.",
                        self.loaded_romanization_lrc
                    );
                }
                Err(e) => {
                    self.loaded_romanization_lrc = None;
                    log::warn!(
                        "[UI Edit] 编辑的罗马音LRC文本解析器返回错误: {}. 关联的LRC数据已清除.",
                        e
                    );
                    self.toasts.add(egui_toast::Toast {
                        text: format!("罗马音LRC内容解析错误: {}", e).into(),
                        kind: egui_toast::ToastKind::Error,
                        options: egui_toast::ToastOptions::default()
                            .duration_in_seconds(4.0)
                            .show_icon(true),
                        style: Default::default(),
                    });
                }
            }
            if self
                .parsed_ttml_paragraphs
                .as_ref()
                .is_some_and(|p| !p.is_empty())
            {
                log::debug!("[UI Edit] 罗马音LRC编辑后，触发 handle_convert");
                self.handle_convert();
            }
        }
    }

    /// 绘制标记信息面板的内容 (通常用于显示 ASS 文件中的 Comment 行标记)。
    pub fn draw_markers_panel_contents(&mut self, ui: &mut egui::Ui, wrap_text_arg: bool) {
        ui.add_space(TITLE_ALIGNMENT_OFFSET);
        ui.heading("标记");
        ui.separator();
        let markers_text_content = self
            .current_markers
            .iter()
            .map(|(ln, txt)| format!("ASS 行 {}: {}", ln, txt))
            .collect::<Vec<_>>()
            .join("\n");

        let scroll_area = if wrap_text_arg {
            // 使用传入的参数
            egui::ScrollArea::vertical().id_salt("markers_panel_scroll_vertical_v4")
        } else {
            egui::ScrollArea::both()
                .id_salt("markers_panel_scroll_both_v4")
                .auto_shrink([false, false])
        };

        scroll_area.auto_shrink([false, false]).show(ui, |s_ui| {
            if markers_text_content.is_empty() {
                s_ui.centered_and_justified(|center_ui| {
                    center_ui.label(egui::RichText::new("无标记信息").weak().italics());
                });
            } else {
                let mut label_widget = egui::Label::new(
                    egui::RichText::new(markers_text_content.as_str())
                        .monospace()
                        .size(13.0),
                )
                .selectable(true);

                if wrap_text_arg {
                    // 使用传入的参数
                    label_widget = label_widget.wrap();
                } else {
                    label_widget = label_widget.extend();
                }
                s_ui.add(label_widget);
            }
            s_ui.allocate_space(s_ui.available_size_before_wrap());
        });
    }

    /// 绘制 QQ 音乐歌词下载的模态窗口。
    pub fn draw_qqmusic_download_modal_window(&mut self, ctx: &egui::Context) {
        if self.show_qqmusic_download_window {
            // 如果需要显示此窗口
            let mut is_open = self.show_qqmusic_download_window; // 控制窗口打开状态

            egui::Window::new("从QQ音乐下载歌词")
                .open(&mut is_open) // 绑定状态，允许通过标题栏关闭
                .collapsible(false) // 不允许折叠
                .resizable(false) // 不允许调整大小
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO) // 窗口居中显示
                .show(ctx, |ui| {
                    // 窗口内容
                    ui.set_min_width(300.0); // 最小宽度

                    ui.vertical_centered_justified(|ui_vc| {
                        // 垂直居中对齐内部元素
                        ui_vc.add_space(5.0);
                        ui_vc.label("输入歌曲名称：");
                        ui_vc.add_space(5.0);
                        // 搜索查询文本框
                        let response = ui_vc.add_sized(
                            [ui_vc.available_width() * 0.9, 0.0], // 占据90%可用宽度
                            egui::TextEdit::singleline(&mut self.qqmusic_query)
                                .hint_text("例如：歌曲名 - 歌手"),
                        );
                        // 如果在文本框失去焦点且按下了回车键，并且查询非空，则触发下载
                        if response.lost_focus()
                            && response.ctx.input(|i| i.key_pressed(egui::Key::Enter))
                            && !self.qqmusic_query.trim().is_empty()
                        {
                            let download_status_locked = self.qq_download_state.lock().unwrap();
                            if !matches!(*download_status_locked, QqMusicDownloadState::Downloading)
                            {
                                // 避免重复触发
                                drop(download_status_locked); // 释放锁
                                self.trigger_qqmusic_download(); // 调用下载处理函数
                            }
                        }
                        ui_vc.add_space(10.0);
                    });

                    // 根据下载状态显示加载动画或按钮
                    let download_status_locked = self.qq_download_state.lock().unwrap();
                    let is_downloading =
                        matches!(&*download_status_locked, QqMusicDownloadState::Downloading);

                    if is_downloading {
                        // 如果正在下载
                        drop(download_status_locked); // 释放锁以允许UI更新
                        ui.horizontal(|ui_s| {
                            ui_s.spinner(); // 显示加载动画
                            ui_s.label("正在下载QRC歌词...");
                        });
                    } else {
                        // 如果未在下载
                        drop(download_status_locked);
                        let mut trigger_download_button = false;
                        ui.vertical_centered(|ui_centered_button| {
                            // 按钮居中
                            if ui_centered_button.button("搜索并载入").clicked() {
                                trigger_download_button = true;
                            }
                        });
                        if trigger_download_button {
                            // 如果点击了按钮
                            if !self.qqmusic_query.trim().is_empty() {
                                self.trigger_qqmusic_download();
                            } else {
                                log::warn!("[Unilyric] QQ音乐搜索：查询为空。");
                            }
                        }
                    }
                    ui.add_space(5.0);
                });
            // 如果窗口被关闭 (例如通过标题栏的关闭按钮)
            if !is_open {
                self.show_qqmusic_download_window = false;
            }
        }
    }

    /// 绘制酷狗音乐KRC歌词下载的模态窗口。
    /// (逻辑与 draw_qqmusic_download_modal_window 非常相似)
    pub fn draw_kugou_download_modal_window(&mut self, ctx: &egui::Context) {
        if self.show_kugou_download_window {
            let mut is_open = self.show_kugou_download_window;

            egui::Window::new("从酷狗音乐下载歌词")
                .open(&mut is_open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_min_width(300.0);

                    ui.vertical_centered_justified(|ui_vc| {
                        ui_vc.add_space(5.0);
                        ui_vc.label("输入歌曲名称：");
                        ui_vc.add_space(5.0);
                        let response = ui_vc.add_sized(
                            [ui_vc.available_width() * 0.9, 0.0],
                            egui::TextEdit::singleline(&mut self.kugou_query)
                                .hint_text("例如：歌曲名 - 歌手"),
                        );
                        let enter_pressed = ui_vc.ctx().input(|i| i.key_pressed(egui::Key::Enter));
                        if response.lost_focus()
                            && enter_pressed
                            && !self.kugou_query.trim().is_empty()
                        {
                            let download_status_locked = self.kugou_download_state.lock().unwrap();
                            if !matches!(*download_status_locked, KrcDownloadState::Downloading) {
                                drop(download_status_locked);
                                self.trigger_kugou_download();
                            }
                        }
                        ui_vc.add_space(10.0);
                    });

                    let download_status_locked = self.kugou_download_state.lock().unwrap();
                    let is_downloading =
                        matches!(&*download_status_locked, KrcDownloadState::Downloading);

                    if is_downloading {
                        drop(download_status_locked);
                        ui.horizontal(|ui_s| {
                            ui_s.spinner();
                            ui_s.label("正在下载KRC歌词...");
                        });
                    } else {
                        drop(download_status_locked);
                        let mut trigger_download_now = false;
                        ui.vertical_centered(|ui_centered_button| {
                            if ui_centered_button.button("搜索并载入").clicked() {
                                trigger_download_now = true;
                            }
                        });
                        if trigger_download_now {
                            if !self.kugou_query.trim().is_empty() {
                                self.trigger_kugou_download();
                            } else {
                                log::warn!("[Unilyric] 酷狗音乐搜索：查询为空。");
                            }
                        }
                    }
                    ui.add_space(5.0);
                });

            if !is_open {
                self.show_kugou_download_window = false;
                // 如果窗口关闭时不是因为成功或错误，则重置状态为 Idle
                let mut download_status_locked = self.kugou_download_state.lock().unwrap();
                if !matches!(
                    *download_status_locked,
                    KrcDownloadState::Downloading
                        | KrcDownloadState::Success(_)
                        | KrcDownloadState::Error(_)
                ) {
                    *download_status_locked = KrcDownloadState::Idle;
                }
            }
        }
    }

    /// 绘制网易云音乐歌词下载的模态窗口。
    /// (逻辑与前两个下载窗口类似，但状态枚举不同)
    pub fn draw_netease_download_modal_window(&mut self, ctx: &egui::Context) {
        if self.show_netease_download_window {
            let mut is_open = self.show_netease_download_window;

            egui::Window::new("从网易云音乐下载歌词")
                .open(&mut is_open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.set_min_width(320.0);

                    let enter_pressed_on_this_frame =
                        ctx.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.vertical_centered_justified(|ui_vc| {
                        ui_vc.add_space(5.0);
                        ui_vc.label("输入歌曲名称：");
                        ui_vc.add_space(5.0);
                        let response = ui_vc.add_sized(
                            [ui_vc.available_width() * 0.9, 0.0],
                            egui::TextEdit::singleline(&mut self.netease_query)
                                .hint_text("例如：歌曲名 - 歌手"),
                        );

                        if response.lost_focus()
                            && enter_pressed_on_this_frame
                            && !self.netease_query.trim().is_empty()
                        {
                            let download_status_locked =
                                self.netease_download_state.lock().unwrap();
                            // 避免在正在初始化客户端或下载时重复触发
                            if !matches!(
                                *download_status_locked,
                                NeteaseDownloadState::Downloading
                                    | NeteaseDownloadState::InitializingClient
                            ) {
                                drop(download_status_locked);
                                self.trigger_netease_download();
                            }
                        }
                        ui_vc.add_space(10.0);
                    });

                    // 获取当前下载状态用于显示
                    let download_status_locked = self.netease_download_state.lock().unwrap();
                    let current_status_display = match &*download_status_locked {
                        NeteaseDownloadState::Idle => "空闲".to_string(),
                        NeteaseDownloadState::InitializingClient => "正在准备下载...".to_string(),
                        NeteaseDownloadState::Downloading => "正在下载歌词...".to_string(),
                        NeteaseDownloadState::Success(_) => "下载成功".to_string(), // 成功后窗口通常会关闭，但保留状态显示
                        NeteaseDownloadState::Error(e) => format!("错误: {:.50}", e), // 显示错误信息的前50个字符
                    };

                    let is_busy = matches!(
                        &*download_status_locked,
                        NeteaseDownloadState::Downloading
                            | NeteaseDownloadState::InitializingClient
                    );

                    if is_busy {
                        // 如果正在初始化或下载
                        drop(download_status_locked);
                        ui.horizontal(|ui_s| {
                            ui_s.spinner();
                            ui_s.label(current_status_display); // 显示当前状态文本
                        });
                    } else {
                        // 如果空闲、成功或错误
                        drop(download_status_locked);
                        let mut trigger_download_now = false;
                        ui.vertical_centered(|ui_centered_button| {
                            // 按钮在查询非空时才可用
                            if ui_centered_button
                                .add_enabled(
                                    !self.netease_query.trim().is_empty(),
                                    egui::Button::new("下载并载入"),
                                )
                                .clicked()
                            {
                                trigger_download_now = true;
                            }
                        });
                        if trigger_download_now {
                            self.trigger_netease_download();
                        }
                    }
                    ui.add_space(5.0);
                });

            if !is_open {
                self.show_netease_download_window = false;
                // 如果窗口关闭时不是因为成功，且不是正在进行中，则重置状态为 Idle
                let mut download_status_locked = self.netease_download_state.lock().unwrap();
                if !matches!(*download_status_locked, NeteaseDownloadState::Success(_))
                    && !matches!(
                        *download_status_locked,
                        NeteaseDownloadState::Downloading
                            | NeteaseDownloadState::InitializingClient
                    )
                {
                    *download_status_locked = NeteaseDownloadState::Idle;
                }
            }
        }
    }

    pub fn draw_amll_download_modal_window(&mut self, ctx: &egui::Context) {
        if !self.show_amll_download_window {
            return;
        }

        let mut is_window_open = self.show_amll_download_window;
        let _ = Window::new("从 AMLL TTML Database 获取歌词")
            .open(&mut is_window_open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .resizable(true)
            .collapsible(false)
            .min_height(600.0)
            .min_width(400.0)
            .show(ctx, |ui| {
                let index_state_clone = self.amll_index_download_state.lock().unwrap().clone();

                // 按钮文本和操作逻辑根据状态变化
                let mut button_text = "加载/刷新索引".to_string();
                let mut hover_text = "检查更新并加载本地或远程索引".to_string();
                let mut force_refresh_on_click = false;
                let mut check_update_on_click = false;

                match &index_state_clone {
                    AmllIndexDownloadState::Idle => {
                        ui.weak("状态: 未初始化/未知");
                        button_text = "检查更新并加载索引".to_string();
                        check_update_on_click = true; // Idle 时优先检查更新
                    }
                    AmllIndexDownloadState::CheckingForUpdate => {
                        ui.horizontal(|h_ui| {
                            h_ui.add(Spinner::new());
                            h_ui.label("正在检查更新...");
                        });
                        // 检查时禁用按钮或不显示
                    }
                    AmllIndexDownloadState::UpdateAvailable(remote_head) => {
                        ui.colored_label(
                            Color32::GOLD,
                            format!(
                                "有可用更新 (新 HEAD: {})",
                                remote_head.chars().take(7).collect::<String>()
                            ),
                        );
                        button_text = "下载更新".to_string();
                        hover_text = format!(
                            "下载版本 {}",
                            remote_head.chars().take(7).collect::<String>()
                        );
                    }
                    AmllIndexDownloadState::Downloading(Some(downloading_head)) => {
                        ui.horizontal(|h_ui| {
                            h_ui.add(Spinner::new());
                            h_ui.label(format!(
                                "正在下载索引 (HEAD: {})...",
                                downloading_head.chars().take(7).collect::<String>()
                            ));
                        });
                    }
                    AmllIndexDownloadState::Downloading(None) => {
                        ui.horizontal(|h_ui| {
                            h_ui.add(Spinner::new());
                            h_ui.label("正在下载索引 (获取最新 HEAD)...");
                        });
                    }
                    AmllIndexDownloadState::Success(loaded_head) => {
                        let index_len = self.amll_index.lock().unwrap().len();
                        ui.colored_label(Color32::GREEN, format!("索引已加载 ({} 条)", index_len));
                        ui.label(format!(
                            "当前版本 HEAD: {}",
                            loaded_head.chars().take(7).collect::<String>()
                        ));

                        // 提供两个按钮：检查更新 和 强制刷新
                        if ui.button("检查是否有新版本").clicked() {
                            check_update_on_click = true;
                        }
                        ui.add_space(5.0);
                        button_text = "强制刷新本地索引".to_string();
                        hover_text = "忽略本地缓存和版本检查，直接下载最新索引".to_string();
                        force_refresh_on_click = true;
                    }
                    AmllIndexDownloadState::Error(err_msg) => {
                        ui.colored_label(
                            ui.style().visuals.error_fg_color,
                            format!("操作失败: {}", err_msg),
                        );
                        button_text = "重试加载/检查更新".to_string();
                        check_update_on_click = true; // 出错后重试也应该先检查
                    }
                }

                // 统一处理按钮点击
                if !matches!(
                    index_state_clone,
                    AmllIndexDownloadState::CheckingForUpdate
                        | AmllIndexDownloadState::Downloading(_)
                ) && !button_text.is_empty()
                {
                    // 只有在有按钮文本时才显示
                    if ui.button(&button_text).on_hover_text(&hover_text).clicked() {
                        if check_update_on_click {
                            self.check_for_amll_index_update();
                        } else {
                            // 包括 force_refresh_on_click 和 UpdateAvailable 的情况
                            self.trigger_amll_index_download(force_refresh_on_click);
                        }
                    }
                }

                ui.add_space(10.0);

                // 搜索部分 (只有在索引成功加载后才应完全可用)
                let search_enabled =
                    matches!(index_state_clone, AmllIndexDownloadState::Success(_));
                ui.add_enabled_ui(search_enabled, |enabled_ui| {
                    enabled_ui.strong("搜索歌词:");
                    enabled_ui.separator();
                    enabled_ui.horizontal(|h_ui| {
                        h_ui.label("搜索字段:");
                        ComboBox::from_id_salt("amll_search_field_combo_modal") // 确保 ID 唯一
                            .selected_text(self.amll_selected_search_field.display_name())
                            .show_ui(h_ui, |combo_ui| {
                                for field_option in AmllSearchField::all_fields() {
                                    combo_ui.selectable_value(
                                        &mut self.amll_selected_search_field,
                                        field_option.clone(),
                                        field_option.display_name(),
                                    );
                                }
                            });
                    });

                    enabled_ui.horizontal(|h_ui| {
                        h_ui.label("搜索词:");
                        let query_input = TextEdit::singleline(&mut self.amll_search_query)
                            .hint_text("输入搜索内容...")
                            .desired_width(f32::INFINITY);
                        let query_response = h_ui.add(query_input);

                        if query_response.lost_focus()
                            && h_ui.input(|i: &egui::InputState| i.key_pressed(egui::Key::Enter))
                            || query_response.changed() && search_enabled
                        // 确保仅在启用时响应变化
                        {
                            if !self.amll_search_query.trim().is_empty() {
                                self.amll_search_results.lock().unwrap().clear();
                                *self.amll_ttml_download_state.lock().unwrap() =
                                    AmllTtmlDownloadState::Idle;
                                self.trigger_amll_lyrics_search_and_download(None);
                            } else {
                                self.amll_search_results.lock().unwrap().clear();
                            }
                        }
                    });
                    if enabled_ui.button("搜索").clicked()
                        && !self.amll_search_query.trim().is_empty()
                    {
                        self.amll_search_results.lock().unwrap().clear();
                        *self.amll_ttml_download_state.lock().unwrap() =
                            AmllTtmlDownloadState::Idle;
                        self.trigger_amll_lyrics_search_and_download(None);
                    }
                });

                ui.add_space(10.0);

                let ttml_dl_state = self.amll_ttml_download_state.lock().unwrap().clone();
                match ttml_dl_state {
                    AmllTtmlDownloadState::SearchingIndex => {
                        ui.horizontal(|h_ui| {
                            h_ui.add(Spinner::new());
                            h_ui.label("正在搜索索引...");
                        });
                    }
                    AmllTtmlDownloadState::DownloadingTtml => {
                        ui.horizontal(|h_ui| {
                            h_ui.add(Spinner::new());
                            h_ui.label("正在下载 TTML 文件...");
                        });
                    }
                    AmllTtmlDownloadState::Error(ref err_msg) => {
                        ui.colored_label(
                            ui.style().visuals.error_fg_color,
                            format!("TTML操作失败: {}", err_msg),
                        );
                    }
                    _ => {}
                }
                ui.strong("搜索结果:");
                let search_results_count = self.amll_search_results.lock().unwrap().len();
                if !self.amll_search_query.trim().is_empty()
                    && ttml_dl_state == AmllTtmlDownloadState::Idle
                    && search_enabled
                {
                    ui.label(format!("找到 {} 条结果。", search_results_count));
                }
                ui.separator();
                ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .max_height(200.0)
                    .show(ui, |scroll_ui| {
                        if !search_enabled {
                            scroll_ui.weak("请先成功加载索引以启用搜索功能。");
                            return;
                        }
                        let search_results_vec = {
                            let search_results_lock = self.amll_search_results.lock().unwrap();
                            search_results_lock.clone()
                        };
                        if search_results_vec.is_empty() {
                            if !self.amll_search_query.trim().is_empty()
                                && ttml_dl_state == AmllTtmlDownloadState::Idle
                            {
                            } else if self.amll_search_query.trim().is_empty() {
                                scroll_ui.label("请输入关键字以搜索");
                            }
                        } else {
                            for (idx, entry) in search_results_vec.iter().enumerate() {
                                let mut display_song_name = "未知歌曲".to_string();
                                let mut display_artists = "未知艺术家".to_string();
                                for (key, values) in &entry.metadata {
                                    if key == AmllSearchField::MusicName.to_key_string()
                                        && !values.is_empty()
                                    {
                                        display_song_name = values.join("/");
                                    } else if key == AmllSearchField::Artists.to_key_string()
                                        && !values.is_empty()
                                    {
                                        display_artists = values.join("/");
                                    }
                                }
                                let display_text =
                                    format!("{} - {}", display_song_name, display_artists);

                                if scroll_ui
                                    .selectable_label(false, display_text)
                                    .on_hover_text(entry.raw_lyric_file.to_string())
                                    .clicked()
                                {
                                    self.trigger_amll_lyrics_search_and_download(Some(
                                        entry.clone(),
                                    ));
                                }
                                if idx < search_results_vec.len() - 1 {
                                    scroll_ui.separator();
                                }
                            }
                        }
                    });
            });

        // 处理窗口关闭逻辑
        if !is_window_open {
            self.show_amll_download_window = false;
            let mut ttml_dl_state_lock = self.amll_ttml_download_state.lock().unwrap();
            if matches!(*ttml_dl_state_lock, AmllTtmlDownloadState::Error(_)) {
                *ttml_dl_state_lock = AmllTtmlDownloadState::Idle;
            }
            // 当窗口关闭时，如果状态是 CheckingForUpdate 或 UpdateAvailable，可能需要重置为 Idle 或上一个稳定状态
            let mut index_dl_state_lock = self.amll_index_download_state.lock().unwrap();
            if matches!(
                *index_dl_state_lock,
                AmllIndexDownloadState::CheckingForUpdate
                    | AmllIndexDownloadState::UpdateAvailable(_)
            ) {
                // 尝试恢复到基于缓存 HEAD 的 Success 状态，如果不可能则 Idle
                if let Some(ref cache_p) = self.amll_index_cache_path {
                    if let Ok(Some(cached_head)) =
                        crate::amll_lyrics_fetcher::amll_fetcher::load_cached_index_head(cache_p)
                    {
                        if !self.amll_index.lock().unwrap().is_empty() {
                            // 确保索引内容也已加载
                            *index_dl_state_lock = AmllIndexDownloadState::Success(cached_head);
                        } else {
                            *index_dl_state_lock = AmllIndexDownloadState::Idle;
                        }
                    } else {
                        *index_dl_state_lock = AmllIndexDownloadState::Idle;
                    }
                } else {
                    *index_dl_state_lock = AmllIndexDownloadState::Idle;
                }
            }
        }
    }

    /// 绘制输出结果面板的内容。
    pub fn draw_output_panel_contents(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|title_ui| {
            title_ui.heading("输出结果");
            title_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |btn_ui| {
                let send_to_player_enabled: bool;
                {
                    let connector_config_guard = self.media_connector_config.lock().unwrap();
                    send_to_player_enabled = connector_config_guard.enabled
                        && !self.output_text.is_empty()
                        && !self.conversion_in_progress;
                }

                let send_button = Button::new("发送到AMLL Player");
                if btn_ui
                    .add_enabled(send_to_player_enabled, send_button)
                    .clicked()
                {
                    if let Some(tx) = &self.media_connector_command_tx {
                        if tx
                            .send(crate::amll_connector::ConnectorCommand::SendLyricTtml(
                                self.output_text.clone(),
                            ))
                            .is_err()
                        {
                            log::error!("[Unilyric UI] 发送 TTML 歌词失败。");
                        } else {
                            log::info!("[Unilyrc UI] 已从输出面板手动发送 TTML。");
                        }
                    }
                }
                btn_ui.add_space(BUTTON_STRIP_SPACING);

                let can_upload_to_db: bool;
                {
                    let store_guard = self.metadata_store.lock().unwrap();
                    let artists_exist_ui = store_guard
                        .get_multiple_values(&CanonicalMetadataKey::Artist)
                        .is_some_and(|v| !v.is_empty() && v.iter().any(|s| !s.trim().is_empty()));
                    let titles_exist_ui = store_guard
                        .get_multiple_values(&CanonicalMetadataKey::Title)
                        .is_some_and(|v| !v.is_empty() && v.iter().any(|s| !s.trim().is_empty()));

                    can_upload_to_db = !self.output_text.is_empty()
                        && self.target_format == LyricFormat::Ttml
                        && artists_exist_ui
                        && titles_exist_ui
                        && !self.ttml_db_upload_in_progress;
                }

                let upload_button_widget = Button::new("上传到 AMLL-DB");
                let upload_button_response = btn_ui
                    .add_enabled(can_upload_to_db, upload_button_widget)
                    .on_hover_text("将当前TTML歌词上传到dpaste并打开amll-ttml-db的Issue");

                if upload_button_response.clicked() {
                    self.trigger_ttml_db_upload();
                }
                btn_ui.add_space(BUTTON_STRIP_SPACING);

                if btn_ui
                    .add_enabled(
                        !self.output_text.is_empty() && !self.conversion_in_progress,
                        Button::new("复制"),
                    )
                    .clicked()
                {
                    btn_ui.ctx().copy_text(self.output_text.clone());
                    self.toasts.add(egui_toast::Toast {
                        text: "输出内容已复制到剪贴板".into(),
                        kind: egui_toast::ToastKind::Success,
                        options: egui_toast::ToastOptions::default().duration_in_seconds(2.0),
                        style: Default::default(),
                    });
                }
            });
        });
        ui.separator();

        if self.ttml_db_upload_in_progress {
            ui.horizontal(|h_ui| {
                h_ui.add(Spinner::new());
                h_ui.label(egui::RichText::new("正在处理请求...").weak());
            });
            ui.add_space(2.0);
        } else if let Some(paste_url) = &self.ttml_db_last_paste_url {
            ui.horizontal(|h_ui| {
                h_ui.label("上次dpaste链接:");
                h_ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
                h_ui.hyperlink_to(paste_url, paste_url.clone())
                    .on_hover_text("点击在浏览器中打开链接");
                if h_ui
                    .button("📋")
                    .on_hover_text("复制上次的dpaste链接")
                    .clicked()
                {
                    h_ui.ctx().copy_text(paste_url.clone());
                    self.toasts.add(egui_toast::Toast {
                        text: "链接已复制!".into(),
                        kind: egui_toast::ToastKind::Success,
                        options: egui_toast::ToastOptions::default().duration_in_seconds(2.0),
                        style: Default::default(),
                    });
                }
            });
            ui.add_space(2.0);
        }

        let scroll_area = if self.wrap_text {
            ScrollArea::vertical().id_salt("output_scroll_vertical_label")
        } else {
            ScrollArea::both()
                .id_salt("output_scroll_both_label_v6")
                .auto_shrink([false, false])
        };

        scroll_area.auto_shrink([false, false]).show(ui, |s_ui| {
            if self.conversion_in_progress {
                s_ui.centered_and_justified(|c_ui| {
                    c_ui.spinner();
                });
            } else {
                let mut label_widget = egui::Label::new(
                    egui::RichText::new(&self.output_text)
                        .monospace()
                        .size(13.0),
                )
                .selectable(true);

                if self.wrap_text {
                    label_widget = label_widget.wrap();
                } else {
                    label_widget = label_widget.extend();
                }
                s_ui.add(label_widget);
            }
        });
    }

    pub fn draw_amll_connector_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(TITLE_ALIGNMENT_OFFSET);
        ui.heading("AMLL Connector");
        ui.separator();

        ui.strong("WebSocket 连接:");

        let current_status = self.media_connector_status.lock().unwrap().clone();
        let websocket_url_display: String;
        {
            let config_guard_display = self.media_connector_config.lock().unwrap();
            websocket_url_display = config_guard_display.websocket_url.clone();
        }

        ui.label(format!("目标 URL: {}", websocket_url_display));

        match current_status {
            WebsocketStatus::断开 => {
                if ui.button("连接到 AMLL Player").clicked() {
                    {
                        let mut config_guard = self.media_connector_config.lock().unwrap();
                        if !config_guard.enabled {
                            log::debug!(
                                "[Unilyric UI] AMLL Connector 功能原为禁用，现设置为启用。"
                            );
                            config_guard.enabled = true;
                        }
                    }
                    amll_connector_manager::ensure_running(self);
                    let current_config_for_command =
                        self.media_connector_config.lock().unwrap().clone();
                    if let Some(tx) = &self.media_connector_command_tx {
                        log::debug!(
                            "[Unilyric UI] 发送 UpdateConfig 命令以触发连接尝试: {:?}",
                            current_config_for_command
                        );
                        if tx
                            .send(ConnectorCommand::UpdateConfig(current_config_for_command))
                            .is_err()
                        {
                            log::error!("[Unilyric UI] 发送启用/连接的 UpdateConfig 命令失败。");
                        }
                    } else {
                        log::error!(
                            "[Unilyric UI] 连接按钮：调用 ensure_running 后，media_connector_command_tx 仍然不可用！"
                        );
                    }
                }
                ui.weak("状态: 未连接");
            }
            WebsocketStatus::连接中 => {
                ui.horizontal(|h_ui| {
                    h_ui.add(Spinner::new());
                    h_ui.label("正在连接...");
                });
            }
            WebsocketStatus::已连接 => {
                if ui.button("断开连接").clicked() {
                    if let Some(tx) = &self.media_connector_command_tx {
                        if tx.send(ConnectorCommand::DisconnectWebsocket).is_err() {
                            log::error!("[Unilyric UI] 发送 DisconnectWebsocket 命令失败。");
                        }
                    } else {
                        log::warn!(
                            "[Unilyric UI] 断开连接按钮：media_connector_command_tx 不可用。"
                        );
                    }
                }
                ui.colored_label(Color32::GREEN, "状态: 已连接");
            }
            WebsocketStatus::错误(err_msg_ref) => {
                if ui.button("重试连接").clicked() {
                    {
                        let mut config_guard = self.media_connector_config.lock().unwrap();
                        if !config_guard.enabled {
                            config_guard.enabled = true;
                        }
                    }
                    amll_connector_manager::ensure_running(self);
                    let current_config_for_command =
                        self.media_connector_config.lock().unwrap().clone();
                    if let Some(tx) = &self.media_connector_command_tx {
                        log::debug!(
                            "[Unilyric UI] 发送 UpdateConfig 命令以触发重试连接: {:?}",
                            current_config_for_command
                        );
                        if tx
                            .send(ConnectorCommand::UpdateConfig(current_config_for_command))
                            .is_err()
                        {
                            log::error!("[Unilyric UI] 错误后重试：发送 UpdateConfig 命令失败。");
                        }
                    } else {
                        log::error!(
                            "[Unilyric UI] 重试连接按钮：调用 ensure_running 后，media_connector_command_tx 仍然不可用！"
                        );
                    }
                }
                ui.colored_label(Color32::RED, "状态: 错误");
                ui.small(err_msg_ref);
            }
        }

        ui.separator();

        // --- SMTC 源选择 UI ---
        ui.strong("SMTC 源应用:");
        {
            let available_sessions_guard = self.available_smtc_sessions.lock().unwrap();
            let mut selected_session_id_guard = self.selected_smtc_session_id.lock().unwrap();

            let mut selected_id_for_combo: Option<String> = selected_session_id_guard.clone();

            let combo_label_text = match selected_id_for_combo.as_ref() {
                Some(id) => available_sessions_guard
                    .iter()
                    .find(|s| &s.session_id == id)
                    .map_or_else(
                        || format!("自动 (选择 '{}' 已失效)", id),
                        |s_info| s_info.display_name.clone(),
                    ),
                None => "自动 (系统默认)".to_string(),
            };

            let combo_changed_smtc =
                egui::ComboBox::from_id_salt("smtc_source_selector_v3_fixed_scoped")
                    .selected_text(combo_label_text)
                    .show_ui(ui, |combo_ui| {
                        let mut changed_in_combo = false;
                        if combo_ui
                            .selectable_label(selected_id_for_combo.is_none(), "自动 (系统默认)")
                            .clicked()
                            && selected_id_for_combo.is_some()
                        {
                            selected_id_for_combo = None;
                            changed_in_combo = true;
                        }
                        for session_info in available_sessions_guard.iter() {
                            if combo_ui
                                .selectable_label(
                                    selected_id_for_combo.as_ref()
                                        == Some(&session_info.session_id),
                                    &session_info.display_name,
                                )
                                .clicked()
                                && selected_id_for_combo.as_ref() != Some(&session_info.session_id)
                            {
                                selected_id_for_combo = Some(session_info.session_id.clone());
                                changed_in_combo = true;
                            }
                        }
                        changed_in_combo
                    })
                    .inner
                    .unwrap_or(false);

            if combo_changed_smtc {
                *selected_session_id_guard = selected_id_for_combo.clone();
                let session_to_send = selected_id_for_combo.unwrap_or_default();

                *self.last_requested_volume_for_session.lock().unwrap() = None;
                *self.current_smtc_volume.lock().unwrap() = None;

                if let Some(tx) = &self.media_connector_command_tx {
                    if tx
                        .send(ConnectorCommand::SelectSmtcSession(session_to_send))
                        .is_err()
                    {
                        log::error!("[Unilyric UI] 发送 SelectSmtcSession 命令失败。");
                    }
                }
            }
        }
        ui.separator();

        // --- SMTC 当前监听信息 ---
        ui.strong("当前监听 (SMTC):");
        match self.current_media_info.try_lock() {
            Ok(media_info_guard) => {
                if let Some(info) = &*media_info_guard {
                    ui.label(format!("歌曲: {}", info.title.as_deref().unwrap_or("未知")));
                    ui.label(format!(
                        "艺术家: {}",
                        info.artist.as_deref().unwrap_or("未知")
                    ));
                    ui.label(format!(
                        "专辑: {}",
                        info.album_title.as_deref().unwrap_or("未知")
                    ));
                    if let Some(playing) = info.is_playing {
                        ui.label(if playing {
                            "状态: 播放中"
                        } else {
                            "状态: 已暂停"
                        });
                    }
                    ui.strong("时间轴偏移:");
                    ui.horizontal(|h_ui| {
                        h_ui.label("偏移量:");
                        let mut current_offset = self.smtc_time_offset_ms;
                        let response = h_ui.add(
                            egui::DragValue::new(&mut current_offset)
                                .speed(10.0)
                                .suffix(" ms"),
                        );
                        if response.changed() {
                            self.smtc_time_offset_ms = current_offset;
                            let mut settings = self.app_settings.lock().unwrap();
                            if settings.smtc_time_offset_ms != self.smtc_time_offset_ms {
                                settings.smtc_time_offset_ms = self.smtc_time_offset_ms;
                                if settings.save().is_err() {
                                    log::error!("[Unilyric UI] 侧边栏偏移量持久化到设置失败。");
                                }
                            }
                        }
                    });
                    if let Some(cover_bytes) = &info.cover_data {
                        if !cover_bytes.is_empty() {
                            let image_id_cow: std::borrow::Cow<'static, str> =
                                info.cover_data_hash.map_or_else(
                                    || {
                                        let mut hasher =
                                            std::collections::hash_map::DefaultHasher::new();
                                        cover_bytes[..std::cmp::min(cover_bytes.len(), 16)]
                                            .hash(&mut hasher);
                                        format!("smtc_cover_data_partial_hash_{}", hasher.finish())
                                            .into()
                                    },
                                    |hash| format!("smtc_cover_hash_{}", hash).into(),
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
                    }
                } else {
                    ui.weak("无SMTC信息 / 未选择特定源");
                }
            }
            Err(_) => {
                ui.weak("SMTC信息读取中...");
            }
        }
        ui.separator();

        ui.strong("本地歌词:");
        let can_save_to_local = !self.output_text.is_empty()
            && self
                .current_media_info
                .try_lock()
                .is_ok_and(|g| g.is_some())
            && self.last_auto_fetch_source_format.is_some();

        let save_button_widget = Button::new("💾 保存输出框歌词到本地");
        let mut response = ui.add_enabled(can_save_to_local, save_button_widget);
        if !can_save_to_local {
            response = response.on_hover_text("需先搜索到歌词才能缓存");
        }
        if response.clicked() {
            self.save_current_lyrics_to_local_cache();
        }
        ui.separator();

        ui.strong("自动歌词搜索状态:");
        let sources_config: Vec<SourceConfigTuple> = vec![
            (
                AutoSearchSource::LocalCache,
                Arc::clone(&self.local_cache_auto_search_status),
                None,
            ),
            (
                AutoSearchSource::QqMusic,
                Arc::clone(&self.qqmusic_auto_search_status), // 克隆 Arc
                Some(Arc::clone(&self.last_qq_search_result)), // 克隆 Arc
            ),
            (
                AutoSearchSource::Kugou,
                Arc::clone(&self.kugou_auto_search_status), // 克隆 Arc
                Some(Arc::clone(&self.last_kugou_search_result)), // 克隆 Arc
            ),
            (
                AutoSearchSource::Netease,
                Arc::clone(&self.netease_auto_search_status), // 克隆 Arc
                Some(Arc::clone(&self.last_netease_search_result)), // 克隆 Arc
            ),
            (
                AutoSearchSource::AmllDb,
                Arc::clone(&self.amll_db_auto_search_status), // 克隆 Arc
                Some(Arc::clone(&self.last_amll_db_search_result)), // 克隆 Arc
            ),
        ];
        let mut action_load_lyrics: Option<(ProcessedLyricsSourceData, AutoSearchSource)> = None;
        for (source_enum, status_arc, opt_result_arc) in sources_config {
            ui.horizontal(|item_ui| {
                item_ui.label(format!("{}:", source_enum.display_name()));
                let status = status_arc.lock().unwrap().clone();
                item_ui.with_layout(Layout::right_to_left(Align::Center), |right_aligned_ui| {
                    let mut show_load_button = false;
                    let mut data_for_load_action_this_iteration: Option<ProcessedLyricsSourceData> =
                        None;
                    if source_enum != AutoSearchSource::LocalCache {
                        if let AutoSearchStatus::Success(_) = status {
                            if let Some(result_arc) = &opt_result_arc {
                                if let Some(ref stored_data) = *result_arc.lock().unwrap() {
                                    show_load_button = true;
                                    data_for_load_action_this_iteration = Some(stored_data.clone());
                                }
                            }
                        }
                    }
                    if show_load_button {
                        if right_aligned_ui
                            .button("载入")
                            .on_hover_text(format!(
                                "使用 {} 找到的歌词",
                                source_enum.display_name()
                            ))
                            .clicked()
                        {
                            if let Some(data) = data_for_load_action_this_iteration {
                                action_load_lyrics = Some((data, source_enum));
                            }
                        }
                        right_aligned_ui.add_space(4.0);
                    }
                    if source_enum != AutoSearchSource::LocalCache
                        && matches!(
                            status,
                            AutoSearchStatus::NotFound | AutoSearchStatus::Error(_)
                        )
                        && right_aligned_ui.button("重搜").clicked()
                    {
                        crate::app_fetch_core::trigger_manual_refetch_for_source(self, source_enum);
                    }
                    let status_display_text = match status {
                        AutoSearchStatus::NotAttempted => "未尝试".to_string(),
                        AutoSearchStatus::Searching => "正在搜索...".to_string(),
                        AutoSearchStatus::Success(_) => "已找到".to_string(),
                        AutoSearchStatus::NotFound => "未找到".to_string(),
                        AutoSearchStatus::Error(_) => "错误".to_string(),
                    };
                    if matches!(status, AutoSearchStatus::Error(_)) {
                        right_aligned_ui.colored_label(
                            right_aligned_ui.visuals().error_fg_color,
                            status_display_text,
                        );
                    } else {
                        right_aligned_ui.label(status_display_text);
                    }
                });
            });
        }
        if let Some((data, source)) = action_load_lyrics {
            self.load_lyrics_from_stored_result(data, source);
        }

        ui.strong("AMLL 歌词库索引:");
        let index_status_clone = self.amll_index_download_state.lock().unwrap().clone();

        let mut show_check_button = false;
        let mut check_button_text = String::new(); // 初始化为空
        let mut check_button_hover = String::new(); // 初始化为空

        let mut show_force_refresh_button = false;
        let force_refresh_button_text = "手动下载索引".to_string();
        let force_refresh_button_hover = "忽略本地缓存和版本检查，直接下载最新索引".to_string();

        let mut show_download_update_button = false;
        let mut download_update_button_text = String::new();
        let mut download_update_button_hover = String::new();

        match &index_status_clone {
            AmllIndexDownloadState::Idle => {
                ui.weak("状态: 未初始化/未知");
                check_button_text = "检查更新并加载索引".to_string();
                check_button_hover = "检查索引是否有新版本，或加载本地缓存".to_string();
                show_check_button = true;
            }
            AmllIndexDownloadState::CheckingForUpdate => {
                ui.horizontal(|h_ui| {
                    h_ui.add(Spinner::new());
                    h_ui.label("检查更新中...");
                });
                // 正在检查时不显示任何操作按钮
            }
            AmllIndexDownloadState::UpdateAvailable(remote_head) => {
                ui.colored_label(
                    Color32::GOLD,
                    format!(
                        "有可用更新 (新 HEAD: {})",
                        remote_head.chars().take(7).collect::<String>()
                    ),
                );
                download_update_button_text = "下载更新".to_string();
                download_update_button_hover = format!(
                    "下载版本 {}",
                    remote_head.chars().take(7).collect::<String>()
                );
                show_download_update_button = true;
                show_force_refresh_button = true;
            }
            AmllIndexDownloadState::Downloading(Some(downloading_head)) => {
                ui.horizontal(|h_ui| {
                    h_ui.add(Spinner::new());
                    h_ui.label(format!(
                        "下载中 ({})...",
                        downloading_head.chars().take(7).collect::<String>()
                    ));
                });
            }
            AmllIndexDownloadState::Downloading(None) => {
                ui.horizontal(|h_ui| {
                    h_ui.add(Spinner::new());
                    h_ui.label("下载中 (最新)...");
                });
            }
            AmllIndexDownloadState::Success(loaded_head) => {
                let index_len = self.amll_index.lock().unwrap().len();
                ui.colored_label(Color32::GREEN, format!("已加载 ({} 条)", index_len));
                ui.label(format!(
                    "当前版本: {}",
                    loaded_head.chars().take(7).collect::<String>()
                ));
                check_button_text = "检查是否有新版本".to_string(); // 成功后，按钮变为检查更新
                check_button_hover = "检查索引是否有新版本".to_string();
                show_check_button = true;
                show_force_refresh_button = true; // 成功加载后，也允许强制刷新
            }
            AmllIndexDownloadState::Error(err_msg) => {
                ui.colored_label(ui.visuals().error_fg_color, "错误");
                ui.small(err_msg);
                check_button_text = "重试".to_string();
                check_button_hover = "再次尝试检查索引更新".to_string();
                show_check_button = true;
                show_force_refresh_button = true;
            }
        }

        // 按钮的绘制逻辑
        if show_check_button
            && !check_button_text.is_empty()
            && ui
                .button(&check_button_text)
                .on_hover_text(&check_button_hover)
                .clicked()
        {
            // "检查更新"、"检查更新并加载索引"、"重试检查更新" 都触发 check_for_amll_index_update
            self.check_for_amll_index_update();
        }

        if show_download_update_button
            && !download_update_button_text.is_empty()
            && ui
                .button(&download_update_button_text)
                .on_hover_text(&download_update_button_hover)
                .clicked()
        {
            // 这个按钮只在 UpdateAvailable 状态下出现，所以总是下载特定更新
            self.trigger_amll_index_download(false);
        }

        if show_force_refresh_button {
            // 确保与上一个按钮有间隔，除非上一个按钮没显示
            if show_check_button || show_download_update_button {
                ui.add_space(BUTTON_STRIP_SPACING); // 假设 BUTTON_STRIP_SPACING 已定义
            }
            if ui
                .button(&force_refresh_button_text)
                .on_hover_text(&force_refresh_button_hover)
                .clicked()
            {
                self.trigger_amll_index_download(true);
            }
        }
    }
}
