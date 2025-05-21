use crate::app::UniLyricApp;
use crate::lrc_parser;
use crate::types::{CanonicalMetadataKey, LrcContentType, LyricFormat};
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
                    log::warn!(
                        "[Unilyric] 无法从文件扩展名 '{}' 判断源格式。请手动选择。",
                        ext_str
                    );
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
                    log::info!("[UniLyricApp] 源格式为LRC，目标格式自动切换为LQE。");
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
    let default_filename = format!("Converted.{}", target_ext);

    let dialog = rfd::FileDialog::new()
        .add_filter(
            format!("{} 文件 (*.{})", app.target_format.to_string(), target_ext).as_str(),
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
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            load_lrc_from_content(app, content, lrc_type, &source_desc);
        }
        Err(e) => {
            log::error!("[Unilyric] 读取LRC文件 '{}' 失败: {}", source_desc, e);
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
        Ok((lines, lrc_meta)) => {
            let log_type_str = match lrc_type {
                LrcContentType::Translation => "翻译",
                LrcContentType::Romanization => "罗马音",
            };
            let num_lines = lines.len();

            if !lrc_meta.is_empty() {
                let mut store = app.metadata_store.lock().unwrap();
                for item in &lrc_meta {
                    // item 是 &AssMetadata { key: String, value: String }
                    // 使用 .parse() 并正确处理 Result
                    match item.key.parse::<CanonicalMetadataKey>() {
                        Ok(c_key_from_lrc) => {
                            // c_key_from_lrc 是 CanonicalMetadataKey
                            // 现在 c_key_from_lrc 是解构后的 CanonicalMetadataKey，可以安全使用
                            let should_add = store
                                .get_multiple_values(&c_key_from_lrc) // 传递 &CanonicalMetadataKey
                                .is_none_or(|vals| {
                                    vals.is_empty() || vals.iter().all(|v| v.is_empty())
                                });

                            if should_add {
                                if lrc_type == LrcContentType::Translation
                                    && matches!(c_key_from_lrc, CanonicalMetadataKey::Language)
                                // 现在 c_key_from_lrc 是 CanonicalMetadataKey
                                {
                                    if let Err(e) =
                                        store.add("translation_language", item.value.clone())
                                    {
                                        log::info!(
                                            "Failed to add translation_language from LRC: {}",
                                            e
                                        );
                                    }
                                } else if let Err(e) = store.add(&item.key, item.value.clone()) {
                                    // 使用原始 item.key 字符串给 store.add
                                    log::info!(
                                        "Failed to add metadata key {} from LRC: {}",
                                        item.key,
                                        e
                                    );
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
                LrcContentType::Translation => app.loaded_translation_lrc = Some(lines),
                LrcContentType::Romanization => app.loaded_romanization_lrc = Some(lines),
            }
            log::info!(
                "[Unilyric] 已从 '{}' 加载 {} LRC ({} 行)。",
                source_description,
                log_type_str,
                num_lines
            );

            // 在所有LRC数据（包括元数据）处理完毕后，重建UI的元数据列表
            app.rebuild_editable_metadata_from_store();

            // 如果主歌词段落已存在，或者即使不存在但目标格式是LQE（可能仅依赖LRC和元数据）
            // 则触发合并和可能的重新转换
            if app.parsed_ttml_paragraphs.is_some() || app.target_format == LyricFormat::Lqe {
                log::info!(
                    "[UniLyricApp load_lrc_from_content] 主段落存在或目标为LQE，触发 handle_convert。"
                );
                app.handle_convert();
            } else {
                // 如果没有主歌词，但加载了LRC，至少更新一下LRC显示面板
                log::info!("[UniLyricApp load_lrc_from_content] 无主段落，仅更新LRC预览。");
                app.merge_lrc_into_paragraphs(); // 这个函数会使用 loaded_translation_lrc/loaded_romanization_lrc
                app.generate_target_format_output(); // 更新LRC面板显示
            }
        }
        Err(e) => {
            log::error!(
                "[Unilyric] 从 '{}' 解析LRC内容失败: {}",
                source_description,
                e
            );
        }
    }
}
