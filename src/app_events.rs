use crate::amll_connector::{ConnectorCommand, WebsocketStatus};
use crate::app_actions::{
    FileAction, LyricsAction, PanelType, PlayerAction, SettingsAction, UIAction, UserAction,
};
use crate::app_definition::UniLyricApp;
use crate::app_fetch_core;
use crate::types::LrcContentType;
use log::{debug, error, info};
use std::io::Write;
use tracing::{trace, warn};
use ws_protocol::Body as ProtocolBody;

#[derive(Debug)]
pub enum ActionResult {
    Success,
    Warning(String),
    Error(String),
}

impl UniLyricApp {
    pub fn trigger_convert(&mut self) {
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
                        text: msg.into(),
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
            UserAction::Lyrics(lyrics_action) => self.handle_lyrics_action(lyrics_action),
            UserAction::File(file_action) => self.handle_file_action(file_action),
            UserAction::UI(ui_action) => self.handle_ui_action(ui_action),
            UserAction::Player(player_action) => self.handle_player_action(player_action),
            UserAction::Settings(settings_action) => self.handle_settings_action(settings_action),
        }
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
                        self.lyrics.parsed_lyric_data = Some(full_result.source_data);

                        if !self.lyrics.metadata_is_user_edited {
                            self.sync_ui_from_parsed_data();
                        }
                        ActionResult::Success
                    }
                    Err(e) => {
                        error!("[Convert Result] 转换任务返回了一个错误: {e}");
                        self.lyrics.output_text.clear();
                        ActionResult::Error(format!("转换失败: {e}"))
                    }
                }
            }
            LyricsAction::ConvertChinese(config_name) => {
                info!("[Convert] Starting Chinese conversion with config: {config_name}");

                if self.lyrics.conversion_in_progress {
                    warn!("[Convert] Conversion already in progress, skipping new request.");
                    return ActionResult::Warning("转换正在进行中".to_string());
                }

                // 确保有内容可以转换
                if self.lyrics.input_text.trim().is_empty()
                    && self.lyrics.parsed_lyric_data.is_none()
                {
                    warn!("[Convert] No lyrics content to perform Chinese conversion on.");
                    return ActionResult::Warning("没有歌词内容可以转换".to_string());
                }

                if let Some(helper) = self.lyrics_helper.as_ref() {
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.lyrics.conversion_result_rx = Some(rx);
                    self.lyrics.conversion_in_progress = true;

                    let helper = helper.clone();

                    // 准备输入数据（与普通转换相同）
                    let main_lyric = lyrics_helper_rs::converter::types::InputFile::new(
                        self.lyrics.input_text.clone(),
                        self.lyrics.source_format,
                        None,
                        None,
                    );
                    let mut translations = vec![];
                    if !self.lyrics.display_translation_lrc_output.trim().is_empty() {
                        translations.push(lyrics_helper_rs::converter::types::InputFile::new(
                            self.lyrics.display_translation_lrc_output.clone(),
                            lyrics_helper_rs::converter::types::LyricFormat::Lrc,
                            Some("zh-Hans".to_string()),
                            None,
                        ));
                    }
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
                    let mut metadata_overrides = std::collections::HashMap::new();

                    // 只有在用户明确编辑了元数据的情况下，才使用覆盖
                    if self.lyrics.metadata_is_user_edited {
                        for entry in &self.lyrics.editable_metadata {
                            if !entry.key.trim().is_empty() && !entry.value.trim().is_empty() {
                                let values = entry
                                    .value
                                    .split(';')
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect::<Vec<String>>();

                                if !values.is_empty() {
                                    metadata_overrides.insert(entry.key.clone(), values);
                                }
                            }
                        }
                    }
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

                    // 使用中文转换选项
                    let options = lyrics_helper_rs::converter::types::ConversionOptions {
                        chinese_conversion: lyrics_helper_rs::converter::types::ChineseConversionOptions {
                            config_name: Some(config_name.clone()),
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
                ActionResult::Success
            }
            LyricsAction::Search => {
                if self.lyrics.search_in_progress {
                    return ActionResult::Warning("搜索正在进行中".to_string());
                }

                let helper = match self.lyrics_helper.as_ref() {
                    Some(h) => std::sync::Arc::clone(h),
                    None => {
                        warn!("[Search] LyricsHelper 未初始化，无法搜索。");
                        return ActionResult::Error("LyricsHelper 未初始化".to_string());
                    }
                };

                self.lyrics.search_in_progress = true;
                self.lyrics.search_results.clear(); // 清除旧结果

                let (tx, rx) = std::sync::mpsc::channel();
                self.lyrics.search_result_rx = Some(rx);

                let query = self.lyrics.search_query.clone();

                self.tokio_runtime.spawn(async move {
                    let track_to_search = lyrics_helper_rs::model::track::Track {
                        title: Some(&query),
                        artists: None, // 简化
                        album: None,
                    };

                    // 调用核心库的 search_track 函数
                    let result = helper.search_track(&track_to_search).await;
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
                        ActionResult::Error(format!("搜索失败: {e}"))
                    }
                }
            }
            LyricsAction::Download(search_result) => {
                if self.lyrics.download_in_progress {
                    return ActionResult::Warning("下载正在进行中".to_string());
                }

                let helper = match self.lyrics_helper.as_ref() {
                    Some(h) => std::sync::Arc::clone(h),
                    None => {
                        warn!("[Download] LyricsHelper 未初始化，无法下载。");
                        return ActionResult::Error("LyricsHelper 未初始化".to_string());
                    }
                };

                self.lyrics.download_in_progress = true;

                let (tx, rx) = std::sync::mpsc::channel();
                self.lyrics.download_result_rx = Some(rx);

                let provider_name = search_result.provider_name.clone();
                let provider_id = search_result.provider_id.clone();

                self.tokio_runtime.spawn(async move {
                    // 调用核心库的 get_full_lyrics 函数
                    let result = helper.get_full_lyrics(&provider_name, &provider_id).await;
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
                        let source = crate::types::AutoSearchSource::from(
                            full_lyrics_result.parsed.source_name.clone(),
                        );
                        info!("[Download] 从 {source:?} 下载歌词成功。");

                        self.ui.show_search_window = false;

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

                        let parsed_data = full_lyrics_result.parsed;
                        let raw_data = full_lyrics_result.raw;

                        // 使用获取到的原始文本和格式填充状态
                        self.lyrics.input_text = raw_data.content;
                        self.lyrics.source_format = parsed_data.source_format;
                        self.fetcher.last_source_format = Some(parsed_data.source_format);

                        self.lyrics.metadata_source_is_download = true;

                        self.trigger_convert();
                        ActionResult::Success
                    }
                    Err(e) => {
                        error!("[Download] 下载任务失败: {e}");
                        ActionResult::Error(format!("下载失败: {e}"))
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
                    self.lyrics.editable_metadata.len() as u32 + rand::thread_rng().r#gen::<u32>();

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
                    ActionResult::Error("无效的元数据索引".to_string())
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
                    ActionResult::Error("无效的元数据索引".to_string())
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
                    ActionResult::Error("无效的元数据索引".to_string())
                }
            }
            LyricsAction::ToggleMetadataPinned(index) => {
                if let Some(entry) = self.lyrics.editable_metadata.get_mut(index) {
                    entry.is_pinned = !entry.is_pinned;
                    self.lyrics.metadata_is_user_edited = true;
                    self.trigger_convert();
                    ActionResult::Success
                } else {
                    ActionResult::Error("无效的元数据索引".to_string())
                }
            }
            LyricsAction::AutoFetchCompleted(auto_fetch_result) => {
                self.handle_auto_fetch_result(auto_fetch_result);
                ActionResult::Success
            }
            LyricsAction::LrcInputChanged(text, content_type) => {
                let lrc_lines =
                    match lyrics_helper_rs::converter::parsers::lrc_parser::parse_lrc(&text) {
                        Ok(parsed) => Some(
                            parsed
                                .lines
                                .into_iter()
                                .map(crate::types::DisplayLrcLine::Parsed)
                                .collect(),
                        ),
                        Err(e) => {
                            log::warn!("[LRC Edit] LRC文本解析失败: {e}");
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
                    PanelType::Log => self.ui.show_bottom_log_panel = false,
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
        }
    }

    fn handle_player_action(&mut self, action: PlayerAction) -> ActionResult {
        match action {
            PlayerAction::WebsocketStatusChanged(status) => {
                let mut ws_status_guard = self.player.status.lock().unwrap();
                let old_status = ws_status_guard.clone();
                *ws_status_guard = status.clone();
                drop(ws_status_guard);
                info!("[UniLyric] AMLL Connector WebSocket 状态改变: {old_status:?} -> {status:?}");

                if status == WebsocketStatus::已连接 && old_status != WebsocketStatus::已连接
                {
                    self.start_progress_timer_if_needed();

                    if let Some(tx) = &self.player.command_tx {
                        // 1. 发送初始播放状态
                        // self.player.is_currently_playing_sensed_by_smtc 是最可靠的状态源
                        let initial_playback_body = if self
                            .player
                            .is_currently_playing_sensed_by_smtc
                        {
                            log::info!(
                                "[UniLyric] WebSocket 已连接，感知到 SMTC 正在播放，发送 OnPlay。"
                            );
                            ProtocolBody::OnResumed
                        } else {
                            log::info!(
                                "[UniLyric] WebSocket 已连接，感知到 SMTC 已暂停，发送 OnPaused。"
                            );
                            ProtocolBody::OnPaused
                        };

                        if tx
                            .send(ConnectorCommand::SendProtocolBody(initial_playback_body))
                            .is_err()
                        {
                            error!("[UniLyric] 连接成功后发送初始播放状态命令失败。");
                        }

                        // 2. 发送当前歌词
                        if !self.lyrics.output_text.is_empty() {
                            log::info!("[UniLyric] WebSocket 已连接，正在自动发送当前 TTML 歌词。");
                            if tx
                                .send(ConnectorCommand::SendLyricTtml(
                                    self.lyrics.output_text.clone(),
                                ))
                                .is_err()
                            {
                                log::error!("[UniLyric] 连接成功后自动发送 TTML 歌词失败。");
                            }
                        } else {
                            log::trace!(
                                "[UniLyric] WebSocket 已连接，但输出框为空，不自动发送歌词。"
                            );
                        }
                    } else {
                        log::warn!(
                            "[UniLyric] WebSocket 已连接，但 command_tx 不可用，无法自动同步状态。"
                        );
                    }
                } else if status != WebsocketStatus::已连接 {
                    // 如果状态不是“已连接”（例如断开、错误），则停止定时器
                    self.stop_progress_timer();
                }
                ActionResult::Success
            }

            PlayerAction::SmtcTrackChanged(new_info_from_event) => {
                let smtc_event_arrival_time = std::time::Instant::now();
                let mut current_event_data = new_info_from_event.clone();

                let mut parsed_artist_successfully = false;
                let temp_original_artist_str_opt = current_event_data.artist.take();
                let mut temp_final_artist: Option<String> = None;
                let mut temp_parsed_album_from_artist_field: Option<String> = None;

                if let Some(original_artist_album_str_val) = temp_original_artist_str_opt {
                    const SEPARATOR: &str = " — ";
                    let parts: Vec<&str> = original_artist_album_str_val
                        .split(SEPARATOR)
                        .map(|s| s.trim())
                        .collect();

                    if !parts.is_empty() {
                        let artist_candidate = parts[0];
                        if !artist_candidate.is_empty() {
                            temp_final_artist = Some(artist_candidate.to_string());
                            parsed_artist_successfully = true;
                            trace!("[UniLyric] 初始艺术家解析自 SMTC: '{artist_candidate}'");

                            if parts.len() > 1 {
                                let album_candidate_from_artist_parts = parts[1..].join(SEPARATOR);
                                if !album_candidate_from_artist_parts.is_empty() {
                                    temp_parsed_album_from_artist_field =
                                        Some(album_candidate_from_artist_parts);
                                    trace!(
                                        "[UniLyric] 从艺术家字段解析出的专辑候选: '{}'",
                                        temp_parsed_album_from_artist_field
                                            .as_deref()
                                            .unwrap_or("")
                                    );
                                }
                            }
                        } else {
                            temp_final_artist = Some(original_artist_album_str_val.clone());
                        }
                    } else {
                        temp_final_artist = Some(original_artist_album_str_val.clone());
                    }

                    if !parsed_artist_successfully && !original_artist_album_str_val.is_empty() {
                        temp_final_artist = Some(original_artist_album_str_val);
                    }
                }
                current_event_data.artist = temp_final_artist;

                if let Some(parsed_album) = temp_parsed_album_from_artist_field {
                    if current_event_data
                        .album_title
                        .as_ref()
                        .is_none_or(|s| s.is_empty())
                    {
                        current_event_data.album_title = Some(parsed_album.clone());
                        trace!(
                            "[UniLyric] 使用从艺术家字段解析的专辑填充 Album: '{:?}'",
                            current_event_data.album_title
                        );
                    } else {
                        trace!(
                            "[UniLyric] 专辑字段已存在值 '{:?}'，未被从艺术家字段解析出的 '{:?}' 覆盖。",
                            current_event_data.album_title, parsed_album
                        );
                    }
                }

                if parsed_artist_successfully {
                    trace!(
                        "[UniLyric] 艺术家/专辑字段解析尝试完成。最终 Artist: {:?}, Album: {:?}",
                        current_event_data.artist, current_event_data.album_title
                    );
                } else if current_event_data.artist.is_some() {
                    trace!(
                        "[UniLyric] 艺术家/专辑字段未经有效分隔符解析。Artist: {:?}, Album: {:?}",
                        current_event_data.artist, current_event_data.album_title
                    );
                }

                let raw_smtc_position_from_event_for_log = current_event_data.position_ms;

                if let Some(original_pos) = current_event_data.position_ms {
                    let mut adjusted_pos_i64 =
                        original_pos as i64 - self.player.smtc_time_offset_ms;
                    adjusted_pos_i64 = adjusted_pos_i64.max(0);
                    if let Some(duration) = current_event_data.duration_ms
                        && duration > 0
                    {
                        adjusted_pos_i64 = adjusted_pos_i64.min(duration as i64);
                    }
                    current_event_data.position_ms = Some(adjusted_pos_i64 as u64);
                }

                if current_event_data.position_report_time.is_none() {
                    current_event_data.position_report_time = Some(smtc_event_arrival_time);
                }

                let mut effective_info_for_app_state = current_event_data.clone();
                let mut is_genuinely_new_song_flag = false;
                let mut cover_actually_changed_flag = false;
                let mut music_info_for_player_needs_update_flag = false;

                let last_true_info_arc_clone =
                    std::sync::Arc::clone(&self.player.last_true_smtc_processed_info);
                let current_media_info_arc_clone =
                    std::sync::Arc::clone(&self.player.current_media_info);
                let tokio_rt_handle = self.tokio_runtime.handle().clone();

                tokio_rt_handle.block_on(async {
                    let mut last_true_guard = last_true_info_arc_clone.lock().await;
                    let opt_previous_true_info = last_true_guard.clone();
                    let mut candidate_for_last_true = current_event_data.clone();
                    let mut candidate_for_simulator_state = current_event_data.clone();

                    if let Some(ref previous_true_info_val) = opt_previous_true_info {
                        if previous_true_info_val.title != current_event_data.title
                            || previous_true_info_val.artist != current_event_data.artist
                        {
                            is_genuinely_new_song_flag = true;
                        }
                        if previous_true_info_val.cover_data_hash
                            != current_event_data.cover_data_hash
                        {
                            cover_actually_changed_flag = true;
                        }
                        if is_genuinely_new_song_flag
                            || previous_true_info_val.album_title != current_event_data.album_title
                            || previous_true_info_val.duration_ms != current_event_data.duration_ms
                        {
                            music_info_for_player_needs_update_flag = true;
                        }

                        let current_is_playing = current_event_data.is_playing.unwrap_or(false);
                        let previous_was_playing =
                            previous_true_info_val.is_playing.unwrap_or(false);

                        if current_is_playing && !previous_was_playing {
                            candidate_for_simulator_state.position_report_time =
                                current_event_data.position_report_time;
                            candidate_for_last_true.position_report_time =
                                current_event_data.position_report_time;
                        } else if current_is_playing && previous_was_playing {
                            if current_event_data.position_ms.is_some()
                                && current_event_data.position_ms
                                    == previous_true_info_val.position_ms
                            {
                                let stable_rt = previous_true_info_val.position_report_time;
                                candidate_for_simulator_state.position_report_time = stable_rt;
                                candidate_for_last_true.position_report_time = stable_rt;
                            } else {
                                candidate_for_simulator_state.position_report_time =
                                    current_event_data.position_report_time;
                                candidate_for_last_true.position_report_time =
                                    current_event_data.position_report_time;
                            }
                        } else {
                            candidate_for_simulator_state.position_report_time =
                                current_event_data.position_report_time;
                            candidate_for_last_true.position_report_time =
                                current_event_data.position_report_time;
                        }
                    } else {
                        is_genuinely_new_song_flag = true;
                        music_info_for_player_needs_update_flag = true;
                        if current_event_data.cover_data_hash.is_some() {
                            cover_actually_changed_flag = true;
                        }
                    }
                    *last_true_guard = Some(candidate_for_last_true);
                    let mut simulator_final_info = current_event_data.clone();
                    simulator_final_info.position_ms = candidate_for_simulator_state.position_ms;
                    simulator_final_info.position_report_time =
                        candidate_for_simulator_state.position_report_time;
                    let mut current_media_guard = current_media_info_arc_clone.lock().await;
                    *current_media_guard = Some(simulator_final_info.clone());
                    effective_info_for_app_state.position_ms = simulator_final_info.position_ms;
                    effective_info_for_app_state.position_report_time =
                        simulator_final_info.position_report_time;
                });

                self.player.last_smtc_position_ms =
                    effective_info_for_app_state.position_ms.unwrap_or(0);
                self.player.last_smtc_position_report_time =
                    effective_info_for_app_state.position_report_time;
                let new_app_is_playing_state = current_event_data.is_playing.unwrap_or(false);
                let previous_app_is_playing_state = self.player.is_currently_playing_sensed_by_smtc;
                self.player.is_currently_playing_sensed_by_smtc = new_app_is_playing_state;
                self.player.current_song_duration_ms = current_event_data.duration_ms.unwrap_or(0);

                info!(
                    "[UniLyric] SMTC 信息更新: 存储位置={}ms (SMTC原始位置: {:?}), 用于计时的存储报告时间={:?}, 播放中={}, 时长={}ms",
                    self.player.last_smtc_position_ms,
                    raw_smtc_position_from_event_for_log,
                    self.player.last_smtc_position_report_time,
                    self.player.is_currently_playing_sensed_by_smtc,
                    self.player.current_song_duration_ms
                );

                if self.websocket_server.enabled {
                    if is_genuinely_new_song_flag {
                        self.process_smtc_update_for_websocket(&current_event_data);
                    } else if new_app_is_playing_state && self.player.last_smtc_position_ms > 0 {
                        self.send_time_update_to_websocket(self.player.last_smtc_position_ms);
                    }
                    if !is_genuinely_new_song_flag
                        && new_app_is_playing_state != previous_app_is_playing_state
                    {
                        trace!("[UniLyric WebSocket] 播放状态改变 (非新歌)，发送 PlaybackInfo。");
                        self.process_smtc_update_for_websocket(&current_event_data);
                    }
                }

                if let Some(command_tx) = &self.player.command_tx {
                    let connector_config_guard = self.player.config.lock().unwrap();
                    if connector_config_guard.enabled {
                        if music_info_for_player_needs_update_flag {
                            let artists_vec =
                                current_event_data
                                    .artist
                                    .as_ref()
                                    .map_or_else(Vec::new, |name| {
                                        vec![ws_protocol::Artist {
                                            id: Default::default(),
                                            name: name.as_str().into(),
                                        }]
                                    });
                            let set_music_info_body = ProtocolBody::SetMusicInfo {
                                music_id: Default::default(),
                                music_name: current_event_data
                                    .title
                                    .clone()
                                    .map_or_else(Default::default, |s| s.as_str().into()),
                                album_id: Default::default(),
                                album_name: current_event_data
                                    .album_title
                                    .clone()
                                    .map_or_else(Default::default, |s| s.as_str().into()),
                                artists: artists_vec,
                                duration: current_event_data.duration_ms.unwrap_or(0),
                            };
                            if command_tx
                                .send(crate::amll_connector::ConnectorCommand::SendProtocolBody(
                                    set_music_info_body,
                                ))
                                .is_err()
                            {
                                error!("[UniLyric] 发送 SetMusicInfo 命令失败。");
                            }
                        }

                        if new_app_is_playing_state != previous_app_is_playing_state {
                            if new_app_is_playing_state {
                                trace!("[UniLyric] 状态从暂停变为播放。");
                                if command_tx
                                    .send(
                                        crate::amll_connector::ConnectorCommand::SendProtocolBody(
                                            ProtocolBody::OnResumed,
                                        ),
                                    )
                                    .is_err()
                                {
                                    error!("[UniLyric] 发送 OnResumed 命令失败。");
                                } else {
                                    trace!("[UniLyric] 已发送 OnResumed 给 AMLL Player。");
                                }
                                let progress_at_resume = ProtocolBody::OnPlayProgress {
                                    progress: self.player.last_smtc_position_ms,
                                };
                                if command_tx
                                    .send(
                                        crate::amll_connector::ConnectorCommand::SendProtocolBody(
                                            progress_at_resume.clone(),
                                        ),
                                    )
                                    .is_err()
                                {
                                    error!(
                                        "[UniLyric] 发送恢复播放时的 OnPlayProgress ({}ms) 失败。",
                                        self.player.last_smtc_position_ms
                                    );
                                } else {
                                    trace!(
                                        "[UniLyric] 状态变为播放后，立即发送 OnPlayProgress ({}ms) 给 AMLL Player。",
                                        self.player.last_smtc_position_ms
                                    );
                                }
                            } else {
                                trace!("[UniLyric] 状态从播放变为暂停。");
                                if command_tx
                                    .send(
                                        crate::amll_connector::ConnectorCommand::SendProtocolBody(
                                            ProtocolBody::OnPaused,
                                        ),
                                    )
                                    .is_err()
                                {
                                    error!("[UniLyric] 发送 OnPaused 命令失败。");
                                } else {
                                    trace!("[UniLyric] 已发送 OnPaused 给 AMLL Player。");
                                }
                                let progress_at_pause = ProtocolBody::OnPlayProgress {
                                    progress: self.player.last_smtc_position_ms,
                                };
                                if command_tx
                                    .send(
                                        crate::amll_connector::ConnectorCommand::SendProtocolBody(
                                            progress_at_pause.clone(),
                                        ),
                                    )
                                    .is_err()
                                {
                                    error!(
                                        "[UniLyric] 发送暂停时的 OnPlayProgress ({}ms) 失败。",
                                        self.player.last_smtc_position_ms
                                    );
                                } else {
                                    trace!(
                                        "[UniLyric] 状态变为暂停后，立即发送 OnPlayProgress ({}ms) 给 AMLL Player。",
                                        self.player.last_smtc_position_ms
                                    );
                                }
                            }
                        }

                        if cover_actually_changed_flag {
                            let cover_data_to_send =
                                current_event_data.cover_data.clone().unwrap_or_default();
                            trace!(
                                "[UniLyric] 封面变化，发送封面数据 (长度: {})",
                                cover_data_to_send.len()
                            );
                            let cover_body = ProtocolBody::SetMusicAlbumCoverImageData {
                                data: cover_data_to_send,
                            };
                            if command_tx
                                .send(crate::amll_connector::ConnectorCommand::SendProtocolBody(
                                    cover_body,
                                ))
                                .is_err()
                            {
                                error!("[UniLyric] 发送封面数据命令失败。");
                            }
                        }
                    }
                }

                if is_genuinely_new_song_flag
                    && !current_event_data
                        .title
                        .as_deref()
                        .unwrap_or("")
                        .trim()
                        .is_empty()
                {
                    let connector_config_guard = self.player.config.lock().unwrap();
                    let connector_is_enabled = connector_config_guard.enabled;
                    drop(connector_config_guard);

                    if connector_is_enabled {
                        trace!(
                            "[UniLyric] 正在自动搜索歌词。 歌曲名: {:?}, 艺术家: {:?}",
                            current_event_data.title, current_event_data.artist
                        );
                        app_fetch_core::update_all_search_status(
                            self,
                            crate::types::AutoSearchStatus::NotAttempted,
                        );
                        app_fetch_core::initial_auto_fetch_and_send_lyrics(
                            self,
                            current_event_data,
                        );
                    } else {
                        trace!("[UniLyric] 检测到新歌，但 AMLL Connector 未启用，不触发自动搜索。");
                    }
                }

                ActionResult::Success
            }

            PlayerAction::SmtcSessionListChanged(smtc_session_infos) => {
                trace!(
                    "[UniLyric] 收到 SMTC 会话列表更新，共 {} 个会话。",
                    smtc_session_infos.len()
                );
                let mut available_sessions_guard =
                    self.player.available_smtc_sessions.lock().unwrap();
                *available_sessions_guard = smtc_session_infos.clone();
                drop(available_sessions_guard);

                let mut selected_id_guard = self.player.selected_smtc_session_id.lock().unwrap();
                if let Some(ref current_selected_id) = *selected_id_guard
                    && !smtc_session_infos
                        .iter()
                        .any(|s| s.session_id == *current_selected_id)
                {
                    trace!(
                        "[UniLyric] 当前选择的 SMTC 会话 ID '{current_selected_id}' 已不再可用，清除选择。"
                    );
                    *selected_id_guard = None;
                }
                ActionResult::Success
            }

            PlayerAction::SelectedSmtcSessionVanished(vanished_session_id) => {
                trace!(
                    "[UniLyric] 收到通知：之前选择的 SMTC 会话 ID '{vanished_session_id}' 已消失。"
                );
                let mut selected_id_guard = self.player.selected_smtc_session_id.lock().unwrap();
                if selected_id_guard.as_ref() == Some(&vanished_session_id) {
                    *selected_id_guard = None;
                    self.ui.toasts.add(egui_toast::Toast {
                        text: format!(
                            "源应用 \"{}\" 已关闭",
                            vanished_session_id
                                .split('!')
                                .next()
                                .unwrap_or(&vanished_session_id)
                        )
                        .into(),
                        kind: egui_toast::ToastKind::Warning,
                        options: egui_toast::ToastOptions::default()
                            .duration_in_seconds(4.0)
                            .show_icon(true),
                        ..Default::default()
                    });
                }
                ActionResult::Success
            }

            PlayerAction::AudioVolumeChanged { volume, is_muted } => {
                trace!("[Unilyric] 收到 AudioVolumeChanged: vol={volume}, mute={is_muted}");
                let mut current_vol_guard = self.player.current_smtc_volume.lock().unwrap();
                *current_vol_guard = Some((volume, is_muted));
                ActionResult::Success
            }

            PlayerAction::SimulatedProgressUpdate(time_ms) => {
                if self.websocket_server.enabled && self.player.is_currently_playing_sensed_by_smtc
                {
                    self.send_time_update_to_websocket(time_ms);
                }
                ActionResult::Success
            }

            PlayerAction::ConnectAmll => {
                // TODO: 实现AMLL连接逻辑
                ActionResult::Success
            }
            PlayerAction::DisconnectAmll => {
                // TODO: 实现AMLL断开逻辑
                ActionResult::Success
            }
            PlayerAction::SelectSmtcSession(_session_id) => {
                // TODO: 实现SMTC会话选择逻辑
                ActionResult::Success
            }
            PlayerAction::SaveToLocalCache => {
                let (media_info, cache_dir, index_path) = match (
                    self.tokio_runtime
                        .block_on(async { self.player.current_media_info.lock().await.clone() }),
                    self.local_cache.dir_path.as_ref(),
                    self.local_cache.index_path.as_ref(),
                ) {
                    (Some(info), Some(dir), Some(path)) => (info, dir, path),
                    _ => {
                        log::warn!("[LocalCache] 缺少SMTC信息或缓存路径，无法保存。");
                        self.ui.toasts.add(egui_toast::Toast {
                            text: "缺少SMTC信息，无法保存到缓存".into(),
                            kind: egui_toast::ToastKind::Warning,
                            options: egui_toast::ToastOptions::default().duration_in_seconds(3.0),
                            style: Default::default(),
                        });
                        return ActionResult::Warning("缺少SMTC信息或缓存路径".to_string());
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

                if let Err(e) = std::fs::write(&file_path, &self.lyrics.output_text) {
                    log::error!("[LocalCache] 写入歌词文件 {file_path:?} 失败: {e}");
                    return ActionResult::Error(format!("写入歌词文件失败: {e}"));
                }

                let entry = crate::types::LocalLyricCacheEntry {
                    smtc_title: title.to_string(),
                    smtc_artists: artists,
                    ttml_filename: final_filename,
                    original_source_format: self.fetcher.last_source_format.map(|f| f.to_string()),
                };

                match std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(index_path)
                {
                    Ok(file) => {
                        let mut writer = std::io::BufWriter::new(file);
                        if let Ok(json_line) = serde_json::to_string(&entry) {
                            if writeln!(writer, "{json_line}").is_ok() {
                                self.local_cache.index.lock().unwrap().push(entry);
                                log::info!("[LocalCache] 成功保存歌词到本地缓存: {file_path:?}");
                                self.ui.toasts.add(egui_toast::Toast {
                                    text: "已保存到本地缓存".into(),
                                    kind: egui_toast::ToastKind::Success,
                                    options: egui_toast::ToastOptions::default()
                                        .duration_in_seconds(2.0),
                                    style: Default::default(),
                                });
                                ActionResult::Success
                            } else {
                                ActionResult::Error("序列化缓存条目失败".to_string())
                            }
                        } else {
                            ActionResult::Error("序列化缓存条目失败".to_string())
                        }
                    }
                    Err(e) => {
                        log::error!("[LocalCache] 打开或写入索引文件 {index_path:?} 失败: {e}");
                        ActionResult::Error(format!("打开或写入索引文件失败: {e}"))
                    }
                }
            }
        }
    }
    fn handle_settings_action(&mut self, action: SettingsAction) -> ActionResult {
        match action {
            SettingsAction::Save(settings) => {
                let old_send_audio_data_setting =
                    self.app_settings.lock().unwrap().send_audio_data_to_player;
                let new_send_audio_data_setting = settings.send_audio_data_to_player;

                match settings.save() {
                    Ok(_) => {
                        let new_settings_clone = settings.clone();
                        let mut app_settings_guard = self.app_settings.lock().unwrap();
                        *app_settings_guard = new_settings_clone;
                        self.player.smtc_time_offset_ms = app_settings_guard.smtc_time_offset_ms;

                        let new_mc_config_from_settings =
                            crate::amll_connector::AMLLConnectorConfig {
                                enabled: app_settings_guard.amll_connector_enabled,
                                websocket_url: app_settings_guard
                                    .amll_connector_websocket_url
                                    .clone(),
                            };
                        let connector_enabled_runtime = new_mc_config_from_settings.enabled;
                        drop(app_settings_guard);

                        let mut current_mc_config_guard = self.player.config.lock().unwrap();
                        let old_mc_config = current_mc_config_guard.clone();
                        *current_mc_config_guard = new_mc_config_from_settings.clone();
                        drop(current_mc_config_guard);

                        info!(
                            "[Settings] 设置已保存。新 AMLL Connector配置: {new_mc_config_from_settings:?}"
                        );

                        if new_mc_config_from_settings.enabled {
                            crate::amll_connector::amll_connector_manager::ensure_running(self);
                            if let Some(tx) = &self.player.command_tx
                                && old_mc_config != new_mc_config_from_settings
                            {
                                debug!(
                                    "[Settings] 发送 UpdateConfig 命令给AMLL Connector worker。"
                                );
                                if tx
                                    .send(crate::amll_connector::ConnectorCommand::UpdateConfig(
                                        new_mc_config_from_settings.clone(),
                                    ))
                                    .is_err()
                                {
                                    error!(
                                        "[Settings] 发送 UpdateConfig 命令给AMLL Connector worker 失败。"
                                    );
                                }
                            }
                        } else {
                            crate::amll_connector::amll_connector_manager::ensure_running(self);
                        }

                        if connector_enabled_runtime
                            && old_send_audio_data_setting != new_send_audio_data_setting
                        {
                            self.player.audio_visualization_is_active = new_send_audio_data_setting;
                            if let Some(tx) = &self.player.command_tx {
                                let command = if new_send_audio_data_setting {
                                    info!("[Settings] 设置更改：启动音频数据转发。");
                                    crate::amll_connector::ConnectorCommand::StartAudioVisualization
                                } else {
                                    info!("[Settings] 设置更改：停止音频数据转发。");
                                    crate::amll_connector::ConnectorCommand::StopAudioVisualization
                                };
                                if tx.send(command).is_err() {
                                    error!(
                                        "[Settings] 应用设置更改时，发送音频可视化控制命令失败。"
                                    );
                                }
                            }
                        }

                        self.ui.show_settings_window = false;
                        ActionResult::Success
                    }
                    Err(e) => {
                        error!("[Settings] 保存应用设置失败: {e}");
                        self.ui.show_settings_window = false;
                        ActionResult::Error(format!("保存设置失败: {e}"))
                    }
                }
            }
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

    /// 处理自动获取结果
    fn handle_auto_fetch_result(&mut self, auto_fetch_result: crate::types::AutoFetchResult) {
        use crate::types::{AutoFetchResult, AutoSearchSource, AutoSearchStatus};
        use crate::websocket_server::{PlaybackInfoPayload, ServerCommand};
        use log::{error, info, warn};

        match auto_fetch_result {
            AutoFetchResult::Success {
                source,
                full_lyrics_result,
            } => {
                info!("[AutoFetch] 自动获取成功，来源: {source:?}");

                // 更新结果缓存
                let result_cache_opt = match source {
                    AutoSearchSource::QqMusic => Some(&self.fetcher.last_qq_result),
                    AutoSearchSource::Kugou => Some(&self.fetcher.last_kugou_result),
                    AutoSearchSource::Netease => Some(&self.fetcher.last_netease_result),
                    AutoSearchSource::AmllDb => Some(&self.fetcher.last_amll_db_result),
                    AutoSearchSource::Musixmatch => Some(&self.fetcher.last_musixmatch_result),
                    AutoSearchSource::LocalCache => None, // 本地缓存不需要缓存
                };
                if let Some(result_cache) = result_cache_opt {
                    *result_cache.lock().unwrap() = Some(full_lyrics_result.clone());
                }

                // 更新状态
                let source_format = full_lyrics_result.parsed.source_format;
                let status_to_update = match source {
                    AutoSearchSource::QqMusic => Some(&self.fetcher.qqmusic_status),
                    AutoSearchSource::Kugou => Some(&self.fetcher.kugou_status),
                    AutoSearchSource::Netease => Some(&self.fetcher.netease_status),
                    AutoSearchSource::AmllDb => Some(&self.fetcher.amll_db_status),
                    AutoSearchSource::Musixmatch => Some(&self.fetcher.musixmatch_status),
                    AutoSearchSource::LocalCache => Some(&self.fetcher.local_cache_status),
                };
                if let Some(status_arc) = status_to_update {
                    *status_arc.lock().unwrap() = AutoSearchStatus::Success(source_format);
                }

                // 如果UI还没有填充，则处理歌词
                if !self.fetcher.current_ui_populated {
                    self.fetcher.current_ui_populated = true;

                    // 将其他搜索状态设为未找到
                    let all_search_status_arcs = [
                        &self.fetcher.local_cache_status,
                        &self.fetcher.qqmusic_status,
                        &self.fetcher.kugou_status,
                        &self.fetcher.netease_status,
                        &self.fetcher.amll_db_status,
                        &self.fetcher.musixmatch_status,
                    ];

                    for status_arc in all_search_status_arcs {
                        let mut guard = status_arc.lock().unwrap();
                        if matches!(*guard, AutoSearchStatus::Searching) {
                            *guard = AutoSearchStatus::NotFound;
                        }
                    }

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

                    let parsed_data = full_lyrics_result.parsed;
                    let raw_data = full_lyrics_result.raw;

                    // 使用获取到的原始文本和格式填充状态
                    self.lyrics.input_text = raw_data.content;
                    self.lyrics.source_format = parsed_data.source_format;
                    self.fetcher.last_source_format = Some(parsed_data.source_format);

                    self.lyrics.metadata_source_is_download = true;

                    // 触发应用的内部转换流水线
                    self.trigger_convert();
                }
            }
            AutoFetchResult::NotFound => {
                info!("[AutoFetch] 自动获取歌词：所有在线源均未找到。");

                // 更新所有搜索状态为未找到
                let sources_to_update_on_not_found = [
                    &self.fetcher.qqmusic_status,
                    &self.fetcher.kugou_status,
                    &self.fetcher.netease_status,
                    &self.fetcher.amll_db_status,
                    &self.fetcher.musixmatch_status,
                ];
                for status_arc in sources_to_update_on_not_found {
                    let mut guard = status_arc.lock().unwrap();
                    if matches!(*guard, AutoSearchStatus::Searching) {
                        *guard = AutoSearchStatus::NotFound;
                    }
                }

                // 如果UI还没有填充且AMLL连接器启用，发送空TTML
                if !self.fetcher.current_ui_populated
                    && self.player.config.lock().unwrap().enabled
                    && let Some(tx) = &self.player.command_tx
                {
                    info!("[AutoFetch] 未找到任何歌词，尝试发送空TTML给AMLL Player。");
                    let empty_ttml_body = ProtocolBody::SetLyricFromTTML { data: "".into() };
                    if tx
                        .send(crate::amll_connector::ConnectorCommand::SendProtocolBody(
                            empty_ttml_body,
                        ))
                        .is_err()
                    {
                        error!("[AutoFetch] (未找到歌词) 发送空TTML失败。");
                    }
                }

                // 如果WebSocket服务器启用且UI还没有填充，发送空歌词
                if self.websocket_server.enabled && !self.fetcher.current_ui_populated {
                    let mut current_title = None;
                    let mut current_artist = None;
                    if let Ok(media_info_guard) = self.player.current_media_info.try_lock()
                        && let Some(info) = &*media_info_guard
                    {
                        current_title = info.title.clone();
                        current_artist = info.artist.clone();
                    }
                    let empty_lyrics_payload = PlaybackInfoPayload {
                        title: current_title,
                        artist: current_artist,
                        ttml_lyrics: None,
                    };
                    if let Some(ws_tx) = &self.websocket_server.command_tx
                        && let Err(e) = ws_tx
                            .try_send(ServerCommand::BroadcastPlaybackInfo(empty_lyrics_payload))
                    {
                        warn!("[AutoFetch] 发送空歌词PlaybackInfo到WebSocket失败: {e}");
                    }
                }
            }
            AutoFetchResult::FetchError(err_msg) => {
                error!("[AutoFetch] 自动获取歌词时发生错误: {err_msg}");
            }
        }
    }
}
