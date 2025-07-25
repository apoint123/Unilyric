use crate::amll_connector::{
    AMLLConnectorConfig, ConnectorCommand, WebsocketStatus, amll_connector_manager,
};
use crate::app_definition::UniLyricApp;

use crate::types::{
    AutoSearchSource, AutoSearchStatus, DisplayLrcLine, EditableMetadataEntry, LrcContentType,
};

use eframe::egui::{self, Align, Button, ComboBox, Layout, ScrollArea, Spinner, TextEdit};
use egui::{Color32, TextWrapMode};
use log::LevelFilter;
use lyrics_helper_rs::converter::LyricFormat;
use lyrics_helper_rs::converter::generators::lrc_generator::format_lrc_time_ms;
use lyrics_helper_rs::converter::parsers::lrc_parser;
use lyrics_helper_rs::model::track::FullLyricsResult;
use rand::Rng;
use std::fmt::Write;
use std::hash::{Hash, Hasher};

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
                let main_lyrics_loaded = (self.parsed_lyric_data.is_some()
                    && self.parsed_lyric_data.as_ref().is_some())
                    || !self.input_text.is_empty();
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
                        .add_enabled(download_enabled, egui::Button::new("搜索歌词..."))
                        .clicked()
                    {
                        // 重置搜索状态并打开新的通用搜索窗口
                        self.search_query.clear();
                        self.search_results.clear();
                        self.show_search_window = true;
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

            ui_bar.menu_button("简繁转换", |tools_menu| {
                let conversion_enabled = !self.input_text.is_empty()
                    || self
                        .parsed_lyric_data
                        .as_ref()
                        .is_some_and(|d| !d.lines.is_empty());
                let disabled_hover_text = "请先加载主歌词";

                tools_menu.label(egui::RichText::new("通用简繁转换").strong());
                if tools_menu
                    .add_enabled(conversion_enabled, egui::Button::new("简体 → 繁体 (通用)"))
                    .on_disabled_hover_text(disabled_hover_text)
                    .clicked()
                {
                    self.handle_chinese_conversion("s2t.json");
                }
                if tools_menu
                    .add_enabled(conversion_enabled, egui::Button::new("繁体 → 简体 (通用)"))
                    .on_disabled_hover_text(disabled_hover_text)
                    .clicked()
                {
                    self.handle_chinese_conversion("t2s.json");
                }
                tools_menu.separator();

                tools_menu.label(egui::RichText::new("地区性转换 (含用语)").strong());
                tools_menu.menu_button("简体 →", |sub_menu| {
                    if sub_menu
                        .add_enabled(conversion_enabled, egui::Button::new("台湾正体"))
                        .on_disabled_hover_text(disabled_hover_text)
                        .clicked()
                    {
                        self.handle_chinese_conversion("s2twp.json");
                    }
                    if sub_menu
                        .add_enabled(conversion_enabled, egui::Button::new("香港繁体"))
                        .on_disabled_hover_text(disabled_hover_text)
                        .clicked()
                    {
                        self.handle_chinese_conversion("s2hk.json");
                    }
                });
                tools_menu.menu_button("繁体 →", |sub_menu| {
                    if sub_menu
                        .add_enabled(conversion_enabled, egui::Button::new("大陆简体 (含用语)"))
                        .on_disabled_hover_text(disabled_hover_text)
                        .clicked()
                    {
                        self.handle_chinese_conversion("tw2sp.json");
                    }
                    if sub_menu
                        .add_enabled(conversion_enabled, egui::Button::new("大陆简体 (仅文字)"))
                        .on_disabled_hover_text(disabled_hover_text)
                        .clicked()
                    {
                        self.handle_chinese_conversion("tw2s.json");
                    }
                });
                tools_menu.separator();

                tools_menu.label(egui::RichText::new("仅文字转换").strong());
                tools_menu.menu_button("繁体互转", |sub_menu| {
                    if sub_menu
                        .add_enabled(conversion_enabled, egui::Button::new("台湾繁体 → 香港繁体"))
                        .on_disabled_hover_text(disabled_hover_text)
                        .clicked()
                    {
                        self.handle_chinese_conversion("tw2t.json");
                    }
                    if sub_menu
                        .add_enabled(conversion_enabled, egui::Button::new("香港繁体 → 台湾繁体"))
                        .on_disabled_hover_text(disabled_hover_text)
                        .clicked()
                    {
                        self.handle_chinese_conversion("hk2t.json");
                    }
                });
                tools_menu.menu_button("其他转换", |sub_menu| {
                    if sub_menu
                        .add_enabled(
                            conversion_enabled,
                            egui::Button::new("简体 → 台湾繁体 (仅文字)"),
                        )
                        .on_disabled_hover_text(disabled_hover_text)
                        .clicked()
                    {
                        self.handle_chinese_conversion("s2tw.json");
                    }
                    if sub_menu
                        .add_enabled(
                            conversion_enabled,
                            egui::Button::new("繁体 → 台湾繁体 (异体字)"),
                        )
                        .on_disabled_hover_text(disabled_hover_text)
                        .clicked()
                    {
                        self.handle_chinese_conversion("t2tw.json");
                    }
                    if sub_menu
                        .add_enabled(
                            conversion_enabled,
                            egui::Button::new("繁体 → 香港繁体 (异体字)"),
                        )
                        .on_disabled_hover_text(disabled_hover_text)
                        .clicked()
                    {
                        self.handle_chinese_conversion("t2hk.json");
                    }
                    if sub_menu
                        .add_enabled(conversion_enabled, egui::Button::new("香港繁体 → 简体"))
                        .on_disabled_hover_text(disabled_hover_text)
                        .clicked()
                    {
                        self.handle_chinese_conversion("hk2s.json");
                    }
                });
                tools_menu.separator();

                tools_menu.label(egui::RichText::new("日语汉字转换").strong());
                if tools_menu
                    .add_enabled(
                        conversion_enabled,
                        egui::Button::new("日语新字体 → 繁体旧字体"),
                    )
                    .on_disabled_hover_text(disabled_hover_text)
                    .clicked()
                {
                    self.handle_chinese_conversion("jp2t.json");
                }
                if tools_menu
                    .add_enabled(
                        conversion_enabled,
                        egui::Button::new("繁体旧字体 → 日语新字体"),
                    )
                    .on_disabled_hover_text(disabled_hover_text)
                    .clicked()
                {
                    self.handle_chinese_conversion("t2jp.json");
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
            let restrict_target_to_line_based = self
                .parsed_lyric_data
                .as_ref()
                .map_or(false, |d| d.is_line_timed_source);
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
                        log::error!("[UniLyricApp] 自动保存源/目标格式到设置失败: {e}");
                    } else {
                        log::trace!(
                            "[UniLyricApp] 已自动保存源格式 ({:?}) 和目标格式 ({:?}) 到设置。",
                            self.source_format,
                            self.target_format
                        );
                    }
                }

                // 再次检查并自动切换目标格式的逻辑 (作为保险)
                if self
                    .parsed_lyric_data
                    .as_ref()
                    .map_or(false, |d| d.is_line_timed_source)
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
                    self.clear_all_data();
                    self.output_text.clear();
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
                ui.strong("自动歌词搜索设置:");
                ui.separator();
                ui.add_space(5.0);

                ui.checkbox(&mut self.temp_edit_settings.always_search_all_sources, "始终并行搜索所有源 (最准，但最慢)");
                ui.add_space(10.0);

                // 【新】添加“使用指定源”的复选框
                ui.checkbox(&mut self.temp_edit_settings.use_provider_subset, "只在以下选择的源中搜索:");
                
                // 【新】创建一个只在上面的复选框被选中时才启用的UI区域
                ui.add_enabled_ui(self.temp_edit_settings.use_provider_subset, |enabled_ui| {
                    egui::Frame::group(enabled_ui.style()).show(enabled_ui, |group_ui| {
                        group_ui.label("选择要使用的提供商:");
                        
                        // 我们需要一个所有可用提供商的列表
                        let all_providers = AutoSearchSource::default_order();
                        
                        for provider in all_providers {
                            // 我们需要将 AutoSearchSource 枚举转换为 String 来进行比较
                            let provider_name = Into::<&'static str>::into(provider).to_string();
                            
                            // 检查当前提供商是否已经在用户的选择列表中
                            let mut is_selected = self.temp_edit_settings.auto_search_provider_subset.contains(&provider_name);
                            
                            if group_ui.checkbox(&mut is_selected, provider.display_name()).changed() {
                                if is_selected {
                                    // 如果用户刚刚勾选了它，就添加到列表中
                                    self.temp_edit_settings.auto_search_provider_subset.push(provider_name);
                                } else {
                                    // 如果用户刚刚取消了勾选，就从列表中移除
                                    self.temp_edit_settings.auto_search_provider_subset.retain(|p| p != &provider_name);
                                }
                            }
                        }
                    });
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                ui.separator();
                ui.add_space(10.0);
                ui.strong("自动删除元数据行设置");
                ui.checkbox(&mut self.temp_edit_settings.enable_online_lyric_stripping, "基于关键词的移除");


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

                        log::debug!("[Unilyric UI] 设置已保存。新 AMLL Connector配置: {new_mc_config_from_settings:?}");

                        if new_mc_config_from_settings.enabled {
                            amll_connector_manager::ensure_running(self);
                            if let Some(tx) = &self.media_connector_command_tx
                                && old_mc_config != new_mc_config_from_settings {
                                    log::debug!("[Unilyric UI] 发送 UpdateConfig 命令给AMLL Connector worker。");
                                    if tx.send(crate::amll_connector::ConnectorCommand::UpdateConfig(new_mc_config_from_settings.clone())).is_err() {
                                        log::error!("[Unilyric UI] 发送 UpdateConfig 命令给AMLL Connector worker 失败。");
                                    }
                                }
                        } else {
                            amll_connector_manager::ensure_running(self); // 确保如果禁用了，worker会停止
                        }

                        if connector_enabled_runtime && old_send_audio_data_setting != new_send_audio_data_setting {
                            self.audio_visualization_is_active = new_send_audio_data_setting;
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
                    self.editable_metadata.len() as u32 + rand::thread_rng().r#gen::<u32>();

                let new_id = egui::Id::new(format!("new_editable_meta_entry_{new_entry_id_num}"));
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
        if let Some(idx_del) = entry_to_delete_idx
            && idx_del < self.editable_metadata.len()
        {
            // 再次确认索引有效
            self.editable_metadata.remove(idx_del);
        }

        if metadata_changed_this_frame {
            self.handle_convert();
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
        let is_translation_panel = true;

        let title = "翻译 (LRC)";
        let lrc_is_currently_considered_active = self.loaded_translation_lrc.is_some()
            || !self.display_translation_lrc_output.trim().is_empty();

        ui.add_space(TITLE_ALIGNMENT_OFFSET);
        ui.label(egui::RichText::new(title).heading());
        ui.separator();

        ui.horizontal(|button_strip_ui| {
            let main_lyrics_exist_for_merge = self.parsed_lyric_data.as_ref().is_some();
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
                                format_lrc_time_ms(lrc_line.start_ms),
                                lrc_line.line_text.as_deref().unwrap_or_default()
                            );
                        }
                        DisplayLrcLine::Raw { original_text } => {
                            let _ = writeln!(reconstructed_display_text, "{original_text}");
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
                .parsed_lyric_data
                .as_ref()
                .is_some_and(|p| !p.lines.is_empty())
            {
                self.handle_convert();
            }
        }

        if clear_action_triggered {
            self.loaded_translation_lrc = None;
            self.display_translation_lrc_output.clear();
            log::info!("已清除翻译 LRC (通过UI按钮)。");
            if self
                .parsed_lyric_data
                .as_ref()
                .is_some_and(|p| !p.lines.is_empty())
            {
                self.handle_convert();
            }
        }

        if text_edited_this_frame {
            // 使用核心库的LRC解析器
            match lyrics_helper_rs::converter::parsers::lrc_parser::parse_lrc(
                &self.display_translation_lrc_output,
            ) {
                Ok(parsed_data) => {
                    // 将解析出的行转换为UI需要的 DisplayLrcLine 格式
                    let display_lines = parsed_data
                        .lines
                        .into_iter()
                        .map(DisplayLrcLine::Parsed)
                        .collect();

                    // 根据面板类型，更新对应的状态字段
                    if is_translation_panel {
                        // (你需要一个布尔值来区分)
                        self.loaded_translation_lrc = Some(display_lines);
                    } else {
                        self.loaded_romanization_lrc = Some(display_lines);
                    }
                }
                Err(e) => {
                    // 解析失败
                    if is_translation_panel {
                        self.loaded_translation_lrc = None;
                    } else {
                        self.loaded_romanization_lrc = None;
                    }
                    log::warn!("[UI Edit] LRC文本解析失败: {e}");
                }
            }
            // 触发主转换流程以合并新的LRC数据
            self.handle_convert();
        }
    }

    /// 绘制罗马音LRC面板的内容。
    pub fn draw_romanization_lrc_panel_contents(&mut self, ui: &mut egui::Ui) {
        let mut clear_action_triggered = false;
        let mut import_action_triggered = false;
        let mut text_edited_this_frame = false;
        let is_translation_panel = false;

        let title = "罗马音 (LRC)";
        let lrc_is_currently_considered_active = self.loaded_romanization_lrc.is_some()
            || !self.display_romanization_lrc_output.trim().is_empty();

        ui.add_space(TITLE_ALIGNMENT_OFFSET);
        ui.label(egui::RichText::new(title).heading());
        ui.separator();

        ui.horizontal(|button_strip_ui| {
            let main_lyrics_exist_for_merge = self
                .parsed_lyric_data
                .as_ref()
                .is_some_and(|p| !p.lines.is_empty());
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
                                format_lrc_time_ms(lrc_line.start_ms),
                                lrc_line.line_text.as_deref().unwrap_or_default()
                            );
                        }
                        DisplayLrcLine::Raw { original_text } => {
                            let _ = writeln!(reconstructed_display_text, "{original_text}");
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
                .parsed_lyric_data
                .as_ref()
                .is_some_and(|p| !p.lines.is_empty())
            {
                self.handle_convert();
            }
        }

        if clear_action_triggered {
            self.loaded_romanization_lrc = None;
            self.display_romanization_lrc_output.clear();
            log::info!("已清除罗马音 LRC (通过UI按钮)。");
            if self
                .parsed_lyric_data
                .as_ref()
                .is_some_and(|p| !p.lines.is_empty())
            {
                self.handle_convert();
            }
        }

        if text_edited_this_frame {
            match lrc_parser::parse_lrc(&self.display_romanization_lrc_output) {
                Ok(parsed_data) => {
                    // 将解析出的行转换为UI需要的 DisplayLrcLine 格式
                    let display_lines = parsed_data
                        .lines
                        .into_iter()
                        .map(DisplayLrcLine::Parsed)
                        .collect();

                    // 根据面板类型，更新对应的状态字段
                    if is_translation_panel {
                        // (你需要一个布尔值来区分)
                        self.loaded_translation_lrc = Some(display_lines);
                    } else {
                        self.loaded_romanization_lrc = Some(display_lines);
                    }
                }

                Err(e) => {
                    self.loaded_romanization_lrc = None;
                    log::warn!(
                        "[UI Edit] 编辑的罗马音LRC文本解析器返回错误: {e}. 关联的LRC数据已清除."
                    );
                    self.toasts.add(egui_toast::Toast {
                        text: format!("罗马音LRC内容解析错误: {e}").into(),
                        kind: egui_toast::ToastKind::Error,
                        options: egui_toast::ToastOptions::default()
                            .duration_in_seconds(4.0)
                            .show_icon(true),
                        style: Default::default(),
                    });
                }
            }
            if self
                .parsed_lyric_data
                .as_ref()
                .is_some_and(|p| !p.lines.is_empty())
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
            .map(|(ln, txt)| format!("ASS 行 {ln}: {txt}"))
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
                    && let Some(tx) = &self.media_connector_command_tx
                {
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

        // ... (WebSocket 连接状态的UI部分保持不变) ...
        ui.strong("WebSocket 连接:");

        let current_status = self.media_connector_status.lock().unwrap().clone();
        let websocket_url_display: String;
        {
            let config_guard_display = self.media_connector_config.lock().unwrap();
            websocket_url_display = config_guard_display.websocket_url.clone();
        }

        ui.label(format!("目标 URL: {websocket_url_display}"));

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
                            "[Unilyric UI] 发送 UpdateConfig 命令以触发连接尝试: {current_config_for_command:?}"
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
                            "[Unilyric UI] 发送 UpdateConfig 命令以触发重试连接: {current_config_for_command:?}"
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

        // ... (SMTC 源选择和当前监听信息的UI部分保持不变) ...
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
                        || format!("自动 (选择 '{id}' 已失效)"),
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

                if let Some(tx) = &self.media_connector_command_tx
                    && tx
                        .send(ConnectorCommand::SelectSmtcSession(session_to_send))
                        .is_err()
                {
                    log::error!("[Unilyric UI] 发送 SelectSmtcSession 命令失败。");
                }
            }
        }
        ui.separator();
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
                    if let Some(cover_bytes) = &info.cover_data
                        && !cover_bytes.is_empty()
                    {
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
        let sources_config = vec![
            (
                AutoSearchSource::LocalCache,
                &self.local_cache_auto_search_status,
                None,
            ),
            (
                AutoSearchSource::QqMusic,
                &self.qqmusic_auto_search_status,
                Some(&self.last_qq_search_result),
            ),
            (
                AutoSearchSource::Kugou,
                &self.kugou_auto_search_status,
                Some(&self.last_kugou_search_result),
            ),
            (
                AutoSearchSource::Netease,
                &self.netease_auto_search_status,
                Some(&self.last_netease_search_result),
            ),
            (
                AutoSearchSource::AmllDb,
                &self.amll_db_auto_search_status,
                Some(&self.last_amll_db_search_result),
            ),
            (
                AutoSearchSource::Musixmatch,
                &self.musixmatch_auto_search_status,
                Some(&self.last_musixmatch_search_result),
            ),
        ];

        let mut action_load_lyrics: Option<(AutoSearchSource, FullLyricsResult)> = None;
        let mut action_refetch: Option<AutoSearchSource> = None; // 【修复】使用一个变量来延迟执行

        for (source_enum, status_arc, opt_result_arc) in sources_config {
            ui.horizontal(|item_ui| {
                item_ui.label(format!("{}:", source_enum.display_name()));
                let status = status_arc.lock().unwrap().clone();

                item_ui.with_layout(Layout::right_to_left(Align::Center), |right_aligned_ui| {
                    let mut stored_data_for_load: Option<FullLyricsResult> = None;
                    if let Some(result_arc) = opt_result_arc {
                        if let Some(ref data) = *result_arc.lock().unwrap() {
                            stored_data_for_load = Some(data.clone());
                        }
                    }

                    if let Some(data) = stored_data_for_load {
                        if right_aligned_ui
                            .button("载入")
                            .on_hover_text(format!(
                                "使用 {} 找到的歌词",
                                source_enum.display_name()
                            ))
                            .clicked()
                        {
                            action_load_lyrics = Some((source_enum, data));
                        }
                        right_aligned_ui.add_space(4.0);
                    }

                    if source_enum != AutoSearchSource::LocalCache {
                        if right_aligned_ui.button("重搜").clicked() {
                            action_refetch = Some(source_enum); // 【修复】不直接调用，而是记录要执行的动作
                        }
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

        // 【修复】在循环结束后，执行记录下的动作
        if let Some((source, result)) = action_load_lyrics {
            self.load_lyrics_from_stored_result(source, result);
        }
        if let Some(source) = action_refetch {
            crate::app_fetch_core::trigger_manual_refetch_for_source(self, source);
        }
    }

    /// 绘制统一的歌词搜索/下载窗口。
    pub fn draw_search_lyrics_window(&mut self, ctx: &egui::Context) {
        if !self.show_search_window {
            return;
        }

        let mut is_open = self.show_search_window;

        let available_rect = ctx.available_rect();

        egui::Window::new("搜索歌词")
            .open(&mut is_open)
            .collapsible(false)
            .resizable(true)
            .default_width(400.0)
            .max_width(available_rect.width() * 0.9)
            .max_height(available_rect.height() * 0.8)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.horizontal(|h_ui| {
                    let response = h_ui.add(
                        egui::TextEdit::singleline(&mut self.search_query)
                            .hint_text("输入歌曲名或“歌曲 - 歌手”")
                            .desired_width(h_ui.available_width() - 50.0),
                    );
                    if response.lost_focus() && h_ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.trigger_lyrics_search();
                    }

                    if h_ui
                        .add_enabled(!self.search_in_progress, egui::Button::new("搜索"))
                        .clicked()
                    {
                        self.trigger_lyrics_search();
                    }
                });

                ui.separator();

                if self.search_in_progress {
                    ui.horizontal(|h_ui| {
                        h_ui.spinner();
                        h_ui.label("正在搜索...");
                    });
                } else if self.download_in_progress {
                    ui.horizontal(|h_ui| {
                        h_ui.spinner();
                        h_ui.label("正在下载歌词...");
                    });
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |s_ui| {
                        if !self.search_results.is_empty() {
                            for result in self.search_results.clone() {
                                let full_label = format!(
                                    "{} - {} ({})",
                                    result.title,
                                    result.artists.join("/"),
                                    result.provider_name
                                );

                                // 【修复3】为了美观，截断过长的文本，并在悬停时显示完整内容
                                let mut display_label = full_label.clone();
                                if display_label.chars().count() > 50 {
                                    // 限制显示长度为50个字符
                                    display_label =
                                        display_label.chars().take(50).collect::<String>() + "...";
                                }

                                if s_ui
                                    .button(&display_label)
                                    .on_hover_text(&full_label)
                                    .clicked()
                                {
                                    self.trigger_lyrics_download(&result);
                                }
                            }
                        } else if !self.search_in_progress && !self.search_query.is_empty() {
                            s_ui.label("未找到结果。");
                        }
                    });
            });

        if !is_open {
            self.show_search_window = false;
        }
    }
}
