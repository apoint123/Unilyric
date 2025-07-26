use crate::app_definition::UniLyricApp;
use crate::types::LrcContentType;
use lyrics_helper_rs::converter::types::LyricFormat;
use std::fs;
use std::path::PathBuf;

/// 处理打开主歌词文件的逻辑。
pub fn handle_open_file(app: &mut UniLyricApp) {
    if let Some(path) = rfd::FileDialog::new().pick_file() {
        load_file_and_convert(app, path);
    }
}

/// 处理保存输出文件的逻辑。
pub fn handle_save_file(app: &mut UniLyricApp) {
    if let Some(path) = rfd::FileDialog::new()
        .set_file_name("lyrics")
        .add_filter(
            format!(
                "{} file",
                app.lyrics.target_format.to_extension_str().to_uppercase()
            ),
            &[app.lyrics.target_format.to_extension_str()],
        )
        .save_file()
    {
        if let Err(e) = fs::write(&path, &app.lyrics.output_text) {
            tracing::error!("保存文件 {path:?} 失败: {e}");
        } else {
            app.lyrics.last_saved_file_path = Some(path);
        }
    }
}

/// 处理打开翻译或罗马音LRC文件的逻辑。
pub fn handle_open_lrc_file(app: &mut UniLyricApp, content_type: LrcContentType) {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("LRC File", &["lrc"])
        .pick_file()
    {
        match fs::read_to_string(&path) {
            Ok(content) => {
                match content_type {
                    LrcContentType::Translation => {
                        app.lyrics.display_translation_lrc_output = content;
                        tracing::info!("已加载翻译LRC文件: {path:?}");
                    }
                    LrcContentType::Romanization => {
                        app.lyrics.display_romanization_lrc_output = content;
                        tracing::info!("已加载罗马音LRC文件: {path:?}");
                    }
                }
                app.send_action(crate::app_actions::UserAction::Lyrics(
                    crate::app_actions::LyricsAction::Convert,
                ));
            }
            Err(e) => {
                tracing::error!("读取LRC文件 {path:?} 失败: {e}");
            }
        }
    }
}

/// 从路径加载文件并触发转换。
pub fn load_file_and_convert(app: &mut UniLyricApp, path: PathBuf) {
    app.send_action(crate::app_actions::UserAction::Lyrics(
        crate::app_actions::LyricsAction::ClearAllData,
    ));
    app.lyrics.last_opened_file_path = Some(path.clone());

    if let Ok(content) = fs::read_to_string(&path) {
        app.lyrics.input_text = content;
        // 尝试从文件扩展名推断源格式
        if let Some(ext) = path.extension().and_then(|s| s.to_str())
            && let Some(format) = LyricFormat::from_string(ext) {
                app.lyrics.source_format = format;
            }
        app.send_action(crate::app_actions::UserAction::Lyrics(
            crate::app_actions::LyricsAction::Convert,
        ));

        // sync_ui_from_parsed_data 会在转换完成后自动调用
    } else {
        tracing::error!("无法读取文件内容: {path:?}");
    }
}
