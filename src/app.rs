use crate::app_definition::UniLyricApp;
use crate::app_update;
use crate::types::{AutoSearchSource, EditableMetadataEntry};
use eframe::egui::{self};
use lyrics_helper_rs::model::track::FullLyricsResult;
use rand::Rng;
use std::time::Duration;
use tracing::{info, warn};

/// TTML 数据库上传用户操作的枚举
#[derive(Clone, Debug)]
pub enum TtmlDbUploadUserAction {
    /// dpaste 已创建，URL已复制到剪贴板，这是打开Issue页面的URL
    PasteReadyAndCopied {
        paste_url: String,                // dpaste 的 URL
        github_issue_url_to_open: String, // GitHub Issue 页面的 URL
    },
    /// 过程中的提示信息
    InProgressUpdate(String),
    /// 准备阶段错误
    PreparationError(String),
    /// 错误信息
    Error(String),
}

// UniLyricApp 的实现块
impl UniLyricApp {
    pub fn handle_convert(&mut self) {
        info!("[Convert] New conversion handler starting.");

        if self.lyrics.conversion_in_progress {
            warn!("[Convert] Conversion already in progress, skipping new request.");
            return;
        }

        if let Some(helper) = self.lyrics_helper.as_ref() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.lyrics.conversion_result_rx = Some(rx);
            self.lyrics.conversion_in_progress = true;

            let helper = helper.clone();

            // 1. 准备主歌词文件
            let main_lyric = lyrics_helper_rs::converter::types::InputFile::new(
                self.lyrics.input_text.clone(),
                self.lyrics.source_format,
                None,
                None,
            );

            // 2. 准备翻译文件列表
            let mut translations = vec![];
            if !self.lyrics.display_translation_lrc_output.trim().is_empty() {
                translations.push(lyrics_helper_rs::converter::types::InputFile::new(
                    self.lyrics.display_translation_lrc_output.clone(),
                    lyrics_helper_rs::converter::types::LyricFormat::Lrc,
                    Some("zh-Hans".to_string()),
                    None,
                ));
            }

            // 3. 准备罗马音文件列表
            let mut romanizations = vec![];
            if !self
                .lyrics
                .display_romanization_lrc_output
                .trim()
                .is_empty()
            {
                romanizations.push(lyrics_helper_rs::converter::types::InputFile::new(
                    self.lyrics.display_romanization_lrc_output.clone(),
                    lyrics_helper_rs::converter::types::LyricFormat::Lrc,
                    Some("ja-Latn".to_string()),
                    None,
                ));
            }

            // 4. 准备用户手动输入的元数据
            // 只有当用户明确编辑了元数据时，才将其作为覆盖传递给转换器
            let mut metadata_overrides = std::collections::HashMap::new();

            info!(
                "[Convert] 元数据状态检查: metadata_is_user_edited={}, editable_metadata.len()={}",
                self.lyrics.metadata_is_user_edited,
                self.lyrics.editable_metadata.len()
            );

            // 只有在用户明确编辑了元数据的情况下，才使用覆盖
            if self.lyrics.metadata_is_user_edited {
                for entry in &self.lyrics.editable_metadata {
                    info!(
                        "[Convert] 检查元数据条目: key='{}', value='{}', is_from_file={}, is_pinned={}",
                        entry.key, entry.value, entry.is_from_file, entry.is_pinned
                    );

                    if !entry.key.trim().is_empty() && !entry.value.trim().is_empty() {
                        // 将UI中的单个字符串值按分号分割回Vec<String>
                        let values = entry
                            .value
                            .split(';')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<String>>();

                        if !values.is_empty() {
                            info!(
                                "[Convert] 添加元数据覆盖: key='{}', values={:?}",
                                entry.key, values
                            );
                            metadata_overrides.insert(entry.key.clone(), values);
                        }
                    }
                }
            }

            info!("[Convert] 最终元数据覆盖: {metadata_overrides:?}");

            let input = lyrics_helper_rs::converter::types::ConversionInput {
                main_lyric,
                translations,
                romanizations,
                target_format: self.lyrics.target_format,
                user_metadata_overrides: if metadata_overrides.is_empty() {
                    None
                } else {
                    Some(metadata_overrides)
                },
            };

            let options = lyrics_helper_rs::converter::types::ConversionOptions::default();

            self.tokio_runtime.spawn(async move {
                let result = helper.convert_lyrics(input, &options).await;
                if tx.send(result).is_err() {
                    warn!("[Convert Task] Failed to send conversion result. Receiver probably dropped.");
                }

        });
        } else {
            warn!("[Convert] LyricsHelper not available for conversion.");
            self.lyrics.conversion_in_progress = false;
        }
    }

    /// 从解析后的数据（`self.lyrics.parsed_lyric_data`）同步UI相关的状态。
    /// 例如，更新元数据编辑器。
    pub fn sync_ui_from_parsed_data(&mut self) {
        if let Some(data) = &self.lyrics.parsed_lyric_data {
            info!("正在根据最新的转换结果同步元数据UI...");

            // 步骤 1: 保留所有被用户固定的条目
            let mut new_metadata: Vec<EditableMetadataEntry> = self
                .lyrics
                .editable_metadata
                .iter()
                .filter(|entry| entry.is_pinned)
                .cloned()
                .collect();

            // 步骤 2: 获取已固定条目的键，避免重复添加
            let pinned_keys: std::collections::HashSet<String> =
                new_metadata.iter().map(|entry| entry.key.clone()).collect();

            // 步骤 3: 遍历从新数据中解析出的元数据
            for (key, values) in &data.raw_metadata {
                // 如果这个键没有被固定，就添加它
                if !pinned_keys.contains(key) {
                    new_metadata.push(EditableMetadataEntry {
                        key: key.clone(),
                        value: values.join("; "),
                        is_pinned: false,
                        is_from_file: true,
                        id: egui::Id::new(format!(
                            "meta_entry_{}",
                            rand::thread_rng().r#gen::<u64>()
                        )),
                    });
                }
            }

            // 步骤 4: 用合并后的新列表替换旧列表
            self.lyrics.editable_metadata = new_metadata;
        }
    }

    pub fn load_lyrics_from_stored_result(
        &mut self,
        source: AutoSearchSource,
        result: FullLyricsResult,
    ) {
        // 处理获取的歌词 - 内联实现
        info!("[ProcessFetched] 处理来自 {source:?} 的歌词");

        // 直接清空数据，不发送事件以避免异步问题
        self.lyrics.input_text.clear();
        self.lyrics.output_text.clear();
        self.lyrics.display_translation_lrc_output.clear();
        self.lyrics.display_romanization_lrc_output.clear();
        self.lyrics.parsed_lyric_data = None;
        self.lyrics.loaded_translation_lrc = None;
        self.lyrics.loaded_romanization_lrc = None;
        self.lyrics.current_markers.clear();
        self.lyrics.metadata_is_user_edited = false;
        self.lyrics
            .editable_metadata
            .retain(|entry| entry.is_pinned);
        for entry in &mut self.lyrics.editable_metadata {
            entry.is_from_file = false;
        }

        let parsed_data = result.parsed;
        let raw_data = result.raw;

        // 使用获取到的原始文本和格式填充状态
        self.lyrics.input_text = raw_data.content;
        self.lyrics.source_format = parsed_data.source_format;
        self.fetcher.last_source_format = Some(parsed_data.source_format);

        self.lyrics.metadata_source_is_download = true;

        // 触发应用的内部转换流水线
        self.trigger_convert();
    }
}

impl eframe::App for UniLyricApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 仅供调试，不要开启！
        // ctx.set_debug_on_hover(true);

        if self.lyrics_helper.is_none() {
            if let Ok(helper) = self.lyrics_helper_rx.try_recv() {
                info!("[UniLyricApp] LyricsHelper 已成功初始化并接收。");
                self.lyrics_helper = Some(helper);
            } else {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label("正在初始化核心模块...");
                    });
                });
                return;
            }
        }

        app_update::handle_conversion_results(self);
        app_update::handle_search_results(self);
        app_update::handle_download_results(self);

        app_update::process_log_messages(self);
        app_update::process_smtc_updates(self);
        app_update::handle_auto_fetch_results(self);

        app_update::process_connector_updates(self);

        // let mut desired_repaint_delay = Duration::from_millis(1000);
        // if self.amll_connector.config.lock().unwrap().enabled {
        //     desired_repaint_delay = desired_repaint_delay.min(Duration::from_millis(100));
        // }
        // ctx.request_repaint_after(desired_repaint_delay);

        ctx.request_repaint_after(Duration::from_millis(1000));

        app_update::draw_ui_elements(self, ctx);
        app_update::handle_file_drops(self, ctx);
        app_update::handle_ttml_db_upload_actions(self);

        self.draw_search_lyrics_window(ctx);
        self.ui.toasts.show(ctx);

        let actions = std::mem::take(&mut self.actions_this_frame);

        if !actions.is_empty() {
            self.handle_actions(actions);
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.shutdown_initiated {
            self.shutdown_initiated = true;
            tracing::trace!("[UniLyricApp 更新循环] 检测到窗口关闭请求。正在启动关闭序列...");

            self.shutdown_amll_actor();
        }
    }
}
