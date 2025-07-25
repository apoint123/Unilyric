use crate::amll_connector::amll_connector_manager::{self};
use crate::app_definition::UniLyricApp;
use crate::app_update;
use crate::types::{AutoSearchSource, EditableMetadataEntry};
use eframe::egui::{self};
use log::{info, warn};
use lyrics_helper_rs::SearchResult;
use lyrics_helper_rs::model::track::FullLyricsResult;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

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

        if self.conversion_in_progress {
            warn!("[Convert] Conversion already in progress, skipping new request.");
            return;
        }

        if let Some(helper) = self.lyrics_helper.as_ref() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.conversion_result_rx = Some(rx);
            self.conversion_in_progress = true;

            let helper = helper.clone();

            // 1. 准备主歌词文件
            let main_lyric = lyrics_helper_rs::converter::types::InputFile::new(
                self.input_text.clone(),
                self.source_format,
                None,
                None,
            );

            // 2. 准备翻译文件列表
            let mut translations = vec![];
            if !self.display_translation_lrc_output.trim().is_empty() {
                translations.push(lyrics_helper_rs::converter::types::InputFile::new(
                    self.display_translation_lrc_output.clone(),
                    lyrics_helper_rs::converter::types::LyricFormat::Lrc,
                    Some("zh-Hans".to_string()),
                    None,
                ));
            }

            // 3. 准备罗马音文件列表
            let mut romanizations = vec![];
            if !self.display_romanization_lrc_output.trim().is_empty() {
                romanizations.push(lyrics_helper_rs::converter::types::InputFile::new(
                    self.display_romanization_lrc_output.clone(),
                    lyrics_helper_rs::converter::types::LyricFormat::Lrc,
                    Some("ja-Latn".to_string()),
                    None,
                ));
            }

            // 4. 准备用户手动输入的元数据
            let mut metadata_overrides = std::collections::HashMap::new();
            for entry in &self.editable_metadata {
                if !entry.key.trim().is_empty() {
                    // 将UI中的单个字符串值按分号分割回Vec<String>
                    let values = entry
                        .value
                        .split(';')
                        .map(|s| s.trim().to_string())
                        .collect();
                    metadata_overrides.insert(entry.key.clone(), values);
                }
            }

            let input = lyrics_helper_rs::converter::types::ConversionInput {
                main_lyric,
                translations,
                romanizations,
                target_format: self.target_format,
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
            self.conversion_in_progress = false;
        }
    }

    /// 从解析后的数据（`self.parsed_lyric_data`）同步UI相关的状态。
    /// 例如，更新元数据编辑器。
    pub fn sync_ui_from_parsed_data(&mut self) {
        if let Some(data) = &self.parsed_lyric_data {
            info!("同步UI与解析出的新元数据...");

            // 步骤1：获取当前已存在的元数据键集合
            let existing_keys: std::collections::HashSet<String> = self
                .editable_metadata
                .iter()
                .map(|entry| entry.key.clone())
                .collect();

            // 步骤2：遍历从新文件中解析出的元数据
            for (key, values) in &data.raw_metadata {
                // 如果这个键已经存在于我们的列表中（很可能是因为被固定了），则跳过
                if existing_keys.contains(key) {
                    continue;
                }

                // 否则，将这个新条目添加到可编辑列表中
                self.editable_metadata.push(EditableMetadataEntry {
                    key: key.clone(),
                    value: values.join("; "),
                    is_pinned: false,
                    is_from_file: true,
                    id: egui::Id::new(format!("meta_entry_{}", key)),
                });
            }
        }
    }

    /// 触发一个异步的歌词搜索任务。
    pub fn trigger_lyrics_search(&mut self) {
        if self.search_in_progress {
            return;
        }

        let helper = match self.lyrics_helper.as_ref() {
            Some(h) => Arc::clone(h),
            None => {
                warn!("[Search] LyricsHelper 未初始化，无法搜索。");
                return;
            }
        };

        self.search_in_progress = true;
        self.search_results.clear(); // 清除旧结果

        let (tx, rx) = std::sync::mpsc::channel();
        self.search_result_rx = Some(rx);

        let query = self.search_query.clone();

        self.tokio_runtime.spawn(async move {
            let track_to_search = lyrics_helper_rs::model::track::Track {
                title: Some(&query),
                artists: None, // 简化：手动搜索时，通常只使用标题
                album: None,
            };

            // 调用核心库的 search_track 函数
            let result = helper.search_track(&track_to_search).await;
            if tx.send(result).is_err() {
                warn!("[Search Task] 发送搜索结果失败，UI可能已关闭。");
            }
        });
    }

    /// 根据用户选择的 SearchResult，触发一个异步的歌词下载任务。
    pub fn trigger_lyrics_download(&mut self, result_to_download: &SearchResult) {
        if self.download_in_progress {
            return;
        }

        let helper = match self.lyrics_helper.as_ref() {
            Some(h) => Arc::clone(h),
            None => {
                warn!("[Download] LyricsHelper 未初始化，无法下载。");
                return;
            }
        };

        self.download_in_progress = true;

        let (tx, rx) = std::sync::mpsc::channel();
        self.download_result_rx = Some(rx);

        let provider_name = result_to_download.provider_name.clone();
        let provider_id = result_to_download.provider_id.clone();

        self.tokio_runtime.spawn(async move {
            // 调用核心库的 get_full_lyrics 函数
            let result = helper.get_full_lyrics(&provider_name, &provider_id).await;
            if tx.send(result).is_err() {
                warn!("[Download Task] 发送下载结果失败，UI可能已关闭。");
            }
        });
    }

    /// 处理任何来源获取到的 `FullLyricsResult` 的通用函数。
    ///
    /// 它会清空当前状态，用获取到的歌词填充UI，然后触发转换。
    pub fn process_fetched_lyrics(
        &mut self,
        source: AutoSearchSource,
        full_lyrics_result: FullLyricsResult,
    ) {
        info!("[ProcessFetched] 处理来自 {:?} 的歌词", source);
        self.clear_all_data();

        let parsed_data = full_lyrics_result.parsed;
        let raw_data = full_lyrics_result.raw;

        // 使用获取到的原始文本和格式填充状态
        self.input_text = raw_data.content;
        self.source_format = parsed_data.source_format;
        self.last_auto_fetch_source_format = Some(parsed_data.source_format);

        self.metadata_source_is_download = true;

        // 触发应用的内部转换流水线
        self.handle_convert();
    }

    /// 清除所有与当前歌词相关的数据和状态。
    pub fn clear_all_data(&mut self) {
        self.input_text.clear();
        self.output_text.clear();
        self.display_translation_lrc_output.clear();
        self.display_romanization_lrc_output.clear();
        self.parsed_lyric_data = None;
        self.loaded_translation_lrc = None;
        self.loaded_romanization_lrc = None;
        self.current_markers.clear();

        self.editable_metadata.retain(|entry| entry.is_pinned);
        for entry in &mut self.editable_metadata {
            entry.is_from_file = false;
        }
    }

    pub fn stop_progress_timer(&mut self) {
        if let Some(shutdown_tx) = self.progress_timer_shutdown_tx.take() {
            let _ = shutdown_tx.send(());
            if let Some(handle) = self.progress_timer_join_handle.take() {
                let _ = self.tokio_runtime.block_on(handle);
            }
            log::trace!("[ProgressTimer] 进度模拟定时器已停止。");
        }
    }

    pub fn start_progress_timer_if_needed(&mut self) {
        let is_playing = self.is_currently_playing_sensed_by_smtc;
        let connector_enabled = self.media_connector_config.lock().unwrap().enabled;

        if connector_enabled && is_playing && self.progress_timer_shutdown_tx.is_none() {
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
            self.progress_timer_shutdown_tx = Some(shutdown_tx);

            let update_tx = self.media_connector_update_tx_for_worker.clone();
            let interval = self.progress_simulation_interval;
            let media_info_arc = Arc::clone(&self.current_media_info);

            let command_tx = self.media_connector_command_tx.clone().unwrap(); // 确保此时 command_tx 存在
            let config_arc = Arc::clone(&self.media_connector_config);

            let task_handle =
                self.tokio_runtime
                    .spawn(amll_connector_manager::run_progress_timer_task(
                        interval,
                        media_info_arc,
                        command_tx,
                        config_arc,
                        shutdown_rx,
                        update_tx,
                    ));

            self.progress_timer_join_handle = Some(task_handle);
            log::trace!("[ProgressTimer] 进度模拟定时器已启动。");
        } else if !is_playing {
            self.stop_progress_timer();
        }
    }

    pub fn process_smtc_update_for_websocket(
        &mut self,
        _track_info: &crate::amll_connector::NowPlayingInfo,
    ) {
        // TODO: 实现将SMTC更新发送到WebSocket客户端的逻辑
    }

    pub fn send_time_update_to_websocket(&mut self, _time_ms: u64) {
        // TODO: 实现将时间更新发送到WebSocket客户端的逻辑
    }

    pub fn load_lyrics_from_stored_result(
        &mut self,
        source: AutoSearchSource,
        result: FullLyricsResult,
    ) {
        self.process_fetched_lyrics(source, result);
    }

    /// 将当前输出框中的TTML歌词和SMTC元数据保存到本地缓存。
    pub fn save_current_lyrics_to_local_cache(&mut self) {
        let (media_info, cache_dir, index_path) = match (
            self.tokio_runtime
                .block_on(async { self.current_media_info.lock().await.clone() }),
            self.local_lyrics_cache_dir_path.as_ref(),
            self.local_lyrics_cache_index_path.as_ref(),
        ) {
            (Some(info), Some(dir), Some(path)) => (info, dir, path),
            _ => {
                log::warn!("[LocalCache] 缺少SMTC信息或缓存路径，无法保存。");
                self.toasts.add(egui_toast::Toast {
                    text: "缺少SMTC信息，无法保存到缓存".into(),
                    kind: egui_toast::ToastKind::Warning,
                    options: egui_toast::ToastOptions::default().duration_in_seconds(3.0),
                    style: Default::default(),
                });
                return;
            }
        };

        let title = media_info.title.as_deref().unwrap_or("unknown_title");
        let artists: Vec<String> = media_info
            .artist
            .map(|s| s.split('/').map(|n| n.trim().to_string()).collect())
            .unwrap_or_default();

        let mut filename = format!("{} - {}", artists.join(", "), title);
        filename = filename
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == ',' || *c == '-')
            .collect();
        let final_filename = format!(
            "{}_{}.ttml",
            filename,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        );

        let file_path = cache_dir.join(&final_filename);

        if let Err(e) = std::fs::write(&file_path, &self.output_text) {
            log::error!("[LocalCache] 写入歌词文件 {:?} 失败: {}", file_path, e);
            return;
        }

        let entry = crate::types::LocalLyricCacheEntry {
            smtc_title: title.to_string(),
            smtc_artists: artists,
            ttml_filename: final_filename,
            original_source_format: self.last_auto_fetch_source_format.map(|f| f.to_string()),
        };

        match std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(index_path)
        {
            Ok(file) => {
                let mut writer = std::io::BufWriter::new(file);
                if let Ok(json_line) = serde_json::to_string(&entry) {
                    if writeln!(writer, "{}", json_line).is_ok() {
                        self.local_lyrics_cache_index.lock().unwrap().push(entry);
                        log::info!("[LocalCache] 成功保存歌词到本地缓存: {:?}", file_path);
                        self.toasts.add(egui_toast::Toast {
                            text: "已保存到本地缓存".into(),
                            kind: egui_toast::ToastKind::Success,
                            options: egui_toast::ToastOptions::default().duration_in_seconds(2.0),
                            style: Default::default(),
                        });
                    }
                }
            }
            Err(e) => {
                log::error!(
                    "[LocalCache] 打开或写入索引文件 {:?} 失败: {}",
                    index_path,
                    e
                );
            }
        }
    }

    /// 处理特定于简繁转换的请求。
    pub fn handle_chinese_conversion(&mut self, config_name: &str) {
        info!(
            "[Convert] Starting Chinese conversion with config: {}",
            config_name
        );

        if self.conversion_in_progress {
            warn!("[Convert] Conversion already in progress, skipping new request.");
            return;
        }

        // 确保有内容可以转换
        if self.input_text.trim().is_empty() && self.parsed_lyric_data.is_none() {
            warn!("[Convert] No lyrics content to perform Chinese conversion on.");
            return;
        }

        if let Some(helper) = self.lyrics_helper.as_ref() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.conversion_result_rx = Some(rx);
            self.conversion_in_progress = true;

            let helper = helper.clone();

            // 这部分逻辑与 handle_convert 完全相同
            let main_lyric = lyrics_helper_rs::converter::types::InputFile::new(
                self.input_text.clone(),
                self.source_format,
                None,
                None,
            );
            let mut translations = vec![];
            if !self.display_translation_lrc_output.trim().is_empty() {
                translations.push(lyrics_helper_rs::converter::types::InputFile::new(
                    self.display_translation_lrc_output.clone(),
                    lyrics_helper_rs::converter::types::LyricFormat::Lrc,
                    Some("zh-Hans".to_string()),
                    None,
                ));
            }
            let mut romanizations = vec![];
            if !self.display_romanization_lrc_output.trim().is_empty() {
                romanizations.push(lyrics_helper_rs::converter::types::InputFile::new(
                    self.display_romanization_lrc_output.clone(),
                    lyrics_helper_rs::converter::types::LyricFormat::Lrc,
                    Some("ja-Latn".to_string()),
                    None,
                ));
            }
            let mut metadata_overrides = std::collections::HashMap::new();
            for entry in &self.editable_metadata {
                if !entry.key.trim().is_empty() {
                    let values = entry
                        .value
                        .split(';')
                        .map(|s| s.trim().to_string())
                        .collect();
                    metadata_overrides.insert(entry.key.clone(), values);
                }
            }
            let input = lyrics_helper_rs::converter::types::ConversionInput {
                main_lyric,
                translations,
                romanizations,
                target_format: self.target_format,
                user_metadata_overrides: if metadata_overrides.is_empty() {
                    None
                } else {
                    Some(metadata_overrides)
                },
            };

            let options = lyrics_helper_rs::converter::types::ConversionOptions {
                chinese_conversion: lyrics_helper_rs::converter::types::ChineseConversionOptions {
                    config_name: Some(config_name.to_string()),
                    mode: lyrics_helper_rs::converter::types::ChineseConversionMode::Replace,
                    ..Default::default()
                },
                ..Default::default()
            };

            self.tokio_runtime.spawn(async move {
                let result = helper.convert_lyrics(input, &options).await;
                if tx.send(result).is_err() {
                    warn!("[Convert Task] Failed to send conversion result. Receiver probably dropped.");
                }
                
        });
        }
    }
}

impl eframe::App for UniLyricApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

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
        app_update::process_connector_updates(self);
        app_update::handle_auto_fetch_results(self);

        let mut desired_repaint_delay = Duration::from_millis(1000);
        if self.media_connector_config.lock().unwrap().enabled {
            desired_repaint_delay = desired_repaint_delay.min(Duration::from_millis(500));
        }
        ctx.request_repaint_after(desired_repaint_delay);

        app_update::draw_ui_elements(self, ctx);
        app_update::handle_file_drops(self, ctx);
        app_update::handle_ttml_db_upload_actions(self);

        self.draw_search_lyrics_window(ctx);
        self.toasts.show(ctx);

        if ctx.input(|i| i.viewport().close_requested()) && !self.shutdown_initiated {
            self.shutdown_initiated = true;
            log::trace!("[UniLyricApp 更新循环] 检测到窗口关闭请求。正在启动关闭序列...");

            if let Some(tx) = &self.media_connector_command_tx {
                if tx
                    .send(crate::amll_connector::ConnectorCommand::Shutdown)
                    .is_err()
                {
                    log::warn!("[UniLyricApp] 向 AMLL Connector Worker 发送 Shutdown 命令失败。");
                }
            }
            if let Some(ws_tx) = self.websocket_server_command_tx.take() {
                self.tokio_runtime.spawn(async move {
                    if ws_tx
                        .send(crate::websocket_server::ServerCommand::Shutdown)
                        .await
                        .is_err()
                    {
                        log::warn!("[UniLyricApp] 向 WebSocket 服务器任务发送 Shutdown 命令失败。");
                    }
                });
            }
        }
    }
}
