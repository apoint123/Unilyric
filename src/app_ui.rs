// 导入 eframe::egui 模块，这是主要的GUI库
use eframe::egui;
// 导入 log::LevelFilter，用于设置日志级别
use log::LevelFilter;
// 从 app模块导入应用核心结构和状态枚举，以及元数据条目结构
use crate::app::{
    EditableMetadataEntry, KrcDownloadState, NeteaseDownloadState, QqMusicDownloadState,
    UniLyricApp,
};
// 从 types 模块导入 LrcContentType（用于区分翻译/罗马音LRC）和 LyricFormat（歌词格式枚举）
use crate::types::{LrcContentType, LyricFormat};
// 导入 rand::Rng，用于生成随机数 (例如为元数据条目生成唯一ID)
use rand::Rng;

// 定义一些UI布局相关的常量
const TITLE_ALIGNMENT_OFFSET: f32 = 6.0; // 标题文本的对齐偏移量
const BUTTON_STRIP_SPACING: f32 = 4.0; // 按钮条中按钮之间的间距

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
                    LyricFormat::Lqe | LyricFormat::Spl | LyricFormat::Lrc
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
                                LyricFormat::Lqe | LyricFormat::Spl | LyricFormat::Lrc
                            ) {
                                enabled = false;
                                hover_text_for_disabled =
                                    Some("LRC源格式只能输出为LQE, SPL, 或 LRC".to_string());
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
                // 再次检查并自动切换目标格式的逻辑 (作为保险)
                if (Self::source_format_is_line_timed(self.source_format)
                    || (matches!(
                        self.source_format,
                        LyricFormat::Ttml | LyricFormat::Json | LyricFormat::Spl
                    ) && self.source_is_line_timed))
                    && truly_word_based_formats_requiring_syllables.contains(&self.target_format)
                    && self.source_format != LyricFormat::Lrc
                // 如果源是LRC，则不执行这个自动切换，因为上面已经限制了目标
                {
                    log::info!(
                        "[Unilyric] 源格式为逐行（非LRC），但目标格式为逐字，已自动切换为LRC"
                    );
                    self.target_format = LyricFormat::Lrc; // 逐行源默认转LRC
                }

                if source_format_changed_this_frame {
                    // 如果是源格式改变
                    log::info!("[UniLyricApp] 源格式已更改为 {:?}.", self.source_format);
                    // 如果输入框有文本，或者新选择的源格式是LRC（LRC可能直接作为输入内容），则触发转换
                    if !self.input_text.is_empty() || self.source_format == LyricFormat::Lrc {
                        self.handle_convert(); // 重新转换
                    } else {
                        self.clear_derived_data(); // 清理已解析的数据
                        self.generate_target_format_output(); // 尝试基于现有状态生成输出 (可能为空)
                    }
                } else if target_format_changed_this_frame {
                    // 仅目标格式改变
                    log::info!(
                        "[UniLyricApp] 目标格式已更改为 {:?}. 重新生成输出。",
                        self.target_format
                    );
                    self.output_text.clear(); // 清空旧输出
                    self.generate_target_format_output(); // 生成新格式的输出
                }
            }
            // --- 格式更改处理结束 ---

            // --- 工具栏右侧按钮 ---
            ui_bar.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui_right| {
                    if ui_right.button("元数据").clicked() {
                        self.show_metadata_panel = true;
                    } // 打开元数据编辑面板
                    let log_button_text = "查看日志";
                    // 切换日志面板的显示状态，如果点击后变为显示，则清除新日志提示
                    if ui_right
                        .toggle_value(&mut self.show_bottom_log_panel, log_button_text)
                        .clicked()
                        && self.show_bottom_log_panel
                    {
                        self.new_trigger_log_exists = false;
                    }
                    // 文本自动换行复选框
                    if ui_right.checkbox(&mut self.wrap_text, "自动换行").changed() {
                        // 可以在这里触发UI重绘或重新布局，如果需要的话
                    }
                    // 设置按钮
                    if ui_right.button("设置").clicked() {
                        // 打开设置窗口前，将当前应用的设置复制到临时编辑变量中
                        self.temp_edit_settings = self.app_settings.lock().unwrap().clone();
                        self.show_settings_window = true;
                    }
                },
            );
        });
    }

    /// 绘制应用设置窗口。
    pub fn draw_settings_window(&mut self, ctx: &egui::Context) {
        let mut is_settings_window_open = self.show_settings_window; // 控制窗口的打开/关闭状态

        // 创建一个模态窗口
        egui::Window::new("应用程序设置")
            .open(&mut is_settings_window_open) // 绑定到可变状态，允许通过标题栏关闭
            .resizable(true) // 允许调整窗口大小
            .default_width(400.0) // 默认宽度
            .scroll([false, true]) // 垂直方向可滚动
            .show(ctx, |ui| {
                // 窗口内容构建闭包
                // let mut settings_have_changed_in_ui = false; // 跟踪UI中是否有更改 (当前未使用此变量的返回值)

                // 使用 Grid 布局来对齐标签和控件
                egui::Grid::new("settings_grid")
                    .num_columns(2) // 两列布局
                    .spacing([40.0, 4.0]) // 列间距和行间距
                    .striped(true) // 条纹背景
                    .show(ui, |grid_ui| {
                        grid_ui.heading("日志设置"); // 分组标题
                        grid_ui.end_row(); // 结束当前行

                        grid_ui.label("启用文件日志:");
                        // 复选框，绑定到临时设置变量
                        /*settings_have_changed_in_ui |=*/
                        grid_ui
                            .checkbox(
                                &mut self.temp_edit_settings.log_settings.enable_file_log,
                                "",
                            )
                            .changed();
                        grid_ui.end_row();

                        grid_ui.label("文件日志级别:");
                        // 下拉框选择文件日志级别
                        /*settings_have_changed_in_ui |=*/
                        egui::ComboBox::from_id_salt("file_log_level_combo")
                            .selected_text(format!(
                                "{:?}",
                                self.temp_edit_settings.log_settings.file_log_level
                            ))
                            .show_ui(grid_ui, |ui_combo| {
                                let mut changed_in_combo = false;
                                // 为每个日志级别添加一个可选条目
                                changed_in_combo |= ui_combo
                                    .selectable_value(
                                        &mut self.temp_edit_settings.log_settings.file_log_level,
                                        LevelFilter::Off,
                                        "Off",
                                    )
                                    .changed();
                                changed_in_combo |= ui_combo
                                    .selectable_value(
                                        &mut self.temp_edit_settings.log_settings.file_log_level,
                                        LevelFilter::Error,
                                        "Error",
                                    )
                                    .changed();
                                changed_in_combo |= ui_combo
                                    .selectable_value(
                                        &mut self.temp_edit_settings.log_settings.file_log_level,
                                        LevelFilter::Warn,
                                        "Warn",
                                    )
                                    .changed();
                                changed_in_combo |= ui_combo
                                    .selectable_value(
                                        &mut self.temp_edit_settings.log_settings.file_log_level,
                                        LevelFilter::Info,
                                        "Info",
                                    )
                                    .changed();
                                changed_in_combo |= ui_combo
                                    .selectable_value(
                                        &mut self.temp_edit_settings.log_settings.file_log_level,
                                        LevelFilter::Debug,
                                        "Debug",
                                    )
                                    .changed();
                                changed_in_combo |= ui_combo
                                    .selectable_value(
                                        &mut self.temp_edit_settings.log_settings.file_log_level,
                                        LevelFilter::Trace,
                                        "Trace",
                                    )
                                    .changed();
                                changed_in_combo
                            })
                            .inner
                            .unwrap_or(false);
                        grid_ui.end_row();

                        grid_ui.label("控制台日志级别:");
                        /*settings_have_changed_in_ui |=*/
                        egui::ComboBox::from_id_salt("console_log_level_combo")
                            .selected_text(format!(
                                "{:?}",
                                self.temp_edit_settings.log_settings.console_log_level
                            ))
                            .show_ui(grid_ui, |ui_combo| {
                                let mut changed_in_combo = false;
                                changed_in_combo |= ui_combo
                                    .selectable_value(
                                        &mut self.temp_edit_settings.log_settings.console_log_level,
                                        LevelFilter::Off,
                                        "Off",
                                    )
                                    .changed();
                                changed_in_combo |= ui_combo
                                    .selectable_value(
                                        &mut self.temp_edit_settings.log_settings.console_log_level,
                                        LevelFilter::Error,
                                        "Error",
                                    )
                                    .changed();
                                changed_in_combo |= ui_combo
                                    .selectable_value(
                                        &mut self.temp_edit_settings.log_settings.console_log_level,
                                        LevelFilter::Warn,
                                        "Warn",
                                    )
                                    .changed();
                                changed_in_combo |= ui_combo
                                    .selectable_value(
                                        &mut self.temp_edit_settings.log_settings.console_log_level,
                                        LevelFilter::Info,
                                        "Info",
                                    )
                                    .changed();
                                changed_in_combo |= ui_combo
                                    .selectable_value(
                                        &mut self.temp_edit_settings.log_settings.console_log_level,
                                        LevelFilter::Debug,
                                        "Debug",
                                    )
                                    .changed();
                                changed_in_combo |= ui_combo
                                    .selectable_value(
                                        &mut self.temp_edit_settings.log_settings.console_log_level,
                                        LevelFilter::Trace,
                                        "Trace",
                                    )
                                    .changed();
                                changed_in_combo
                            })
                            .inner
                            .unwrap_or(false);
                        grid_ui.end_row();

                        // --- 在这里可以添加其他应用设置的UI编辑逻辑 ---
                        // 例如:
                        // grid_ui.heading("常规设置");
                        // grid_ui.end_row();
                        // grid_ui.label("默认输出格式:");
                        // grid_ui.text_edit_singleline(&mut self.temp_edit_settings.default_output_format.get_or_insert_with(String::new));
                        // grid_ui.end_row();
                    }); // Grid 结束

                ui.add_space(15.0); // 添加一些垂直间距
                ui.separator(); // 分割线
                ui.add_space(10.0);

                // 窗口底部的按钮 (保存并应用 / 取消)
                ui.horizontal(|bottom_buttons_ui| {
                    if bottom_buttons_ui
                        .button("保存并应用")
                        .on_hover_text("保存设置到文件。部分日志设置可能需要重启应用才能完全生效。")
                        .clicked()
                    {
                        if self.temp_edit_settings.save().is_ok() {
                            // 调用 AppSettings 的 save 方法
                            // 如果保存成功，更新应用内部持有的设置实例
                            *self.app_settings.lock().unwrap() = self.temp_edit_settings.clone();
                            log::info!("应用设置已保存。日志设置将在下次启动时应用。");
                            // TODO: 可以考虑添加一个对话框提示用户某些设置（如日志级别）需要重启才能完全生效
                        } else {
                            log::error!("保存应用设置失败。");
                        }
                        self.show_settings_window = false; // 关闭设置窗口
                    }
                    if bottom_buttons_ui.button("取消").clicked() {
                        // 不保存更改，直接关闭窗口
                        // temp_edit_settings 中的更改将被丢弃，下次打开时会重新从 app_settings 加载
                        self.show_settings_window = false;
                    }
                });
            });

        // 如果窗口通过标题栏的关闭按钮或其他方式关闭，也更新 show_settings_window 状态
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
                                .id_source(item_id.with("key_edit")) // 控件ID
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
                                .id_source(item_id.with("value_edit"))
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
        ui.add_space(TITLE_ALIGNMENT_OFFSET); // 标题对齐
        ui.horizontal(|title_ui| {
            // 标题和右侧按钮在同一行
            title_ui.heading("输入歌词");
            title_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |btn_ui| {
                // 清空按钮：当输入或输出非空时可用
                if btn_ui
                    .add_enabled(
                        !self.input_text.is_empty() || !self.output_text.is_empty(),
                        egui::Button::new("清空"),
                    )
                    .clicked()
                {
                    self.clear_all_data(); // 清理所有数据
                }
                btn_ui.add_space(BUTTON_STRIP_SPACING);
                // 复制按钮：当输入非空时可用
                if btn_ui
                    .add_enabled(!self.input_text.is_empty(), egui::Button::new("复制"))
                    .clicked()
                {
                    btn_ui.ctx().copy_text(self.input_text.clone()); // 复制输入框内容到剪贴板
                }
                btn_ui.add_space(BUTTON_STRIP_SPACING);
                // 粘贴按钮
                if btn_ui.button("粘贴").clicked() {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        // 尝试访问系统剪贴板
                        if let Ok(text) = clipboard.get_text() {
                            // 获取剪贴板文本
                            self.input_text = text; // 更新输入框内容
                            self.handle_convert(); // 触发转换
                        } else {
                            log::error!("无法从剪贴板获取文本");
                        }
                    } else {
                        log::error!("无法访问剪贴板");
                    }
                }
            });
        });
        ui.separator(); // 分割线

        // 使用可滚动的多行文本编辑框作为输入区域
        egui::ScrollArea::vertical()
            .id_salt("input_scroll_always_vertical") // 唯一ID
            .auto_shrink([false, false]) // 不自动缩小
            .show(ui, |s_ui| {
                let text_edit_widget = egui::TextEdit::multiline(&mut self.input_text)
                    .hint_text("在此处粘贴或拖放主歌词文件") // 输入提示
                    .font(egui::TextStyle::Monospace) // 使用等宽字体
                    .interactive(!self.conversion_in_progress) // 如果正在转换，则禁用编辑
                    .desired_width(f32::INFINITY) // 占据所有可用宽度
                    .desired_rows(8); // 期望的初始行数 (可滚动)

                let response = s_ui.add(text_edit_widget); // 添加文本编辑框到UI
                // 如果文本内容发生改变且当前没有转换在进行，则触发转换
                if response.changed() && !self.conversion_in_progress {
                    self.handle_convert();
                }
            });
    }

    /// 绘制翻译LRC面板的内容。
    pub fn draw_translation_lrc_panel_contents(&mut self, ui: &mut egui::Ui) {
        let mut clear_action_triggered = false; // 标记是否点击了清除按钮
        let title = "翻译 (LRC)";
        // 使用 display_translation_lrc_output 作为显示内容，这个字段会在LRC加载或转换后更新
        let text_content_for_display = self.display_translation_lrc_output.clone();
        let lrc_is_currently_loaded = self.loaded_translation_lrc.is_some(); // 判断是否有已加载的翻译LRC数据

        ui.add_space(TITLE_ALIGNMENT_OFFSET);
        ui.label(egui::RichText::new(title).heading()); // 面板标题
        ui.separator();

        // 顶部的按钮条
        ui.horizontal(|button_strip_ui| {
            // 导入按钮：当主歌词已加载且无转换进行时可用
            let main_lyrics_loaded =
                self.parsed_ttml_paragraphs.is_some() && !self.input_text.is_empty();
            let import_enabled = main_lyrics_loaded && !self.conversion_in_progress;
            let import_button_widget = egui::Button::new("导入");
            let mut import_button_response =
                button_strip_ui.add_enabled(import_enabled, import_button_widget);
            if !import_enabled {
                import_button_response =
                    import_button_response.on_disabled_hover_text("请先加载歌词文件");
            }
            if import_button_response.clicked() {
                crate::io::handle_open_lrc_file(self, LrcContentType::Translation); // 打开文件对话框加载翻译LRC
            }

            // 右对齐的按钮 (清除、复制)
            button_strip_ui.allocate_ui_with_layout(
                button_strip_ui.available_size_before_wrap(),
                egui::Layout::right_to_left(egui::Align::Center), // 右对齐布局
                |right_aligned_buttons_ui| {
                    // 清除按钮：当有已加载的翻译LRC时可用
                    if right_aligned_buttons_ui
                        .add_enabled(lrc_is_currently_loaded, egui::Button::new("清除"))
                        .clicked()
                    {
                        clear_action_triggered = true;
                    }
                    right_aligned_buttons_ui.add_space(BUTTON_STRIP_SPACING);
                    // 复制按钮：当显示内容非空时可用
                    if right_aligned_buttons_ui
                        .add_enabled(
                            !text_content_for_display.is_empty(),
                            egui::Button::new("复制"),
                        )
                        .clicked()
                    {
                        right_aligned_buttons_ui
                            .ctx()
                            .copy_text(text_content_for_display.clone());
                    }
                },
            );
        });

        // 根据是否启用文本换行选择不同的滚动区域类型
        let scroll_area = if self.wrap_text {
            egui::ScrollArea::vertical() // 仅垂直滚动
        } else {
            egui::ScrollArea::both().auto_shrink([false, true]) // 水平和垂直滚动，水平不自动缩小
        };

        scroll_area
            .id_salt("translation_lrc_scroll_area") // 唯一ID
            .auto_shrink([false, false])
            .show(ui, |s_ui_content| {
                if text_content_for_display.is_empty() {
                    // 如果没有内容显示
                    s_ui_content.centered_and_justified(|center_ui| {
                        // 居中显示提示文本
                        let hint_text = format!(
                            "通过上方“导入”按钮或“文件”菜单加载 {}",
                            title.split('(').next().unwrap_or("内容").trim()
                        );
                        center_ui.label(egui::RichText::new(hint_text).weak().italics());
                    });
                } else {
                    // 显示LRC文本内容
                    let rich_text = egui::RichText::new(text_content_for_display.as_str())
                        .monospace()
                        .size(13.0);
                    let mut label_widget = egui::Label::new(rich_text).selectable(true); // 允许选择文本
                    if self.wrap_text {
                        label_widget = label_widget.wrap();
                    }
                    // 根据设置启用/禁用换行
                    else {
                        label_widget = label_widget.extend();
                    }
                    s_ui_content.add(label_widget);
                }
                // 确保滚动区域至少有其声明的大小
                s_ui_content.allocate_space(s_ui_content.available_size_before_wrap());
            });

        // 如果点击了清除按钮
        if clear_action_triggered {
            self.loaded_translation_lrc = None; // 清除已加载的翻译数据
            self.display_translation_lrc_output.clear(); // 清空显示内容
            log::info!("已清除加载的翻译 LRC。");
            // 如果主歌词仍然存在，触发一次转换以更新（移除翻译后的）输出
            if self.parsed_ttml_paragraphs.is_some() {
                self.handle_convert();
            }
        }
    }

    /// 绘制罗马音LRC面板的内容。
    /// (逻辑与 draw_translation_lrc_panel_contents 非常相似，只是处理的是罗马音相关的数据)
    pub fn draw_romanization_lrc_panel_contents(&mut self, ui: &mut egui::Ui) {
        let mut clear_action_triggered = false;
        let title = "罗马音 (LRC)";
        let text_content_for_display = self.display_romanization_lrc_output.clone();
        let lrc_is_currently_loaded = self.loaded_romanization_lrc.is_some();

        ui.add_space(TITLE_ALIGNMENT_OFFSET);
        ui.label(egui::RichText::new(title).heading());
        ui.separator();

        ui.horizontal(|button_strip_ui| {
            let main_lyrics_loaded =
                self.parsed_ttml_paragraphs.is_some() && !self.input_text.is_empty();
            let import_enabled = main_lyrics_loaded && !self.conversion_in_progress;
            let import_button_widget = egui::Button::new("导入");
            let mut import_button_response =
                button_strip_ui.add_enabled(import_enabled, import_button_widget);
            if !import_enabled {
                import_button_response =
                    import_button_response.on_disabled_hover_text("请先加载主歌词文件");
            }
            if import_button_response.clicked() {
                crate::io::handle_open_lrc_file(self, LrcContentType::Romanization); // 加载罗马音LRC
            }

            button_strip_ui.allocate_ui_with_layout(
                button_strip_ui.available_size_before_wrap(),
                egui::Layout::right_to_left(egui::Align::Center),
                |right_aligned_buttons_ui| {
                    if right_aligned_buttons_ui
                        .add_enabled(lrc_is_currently_loaded, egui::Button::new("清除"))
                        .clicked()
                    {
                        clear_action_triggered = true;
                    }
                    right_aligned_buttons_ui.add_space(BUTTON_STRIP_SPACING);
                    if right_aligned_buttons_ui
                        .add_enabled(
                            !text_content_for_display.is_empty(),
                            egui::Button::new("复制"),
                        )
                        .clicked()
                    {
                        right_aligned_buttons_ui
                            .ctx()
                            .copy_text(text_content_for_display.clone());
                    }
                },
            );
        });
        let scroll_area = if self.wrap_text {
            egui::ScrollArea::vertical()
        } else {
            egui::ScrollArea::both().auto_shrink([false, true])
        };

        scroll_area
            .id_salt("romanization_lrc_scroll_area")
            .auto_shrink([false, false])
            .show(ui, |s_ui_content| {
                if text_content_for_display.is_empty() {
                    s_ui_content.centered_and_justified(|center_ui| {
                        let hint_text = format!(
                            "通过上方“导入”按钮或“文件”菜单加载 {}",
                            title.split('(').next().unwrap_or("内容").trim()
                        );
                        center_ui.label(egui::RichText::new(hint_text).weak().italics());
                    });
                } else {
                    let rich_text = egui::RichText::new(text_content_for_display.as_str())
                        .monospace()
                        .size(13.0);
                    let mut label_widget = egui::Label::new(rich_text).selectable(true);
                    if self.wrap_text {
                        label_widget = label_widget.wrap();
                    } else {
                        label_widget = label_widget.extend();
                    }
                    s_ui_content.add(label_widget);
                }
                s_ui_content.allocate_space(s_ui_content.available_size_before_wrap());
            });

        if clear_action_triggered {
            self.loaded_romanization_lrc = None;
            self.display_romanization_lrc_output.clear();
            log::info!("已清除加载的罗马音 LRC。");
            if self.parsed_ttml_paragraphs.is_some() {
                self.handle_convert();
            }
        }
    }

    /// 绘制标记信息面板的内容 (通常用于显示 ASS 文件中的 Comment 行标记)。
    pub fn draw_markers_panel_contents(&mut self, ui: &mut egui::Ui, wrap_text: bool) {
        ui.add_space(TITLE_ALIGNMENT_OFFSET);
        ui.heading("标记"); // 面板标题
        ui.separator();
        // 将标记信息 (行号, 文本) 格式化为多行字符串
        let markers_text = self
            .current_markers
            .iter()
            .map(|(ln, txt)| format!("ASS 行 {}: {}", ln, txt))
            .collect::<Vec<_>>()
            .join("\n");

        let scroll_area = if wrap_text {
            // 根据设置选择滚动条类型
            egui::ScrollArea::vertical()
        } else {
            egui::ScrollArea::both().auto_shrink([false, true])
        };

        scroll_area
            .id_salt("markers_panel")
            .auto_shrink([false, false])
            .show(ui, |s_ui| {
                if markers_text.is_empty() {
                    // 如果没有标记信息
                    s_ui.centered_and_justified(|center_ui| {
                        center_ui.label(egui::RichText::new("无标记信息").weak().italics());
                    });
                } else {
                    // 显示标记文本
                    let rich_text = egui::RichText::new(markers_text.as_str())
                        .monospace()
                        .size(13.0);
                    let mut label_widget = egui::Label::new(rich_text).selectable(true);
                    if wrap_text {
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
                            let download_status_locked = self.download_state.lock().unwrap();
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
                    let download_status_locked = self.download_state.lock().unwrap();
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

    /// 绘制输出结果面板的内容。
    pub fn draw_output_panel_contents(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|title_ui| {
            // 标题和右侧按钮在同一行
            title_ui.heading("输出结果");
            title_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |btn_ui| {
                // 复制按钮：当输出文本非空且无转换进行时可用
                if btn_ui
                    .add_enabled(
                        !self.output_text.is_empty() && !self.conversion_in_progress,
                        egui::Button::new("复制"),
                    )
                    .clicked()
                {
                    btn_ui.ctx().copy_text(self.output_text.clone()); // 复制输出内容到剪贴板
                }
            });
        });
        ui.separator(); // 分割线

        // 根据是否启用文本换行选择不同的滚动区域类型
        let scroll_area = if self.wrap_text {
            egui::ScrollArea::vertical().id_salt("output_scroll_vertical_label")
        } else {
            egui::ScrollArea::both().id_salt("output_scroll_both_label")
        };

        scroll_area.auto_shrink([false, false]).show(ui, |s_ui| {
            if self.conversion_in_progress {
                // 如果正在转换，显示加载动画
                s_ui.centered_and_justified(|c_ui| {
                    c_ui.spinner();
                });
            } else {
                // 显示输出文本
                // 使用 Label 显示输出文本，允许选择，使用等宽字体
                let mut label_widget = egui::Label::new(
                    egui::RichText::new(&self.output_text)
                        .monospace()
                        .size(13.0), // 稍小字体
                );

                if self.wrap_text {
                    label_widget = label_widget.wrap();
                }
                // 根据设置启用/禁用换行
                else {
                    label_widget = label_widget.extend();
                }
                s_ui.add(label_widget);
            }
        });
    }
}
