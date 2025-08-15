use std::sync::Arc;

use crate::amll_connector::types::ActorSettings;
use crate::amll_connector::{AMLLConnectorConfig, ConnectorCommand};
use crate::app_actions::{
    AmllConnectorAction, FileAction, LyricsAction, PanelType, PlayerAction, SettingsAction,
    UIAction, UserAction,
};
use crate::app_definition::UniLyricApp;
use crate::app_handlers::ConnectorCommand::SendLyric;
use crate::app_handlers::ConnectorCommand::UpdateActorSettings;
use crate::error::{AppError, AppResult};
use crate::types::{AutoSearchStatus, EditableMetadataEntry, LrcContentType, ProviderState};
use ferrous_opencc::config::BuiltinConfig;
use lyrics_helper_core::{
    ChineseConversionMode, ChineseConversionOptions, ConversionInput, ConversionOptions, InputFile,
    LyricFormat, Track,
};
use rand::Rng;
use smtc_suite::{MediaCommand, TextConversionMode};
use tracing::warn;
use tracing::{debug, error, info};

#[derive(Debug)]
pub enum ActionResult {
    Success,
    Warning(String),
    Error(AppError),
}

impl UniLyricApp {
    fn write_lyrics_file(&self, path: &std::path::Path, content: &str) -> AppResult<()> {
        std::fs::write(path, content).map_err(AppError::from)
    }

    fn validate_media_info(&self) -> AppResult<()> {
        if self.player.current_now_playing.title.is_none() {
            return Err(AppError::Custom("媒体标题为空".to_string()));
        }

        Ok(())
    }

    fn write_cache_index_entry(
        &mut self,
        index_path: &std::path::Path,
        entry: crate::types::LocalLyricCacheEntry,
    ) -> AppResult<()> {
        use std::io::Write;

        let file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(index_path)
            .map_err(AppError::from)?;

        let mut writer = std::io::BufWriter::new(file);
        let json_line = serde_json::to_string(&entry).map_err(AppError::from)?;

        writeln!(writer, "{json_line}").map_err(AppError::from)?;

        self.local_cache.index.lock().unwrap().push(entry);

        Ok(())
    }

    fn generate_safe_filename(&self, title: &str, artists: &[String]) -> String {
        let mut filename = format!("{} - {}", artists.join(", "), title);
        filename = filename
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == ',' || *c == '-')
            .collect();
        format!(
            "{}_{}.ttml",
            filename,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        )
    }

    pub fn trigger_convert(&mut self) {
        self.dispatch_conversion_task(Default::default());
    }

    fn dispatch_conversion_task(&mut self, options: ConversionOptions) {
        if self.lyrics.conversion_in_progress {
            warn!("[Convert] 转换已在进行中，跳过新的请求。");
            return;
        }
        info!("[Convert] 派发新的转换任务，选项: {:?}", options);

        let (tx, rx) = std::sync::mpsc::channel();
        self.lyrics.conversion_result_rx = Some(rx);
        self.lyrics.conversion_in_progress = true;
        let helper = self.lyrics_helper_state.helper.clone();

        // 1. 准备主歌词文件
        let main_lyric = InputFile::new(
            self.lyrics.input_text.clone(),
            self.lyrics.source_format,
            None,
            None,
        );

        // 2. 准备翻译文件列表
        let translations = if !self.lyrics.display_translation_lrc_output.trim().is_empty() {
            vec![InputFile::new(
                self.lyrics.display_translation_lrc_output.clone(),
                LyricFormat::Lrc,
                Some("zh-Hans".to_string()),
                None,
            )]
        } else {
            vec![]
        };

        // 3. 准备罗马音文件列表
        let romanizations = if !self
            .lyrics
            .display_romanization_lrc_output
            .trim()
            .is_empty()
        {
            vec![InputFile::new(
                self.lyrics.display_romanization_lrc_output.clone(),
                LyricFormat::Lrc,
                Some("ja-Latn".to_string()),
                None,
            )]
        } else {
            vec![]
        };

        // 4. 准备用户手动输入的元数据
        let metadata_overrides = if self.lyrics.metadata_is_user_edited {
            let mut overrides = std::collections::HashMap::new();
            for entry in &self.lyrics.editable_metadata {
                if !entry.key.trim().is_empty() && !entry.value.trim().is_empty() {
                    let values = entry
                        .value
                        .split(';')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<String>>();
                    if !values.is_empty() {
                        overrides.insert(entry.key.clone(), values);
                    }
                }
            }
            if overrides.is_empty() {
                None
            } else {
                Some(overrides)
            }
        } else {
            None
        };

        let input = ConversionInput {
            main_lyric,
            translations,
            romanizations,
            target_format: self.lyrics.target_format,
            user_metadata_overrides: metadata_overrides,
        };

        self.tokio_runtime.spawn(async move {
            let result = helper.lock().await.convert_lyrics(&input, &options);
            if tx.send(result).is_err() {
                warn!("[Convert Task] 发送转换结果失败，接收端可能已关闭。");
            }
        });
    }

    // 用于发送事件的辅助函数
    pub fn send_action(&mut self, action: UserAction) {
        self.actions_this_frame.push(action);
    }

    // 主事件处理函数
    pub fn handle_actions(&mut self, actions: Vec<UserAction>) {
        let mut results = Vec::new();

        for action in actions {
            debug!("处理事件: {:?}", std::mem::discriminant(&action));
            results.push(self.handle_single_action(action));
        }

        // 统一处理结果
        for result in results {
            match result {
                ActionResult::Error(msg) => {
                    self.ui.toasts.add(egui_toast::Toast {
                        text: msg.to_string().into(),
                        kind: egui_toast::ToastKind::Error,
                        options: egui_toast::ToastOptions::default().duration_in_seconds(5.0),
                        style: Default::default(),
                    });
                }
                ActionResult::Warning(msg) => {
                    self.ui.toasts.add(egui_toast::Toast {
                        text: msg.into(),
                        kind: egui_toast::ToastKind::Warning,
                        options: egui_toast::ToastOptions::default().duration_in_seconds(3.0),
                        style: Default::default(),
                    });
                }
                ActionResult::Success => {
                    // 成功时通常不需要显示通知
                }
            }
        }
    }

    /// 单个事件处理逻辑
    fn handle_single_action(&mut self, action: UserAction) -> ActionResult {
        match action {
            UserAction::Lyrics(lyrics_action) => self.handle_lyrics_action(*lyrics_action),
            UserAction::File(file_action) => self.handle_file_action(file_action),
            UserAction::UI(ui_action) => self.handle_ui_action(ui_action),
            UserAction::Player(player_action) => self.handle_player_action(player_action),
            UserAction::Settings(settings_action) => self.handle_settings_action(settings_action),
            UserAction::AmllConnector(connector_action) => {
                self.handle_amll_connector_action(connector_action)
            }
        }
    }

    fn handle_amll_connector_action(&mut self, action: AmllConnectorAction) -> ActionResult {
        if let Some(tx) = &self.amll_connector.command_tx {
            let command = match action {
                AmllConnectorAction::Connect | AmllConnectorAction::Retry => {
                    tracing::info!("[AMLL Action] 请求连接...");
                    let mut config = self.amll_connector.config.lock().unwrap();
                    config.enabled = true;

                    Some(ConnectorCommand::UpdateConfig(config.clone()))
                }
                AmllConnectorAction::Disconnect => {
                    tracing::info!("[AMLL Action] 请求断开...");
                    Some(ConnectorCommand::DisconnectWebsocket)
                }
            };

            if let Some(cmd) = command
                && let Err(e) = tx.try_send(cmd)
            {
                tracing::error!("[AMLL Action] 发送命令到 actor 失败: {}", e);
            }
        }
        ActionResult::Success
    }

    /// 子事件处理器
    fn handle_lyrics_action(&mut self, action: LyricsAction) -> ActionResult {
        match action {
            LyricsAction::Convert => {
                self.trigger_convert();
                ActionResult::Success
            }
            LyricsAction::ConvertCompleted(result) => {
                self.lyrics.conversion_in_progress = false;
                match result {
                    Ok(full_result) => {
                        info!("[Convert Result] 转换任务成功完成。");
                        self.lyrics.output_text = full_result.output_lyrics;
                        self.lyrics.parsed_lyric_data = Some(full_result.source_data.clone());

                        if !self.lyrics.metadata_is_user_edited {
                            self.sync_ui_from_parsed_data();
                        }

                        if self.amll_connector.config.lock().unwrap().enabled {
                            if let Some(tx) = &self.amll_connector.command_tx {
                                tracing::info!(
                                    "[AMLL] 转换完成，正在自动发送 TTML 歌词到 Player。"
                                );
                                if tx.try_send(SendLyric(full_result.source_data)).is_err() {
                                    tracing::error!(
                                        "[AMLL] (转换完成时) 发送 TTML 歌词失败 (通道已满或关闭)。"
                                    );
                                }
                            } else {
                                tracing::warn!(
                                    "[AMLL] AMLL Connector 已启用但 command_tx 不可用。"
                                );
                            }
                        }
                        ActionResult::Success
                    }
                    Err(e) => {
                        error!("[Convert Result] 转换任务返回了一个错误: {e}");
                        self.lyrics.output_text.clear();
                        ActionResult::Error(AppError::Custom("转换失败: {e}".to_string()))
                    }
                }
            }
            LyricsAction::ConvertChinese(variant) => {
                info!("[Convert] 请求简繁转换，变体: {variant:?}");

                if self.lyrics.input_text.trim().is_empty()
                    && self.lyrics.parsed_lyric_data.is_none()
                {
                    return ActionResult::Warning("没有歌词内容可以转换".to_string());
                }

                let options = ConversionOptions {
                    chinese_conversion: ChineseConversionOptions {
                        config: Some(variant),
                        mode: ChineseConversionMode::Replace,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                self.dispatch_conversion_task(options);
                ActionResult::Success
            }
            LyricsAction::SourceFormatChanged(format) => {
                info!("源格式改变为: {format:?}");
                self.lyrics.source_format = format;

                // 保存到设置
                if let Ok(mut settings) = self.app_settings.lock() {
                    settings.last_source_format = format;
                    if let Err(e) = settings.save() {
                        return ActionResult::Warning(format!("保存源格式设置失败: {e}"));
                    }
                }

                // 源格式变化时，如果有输入内容且不在转换中，则触发转换
                if !self.lyrics.input_text.trim().is_empty() && !self.lyrics.conversion_in_progress
                {
                    self.trigger_convert();
                }

                ActionResult::Success
            }
            LyricsAction::TargetFormatChanged(format) => {
                info!("目标格式改变为: {format:?}");
                self.lyrics.target_format = format;

                // 保存到设置
                if let Ok(mut settings) = self.app_settings.lock() {
                    settings.last_target_format = format;
                    if let Err(e) = settings.save() {
                        return ActionResult::Warning(format!("保存目标格式设置失败: {e}"));
                    }
                }

                // 目标格式变化时，如果有输入内容且不在转换中，则触发转换
                if !self.lyrics.input_text.trim().is_empty() && !self.lyrics.conversion_in_progress
                {
                    self.trigger_convert();
                }

                ActionResult::Success
            }
            LyricsAction::ClearAllData => {
                self.clear_lyrics_state_for_new_song_internal();
                ActionResult::Success
            }
            LyricsAction::Search => {
                if self.lyrics.search_in_progress {
                    return ActionResult::Warning("搜索正在进行中".to_string());
                }

                match self.lyrics_helper_state.provider_state {
                    ProviderState::Ready => {}
                    ProviderState::Loading => {
                        return ActionResult::Warning("功能正在加载，请稍候...".to_string());
                    }
                    _ => {
                        return ActionResult::Error(AppError::Custom(
                            "在线搜索功能不可用或加载失败。".to_string(),
                        ));
                    }
                }

                let helper = Arc::clone(&self.lyrics_helper_state.helper);

                self.lyrics.search_in_progress = true;
                self.lyrics.search_results.clear(); // 清除旧结果

                let (tx, rx) = std::sync::mpsc::channel();
                self.lyrics.search_result_rx = Some(rx);

                let query = self.lyrics.search_query.clone();

                self.tokio_runtime.spawn(async move {
                    let track_to_search = Track {
                        title: Some(&query),
                        artists: None, // 简化
                        album: None,
                        duration: None,
                    };

                    let helper_clone = Arc::clone(&helper);

                    let result = {
                        let helper_guard = helper_clone.lock().await;
                        helper_guard.search_track(&track_to_search).await
                    };
                    if tx.send(result).is_err() {
                        warn!("[Search Task] 发送搜索结果失败，UI可能已关闭。");
                    }
                });
                ActionResult::Success
            }
            LyricsAction::SearchCompleted(result) => {
                self.lyrics.search_in_progress = false;
                match result {
                    Ok(results) => {
                        info!("[Search] 搜索成功，找到 {} 条结果。", results.len());
                        self.lyrics.search_results = results;
                        ActionResult::Success
                    }
                    Err(e) => {
                        error!("[Search] 搜索任务失败: {e}");
                        ActionResult::Error(AppError::Custom(format!("搜索失败: {e}")))
                    }
                }
            }
            LyricsAction::Download(search_result) => {
                if self.lyrics.download_in_progress {
                    return ActionResult::Warning("下载正在进行中".to_string());
                }

                match self.lyrics_helper_state.provider_state {
                    ProviderState::Ready => {}
                    ProviderState::Loading => {
                        return ActionResult::Warning("正在加载，请稍候...".to_string());
                    }
                    _ => {
                        return ActionResult::Error(AppError::Custom(
                            "下载功能不可用或加载失败。".to_string(),
                        ));
                    }
                }

                let helper = Arc::clone(&self.lyrics_helper_state.helper);

                self.lyrics.download_in_progress = true;

                let (tx, rx) = std::sync::mpsc::channel();
                self.lyrics.download_result_rx = Some(rx);

                let provider_name = search_result.provider_name.clone();
                let provider_id = search_result.provider_id.clone();

                self.tokio_runtime.spawn(async move {
                    let helper_clone = Arc::clone(&helper);

                    let result = {
                        let future_result = {
                            let helper_guard = helper_clone.lock().await;
                            helper_guard.get_full_lyrics(&provider_name, &provider_id)
                        };

                        match future_result {
                            Ok(future) => future.await,
                            Err(e) => Err(e),
                        }
                    };

                    if tx.send(result).is_err() {
                        warn!("[Download Task] 发送下载结果失败，UI可能已关闭。");
                    }
                });
                ActionResult::Success
            }
            LyricsAction::DownloadCompleted(result) => {
                self.lyrics.download_in_progress = false;
                match result {
                    Ok(full_lyrics_result) => {
                        let source_name = full_lyrics_result.parsed.source_name.clone();
                        info!("[Download] 从 {source_name} 下载歌词成功，将立即加载。");

                        self.ui.show_search_window = false;

                        info!("[ProcessFetched] 开始处理已下载的歌词结果。");

                        self.clear_lyrics_state_for_new_song_internal();

                        let parsed_data = full_lyrics_result.parsed;
                        let raw_data = full_lyrics_result.raw;

                        self.lyrics.input_text = raw_data.content;
                        self.lyrics.source_format = parsed_data.source_format;
                        self.fetcher.last_source_format = Some(parsed_data.source_format);
                        self.lyrics.metadata_source_is_download = true;

                        self.trigger_convert();

                        ActionResult::Success
                    }
                    Err(e) => {
                        error!("[Download] 下载任务失败: {e}");
                        ActionResult::Error(AppError::Custom(format!("下载失败: {e}")))
                    }
                }
            }
            LyricsAction::MetadataChanged => {
                self.lyrics.metadata_is_user_edited = true;
                ActionResult::Success
            }
            LyricsAction::AddMetadata => {
                use rand::Rng;
                // 为新条目生成一个相对唯一的ID
                let new_entry_id_num =
                    self.lyrics.editable_metadata.len() as u32 + rand::rng().random::<u32>();

                let new_id = egui::Id::new(format!("new_editable_meta_entry_{new_entry_id_num}"));
                self.lyrics
                    .editable_metadata
                    .push(crate::types::EditableMetadataEntry {
                        key: format!("新键_{}", new_entry_id_num % 100), // 默认键名
                        value: "".to_string(),                           // 默认空值
                        is_pinned: false,                                // 默认不固定
                        is_from_file: false,                             // 新添加的不是来自文件
                        id: new_id,                                      // UI ID
                    });
                self.lyrics.metadata_is_user_edited = true;
                self.trigger_convert();
                ActionResult::Success
            }
            LyricsAction::DeleteMetadata(index) => {
                if index < self.lyrics.editable_metadata.len() {
                    self.lyrics.editable_metadata.remove(index);
                    self.lyrics.metadata_is_user_edited = true;
                    self.trigger_convert();
                    ActionResult::Success
                } else {
                    ActionResult::Error(AppError::Custom("无效的元数据索引".to_string()))
                }
            }
            LyricsAction::UpdateMetadataKey(index, new_key) => {
                if let Some(entry) = self.lyrics.editable_metadata.get_mut(index) {
                    entry.key = new_key;
                    entry.is_from_file = false;
                    self.lyrics.metadata_is_user_edited = true;
                    self.trigger_convert();
                    ActionResult::Success
                } else {
                    ActionResult::Error(AppError::Custom("无效的元数据索引".to_string()))
                }
            }
            LyricsAction::UpdateMetadataValue(index, new_value) => {
                if let Some(entry) = self.lyrics.editable_metadata.get_mut(index) {
                    entry.value = new_value;
                    entry.is_from_file = false;
                    self.lyrics.metadata_is_user_edited = true;
                    self.trigger_convert();
                    ActionResult::Success
                } else {
                    ActionResult::Error(AppError::Custom("无效的元数据索引".to_string()))
                }
            }
            LyricsAction::ToggleMetadataPinned(index) => {
                if let Some(entry) = self.lyrics.editable_metadata.get_mut(index) {
                    entry.is_pinned = !entry.is_pinned;
                    self.lyrics.metadata_is_user_edited = true;
                    self.trigger_convert();
                    ActionResult::Success
                } else {
                    ActionResult::Error(AppError::Custom("无效的元数据索引".to_string()))
                }
            }
            LyricsAction::LoadFetchedResult(result) => {
                // 手动加载的逻辑，无条件执行
                info!("[ProcessFetched] 用户或系统请求加载一个歌词结果。");

                self.fetcher.current_ui_populated = true;

                self.clear_lyrics_state_for_new_song_internal();

                let parsed_data = result.parsed;
                let raw_data = result.raw;

                self.lyrics.input_text = raw_data.content;
                self.lyrics.source_format = parsed_data.source_format;
                self.fetcher.last_source_format = Some(parsed_data.source_format);
                self.lyrics.metadata_source_is_download = true;

                self.trigger_convert();
                ActionResult::Success
            }
            LyricsAction::LrcInputChanged(text, content_type) => {
                let lrc_lines = match lyrics_helper_rs::converter::parsers::lrc_parser::parse_lrc(
                    &text,
                    &Default::default(),
                ) {
                    Ok(parsed) => Some(
                        parsed
                            .lines
                            .into_iter()
                            .map(|line| crate::types::DisplayLrcLine::Parsed(Box::new(line)))
                            .collect(),
                    ),
                    Err(e) => {
                        tracing::warn!("[LRC Edit] LRC文本解析失败: {e}");
                        None
                    }
                };

                match content_type {
                    LrcContentType::Translation => self.lyrics.loaded_translation_lrc = lrc_lines,
                    LrcContentType::Romanization => self.lyrics.loaded_romanization_lrc = lrc_lines,
                }

                // 触发转换
                self.trigger_convert();
                ActionResult::Success
            }
            LyricsAction::MainInputChanged(_text) => {
                // 主输入文本框内容改变，触发转换
                // 但只有在不是转换过程中时才触发，避免无限循环
                if !self.lyrics.conversion_in_progress && !self.lyrics.input_text.trim().is_empty()
                {
                    self.trigger_convert();
                }
                ActionResult::Success
            }
            LyricsAction::ApplyFetchedLyrics(lyrics_and_metadata) => {
                if self.lyrics.conversion_in_progress {
                    return ActionResult::Warning("转换正在进行中".to_string());
                }

                self.fetcher.current_ui_populated = true;
                self.clear_lyrics_state_for_new_song_internal();

                let parsed_data = lyrics_and_metadata.lyrics.parsed;
                let raw_data = lyrics_and_metadata.lyrics.raw;

                self.lyrics.input_text = raw_data.content;
                self.lyrics.source_format = parsed_data.source_format;
                self.fetcher.last_source_format = Some(parsed_data.source_format);
                self.lyrics.metadata_source_is_download = true;

                let (tx, rx) = std::sync::mpsc::channel();
                self.lyrics.conversion_result_rx = Some(rx);
                self.lyrics.conversion_in_progress = true;

                let target_format = self.lyrics.target_format;

                let user_metadata_overrides: Option<
                    std::collections::HashMap<String, Vec<String>>,
                > = None;
                let options = ConversionOptions::default();

                self.tokio_runtime.spawn(async move {
                    let result = lyrics_helper_rs::LyricsHelper::generate_lyrics_from_parsed(
                        parsed_data,
                        target_format,
                        options,
                        user_metadata_overrides,
                    )
                    .await;

                    let converted_result = result.map_err(|e| e.to_string());
                    if tx.send(Ok(converted_result.unwrap())).is_err() {
                        warn!("[Generate Task] 发送结果失败，UI可能已关闭。");
                    }
                });

                ActionResult::Success
            }
        }
    }

    pub(super) fn clear_lyrics_state_for_new_song_internal(&mut self) {
        info!("[State] 正在为新歌曲清理歌词状态。");
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
    }

    fn handle_file_action(&mut self, action: FileAction) -> ActionResult {
        match action {
            FileAction::Open => {
                crate::io::handle_open_file(self);
                ActionResult::Success
            }
            FileAction::Save => {
                crate::io::handle_save_file(self);
                ActionResult::Success
            }
            FileAction::LoadTranslationLrc => {
                crate::io::handle_open_lrc_file(self, LrcContentType::Translation);
                self.trigger_convert();
                ActionResult::Success
            }
            FileAction::LoadRomanizationLrc => {
                crate::io::handle_open_lrc_file(self, LrcContentType::Romanization);
                self.trigger_convert();
                ActionResult::Success
            }
        }
    }

    fn handle_ui_action(&mut self, action: UIAction) -> ActionResult {
        match action {
            UIAction::SetPanelVisibility(panel, is_visible) => {
                let panel_state_mut: &mut bool = match panel {
                    PanelType::Log => &mut self.ui.show_bottom_log_panel,
                    PanelType::Markers => &mut self.ui.show_markers_panel,
                    PanelType::Translation => &mut self.ui.show_translation_lrc_panel,
                    PanelType::Romanization => &mut self.ui.show_romanization_lrc_panel,
                    PanelType::Settings => &mut self.ui.show_settings_window,
                    PanelType::Metadata => &mut self.ui.show_metadata_panel,
                    PanelType::Search => &mut self.ui.show_search_window,
                    PanelType::AmllConnector => &mut self.ui.show_amll_connector_sidebar,
                };

                // 用事件携带的值来更新核心状态
                *panel_state_mut = is_visible;

                if matches!(panel, PanelType::Log) && is_visible {
                    self.ui.new_trigger_log_exists = false;
                }

                ActionResult::Success
            }
            UIAction::ShowPanel(panel) => {
                match panel {
                    PanelType::Log => self.ui.show_bottom_log_panel = true,
                    PanelType::Markers => self.ui.show_markers_panel = true,
                    PanelType::Translation => self.ui.show_translation_lrc_panel = true,
                    PanelType::Romanization => self.ui.show_romanization_lrc_panel = true,
                    PanelType::Settings => {
                        self.ui.temp_edit_settings = self.app_settings.lock().unwrap().clone();
                        self.ui.show_settings_window = true;
                    }
                    PanelType::Metadata => self.ui.show_metadata_panel = true,
                    PanelType::Search => self.ui.show_search_window = true,
                    PanelType::AmllConnector => self.ui.show_amll_connector_sidebar = true,
                }
                ActionResult::Success
            }
            UIAction::HidePanel(panel) => {
                match panel {
                    PanelType::Log => {
                        self.ui.show_bottom_log_panel = false;
                        self.ui.new_trigger_log_exists = false;
                    }
                    PanelType::Markers => self.ui.show_markers_panel = false,
                    PanelType::Translation => self.ui.show_translation_lrc_panel = false,
                    PanelType::Romanization => self.ui.show_romanization_lrc_panel = false,
                    PanelType::Settings => self.ui.show_settings_window = false,
                    PanelType::Metadata => self.ui.show_metadata_panel = false,
                    PanelType::Search => self.ui.show_search_window = false,
                    PanelType::AmllConnector => self.ui.show_amll_connector_sidebar = false,
                }
                ActionResult::Success
            }
            UIAction::SetWrapText(wrap) => {
                self.ui.wrap_text = wrap;
                ActionResult::Success
            }
            UIAction::ClearLogs => {
                self.ui.log_display_buffer.clear();
                ActionResult::Success
            }
            UIAction::StopOtherSearches => {
                self.set_searching_providers_to_not_found();
                ActionResult::Success
            }
        }
    }

    fn handle_player_action(&mut self, action: PlayerAction) -> ActionResult {
        let command_tx = if let Some(tx) = &self.player.command_tx {
            tx.clone()
        } else {
            warn!("[PlayerAction] 无法处理播放器动作，因为 smtc-suite 控制器不可用。");
            return ActionResult::Warning("媒体服务未初始化".to_string());
        };

        let send_result = match action {
            PlayerAction::Control(control_command) => {
                tracing::debug!("[PlayerAction] 发送媒体控制命令: {:?}", control_command);
                command_tx.try_send(MediaCommand::Control(control_command))
            }
            PlayerAction::SelectSmtcSession(session_id) => {
                let session_id_for_state: Option<String> = if session_id.is_empty() {
                    tracing::info!("[PlayerAction] 自动选择会话。");
                    None
                } else {
                    tracing::info!("[PlayerAction] 选择新的 SMTC 会话: {}", session_id);
                    Some(session_id.clone())
                };

                self.player.last_requested_session_id = session_id_for_state.clone();
                if let Ok(mut settings) = self.app_settings.lock() {
                    settings.last_selected_smtc_session_id = session_id_for_state;
                    if let Err(e) = settings.save() {
                        tracing::warn!("[PlayerAction] 保存上次选择的SMTC会话ID失败: {}", e);
                    }
                }
                command_tx.try_send(MediaCommand::SelectSession(session_id))
            }
            PlayerAction::SaveToLocalCache => {
                return match self.save_lyrics_to_local_cache() {
                    Ok(()) => ActionResult::Success,
                    Err(e) => ActionResult::Error(e),
                };
            }
            PlayerAction::UpdateCover(cover_data) => {
                self.player.current_now_playing.cover_data = cover_data.clone();

                if let Some(cover_bytes) = cover_data
                    && let Some(command_tx) = &self.amll_connector.command_tx
                {
                    let send_result = command_tx.try_send(
                        crate::amll_connector::types::ConnectorCommand::SendCover(cover_bytes),
                    );
                    if let Err(e) = send_result {
                        warn!("[PlayerAction] 发送封面到 WebSocket 失败: {}", e);
                    }
                }

                self.egui_ctx.request_repaint();
                return ActionResult::Success;
            }
            PlayerAction::ToggleAudioCapture(enable) => {
                let smtc_command = if enable {
                    MediaCommand::StartAudioCapture
                } else {
                    MediaCommand::StopAudioCapture
                };
                tracing::info!("[PlayerAction] 发送音频捕获命令: {:?}", smtc_command);
                command_tx.try_send(smtc_command)
            }
        };

        if let Err(e) = send_result {
            error!("[PlayerAction] 发送命令到 smtc-suite 失败: {}", e);
            return ActionResult::Error(AppError::Custom("发送命令失败".to_string()));
        }

        ActionResult::Success
    }

    fn save_lyrics_to_local_cache(&mut self) -> AppResult<()> {
        self.validate_media_info()?;

        let cache_dir = self
            .local_cache
            .dir_path
            .as_ref()
            .ok_or_else(|| AppError::Custom("缺少缓存目录路径".to_string()))?
            .clone();
        let index_path = self
            .local_cache
            .index_path
            .as_ref()
            .ok_or_else(|| AppError::Custom("缺少缓存索引路径".to_string()))?
            .clone();

        let media_info = self.player.current_now_playing.clone();

        let title = media_info.title.as_deref().unwrap_or("unknown_title");
        let artists: Vec<String> = media_info
            .artist
            .map(|s| {
                s.split(['/', ';', '、'])
                    .map(|n| n.trim().to_string())
                    .collect()
            })
            .unwrap_or_default();

        let final_filename = self.generate_safe_filename(title, &artists);

        let file_path = cache_dir.join(&final_filename);

        self.write_lyrics_file(&file_path, &self.lyrics.output_text)
            .map_err(|e| {
                tracing::error!("[LocalCache] 写入歌词文件 {file_path:?} 失败: {e}");
                AppError::Custom(format!("写入歌词文件失败: {e}"))
            })?;

        let entry = crate::types::LocalLyricCacheEntry {
            smtc_title: title.to_string(),
            smtc_artists: artists,
            ttml_filename: final_filename,
            original_source_format: self.fetcher.last_source_format.map(|f| f.to_string()),
        };

        self.write_cache_index_entry(&index_path, entry)?;

        tracing::info!("[LocalCache] 成功保存歌词到本地缓存: {file_path:?}");
        self.ui.toasts.add(egui_toast::Toast {
            text: "已保存到本地缓存".into(),
            kind: egui_toast::ToastKind::Success,
            options: egui_toast::ToastOptions::default().duration_in_seconds(2.0),
            style: Default::default(),
        });

        Ok(())
    }

    fn handle_settings_action(&mut self, action: SettingsAction) -> ActionResult {
        match action {
            SettingsAction::Save(settings) => match settings.save() {
                Ok(_) => {
                    let old_audio_capture_setting =
                        self.app_settings.lock().unwrap().send_audio_data_to_player;

                    {
                        let mut app_settings_guard = self.app_settings.lock().unwrap();
                        *app_settings_guard = *settings.clone();
                        self.player.smtc_time_offset_ms = app_settings_guard.smtc_time_offset_ms;
                    }

                    if settings.send_audio_data_to_player != old_audio_capture_setting {
                        self.send_action(UserAction::Player(PlayerAction::ToggleAudioCapture(
                            settings.send_audio_data_to_player,
                        )));
                    }

                    let new_mc_config_from_settings = AMLLConnectorConfig {
                        enabled: settings.amll_connector_enabled,
                        websocket_url: settings.amll_connector_websocket_url.clone(),
                    };

                    let new_actor_settings = ActorSettings {};

                    let conversion_mode = if settings.enable_t2s_for_auto_search {
                        TextConversionMode::TraditionalToSimplified
                    } else {
                        TextConversionMode::Off
                    };

                    if let Some(tx) = &self.player.command_tx {
                        let command = MediaCommand::SetTextConversion(conversion_mode);
                        if let Err(e) = tx.try_send(command) {
                            error!("[Settings] 发送 SetTextConversion 命令失败: {}", e);
                        } else {
                            info!(
                                "[Settings] 已发送 SetTextConversion 命令: {:?}",
                                conversion_mode
                            );
                        }
                    }

                    if let Some(tx) = &self.amll_connector.command_tx {
                        debug!(
                            "[Settings] 发送 UpdateActorSettings 命令给 AMLL Connector worker。"
                        );
                        if tx
                            .try_send(UpdateActorSettings(new_actor_settings))
                            .is_err()
                        {
                            error!(
                                "[Settings] 发送 UpdateActorSettings 命令给 AMLL Connector worker 失败。"
                            );
                        }
                    }

                    let old_mc_config = self.amll_connector.config.lock().unwrap().clone();
                    if new_mc_config_from_settings.enabled
                        && old_mc_config != new_mc_config_from_settings
                        && let Some(tx) = &self.amll_connector.command_tx
                    {
                        debug!("[Settings] 发送 UpdateConfig 命令给AMLL Connector worker。");
                        if tx
                            .try_send(crate::amll_connector::ConnectorCommand::UpdateConfig(
                                new_mc_config_from_settings.clone(),
                            ))
                            .is_err()
                        {
                            error!(
                                "[Settings] 发送 UpdateConfig 命令给AMLL Connector worker 失败。"
                            );
                        }
                    }

                    *self.amll_connector.config.lock().unwrap() = new_mc_config_from_settings;

                    self.ui.show_settings_window = false;
                    ActionResult::Success
                }
                Err(e) => {
                    error!("[Settings] 保存应用设置失败: {e}");
                    self.ui.show_settings_window = false;
                    ActionResult::Error(AppError::Custom(format!("保存设置失败: {e}")))
                }
            },
            SettingsAction::Cancel => {
                self.ui.show_settings_window = false;
                ActionResult::Success
            }
            SettingsAction::Reset => {
                self.ui.temp_edit_settings = self.app_settings.lock().unwrap().clone();
                ActionResult::Success
            }
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
                        id: egui::Id::new(format!("meta_entry_{}", rand::rng().random::<u64>())),
                    });
                }
            }

            // 步骤 4: 用合并后的新列表替换旧列表
            self.lyrics.editable_metadata = new_metadata;
        }
    }

    pub(super) fn draw_chinese_conversion_menu_item(
        &mut self,
        ui: &mut egui::Ui,
        variant: BuiltinConfig,
        label: &str,
        enabled: bool,
    ) {
        if ui
            .add_enabled(enabled, egui::Button::new(label))
            .on_disabled_hover_text("请先加载主歌词")
            .clicked()
        {
            self.send_action(UserAction::Lyrics(Box::new(LyricsAction::ConvertChinese(
                variant,
            ))));
        }
    }

    pub fn trigger_provider_loading(&mut self) {
        if self.lyrics_helper_state.provider_state != ProviderState::Uninitialized {
            return;
        }

        info!("[LyricsHelper] 正在加载提供商...");
        self.lyrics_helper_state.provider_state = ProviderState::Loading;

        let (tx, rx) = std::sync::mpsc::channel();
        self.lyrics_helper_state.provider_load_result_rx = Some(rx);

        let helper_clone = Arc::clone(&self.lyrics_helper_state.helper);

        self.tokio_runtime.spawn(async move {
            let result = match helper_clone.lock().await.load_providers().await {
                Ok(_) => Ok(()),
                Err(e) => Err(e.to_string()),
            };

            if tx.send(result).is_err() {
                warn!("[LyricsHelper Task] 发送提供商加载结果失败，UI可能已关闭。");
            }
        });
    }

    pub(super) fn set_searching_providers_to_not_found(&mut self) {
        let all_search_status_arcs = [
            &self.fetcher.local_cache_status,
            &self.fetcher.qqmusic_status,
            &self.fetcher.kugou_status,
            &self.fetcher.netease_status,
            &self.fetcher.amll_db_status,
        ];

        for status_arc in all_search_status_arcs {
            let mut guard = status_arc.lock().unwrap();
            if matches!(*guard, AutoSearchStatus::Searching) {
                *guard = AutoSearchStatus::NotFound;
            }
        }
    }
}
