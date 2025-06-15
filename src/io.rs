use crate::app_definition::UniLyricApp;
use crate::lrc_parser;
use crate::types::{CanonicalMetadataKey, DisplayLrcLine, LrcContentType, LyricFormat};
use std::fs;
use std::path::{Path, PathBuf};

pub fn load_file_and_convert(app: &mut UniLyricApp, path: PathBuf) {
    app.clear_all_data();
    app.metadata_source_is_download = false;
    app.last_opened_file_path = Some(path.clone());

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            app.input_text = content;

            if let Some(ext_str) = path.extension().and_then(|e| e.to_str()) {
                if let Some(fmt) = LyricFormat::from_string(ext_str) {
                    app.source_format = fmt;
                    log::info!(
                        "[Unilyric] 从文件 '{}' 设置源格式为: {:?}",
                        path.display(),
                        fmt
                    );
                } else {
                    log::warn!("[Unilyric] 无法从文件扩展名 '{ext_str}' 判断源格式。请手动选择。");
                }
            } else {
                log::warn!(
                    "[Unilyric] 文件 '{}' 没有扩展名，无法自动设置源格式。请手动选择。",
                    path.display()
                );
            }

            if !app.input_text.is_empty() {
                if app.source_format == LyricFormat::Lrc
                    && !matches!(
                        app.target_format,
                        LyricFormat::Lqe | LyricFormat::Spl | LyricFormat::Lrc
                    )
                {
                    log::info!("[UniLyric] 源格式为LRC，目标格式自动切换为LQE。");
                    app.target_format = LyricFormat::Lqe;
                }
                app.handle_convert();
            }
        }
        Err(e) => {
            log::error!("[Unilyric] 读取文件 '{}' 失败: {}", path.display(), e);
        }
    }
}

pub fn handle_open_file(app: &mut UniLyricApp) {
    if app.conversion_in_progress {
        return;
    }

    let supported_main_extensions = [
        "ass", "ssa", "ttml", "xml", "json", "lys", "qrc", "yrc", "krc", "spl", "lqe", "Lyl",
    ];

    let dialog = rfd::FileDialog::new()
        .add_filter("支持的歌词文件", &supported_main_extensions)
        .add_filter("所有文件", &["*"])
        .set_title("打开歌词文件...");

    let initial_dir = app
        .last_opened_file_path
        .as_ref()
        .and_then(|p| p.parent())
        .unwrap_or_else(|| Path::new("."));

    if let Some(path) = dialog.set_directory(initial_dir).pick_file() {
        crate::io::load_file_and_convert(app, path)
    }
}

pub fn handle_save_file(app: &mut UniLyricApp) {
    if app.conversion_in_progress {
        return;
    }

    if app.output_text.is_empty() {
        log::error!("[Unilyric] 没有可保存的输出内容。");
        return;
    }

    let target_ext = app.target_format.to_extension_str();
    let default_filename = format!("Converted.{target_ext}");

    let dialog = rfd::FileDialog::new()
        .add_filter(
            format!("{} 文件 (*.{})", app.target_format, target_ext).as_str(),
            &[target_ext],
        )
        .set_file_name(&default_filename)
        .set_title("保存输出为...");

    let initial_dir = app
        .last_saved_file_path
        .as_ref()
        .and_then(|p| p.parent())
        .or_else(|| app.last_opened_file_path.as_ref().and_then(|p| p.parent()))
        .unwrap_or_else(|| Path::new("."));

    if let Some(path) = dialog.set_directory(initial_dir).save_file() {
        let final_path = if path.extension().and_then(|s| s.to_str()) == Some(target_ext) {
            path
        } else {
            path.with_extension(target_ext)
        };

        app.last_saved_file_path = Some(final_path.clone());

        if let Err(e) = std::fs::write(&final_path, &app.output_text) {
            log::error!("[Unilyric] 写入文件 '{}' 失败: {}", final_path.display(), e);
        } else {
            log::info!("[Unilyric] 文件已成功保存到: {}", final_path.display());
        }
    }
}

pub fn handle_open_lrc_file(app: &mut UniLyricApp, lrc_type: LrcContentType) {
    let dialog_title = match lrc_type {
        LrcContentType::Translation => "打开翻译 LRC 文件",
        LrcContentType::Romanization => "打开罗马音 LRC 文件",
    };

    let dialog = rfd::FileDialog::new()
        .add_filter("LRC 文件", &["lrc"])
        .add_filter("所有文件", &["*"])
        .set_title(dialog_title);

    let initial_dir = app
        .last_opened_file_path
        .as_ref()
        .and_then(|p| p.parent())
        .unwrap_or_else(|| Path::new("."));

    if let Some(path) = dialog.set_directory(initial_dir).pick_file() {
        load_lrc_file_from_path(app, path, lrc_type);
    }
}

fn load_lrc_file_from_path(app: &mut UniLyricApp, path: PathBuf, lrc_type: LrcContentType) {
    let source_desc = path.display().to_string();
    match fs::read_to_string(&path) {
        Ok(content) => {
            app.last_opened_file_path = Some(path);
            load_lrc_from_content(app, content, lrc_type, &source_desc);
        }
        Err(e) => {
            log::error!("[Unilyric] 读取LRC文件 '{source_desc}' 失败: {e}");
            app.toasts.add(egui_toast::Toast {
                text: format!("读取LRC文件失败: {e}").into(),
                kind: egui_toast::ToastKind::Error,
                options: egui_toast::ToastOptions::default()
                    .duration_in_seconds(3.0)
                    .show_icon(true),
                style: Default::default(),
            });
        }
    }
}

fn load_lrc_from_content(
    app: &mut UniLyricApp,
    content: String,
    lrc_type: LrcContentType,
    source_description: &str,
) {
    match lrc_parser::parse_lrc_text_to_lines(&content) {
        Ok((lines, _bilingual_translations, lrc_meta)) => {
            // 忽略 _bilingual_translations
            let log_type_str = match lrc_type {
                LrcContentType::Translation => "翻译",
                LrcContentType::Romanization => "罗马音",
            };
            let num_lines = lines.len();

            // 将解析后的 DisplayLrcLine 转换为用于UI显示的纯文本字符串
            let lrc_text_for_display = lines
                .iter()
                .map(|line_entry| match line_entry {
                    DisplayLrcLine::Parsed(lrc_line) => {
                        format!(
                            "{}{}",
                            crate::utils::format_lrc_time_ms(lrc_line.timestamp_ms),
                            lrc_line.text
                        )
                    }
                    DisplayLrcLine::Raw { original_text } => original_text.clone(),
                })
                .collect::<Vec<String>>()
                .join("\n");

            if !lrc_meta.is_empty() {
                let mut store = app.metadata_store.lock().unwrap();
                for item in &lrc_meta {
                    match item.key.parse::<CanonicalMetadataKey>() {
                        Ok(c_key_from_lrc) => {
                            let should_add =
                                store
                                    .get_multiple_values(&c_key_from_lrc)
                                    .is_none_or(|vals| {
                                        vals.is_empty() || vals.iter().all(|v| v.is_empty())
                                    });

                            if should_add {
                                if lrc_type == LrcContentType::Translation
                                    && matches!(c_key_from_lrc, CanonicalMetadataKey::Language)
                                {
                                    if let Err(e) =
                                        store.add("translation_language", item.value.clone())
                                    {
                                        log::info!("从LRC加载翻译语言失败: {e}");
                                    }
                                } else if let Err(e) = store.add(&item.key, item.value.clone()) {
                                    log::info!("未能从LRC添加元数据键 {}：{}", item.key, e);
                                }
                            } else {
                                log::info!(
                                    "[LRC Load] 主数据中已存在元数据键 '{}' (解析为 {:?})，忽略来自辅助LRC '{}' 的值 '{}'",
                                    item.key,
                                    c_key_from_lrc,
                                    source_description,
                                    item.value
                                );
                            }
                        }
                        Err(e) => {
                            log::info!(
                                "[LRC Load] 无法将LRC元数据键 '{}' 解析为 CanonicalMetadataKey: {}。保留原始键进行添加。",
                                item.key,
                                e
                            );
                        }
                    }
                }
                // 元数据更新后，需要重建UI列表，这通常在 handle_convert 结束时统一进行
                // 或者如果这里是独立加载，则可以在这里调用
                // drop(store); // 释放锁
                // self.rebuild_editable_metadata_from_store(); // 移到外部，在所有LRC数据处理完之后
            }

            match lrc_type {
                LrcContentType::Translation => {
                    app.loaded_translation_lrc = Some(lines);
                    // 更新UI显示文本
                    app.display_translation_lrc_output = if lrc_text_for_display.trim().is_empty() {
                        String::new()
                    } else {
                        lrc_text_for_display.trim_end_matches('\n').to_string() + "\n"
                    };
                }
                LrcContentType::Romanization => {
                    app.loaded_romanization_lrc = Some(lines);
                    // 更新UI显示文本
                    app.display_romanization_lrc_output = if lrc_text_for_display.trim().is_empty()
                    {
                        String::new()
                    } else {
                        lrc_text_for_display.trim_end_matches('\n').to_string() + "\n"
                    };
                }
            }
            log::info!(
                "[Unilyric] 已从 '{source_description}' 加载 {log_type_str} LRC ({num_lines} 行)。"
            );

            app.rebuild_editable_metadata_from_store();

            if app.parsed_ttml_paragraphs.is_some() || app.target_format == LyricFormat::Lqe {
                log::info!(
                    "[UniLyricApp load_lrc_from_content] 主段落存在或目标为LQE，触发 handle_convert。"
                );
                app.handle_convert();
            } else {
                log::info!("[UniLyricApp load_lrc_from_content] 无主段落，仅更新LRC预览。");
                // 即使没有主段落，也应该尝试合并（虽然可能没什么可合并的），并生成输出
                // 以便LRC面板的内容能反映到可能的LQE输出中（如果目标是LQE）
                // 或者至少确保如果目标是LRC，输出是基于加载的LRC。
                // 考虑到 handle_convert 内部会处理 loaded_xxx_lrc，这里直接调用它更统一。
                app.handle_convert();
            }
        }
        Err(e) => {
            log::error!("[Unilyric] 从 '{source_description}' 解析LRC内容失败: {e}");
            app.toasts.add(egui_toast::Toast {
                text: format!("LRC文件解析失败: {e}").into(),
                kind: egui_toast::ToastKind::Error,
                options: egui_toast::ToastOptions::default()
                    .duration_in_seconds(3.0)
                    .show_icon(true),
                style: Default::default(),
            });
        }
    }
}
