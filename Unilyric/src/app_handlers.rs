use std::fmt::Write;
use std::sync::Arc;

use crate::amll_connector::types::ActorSettings;
use crate::amll_connector::{AMLLConnectorConfig, ConnectorCommand};
use crate::app_actions::{
    AmllConnectorAction, BatchConverterAction, DownloaderAction, FileAction, LyricsAction,
    PanelType, PlayerAction, ProcessorType, SettingsAction, UIAction, UserAction,
};
use crate::app_definition::{
    AppView, BatchConverterStatus, DownloaderState, PreviewState, SearchState, UniLyricApp,
};
use crate::app_handlers::ConnectorCommand::SendLyric;
use crate::app_handlers::ConnectorCommand::UpdateActorSettings;
use crate::app_settings::AppAmllMirror;
use crate::error::{AppError, AppResult};
use crate::types::{AutoSearchStatus, LrcContentType, ProviderState};
use lyrics_helper_core::{
    ChineseConversionConfig, ChineseConversionMode, ChineseConversionOptions, ContentType,
    ConversionInput, ConversionOptions, InputFile, LyricFormat, Track,
};
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

        self.tokio_runtime.spawn(async move {
            let result = lyrics_helper_rs::LyricsHelper::generate_lyrics_from_parsed::<
                std::hash::RandomState,
            >(parsed_data, target_format, Default::default(), None)
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

        let main_lyric = InputFile::new(
            self.lyrics.input_text.clone(),
            self.lyrics.source_format,
            None,
            None,
        );

        let additional_metadata = None;

        let input = ConversionInput {
            main_lyric,
            translations: vec![],
            romanizations: vec![],
            target_format: self.lyrics.target_format,
            user_metadata_overrides: None,
            additional_metadata,
        };

        self.tokio_runtime.spawn(async move {
            let result = helper.lock().await.convert_lyrics(&input, &options);
            if tx.send(result).is_err() {
                warn!("[Convert Task] 发送转换结果失败，接收端可能已关闭。");
            }
        });
    }

    pub fn send_action(&mut self, action: UserAction) {
        self.actions_this_frame.push(action);
    }

    pub fn handle_actions(&mut self, actions: Vec<UserAction>) {
        let mut results = Vec::new();

        for action in actions {
            debug!("处理事件: {:?}", std::mem::discriminant(&action));
            results.push(self.handle_single_action(action));
        }

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
                ActionResult::Success => {}
            }
        }
    }

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
            UserAction::Downloader(downloader_action) => {
                self.handle_downloader_action(*downloader_action)
            }
            UserAction::BatchConverter(batch_action) => {
                self.handle_batch_converter_action(batch_action)
            }
        }
    }

    fn handle_amll_connector_action(&mut self, action: AmllConnectorAction) -> ActionResult {
        if let Some(tx) = &self.amll_connector.command_tx {
            let command = match action {
                AmllConnectorAction::Connect | AmllConnectorAction::Retry => {
                    let mut config = self.amll_connector.config.lock().unwrap();
                    config.enabled = true;

                    Some(ConnectorCommand::StartConnection)
                }
                AmllConnectorAction::Disconnect => Some(ConnectorCommand::DisconnectWebsocket),
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

    fn handle_load_full_lyrics_result(
        &mut self,
        result: lyrics_helper_core::model::track::FullLyricsResult,
    ) -> ActionResult {
        self.clear_lyrics_state_for_new_song_internal();
        self.fetcher.current_ui_populated = true;
        self.lyrics.input_text = result.raw.content;
        self.lyrics.source_format = result.parsed.source_format;
        self.lyrics.current_warnings = result.parsed.warnings.clone();

        let mut final_parsed_data = result.parsed;

        {
            let settings = self.app_settings.lock().unwrap();
            if settings.auto_apply_metadata_stripper {
                lyrics_helper_rs::converter::processors::metadata_stripper::strip_descriptive_metadata_lines(
                    &mut final_parsed_data.lines,
                    &settings.metadata_stripper,
                );
            }

            if settings.auto_apply_agent_recognizer {
                lyrics_helper_rs::converter::processors::agent_recognizer::recognize_agents(
                    &mut final_parsed_data,
                );
            }
        }

        self.lyrics.parsed_lyric_data = Some(final_parsed_data);
        self.dispatch_regeneration_task();
        ActionResult::Success
    }

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
                        self.lyrics.current_warnings = full_result.source_data.warnings.clone();

                        self.lyrics.display_translation_lrc_output =
                            self.generate_lrc_from_aux_track(&full_result.source_data, true);
                        self.lyrics.display_romanization_lrc_output =
                            self.generate_lrc_from_aux_track(&full_result.source_data, false);

                        if self.amll_connector.config.lock().unwrap().enabled
                            && let Some(tx) = &self.amll_connector.command_tx
                            && tx.try_send(SendLyric(full_result.source_data)).is_err()
                        {
                            tracing::error!("[AMLL] 发送 TTML 歌词失败。");
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

                if let Ok(mut settings) = self.app_settings.lock() {
                    settings.last_source_format = format;
                    if let Err(e) = settings.save() {
                        return ActionResult::Warning(format!("保存源格式设置失败: {e}"));
                    }
                }

                if !self.lyrics.input_text.trim().is_empty() && !self.lyrics.conversion_in_progress
                {
                    self.trigger_convert();
                }

                ActionResult::Success
            }
            LyricsAction::TargetFormatChanged(format) => {
                info!("目标格式改变为: {format:?}");
                self.lyrics.target_format = format;

                if let Ok(mut settings) = self.app_settings.lock() {
                    settings.last_target_format = format;
                    if let Err(e) = settings.save() {
                        return ActionResult::Warning(format!("保存目标格式设置失败: {e}"));
                    }
                }

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
            LyricsAction::LrcInputChanged(text, content_type) => {
                let lrc_lines = match lyrics_helper_rs::converter::parsers::lrc_parser::parse_lrc(
                    &text,
                    &Default::default(),
                ) {
                    Ok(parsed) => Some(
                        parsed
                            .lines
                            .into_iter()
                            .map(|_| crate::types::DisplayLrcLine::Parsed)
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

                self.trigger_convert();
                ActionResult::Success
            }
            LyricsAction::MainInputChanged(text) => {
                self.clear_lyrics_state_for_new_song_internal();
                self.lyrics.input_text = text;
                if !self.lyrics.conversion_in_progress && !self.lyrics.input_text.trim().is_empty()
                {
                    self.trigger_convert();
                }
                ActionResult::Success
            }
            LyricsAction::ApplyFetchedLyrics(lyrics_and_metadata_box) => {
                self.lyrics.current_warnings =
                    lyrics_and_metadata_box.lyrics.parsed.warnings.clone();
                self.handle_load_full_lyrics_result(lyrics_and_metadata_box.lyrics)
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
                            parsed_data,
                        );
                    }
                }
                self.dispatch_regeneration_task();
                ActionResult::Success
            }
        }
    }

    fn handle_downloader_action(&mut self, action: DownloaderAction) -> ActionResult {
        match action {
            DownloaderAction::FillFromSmtc => {
                let smtc_info = &self.player.current_now_playing;
                if smtc_info.title.is_none() {
                    return ActionResult::Warning("无 SMTC 信息可供填充".to_string());
                }

                self.downloader.title_input = smtc_info.title.clone().unwrap_or_default();
                self.downloader.artist_input = smtc_info.artist.clone().unwrap_or_default();
                self.downloader.album_input = smtc_info.album_title.clone().unwrap_or_default();
                self.downloader.duration_ms_input = smtc_info.duration_ms.unwrap_or_default();

                ActionResult::Success
            }
            DownloaderAction::PerformSearch => {
                if self.downloader.title_input.trim().is_empty() {
                    return ActionResult::Warning("歌曲名不能为空".to_string());
                }
                self.downloader.search_state = SearchState::Searching;
                self.downloader.preview_state = PreviewState::Idle;
                self.downloader.selected_result_for_preview = None;
                self.downloader.selected_full_lyrics = None;

                let helper = self.lyrics_helper_state.helper.clone();
                let title = self.downloader.title_input.clone();
                let artist = self.downloader.artist_input.clone();
                let album = self.downloader.album_input.clone();
                let duration = self.downloader.duration_ms_input;
                let action_tx = self.action_tx.clone();

                self.tokio_runtime.spawn(async move {
                    let artists_vec: Vec<&str> = if artist.is_empty() {
                        vec![]
                    } else {
                        artist.split(&['/', ',', ';']).map(|s| s.trim()).collect()
                    };

                    let track_to_search = Track {
                        title: Some(&title),
                        artists: if artists_vec.is_empty() {
                            None
                        } else {
                            Some(&artists_vec)
                        },
                        album: if album.is_empty() { None } else { Some(&album) },
                        duration: if duration == 0 { None } else { Some(duration) },
                    };

                    let result = helper.lock().await.search_track(&track_to_search).await;

                    let _ = action_tx.send(UserAction::Downloader(Box::new(
                        DownloaderAction::SearchCompleted(result.map_err(AppError::from)),
                    )));
                });

                ActionResult::Success
            }
            DownloaderAction::SearchCompleted(result) => {
                self.downloader.search_state = match result {
                    Ok(results) => SearchState::Success(results),
                    Err(e) => SearchState::Error(e.to_string()),
                };
                ActionResult::Success
            }
            DownloaderAction::SelectResultForPreview(search_result) => {
                self.downloader.selected_result_for_preview = Some(search_result.clone());
                self.downloader.preview_state = PreviewState::Loading;
                self.downloader.selected_full_lyrics = None;

                let helper = self.lyrics_helper_state.helper.clone();
                let action_tx = self.action_tx.clone();

                self.tokio_runtime.spawn(async move {
                    let result = {
                        let future_result = {
                            helper.lock().await.get_full_lyrics(
                                &search_result.provider_name,
                                &search_result.provider_id,
                            )
                        };
                        match future_result {
                            Ok(future) => future.await,
                            Err(e) => Err(e),
                        }
                    };
                    let _ = action_tx.send(UserAction::Downloader(Box::new(
                        DownloaderAction::PreviewDownloadCompleted(result.map_err(AppError::from)),
                    )));
                });

                ActionResult::Success
            }
            DownloaderAction::PreviewDownloadCompleted(result) => {
                match result {
                    Ok(full_lyrics) => {
                        let main_text = self.generate_lrc_from_main_track(&full_lyrics.parsed);

                        self.downloader.preview_state =
                            PreviewState::Success(main_text.to_string());
                        self.downloader.selected_full_lyrics = Some(full_lyrics);
                    }
                    Err(e) => {
                        self.downloader.preview_state = PreviewState::Error(e.to_string());
                    }
                }
                ActionResult::Success
            }
            DownloaderAction::ApplyAndClose => {
                if let Some(lyrics_to_apply) = self.downloader.selected_full_lyrics.clone() {
                    let lyrics_and_metadata =
                        Box::new(lyrics_helper_core::model::track::LyricsAndMetadata {
                            lyrics: lyrics_to_apply,
                            source_track: Default::default(),
                        });
                    self.send_action(UserAction::Lyrics(Box::new(
                        LyricsAction::ApplyFetchedLyrics(lyrics_and_metadata),
                    )));
                    self.send_action(UserAction::Downloader(Box::new(DownloaderAction::Close)));
                } else {
                    return ActionResult::Warning("没有可应用的歌词".to_string());
                }
                ActionResult::Success
            }
            DownloaderAction::Close => {
                self.ui.current_view = AppView::Editor;
                self.downloader = DownloaderState::default();
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
        self.lyrics.current_warnings.clear();
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
                    PanelType::Translation => &mut self.ui.show_translation_lrc_panel,
                    PanelType::Romanization => &mut self.ui.show_romanization_lrc_panel,
                    PanelType::Settings => &mut self.ui.show_settings_window,
                    PanelType::AmllConnector => &mut self.ui.show_amll_connector_sidebar,
                    PanelType::Warnings => &mut self.ui.show_warnings_panel,
                };

                *panel_state_mut = is_visible;

                if matches!(panel, PanelType::Log) && is_visible {
                    self.ui.new_trigger_log_exists = false;
                }

                ActionResult::Success
            }
            UIAction::SetView(view) => {
                self.ui.current_view = view;
                ActionResult::Success
            }
            UIAction::ShowPanel(panel) => {
                match panel {
                    PanelType::Log => self.ui.show_bottom_log_panel = true,
                    PanelType::Translation => self.ui.show_translation_lrc_panel = true,
                    PanelType::Romanization => self.ui.show_romanization_lrc_panel = true,
                    PanelType::Settings => {
                        self.ui.temp_edit_settings = self.app_settings.lock().unwrap().clone();
                        self.ui.show_settings_window = true;
                    }
                    PanelType::AmllConnector => self.ui.show_amll_connector_sidebar = true,
                    PanelType::Warnings => self.ui.show_warnings_panel = true,
                }
                ActionResult::Success
            }
            UIAction::HidePanel(panel) => {
                match panel {
                    PanelType::Log => {
                        self.ui.show_bottom_log_panel = false;
                        self.ui.new_trigger_log_exists = false;
                    }
                    PanelType::Translation => self.ui.show_translation_lrc_panel = false,
                    PanelType::Romanization => self.ui.show_romanization_lrc_panel = false,
                    PanelType::Settings => self.ui.show_settings_window = false,
                    PanelType::AmllConnector => self.ui.show_amll_connector_sidebar = false,
                    PanelType::Warnings => self.ui.show_warnings_panel = false,
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
        match action {
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
                if let Err(e) = command_tx.try_send(MediaCommand::SelectSession(session_id)) {
                    error!("[PlayerAction] 发送命令到 smtc-suite 失败: {}", e);
                    return ActionResult::Error(AppError::Custom("发送命令失败".to_string()));
                }
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
            }
            PlayerAction::ToggleAudioCapture(enable) => {
                let smtc_command = if enable {
                    MediaCommand::StartAudioCapture
                } else {
                    MediaCommand::StopAudioCapture
                };
                tracing::info!("[PlayerAction] 发送音频捕获命令: {:?}", smtc_command);
                if let Err(e) = command_tx.try_send(smtc_command) {
                    error!("[PlayerAction] 发送命令到 smtc-suite 失败: {}", e);
                    return ActionResult::Error(AppError::Custom("发送命令失败".to_string()));
                }
            }
            PlayerAction::SetSmtcTimeOffset(offset) => {
                self.player.smtc_time_offset_ms = offset;

                if let Ok(mut settings) = self.app_settings.lock()
                    && settings.smtc_time_offset_ms != offset
                {
                    settings.smtc_time_offset_ms = offset;
                    if let Err(e) = settings.save() {
                        warn!("[PlayerAction] 保存偏移量设置失败: {}", e);
                    }
                }
                if let Err(e) = command_tx.try_send(MediaCommand::SetProgressOffset(offset)) {
                    error!("[PlayerAction] 发送命令到 smtc-suite 失败: {}", e);
                    return ActionResult::Error(AppError::Custom("发送命令失败".to_string()));
                }
            }
        }
        ActionResult::Success
    }

    fn handle_batch_converter_action(&mut self, action: BatchConverterAction) -> ActionResult {
        match action {
            BatchConverterAction::SelectInputDir => {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.batch_converter.input_dir = Some(path);
                }
                ActionResult::Success
            }
            BatchConverterAction::SelectOutputDir => {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.batch_converter.output_dir = Some(path);
                }
                ActionResult::Success
            }
            BatchConverterAction::ScanTasks => {
                let Some(input_dir) = self.batch_converter.input_dir.clone() else {
                    return ActionResult::Warning("输入目录未设置".to_string());
                };
                let target_format = self.batch_converter.target_format;

                match lyrics_helper_rs::converter::processors::batch_processor::discover_and_pair_files(&input_dir) {
                    Ok(file_groups) => {
                        let (tasks, file_lookup) =
                            lyrics_helper_rs::converter::processors::batch_processor::create_batch_tasks(
                                file_groups,
                                target_format,
                            );
                        self.batch_converter.tasks = tasks;
                        self.batch_converter.file_lookup = file_lookup;
                        self.batch_converter.status = BatchConverterStatus::Ready;
                    }
                    Err(e) => {
                        self.batch_converter.status = BatchConverterStatus::Failed(e.to_string());
                    }
                }
                ActionResult::Success
            }
            BatchConverterAction::StartConversion => {
                if self.batch_converter.status != BatchConverterStatus::Ready {
                    return ActionResult::Warning("当前状态无法开始转换。".to_string());
                }

                self.batch_converter.status = BatchConverterStatus::Converting;

                let mut tasks = self.batch_converter.tasks.clone();
                let file_lookup = self.batch_converter.file_lookup.clone();
                let output_dir = self.batch_converter.output_dir.clone().unwrap();
                let options = self.build_conversion_options();
                let action_tx = self.action_tx.clone();

                self.tokio_runtime.spawn(async move {
                    let result = lyrics_helper_rs::converter::processors::batch_processor::execute_batch_conversion(
                        &mut tasks,
                        &file_lookup,
                        &output_dir,
                        &options
                    );

                    match result {
                        Ok(()) => {
                            for task in tasks {
                                 let update_msg = lyrics_helper_core::BatchTaskUpdate {
                                     entry_config_id: task.id,
                                     new_status: task.status,
                                 };
                                 let _ = action_tx.send(UserAction::BatchConverter(BatchConverterAction::TaskUpdate(update_msg)));
                            }
                            let _ = action_tx.send(UserAction::BatchConverter(BatchConverterAction::ConversionCompleted));
                        }
                        Err(e) => {
                             error!("[BatchConvert] 批量转换执行失败: {}", e);
                        }
                    }
                });

                ActionResult::Success
            }
            BatchConverterAction::TaskUpdate(update) => {
                if let Some(task) = self
                    .batch_converter
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == update.entry_config_id)
                {
                    task.status = update.new_status;
                }
                ActionResult::Success
            }
            BatchConverterAction::ConversionCompleted => {
                self.batch_converter.status = BatchConverterStatus::Completed;
                ActionResult::Success
            }
            BatchConverterAction::Reset => {
                self.batch_converter = Default::default();
                ActionResult::Success
            }
        }
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
                        mirror_changed = &old_settings.amll_mirror != new_mirror;

                        let core_config = CoreAmllConfig {
                            mirror: new_mirror.clone().into(),
                        };
                        match serde_json::to_string_pretty(&core_config) {
                            Ok(json_string) => {
                                if let Ok(config_path) =
                                    lyrics_helper_rs::config::get_config_file_path(
                                        "amll_config.json",
                                    )
                                {
                                    if let Err(e) = std::fs::write(&config_path, json_string) {
                                        error!("[Settings] 写入 amll_config.json 失败: {}", e);
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

                    {
                        let mut app_settings_guard = self.app_settings.lock().unwrap();
                        *app_settings_guard = *settings.clone();
                        self.player.smtc_time_offset_ms = app_settings_guard.smtc_time_offset_ms;
                    }

                    let old_offset = self.app_settings.lock().unwrap().smtc_time_offset_ms;
                    let old_audio_capture_setting =
                        self.app_settings.lock().unwrap().send_audio_data_to_player;

                    {
                        let mut app_settings_guard = self.app_settings.lock().unwrap();
                        *app_settings_guard = *settings.clone();
                    }

                    if settings.smtc_time_offset_ms != old_offset {
                        self.send_action(UserAction::Player(PlayerAction::SetSmtcTimeOffset(
                            settings.smtc_time_offset_ms,
                        )));
                    }

                    if settings.send_audio_data_to_player != old_audio_capture_setting {
                        self.send_action(UserAction::Player(PlayerAction::ToggleAudioCapture(
                            settings.send_audio_data_to_player,
                        )));
                    }

                    let new_mc_config_from_settings = AMLLConnectorConfig {
                        enabled: settings.amll_connector_enabled,
                        websocket_url: settings.amll_connector_websocket_url.clone(),
                        mode: settings.amll_connector_mode,
                        server_port: settings.amll_connector_server_port,
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

    pub fn draw_chinese_conversion_menu_item(
        &mut self,
        ui: &mut egui::Ui,
        variant: ChineseConversionConfig,
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

    pub fn set_searching_providers_to_not_found(&mut self) {
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

    pub fn generate_lrc_from_aux_track(
        &self,
        parsed_data: &lyrics_helper_core::ParsedSourceData,
        is_translation: bool,
    ) -> String {
        let mut lrc_output = String::new();

        for line in &parsed_data.lines {
            if let Some(main_track) = line.main_track() {
                let aux_tracks = if is_translation {
                    &main_track.translations
                } else {
                    &main_track.romanizations
                };

                if let Some(first_aux_track) = aux_tracks.first() {
                    let text = first_aux_track.text();

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

    pub fn generate_lrc_from_main_track(
        &self,
        parsed_data: &lyrics_helper_core::ParsedSourceData,
    ) -> String {
        let mut lrc_output = String::new();

        for line in &parsed_data.lines {
            if let Some(main_track) = line
                .tracks
                .iter()
                .find(|t| t.content_type == ContentType::Main)
            {
                let text: String = main_track.content.text();
                if !text.is_empty() {
                    let minutes = line.start_ms / 60000;
                    let seconds = (line.start_ms % 60000) / 1000;
                    let millis = (line.start_ms % 1000) / 10;
                    let timestamp = format!("[{:02}:{:02}.{:02}]", minutes, seconds, millis);

                    let _ = writeln!(&mut lrc_output, "{}{}", timestamp, text);
                }
            }
        }
        lrc_output
    }
}
