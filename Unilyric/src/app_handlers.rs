use std::fmt::Write;
use std::sync::Arc;

use crate::amll_connector::types::ActorSettings;
use crate::amll_connector::{AMLLConnectorConfig, ConnectorCommand};
use crate::app_actions::{
    AmllConnectorAction, FileAction, LyricsAction, PanelType, PlayerAction, ProcessorType,
    SettingsAction, UIAction, UserAction,
};
use crate::app_definition::UniLyricApp;
use crate::app_handlers::ConnectorCommand::SendLyric;
use crate::app_handlers::ConnectorCommand::UpdateActorSettings;
use crate::app_settings::AppAmllMirror;
use crate::error::{AppError, AppResult};
use crate::types::{AutoSearchStatus, LrcContentType, ProviderState};
use ferrous_opencc::config::BuiltinConfig;
use lyrics_helper_core::{
    ChineseConversionMode, ChineseConversionOptions, ContentType, ConversionInput,
    ConversionOptions, InputFile, LyricFormat, LyricTrack, Track,
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

#[derive(serde::Serialize, serde::Deserialize)]
struct CoreAmllConfig {
    mirror: CoreAmllMirror,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum CoreAmllMirror {
    GitHub,
    Dimeta,
    Bikonoo,
    Custom {
        index_url: String,
        lyrics_url_template: String,
    },
}
impl From<AppAmllMirror> for CoreAmllMirror {
    fn from(value: AppAmllMirror) -> Self {
        match value {
            AppAmllMirror::GitHub => Self::GitHub,
            AppAmllMirror::Dimeta => Self::Dimeta,
            AppAmllMirror::Bikonoo => Self::Bikonoo,
            AppAmllMirror::Custom {
                index_url,
                lyrics_url_template,
            } => Self::Custom {
                index_url,
                lyrics_url_template,
            },
        }
    }
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
        let options = self.build_conversion_options();
        self.dispatch_conversion_task(options);
    }

    fn build_conversion_options(&self) -> ConversionOptions {
        let settings = self.app_settings.lock().unwrap();
        ConversionOptions {
            metadata_stripper: settings.metadata_stripper.clone(),
            ..Default::default()
        }
    }

    fn dispatch_regeneration_task(&mut self) {
        if self.lyrics.conversion_in_progress {
            warn!("[Regenerate] 重新生成已在进行中，跳过新的请求。");
            return;
        }

        let Some(parsed_data) = self.lyrics.parsed_lyric_data.clone() else {
            warn!("[Regenerate] 没有已解析的数据可供重新生成。");
            return;
        };

        let (tx, rx) = std::sync::mpsc::channel();
        self.lyrics.conversion_result_rx = Some(rx);
        self.lyrics.conversion_in_progress = true;

        let target_format = self.lyrics.target_format;

        self.lyrics.metadata_manager.sync_store_from_ui_entries();
        let metadata_overrides = Some(
            self.lyrics
                .metadata_manager
                .store
                .get_all_data()
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        );

        self.tokio_runtime.spawn(async move {
            let result = lyrics_helper_rs::LyricsHelper::generate_lyrics_from_parsed::<
                std::hash::RandomState,
            >(
                parsed_data,
                target_format,
                Default::default(),
                metadata_overrides,
            )
            .await;

            if tx.send(result).is_err() {
                warn!("[Regenerate Task] 发送重新生成结果失败，接收端可能已关闭。");
            }
        });
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
        self.lyrics.metadata_manager.sync_store_from_ui_entries();
        let metadata_overrides = Some(
            self.lyrics
                .metadata_manager
                .store
                .get_all_data()
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        );

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
                AmllConnectorAction::CheckIndexUpdate => None,
                AmllConnectorAction::ReloadProviders => {
                    info!("[AMLL Action] 重新加载提供商...");
                    self.lyrics_helper_state.provider_state = ProviderState::Uninitialized;
                    self.trigger_provider_loading();
                    return ActionResult::Success;
                }
            };

            if let Some(cmd) = command
                && let Err(e) = tx.try_send(cmd)
            {
                tracing::error!("[AMLL Action] 发送命令到 actor 失败: {}", e);
            }
        }
        match action {
            AmllConnectorAction::CheckIndexUpdate => {
                info!("[AMLL Action] 正在检查索引更新...");
                let helper = self.lyrics_helper_state.helper.clone();
                let action_tx = self.action_tx.clone();

                self.tokio_runtime.spawn(async move {
                    let result = helper.lock().await.force_update_amll_index().await;

                    let toast = match result {
                        Ok(_) => {
                            info!("[AMLL Update] 索引更新成功。");
                            egui_toast::Toast {
                                text: "AMLL 索引检查完成，已更新到最新版本。".into(),
                                kind: egui_toast::ToastKind::Success,
                                options: egui_toast::ToastOptions::default()
                                    .duration_in_seconds(3.0),
                                style: Default::default(),
                            }
                        }
                        Err(e) => {
                            error!("[AMLL Update] 索引更新失败: {}", e);
                            egui_toast::Toast {
                                text: format!("AMLL 索引更新失败: {}", e).into(),
                                kind: egui_toast::ToastKind::Error,
                                options: egui_toast::ToastOptions::default()
                                    .duration_in_seconds(5.0),
                                style: Default::default(),
                            }
                        }
                    };
                    let _ = action_tx.send(UserAction::UI(UIAction::ShowToast(Box::new(toast))));
                });
                ActionResult::Success
            }
            _ => ActionResult::Success,
        }
    }

    /// 子事件处理器
    fn handle_lyrics_action(&mut self, action: LyricsAction) -> ActionResult {
        match action {
            LyricsAction::LoadFileContent(content, path) => {
                self.clear_lyrics_state_for_new_song_internal();
                self.lyrics.last_opened_file_path = Some(path.clone());
                self.lyrics.metadata_source_is_download = false;
                self.lyrics.input_text = content;
                if let Some(ext) = path.extension().and_then(|s| s.to_str())
                    && let Some(format) = LyricFormat::from_string(ext)
                {
                    self.lyrics.source_format = format;
                }
                self.trigger_convert();
                ActionResult::Success
            }
            LyricsAction::Convert => {
                self.trigger_convert();
                ActionResult::Success
            }
            LyricsAction::ConvertCompleted(result) => {
                self.lyrics.conversion_in_progress = false;
                match result {
                    Ok(full_result) => {
                        self.lyrics.output_text = full_result.output_lyrics;
                        self.lyrics.parsed_lyric_data = Some(full_result.source_data.clone());

                        self.lyrics.display_translation_lrc_output =
                            self.generate_lrc_from_aux_track(&full_result.source_data, true);
                        self.lyrics.display_romanization_lrc_output =
                            self.generate_lrc_from_aux_track(&full_result.source_data, false);

                        self.lyrics
                            .metadata_manager
                            .load_from_parsed_data(&full_result.source_data);

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

                let mut options = self.build_conversion_options();
                options.chinese_conversion = ChineseConversionOptions {
                    config: Some(variant),
                    mode: ChineseConversionMode::Replace,
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
            LyricsAction::AddMetadata => {
                let new_entry_id_num = self.lyrics.metadata_manager.ui_entries.len() as u32
                    + rand::rng().random::<u32>();

                let new_id = egui::Id::new(format!("new_editable_meta_entry_{new_entry_id_num}"));
                self.lyrics
                    .metadata_manager
                    .ui_entries
                    .push(crate::types::EditableMetadataEntry {
                        key: format!("新键_{}", new_entry_id_num % 100),
                        value: "".to_string(),
                        is_pinned: false,
                        is_from_file: false,
                        id: new_id,
                    });
                self.trigger_convert();
                ActionResult::Success
            }
            LyricsAction::DeleteMetadata(index) => {
                if index < self.lyrics.metadata_manager.ui_entries.len() {
                    self.lyrics.metadata_manager.ui_entries.remove(index);
                    self.lyrics.metadata_manager.sync_store_from_ui_entries();
                    self.dispatch_regeneration_task();
                } else {
                    "无效的元数据索引".to_string();
                }
                ActionResult::Success
            }
            LyricsAction::UpdateMetadataKey(index, new_key) => {
                if let Some(entry) = self.lyrics.metadata_manager.ui_entries.get_mut(index) {
                    entry.key = new_key;
                    entry.is_from_file = false;
                    self.lyrics.metadata_manager.sync_store_from_ui_entries();
                    self.dispatch_regeneration_task();
                } else {
                    "无效的元数据索引".to_string();
                }
                ActionResult::Success
            }
            LyricsAction::UpdateMetadataValue(index, new_value) => {
                if let Some(entry) = self.lyrics.metadata_manager.ui_entries.get_mut(index) {
                    entry.value = new_value;
                    entry.is_from_file = false;
                    self.lyrics.metadata_manager.sync_store_from_ui_entries();
                    self.dispatch_regeneration_task();
                } else {
                    "无效的元数据索引".to_string();
                }
                ActionResult::Success
            }
            LyricsAction::ToggleMetadataPinned(index) => {
                if let Some(entry) = self.lyrics.metadata_manager.ui_entries.get_mut(index) {
                    entry.is_pinned = !entry.is_pinned;
                    entry.is_from_file = false;
                    self.lyrics.metadata_manager.sync_store_from_ui_entries();
                    self.dispatch_regeneration_task();
                } else {
                    "无效的元数据索引".to_string();
                }
                ActionResult::Success
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
            LyricsAction::MainInputChanged(text) => {
                self.lyrics.input_text = text;
                self.lyrics.metadata_manager.store.clear();
                self.lyrics.metadata_manager.ui_entries.clear();

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

                let parsed_data = &lyrics_and_metadata.lyrics.parsed;
                let raw_data = lyrics_and_metadata.lyrics.raw;

                self.lyrics.source_format = parsed_data.source_format;
                self.fetcher.last_source_format = Some(parsed_data.source_format);
                self.lyrics.input_text = raw_data.content;

                self.trigger_convert();

                ActionResult::Success
            }
            LyricsAction::ApplyProcessor(processor) => {
                let Some(parsed_data) = self.lyrics.parsed_lyric_data.as_mut() else {
                    return ActionResult::Warning("没有已解析的歌词可供处理".to_string());
                };

                info!("[Processor] 应用后处理器: {:?}", processor);

                let (stripper_options, smoother_options) = {
                    let settings = self.app_settings.lock().unwrap();
                    (
                        settings.metadata_stripper.clone(),
                        settings.syllable_smoothing,
                    )
                };

                match processor {
                    ProcessorType::MetadataStripper => {
                        lyrics_helper_rs::converter::processors::metadata_stripper::strip_descriptive_metadata_lines(
                            &mut parsed_data.lines,
                            &stripper_options,
                        );
                    }
                    ProcessorType::SyllableSmoother => {
                        lyrics_helper_rs::converter::processors::syllable_smoothing::apply_smoothing(
                            &mut parsed_data.lines,
                            &smoother_options,
                        );
                    }
                    ProcessorType::AgentRecognizer => {
                        lyrics_helper_rs::converter::processors::agent_recognizer::recognize_agents(
                            &mut parsed_data.lines,
                        );
                    }
                }
                self.dispatch_regeneration_task();
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
        self.lyrics.metadata_manager.ui_entries.clear();
        self.lyrics.metadata_manager.store.clear();
    }

    fn handle_file_action(&mut self, action: FileAction) -> ActionResult {
        match action {
            FileAction::Open => {
                self.clear_lyrics_state_for_new_song_internal();
                self.lyrics.metadata_source_is_download = false;
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
            UIAction::ShowToast(toast) => {
                self.ui.toasts.add(*toast);
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
            .cloned()
            .ok_or_else(|| AppError::Custom("缺少缓存目录路径".to_string()))?;
        let index_path = self
            .local_cache
            .index_path
            .as_ref()
            .cloned()
            .ok_or_else(|| AppError::Custom("缺少缓存索引路径".to_string()))?;

        let max_cache_count = self.app_settings.lock().unwrap().auto_cache_max_count;
        let mut index_guard = self.local_cache.index.lock().unwrap();

        while index_guard.len() >= max_cache_count && !index_guard.is_empty() {
            let oldest_entry = index_guard.remove(0);
            info!(
                "[LocalCache] 缓存已满，移除最旧的条目: {}",
                oldest_entry.ttml_filename
            );
            let file_to_delete = cache_dir.join(oldest_entry.ttml_filename);
            if let Err(e) = std::fs::remove_file(&file_to_delete) {
                warn!(
                    "[LocalCache] 删除旧缓存文件 {:?} 失败: {}",
                    file_to_delete, e
                );
            }
        }

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
            saved_timestamp: chrono::Utc::now().timestamp(),
        };

        index_guard.push(entry);

        let lines: Vec<String> = index_guard
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .collect();
        std::fs::write(&index_path, lines.join("\n")).map_err(AppError::from)?;

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
                    let mut mirror_changed = false;

                    {
                        let old_settings = self.app_settings.lock().unwrap();
                        let new_mirror = &settings.amll_mirror;
                        if &old_settings.amll_mirror != new_mirror {
                            let core_config = CoreAmllConfig {
                                mirror: new_mirror.clone().into(),
                            };
                            match serde_json::to_string_pretty(&core_config) {
                                Ok(json_string) => {
                                    if let Ok(config_path) =
                                        lyrics_helper_rs::config::native::get_config_file_path(
                                            "amll_config.json",
                                        )
                                    {
                                        if let Err(e) = std::fs::write(&config_path, json_string) {
                                            error!("[Settings] 写入 amll_config.json 失败: {}", e);
                                        } else {
                                            mirror_changed = true;
                                        }
                                    } else {
                                        error!("[Settings] 无法获取 amll_config.json 的路径");
                                    }
                                }
                                Err(e) => {
                                    error!("[Settings] 序列化核心库 AMLL 配置失败: {}", e);
                                }
                            }
                        }
                    }

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

                    if mirror_changed {
                        let toast = egui_toast::Toast {
                            text: "AMLL 镜像设置已保存。\n需要重新启动才能生效。".into(),
                            kind: egui_toast::ToastKind::Info,
                            options: egui_toast::ToastOptions::default()
                                .duration_in_seconds(10.0)
                                .show_progress(true),
                            style: Default::default(),
                        };
                        self.ui.toasts.add(toast);
                    }

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

    pub(super) fn generate_lrc_from_aux_track(
        &self,
        parsed_data: &lyrics_helper_core::ParsedSourceData,
        is_translation: bool,
    ) -> String {
        let mut lrc_output = String::new();

        for line in &parsed_data.lines {
            if let Some(main_track) = line
                .tracks
                .iter()
                .find(|t| t.content_type == ContentType::Main)
            {
                let aux_tracks: &Vec<LyricTrack> = if is_translation {
                    &main_track.translations
                } else {
                    &main_track.romanizations
                };

                if let Some(first_aux_track) = aux_tracks.first() {
                    let text: String = first_aux_track
                        .words
                        .iter()
                        .flat_map(|w| &w.syllables)
                        .map(|s| s.text.as_str())
                        .collect();

                    if !text.is_empty() {
                        let minutes = line.start_ms / 60000;
                        let seconds = (line.start_ms % 60000) / 1000;
                        let millis = (line.start_ms % 1000) / 10;
                        let timestamp = format!("[{:02}:{:02}.{:02}]", minutes, seconds, millis);

                        let _ = writeln!(&mut lrc_output, "{}{}", timestamp, text);
                    }
                }
            }
        }
        lrc_output
    }
}
