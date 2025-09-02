use crate::app_actions::UserAction;
use crate::app_definition::UniLyricApp;
use crate::types::LrcContentType;
use lyrics_helper_rs::{
    providers::kugou::decrypter::decrypt_krc_from_bytes,
    providers::qq::qrc_codec::{decrypt_qrc, decrypt_qrc_local},
};
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
                app.send_action(crate::app_actions::UserAction::Lyrics(Box::new(
                    crate::app_actions::LyricsAction::Convert,
                )));
            }
            Err(e) => {
                tracing::error!("读取LRC文件 {path:?} 失败: {e}");
            }
        }
    }
}

/// 从路径加载文件并触发转换。
pub fn load_file_and_convert(app: &mut UniLyricApp, path: PathBuf) {
    match fs::read(&path) {
        Ok(bytes) => {
            let mut final_content: Option<String> = None;
            let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");

            match extension.to_lowercase().as_str() {
                "krc" => {
                    const KRC_MAGIC_HEADER: &[u8] = b"krc1";
                    if bytes.starts_with(KRC_MAGIC_HEADER) {
                        tracing::info!("[IO] 检测到加密的 KRC 文件，尝试解密...");
                        match decrypt_krc_from_bytes(&bytes) {
                            Ok(decrypted_content) => {
                                tracing::info!("[IO] KRC 解密成功。");
                                final_content = Some(decrypted_content);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "[IO] KRC 解密失败: {}。尝试将文件作为 UTF-8 文本加载。",
                                    e
                                );
                            }
                        }
                    }
                }
                "qrc" => {
                    const QRC_LOCAL_MAGIC_HEADER: &[u8] =
                        &[0x98, 0x25, 0xb0, 0xac, 0xe3, 0x02, 0x83, 0x68, 0xe8, 0xfc];

                    if bytes.starts_with(QRC_LOCAL_MAGIC_HEADER) {
                        tracing::info!("[IO] 检测到本地 QRC 加密文件，尝试解密...");
                        match decrypt_qrc_local(&bytes) {
                            Ok(decrypted_content) => {
                                tracing::info!("[IO] QRC 解密成功。");
                                final_content = Some(decrypted_content);
                            }
                            Err(e) => {
                                tracing::error!("[IO] QRC 解密失败: {}", e);
                            }
                        }
                    } else if let Some(first_char) = bytes.first().map(|&b| b as char)
                        && first_char != '['
                        && first_char != '<'
                        && first_char.is_ascii_alphanumeric()
                    {
                        tracing::info!("[IO] 检测到 HEX 编码的 QRC，尝试解密...");
                        if let Ok(text) = String::from_utf8(bytes.clone()) {
                            match decrypt_qrc(&text) {
                                Ok(decrypted_content) => {
                                    tracing::info!("[IO] QRC 解密成功。");
                                    final_content = Some(decrypted_content);
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "[IO] QRC 解密失败: {}。尝试将文件作为 UTF-8 文本加载。",
                                        e
                                    );
                                }
                            }
                        }
                    }
                }
                _ => {}
            }

            let content_to_load =
                final_content.unwrap_or_else(|| String::from_utf8_lossy(&bytes).to_string());

            app.send_action(UserAction::Lyrics(Box::new(
                crate::app_actions::LyricsAction::LoadFileContent(content_to_load, path),
            )));
        }
        Err(e) => {
            tracing::error!("无法读取文件 {:?}: {}", path, e);
        }
    }
}
