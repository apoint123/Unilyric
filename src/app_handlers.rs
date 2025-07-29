use crate::amll_connector::types::ActorSettings;
use crate::amll_connector::{AMLLConnectorConfig, ConnectorCommand};
use crate::app_actions::{
    AmllConnectorAction, FileAction, LyricsAction, PanelType, PlayerAction, SettingsAction,
    UIAction, UserAction,
};
use crate::app_definition::UniLyricApp;
use crate::app_handlers::ConnectorCommand::SendLyric;
use crate::app_handlers::ConnectorCommand::UpdateActorSettings;
use crate::types::{ChineseConversionVariant, EditableMetadataEntry, LrcContentType};
use rand::Rng;
use smtc_suite::MediaCommand;
use tracing::warn;
use tracing::{debug, error, info};

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
                        ActionResult::Error(format!("转换失败: {e}"))
                    }
                }
            }
            LyricsAction::ConvertChinese(variant) => {
                info!("[Convert] Starting Chinese conversion with variant: {variant:?}");

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
                            config_name: Some(variant.to_filename().to_string()),
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
                self.clear_lyrics_state_for_new_song_internal();
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
                // 这是“自动加载”的逻辑
                if let crate::types::AutoFetchResult::Success {
                    source,
                    full_lyrics_result,
                } = auto_fetch_result
                {
                    if !self.fetcher.current_ui_populated {
                        info!("[AutoFetch] UI未被填充，正在自动加载来自 {source:?} 的歌词。");

                        self.send_action(UserAction::Lyrics(LyricsAction::LoadFetchedResult(
                            full_lyrics_result,
                        )));
                    } else {
                        info!("[AutoFetch] UI已被填充，跳过对 {source:?} 结果的自动加载。");
                    }
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
        }
    }

    fn clear_lyrics_state_for_new_song_internal(&mut self) {
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
                command_tx.send(MediaCommand::Control(control_command))
            }
            PlayerAction::SelectSmtcSession(session_id) => {
                let session_id_for_state: Option<String>;
                if session_id.is_empty() {
                    tracing::info!("[PlayerAction] 自动选择会话。");
                    session_id_for_state = None;
                } else {
                    tracing::info!("[PlayerAction] 选择新的 SMTC 会话: {}", session_id);
                    session_id_for_state = Some(session_id.clone());
                }

                self.player.last_requested_session_id = session_id_for_state.clone();
                if let Ok(mut settings) = self.app_settings.lock() {
                    settings.last_selected_smtc_session_id = session_id_for_state;
                    if let Err(e) = settings.save() {
                        tracing::warn!("[PlayerAction] 保存上次选择的SMTC会话ID失败: {}", e);
                    }
                }
                command_tx.send(MediaCommand::SelectSession(session_id))
            }
            PlayerAction::SaveToLocalCache => {
                return self.save_lyrics_to_local_cache();
            }
        };

        if let Err(e) = send_result {
            error!("[PlayerAction] 发送命令到 smtc-suite 失败: {}", e);
            return ActionResult::Error("向媒体服务发送命令失败".to_string());
        }

        ActionResult::Success
    }

    fn save_lyrics_to_local_cache(&mut self) -> ActionResult {
        let (media_info, cache_dir, index_path) = match (
            self.player.current_now_playing.clone(),
            self.local_cache.dir_path.as_ref(),
            self.local_cache.index_path.as_ref(),
        ) {
            // 确保标题存在
            (info, Some(dir), Some(path)) if info.title.is_some() => (info, dir, path),
            _ => {
                tracing::warn!("[LocalCache] 缺少SMTC信息或缓存路径，无法保存。");
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
            .map(|s| {
                s.split(['/', ';', '、'])
                    .map(|n| n.trim().to_string())
                    .collect()
            })
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
            tracing::error!("[LocalCache] 写入歌词文件 {file_path:?} 失败: {e}");
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
                use std::io::Write;
                let mut writer = std::io::BufWriter::new(file);
                if let Ok(json_line) = serde_json::to_string(&entry) {
                    if writeln!(writer, "{json_line}").is_ok() {
                        self.local_cache.index.lock().unwrap().push(entry);
                        tracing::info!("[LocalCache] 成功保存歌词到本地缓存: {file_path:?}");
                        self.ui.toasts.add(egui_toast::Toast {
                            text: "已保存到本地缓存".into(),
                            kind: egui_toast::ToastKind::Success,
                            options: egui_toast::ToastOptions::default().duration_in_seconds(2.0),
                            style: Default::default(),
                        });
                        ActionResult::Success
                    } else {
                        ActionResult::Error("写入缓存索引失败".to_string())
                    }
                } else {
                    ActionResult::Error("序列化缓存条目失败".to_string())
                }
            }
            Err(e) => {
                tracing::error!("[LocalCache] 打开或写入索引文件 {index_path:?} 失败: {e}");
                ActionResult::Error(format!("打开或写入索引文件失败: {e}"))
            }
        }
    }

    fn handle_settings_action(&mut self, action: SettingsAction) -> ActionResult {
        match action {
            SettingsAction::Save(settings) => match settings.save() {
                Ok(_) => {
                    let new_settings_clone = settings.clone();
                    let mut app_settings_guard = self.app_settings.lock().unwrap();
                    *app_settings_guard = new_settings_clone;
                    self.player.smtc_time_offset_ms = app_settings_guard.smtc_time_offset_ms;

                    let new_mc_config_from_settings = AMLLConnectorConfig {
                        enabled: app_settings_guard.amll_connector_enabled,
                        websocket_url: app_settings_guard.amll_connector_websocket_url.clone(),
                    };

                    let new_actor_settings = ActorSettings {
                        enable_t2s_conversion: app_settings_guard.enable_t2s_for_auto_search,
                    };

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

                    drop(app_settings_guard);

                    let mut current_mc_config_guard = self.amll_connector.config.lock().unwrap();
                    let old_mc_config = current_mc_config_guard.clone();
                    *current_mc_config_guard = new_mc_config_from_settings.clone();
                    drop(current_mc_config_guard);

                    info!(
                        "[Settings] 设置已保存。新 AMLL Connector配置: {new_mc_config_from_settings:?}"
                    );

                    if new_mc_config_from_settings.enabled
                        && let Some(tx) = &self.amll_connector.command_tx
                        && old_mc_config != new_mc_config_from_settings
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

                    self.ui.show_settings_window = false;
                    ActionResult::Success
                }
                Err(e) => {
                    error!("[Settings] 保存应用设置失败: {e}");
                    self.ui.show_settings_window = false;
                    ActionResult::Error(format!("保存设置失败: {e}"))
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

    /// 处理自动获取结果
    fn handle_auto_fetch_result(&mut self, auto_fetch_result: crate::types::AutoFetchResult) {
        use crate::types::{AutoFetchResult, AutoSearchSource, AutoSearchStatus};
        use tracing::{error, info};

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

                    self.clear_lyrics_state_for_new_song_internal();

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
                    && self.amll_connector.config.lock().unwrap().enabled
                    && let Some(tx) = &self.amll_connector.command_tx
                {
                    info!("[UniLyricApp] 未找到任何歌词，尝试发送空TTML给AMLL Player。");
                    let empty_ttml =
                        crate::amll_connector::protocol::ClientMessage::SetLyricFromTTML {
                            data: "".into(),
                        };
                    if tx
                        .try_send(crate::amll_connector::ConnectorCommand::SendClientMessage(
                            empty_ttml,
                        ))
                        .is_err()
                    {
                        error!("[UniLyricApp] (未找到歌词) 发送空TTML失败。");
                    }
                }
            }
            AutoFetchResult::FetchError(err_msg) => {
                error!("[AutoFetch] 自动获取歌词时发生错误: {err_msg}");
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
        variant: ChineseConversionVariant,
        label: &str,
        enabled: bool,
    ) {
        if ui
            .add_enabled(enabled, egui::Button::new(label))
            .on_disabled_hover_text("请先加载主歌词")
            .clicked()
        {
            self.send_action(UserAction::Lyrics(LyricsAction::ConvertChinese(variant)));
        }
    }
}
