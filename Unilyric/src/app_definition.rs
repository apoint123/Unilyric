use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::sync::{
    Arc, Mutex as StdMutex,
    mpsc::{Receiver as StdReceiver, Sender as StdSender, channel as std_channel},
};

use egui_toast::Toasts;
use lyrics_helper_core::{
    CanonicalMetadataKey, FullConversionResult, LyricFormat, MetadataStore, ParsedSourceData,
};
use lyrics_helper_core::{SearchResult, model::track::FullLyricsResult};
use lyrics_helper_rs::LyricsHelperError;
use rand::Rng;
use smtc_suite::{MediaCommand, NowPlayingInfo, SmtcSessionInfo, TextConversionMode};
use tokio::{
    sync::Mutex as TokioMutex,
    sync::mpsc::{Sender as TokioSender, channel as tokio_channel},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::amll_connector::types::UiUpdate;
use crate::app_ui::SettingsCategory;
use crate::types::{EditableMetadataEntry, ProviderState};
use crate::{
    amll_connector::{AMLLConnectorConfig, ConnectorCommand, WebsocketStatus},
    app_actions::UserAction,
    app_settings::AppSettings,
    types::{AutoFetchResult, AutoSearchStatus, LocalLyricCacheEntry, LogEntry},
    utils,
};

pub(super) type ConversionResultRx = StdReceiver<Result<FullConversionResult, LyricsHelperError>>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum AppView {
    #[default]
    Editor,
    Downloader,
}

#[derive(Debug, Clone, Default)]
pub(super) enum SearchState {
    #[default]
    Idle,
    Searching,
    Success(Vec<SearchResult>),
    Error(String),
}

#[derive(Debug, Clone, Default)]
pub(super) enum PreviewState {
    #[default]
    Idle,
    Loading,
    Success(String),
    Error(String),
}

#[derive(Debug, Clone, Default)]
pub(super) struct DownloaderState {
    pub(super) title_input: String,
    pub(super) artist_input: String,
    pub(super) album_input: String,
    pub(super) duration_ms_input: u64,
    pub(super) search_state: SearchState,
    pub(super) selected_result_for_preview: Option<SearchResult>,
    pub(super) preview_state: PreviewState,
    pub(super) selected_full_lyrics: Option<FullLyricsResult>,
}

pub(super) struct UiState {
    pub(super) show_bottom_log_panel: bool,
    pub(super) new_trigger_log_exists: bool,
    pub(super) show_romanization_lrc_panel: bool,
    pub(super) show_translation_lrc_panel: bool,
    pub(super) wrap_text: bool,
    pub(super) show_settings_window: bool,
    pub(super) show_amll_connector_sidebar: bool,
    pub(super) show_metadata_panel: bool,
    pub(super) show_markers_panel: bool,
    pub(super) log_display_buffer: Vec<LogEntry>,
    pub(super) temp_edit_settings: AppSettings,
    pub(super) toasts: Toasts,
    pub(super) available_system_fonts: Vec<String>,
    pub(super) current_settings_category: SettingsCategory,
    pub(super) current_view: AppView,
}

impl UiState {
    fn new(settings: &AppSettings) -> Self {
        let toasts = Toasts::new()
            .anchor(egui::Align2::LEFT_TOP, (10.0, 10.0))
            .direction(egui::Direction::TopDown);

        Self {
            toasts,
            show_amll_connector_sidebar: settings.amll_connector_enabled,
            temp_edit_settings: settings.clone(),
            show_bottom_log_panel: false,
            new_trigger_log_exists: false,
            show_romanization_lrc_panel: false,
            show_translation_lrc_panel: false,
            wrap_text: true,
            show_settings_window: false,
            show_metadata_panel: false,
            show_markers_panel: false,
            log_display_buffer: Vec::with_capacity(200),
            available_system_fonts: Vec::new(),
            current_settings_category: SettingsCategory::default(),
            current_view: AppView::default(),
        }
    }
}

pub(super) struct LyricState {
    pub(super) input_text: String,
    pub(super) output_text: String,
    pub(super) display_translation_lrc_output: String,
    pub(super) display_romanization_lrc_output: String,
    pub(super) parsed_lyric_data: Option<ParsedSourceData>,
    pub(super) loaded_translation_lrc: Option<Vec<crate::types::DisplayLrcLine>>,
    pub(super) loaded_romanization_lrc: Option<Vec<crate::types::DisplayLrcLine>>,
    pub(super) metadata_manager: UiMetadataManager,
    pub(super) metadata_source_is_download: bool,
    pub(super) current_markers: Vec<(usize, String)>,
    pub(super) source_format: LyricFormat,
    pub(super) target_format: LyricFormat,
    pub(super) available_formats: Vec<LyricFormat>,
    pub(super) last_opened_file_path: Option<std::path::PathBuf>,
    pub(super) last_saved_file_path: Option<std::path::PathBuf>,
    pub(super) conversion_in_progress: bool,
    pub(super) conversion_result_rx: Option<ConversionResultRx>,
}

pub(super) struct LyricsHelperState {
    pub(super) helper: Arc<TokioMutex<lyrics_helper_rs::LyricsHelper>>,
    pub(super) provider_state: ProviderState,
    pub(super) provider_load_result_rx: Option<StdReceiver<Result<(), String>>>,
}

impl LyricState {
    fn new(_settings: &AppSettings) -> Self {
        Self {
            input_text: String::new(),
            output_text: String::new(),
            display_translation_lrc_output: String::new(),
            display_romanization_lrc_output: String::new(),
            parsed_lyric_data: None,
            loaded_translation_lrc: None,
            loaded_romanization_lrc: None,
            metadata_manager: UiMetadataManager::default(),
            metadata_source_is_download: false,
            current_markers: Vec::new(),
            source_format: LyricFormat::Lrc,
            target_format: LyricFormat::Ttml,
            available_formats: vec![
                LyricFormat::Ass,
                LyricFormat::Ttml,
                LyricFormat::AppleMusicJson,
                LyricFormat::Lys,
                LyricFormat::Lrc,
                LyricFormat::EnhancedLrc,
                LyricFormat::Qrc,
                LyricFormat::Yrc,
                LyricFormat::Lyl,
                LyricFormat::Spl,
                LyricFormat::Lqe,
                LyricFormat::Krc,
            ],
            last_opened_file_path: None,
            last_saved_file_path: None,
            conversion_in_progress: false,
            conversion_result_rx: None,
        }
    }
}

pub(super) struct PlayerState {
    pub(super) command_tx: Option<TokioSender<MediaCommand>>,
    pub(super) current_now_playing: NowPlayingInfo,
    pub(super) available_sessions: Vec<SmtcSessionInfo>,
    pub(super) smtc_time_offset_ms: i64,
    pub(super) last_requested_session_id: Option<String>,
    pub(super) is_first_song_processed: bool,
}

impl PlayerState {
    fn new(settings: &AppSettings, command_tx: Option<TokioSender<MediaCommand>>) -> Self {
        Self {
            command_tx,
            current_now_playing: NowPlayingInfo::default(),
            available_sessions: Vec::new(),
            smtc_time_offset_ms: settings.smtc_time_offset_ms,
            last_requested_session_id: None,
            is_first_song_processed: false,
        }
    }
}

pub(super) struct AmllConnectorState {
    pub command_tx: Option<TokioSender<ConnectorCommand>>,
    pub actor_handle: Option<JoinHandle<()>>,
    pub status: Arc<StdMutex<WebsocketStatus>>,
    pub config: Arc<StdMutex<AMLLConnectorConfig>>,
    pub update_rx: std::sync::mpsc::Receiver<UiUpdate>,
}

impl AmllConnectorState {
    fn new(
        command_tx: TokioSender<ConnectorCommand>,
        update_rx: StdReceiver<UiUpdate>,
        actor_handle: tokio::task::JoinHandle<()>,
        config: AMLLConnectorConfig,
    ) -> Self {
        Self {
            command_tx: Some(command_tx),
            actor_handle: Some(actor_handle),
            status: Arc::new(StdMutex::new(WebsocketStatus::default())),
            config: Arc::new(StdMutex::new(config)),
            update_rx,
        }
    }
    fn new_disabled() -> Self {
        let (_tx, rx) = std_channel();
        Self {
            command_tx: None,
            actor_handle: None,
            status: Arc::new(StdMutex::new(WebsocketStatus::Disconnected)),
            config: Arc::new(StdMutex::new(AMLLConnectorConfig {
                enabled: false,
                ..Default::default()
            })),
            update_rx: rx,
        }
    }
}

pub(super) struct AutoFetchState {
    pub(super) result_rx: StdReceiver<AutoFetchResult>,
    pub(super) result_tx: StdSender<AutoFetchResult>,

    pub current_fetch_cancellation_token: Option<CancellationToken>,
    pub(super) current_ui_populated: bool,
    pub(super) last_source_format: Option<LyricFormat>,
    pub(super) local_cache_status: Arc<StdMutex<AutoSearchStatus>>,
    pub(super) qqmusic_status: Arc<StdMutex<AutoSearchStatus>>,
    pub(super) kugou_status: Arc<StdMutex<AutoSearchStatus>>,
    pub(super) netease_status: Arc<StdMutex<AutoSearchStatus>>,
    pub(super) amll_db_status: Arc<StdMutex<AutoSearchStatus>>,
    pub(super) last_qq_result: Arc<StdMutex<Option<FullLyricsResult>>>,
    pub(super) last_kugou_result: Arc<StdMutex<Option<FullLyricsResult>>>,
    pub(super) last_netease_result: Arc<StdMutex<Option<FullLyricsResult>>>,
    pub(super) last_amll_db_result: Arc<StdMutex<Option<FullLyricsResult>>>,
}

impl AutoFetchState {
    fn new(result_tx: StdSender<AutoFetchResult>, result_rx: StdReceiver<AutoFetchResult>) -> Self {
        Self {
            result_rx,
            result_tx,
            current_fetch_cancellation_token: None,
            current_ui_populated: false,
            last_source_format: None,
            local_cache_status: Arc::new(StdMutex::new(AutoSearchStatus::default())),
            qqmusic_status: Arc::new(StdMutex::new(AutoSearchStatus::default())),
            kugou_status: Arc::new(StdMutex::new(AutoSearchStatus::default())),
            netease_status: Arc::new(StdMutex::new(AutoSearchStatus::default())),
            amll_db_status: Arc::new(StdMutex::new(AutoSearchStatus::default())),
            last_qq_result: Arc::new(StdMutex::new(None)),
            last_kugou_result: Arc::new(StdMutex::new(None)),
            last_netease_result: Arc::new(StdMutex::new(None)),
            last_amll_db_result: Arc::new(StdMutex::new(None)),
        }
    }
}

#[derive(Default)]
pub(super) struct LocalCacheState {
    pub(super) index: Arc<StdMutex<Vec<LocalLyricCacheEntry>>>,
    pub(super) index_path: Option<std::path::PathBuf>,
    pub(super) dir_path: Option<std::path::PathBuf>,
    pub(super) cover_cache_dir: Option<std::path::PathBuf>,
}

pub(super) struct UniLyricApp {
    // --- 状态模块 ---
    pub(super) ui: UiState,
    pub(super) lyrics: LyricState,
    pub(super) player: PlayerState,
    pub(super) fetcher: AutoFetchState,
    pub(super) local_cache: LocalCacheState,
    pub(super) amll_connector: AmllConnectorState,
    pub(super) downloader: DownloaderState,

    // --- 核心依赖与配置 ---
    pub(super) lyrics_helper_state: LyricsHelperState,
    pub(super) app_settings: Arc<StdMutex<AppSettings>>,
    pub(super) tokio_runtime: Arc<tokio::runtime::Runtime>,
    pub(super) ui_log_receiver: StdReceiver<LogEntry>,

    // --- 事件系统 ---
    pub(super) action_tx: StdSender<UserAction>,
    pub(super) action_rx: StdReceiver<UserAction>,
    pub(super) egui_ctx: egui::Context,
    pub(super) actions_this_frame: Vec<UserAction>,

    // --- 标记 ---
    pub(super) shutdown_initiated: bool,
    pub(super) auto_fetch_trigger_time: Option<std::time::Instant>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct UiMetadataManager {
    pub(super) store: MetadataStore,
    pub(super) ui_entries: Vec<EditableMetadataEntry>,
}

impl UiMetadataManager {
    pub fn add_new_ui_entry(&mut self, key: CanonicalMetadataKey) {
        let new_entry_id_num = self.ui_entries.len() as u32 + rand::rng().random::<u32>();
        let new_id = egui::Id::new(format!("new_editable_meta_entry_{new_entry_id_num}"));
        self.ui_entries.push(EditableMetadataEntry {
            key,
            value: "".to_string(),
            is_pinned: false,
            is_from_file: false,
            id: new_id,
        });
    }

    pub fn remove_ui_entry(&mut self, index: usize) -> bool {
        if index < self.ui_entries.len() {
            self.ui_entries.remove(index);
            true
        } else {
            false
        }
    }

    pub fn get_metadata_for_backend(&self) -> std::collections::HashMap<String, Vec<String>> {
        let mut grouped_by_key = std::collections::HashMap::<String, Vec<String>>::new();
        for entry in &self.ui_entries {
            let key_string = entry.key.to_string();
            if !key_string.trim().is_empty() {
                grouped_by_key
                    .entry(key_string)
                    .or_default()
                    .push(entry.value.clone());
            }
        }
        grouped_by_key
    }

    pub fn merge_from_backend(&mut self, parsed: &ParsedSourceData) {
        let old_entries = std::mem::take(&mut self.ui_entries);
        let pinned_entries: Vec<EditableMetadataEntry> =
            old_entries.into_iter().filter(|e| e.is_pinned).collect();
        let pinned_keys: std::collections::HashSet<CanonicalMetadataKey> =
            pinned_entries.iter().map(|e| e.key.clone()).collect();
        let mut new_non_conflicting_entries: Vec<EditableMetadataEntry> = Vec::new();
        let mut loaded_count = 0;
        for (key_str, values) in &parsed.raw_metadata {
            if let Ok(canonical_key) = key_str.parse::<CanonicalMetadataKey>() {
                if !pinned_keys.contains(&canonical_key) {
                    for value in values {
                        loaded_count += 1;
                        new_non_conflicting_entries.push(EditableMetadataEntry {
                            key: canonical_key.clone(),
                            value: value.clone(),
                            is_pinned: false,
                            is_from_file: true,
                            id: egui::Id::new(format!(
                                "meta_entry_{}",
                                rand::rng().random::<u64>()
                            )),
                        });
                    }
                }
            } else {
                warn!("无法从源解析元数据键 '{}'，已跳过。", key_str);
            }
        }
        let mut final_entries = pinned_entries;
        final_entries.extend(new_non_conflicting_entries);

        final_entries.sort_unstable_by(|a, b| {
            let rank_a = a.key.get_order_rank();
            let rank_b = b.key.get_order_rank();
            if rank_a != rank_b {
                rank_a.cmp(&rank_b)
            } else if let (
                CanonicalMetadataKey::Custom(key_a),
                CanonicalMetadataKey::Custom(key_b),
            ) = (&a.key, &b.key)
            {
                key_a.cmp(key_b)
            } else {
                std::cmp::Ordering::Equal
            }
        });
        self.ui_entries = final_entries;
        self.sync_store_from_ui_entries();
    }

    pub fn sync_store_from_ui_entries(&mut self) {
        self.store.clear();
        let mut grouped_by_key = HashMap::<String, Vec<String>>::new();
        for entry in &self.ui_entries {
            let key_string = entry.key.to_string();
            grouped_by_key
                .entry(key_string)
                .or_default()
                .push(entry.value.clone());
        }

        for (key, values) in grouped_by_key {
            self.store.set_multiple(&key, values);
        }
    }

    pub fn load_from_parsed_data(&mut self, parsed: &ParsedSourceData) {
        self.store.clear();
        self.store.load_from_raw(&parsed.raw_metadata);
        self.merge_from_backend(parsed);
    }
}

impl UniLyricApp {
    pub(super) fn new(
        cc: &eframe::CreationContext,
        settings: AppSettings,
        ui_log_receiver: StdReceiver<LogEntry>,
    ) -> Self {
        let (action_tx, action_rx) = std_channel::<UserAction>();

        let egui_ctx = cc.egui_ctx.clone();
        Self::setup_fonts(&cc.egui_ctx, &settings);
        let tokio_runtime = Self::create_tokio_runtime();
        let (auto_fetch_tx, auto_fetch_rx) = std_channel::<AutoFetchResult>();
        let auto_fetch_state = AutoFetchState::new(auto_fetch_tx, auto_fetch_rx);
        let local_cache = LocalCacheState::default();

        let lyric_state = LyricState::new(&settings);
        let player_state;
        let amll_connector_state;

        if settings.amll_connector_enabled {
            let (smtc_controller, smtc_update_rx) =
                smtc_suite::MediaManager::start().expect("smtc-suite 启动失败");

            player_state = PlayerState::new(&settings, Some(smtc_controller.command_tx.clone()));

            let _ = smtc_controller
                .command_tx
                .try_send(MediaCommand::SetProgressOffset(
                    settings.smtc_time_offset_ms,
                ));

            let conversion_mode = if settings.enable_t2s_for_auto_search {
                TextConversionMode::TraditionalToSimplified
            } else {
                TextConversionMode::Off
            };
            let _ = smtc_controller
                .command_tx
                .try_send(MediaCommand::SetTextConversion(conversion_mode));

            let audio_capture_command = if settings.send_audio_data_to_player {
                MediaCommand::StartAudioCapture
            } else {
                MediaCommand::StopAudioCapture
            };
            let _ = smtc_controller.command_tx.try_send(audio_capture_command);

            let mc_config = AMLLConnectorConfig {
                enabled: settings.amll_connector_enabled,
                websocket_url: settings.amll_connector_websocket_url.clone(),
            };

            let (amll_update_tx, amll_update_rx) = std_channel::<UiUpdate>();
            let (amll_command_tx, amll_command_rx) = tokio_channel::<ConnectorCommand>(32);

            let amll_actor_handle =
                tokio_runtime.spawn(crate::amll_connector::worker::amll_connector_actor(
                    amll_command_rx,
                    amll_update_tx,
                    mc_config.clone(),
                    smtc_controller.command_tx.clone(),
                    smtc_update_rx,
                ));

            amll_connector_state = AmllConnectorState::new(
                amll_command_tx,
                amll_update_rx,
                amll_actor_handle,
                mc_config,
            );
        } else {
            player_state = PlayerState::new(&settings, None);
            amll_connector_state = AmllConnectorState::new_disabled();
        }

        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let mut font_families: Vec<String> = db
            .faces()
            .flat_map(|face| face.families.clone())
            .map(|(family_name, _lang)| family_name)
            .collect();
        font_families.sort();
        font_families.dedup();

        let mut ui_state = UiState::new(&settings);
        ui_state.available_system_fonts = font_families;

        let helper = Arc::new(TokioMutex::new(lyrics_helper_rs::LyricsHelper::new()));
        let lyrics_helper_state = LyricsHelperState {
            helper,
            provider_state: ProviderState::Uninitialized,
            provider_load_result_rx: None,
        };

        let mut app = Self {
            ui: ui_state,
            lyrics: lyric_state,
            player: player_state,
            amll_connector: amll_connector_state,
            fetcher: auto_fetch_state,
            local_cache,
            downloader: DownloaderState::default(),
            lyrics_helper_state,
            app_settings: Arc::new(StdMutex::new(settings)),
            tokio_runtime,
            ui_log_receiver,
            action_tx: action_tx.clone(),
            action_rx,
            egui_ctx,
            actions_this_frame: Vec::new(),
            shutdown_initiated: false,
            auto_fetch_trigger_time: None,
        };

        app.load_local_cache();
        app.trigger_provider_loading();
        app.trigger_cache_cleanup();

        app
    }

    fn setup_fonts(ctx: &egui::Context, settings: &AppSettings) {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "SarasaUiSC".to_owned(),
            egui::FontData::from_static(include_bytes!("../assets/fonts/SarasaUiSC-Regular.ttf"))
                .into(),
        );

        let mut user_font_loaded = false;
        if let Some(font_family_name) = &settings.selected_font_family {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();

            let query = fontdb::Query {
                families: &[fontdb::Family::Name(font_family_name)],
                weight: fontdb::Weight::NORMAL,
                stretch: fontdb::Stretch::Normal,
                style: fontdb::Style::Normal,
            };

            if let Some(face_id) = db.query(&query) {
                let loaded_font = db.with_face_data(face_id, |font_data, _face_index| {
                    egui::FontData::from_owned(font_data.to_vec())
                });

                if let Some(font_data) = loaded_font {
                    let font_key = format!("user_{}", font_family_name);
                    fonts.font_data.insert(font_key.clone(), font_data.into());
                    fonts
                        .families
                        .entry(egui::FontFamily::Proportional)
                        .or_default()
                        .insert(0, font_key);
                    user_font_loaded = true;
                } else {
                    error!("读取字体文件失败: {}", font_family_name);
                }
            } else {
                warn!("未找到上次选择的字体: {}", font_family_name);
            }
        }

        let proportional_fonts = fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default();
        if !user_font_loaded {
            proportional_fonts.insert(0, "SarasaUiSC".to_owned());
        } else {
            proportional_fonts.push("SarasaUiSC".to_owned());
        }
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push("SarasaUiSC".to_owned());
        ctx.set_fonts(fonts);

        egui_extras::install_image_loaders(ctx);
    }

    fn create_tokio_runtime() -> Arc<tokio::runtime::Runtime> {
        Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .thread_name("unilyric-app-tokio")
                .build()
                .expect("无法为应用创建 Tokio 运行时"),
        )
    }

    fn load_local_cache(&mut self) {
        if let Some(data_dir) = utils::get_app_data_dir() {
            let cache_dir = data_dir.join("local_lyrics_cache");
            if !cache_dir.exists()
                && let Err(e) = std::fs::create_dir_all(&cache_dir)
            {
                error!("[UniLyricApp] 无法创建本地歌词目录 {cache_dir:?}: {e}");
                return;
            }
            self.local_cache.dir_path = Some(cache_dir.clone());

            let cover_cache_dir = data_dir.join("local_cover_cache");

            if !cover_cache_dir.exists()
                && let Err(e) = std::fs::create_dir_all(&cover_cache_dir)
            {
                error!("[UniLyricApp] 无法创建封面缓存目录 {cover_cache_dir:?}: {e}");
            }

            self.local_cache.cover_cache_dir = Some(cover_cache_dir);

            let index_file = cache_dir.join("local_lyrics_index.jsonl");
            if index_file.exists()
                && let Ok(file) = File::open(&index_file)
            {
                let reader = BufReader::new(file);
                let cache_entries: Vec<LocalLyricCacheEntry> = reader
                    .lines()
                    .map_while(Result::ok)
                    .filter(|line| !line.trim().is_empty())
                    .filter_map(|line| serde_json::from_str(&line).ok())
                    .collect();

                info!(
                    "[UniLyricApp] 从 {:?} 加载了 {} 条本地缓存歌词索引。",
                    index_file,
                    cache_entries.len()
                );
                *self.local_cache.index.lock().unwrap() = cache_entries;
            }
            self.local_cache.index_path = Some(index_file);
        }
    }

    pub(super) fn send_shutdown_signals(&mut self) {
        if let Some(tx) = &self.player.command_tx {
            debug!("[Shutdown] 正在发送 Shutdown 命令到 smtc-suite ...");
            let _ = tx.try_send(MediaCommand::Shutdown);
        }

        if let Some(tx) = &self.amll_connector.command_tx {
            debug!("[Shutdown] 正在发送 Shutdown 命令到 actor...");
            let _ = tx.try_send(ConnectorCommand::Shutdown);
        }

        if let Some(handle) = self.amll_connector.actor_handle.take() {
            info!("[Shutdown] 正在等待 AMLL connector actor 任务结束...");
            let _ = self.tokio_runtime.block_on(handle);
            info!("[Shutdown] AMLL connector actor 任务已成功结束。");
        }
    }

    fn trigger_cache_cleanup(&self) {
        let settings = self.app_settings.lock().unwrap().clone();
        if !settings.enable_cover_cache_cleanup {
            return;
        }

        if let Some(cover_cache_dir) = self.local_cache.cover_cache_dir.clone() {
            self.tokio_runtime.spawn(async move {
                cleanup_cover_cache(cover_cache_dir, settings.max_cover_cache_files).await;
            });
        }
    }
}

async fn cleanup_cover_cache(cache_dir: std::path::PathBuf, max_files: usize) {
    let entries = match std::fs::read_dir(&cache_dir) {
        Ok(entries) => entries,
        Err(e) => {
            error!("[CacheCleanup] 无法读取缓存目录 {:?}: {}", cache_dir, e);
            return;
        }
    };

    let mut files_with_meta = Vec::new();
    for entry in entries.flatten() {
        if let Ok(metadata) = entry.metadata()
            && metadata.is_file()
            && let Ok(modified) = metadata.modified()
        {
            files_with_meta.push((entry.path(), modified));
        }
    }

    if files_with_meta.len() <= max_files {
        return;
    }

    files_with_meta.sort_unstable_by_key(|a| a.1);

    let files_to_delete_count = files_with_meta.len() - max_files;
    let mut deleted_count = 0;

    for (path, _) in files_with_meta.iter().take(files_to_delete_count) {
        if std::fs::remove_file(path).is_ok() {
            deleted_count += 1;
        } else {
            warn!("[CacheCleanup] 删除文件 {:?} 失败", path);
        }
    }

    if deleted_count > 0 {
        info!("[CacheCleanup] 删除了 {} 个封面。", deleted_count);
    }

    info!("[CacheCleanup] 封面缓存清理任务完成。");
}
