use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::sync::{
    Arc, Mutex as StdMutex,
    mpsc::{Receiver as StdReceiver, Sender as StdSender, channel as std_channel},
};

use egui_toast::Toasts;
use lyrics_helper_core::{
    BatchConversionConfig, BatchFileId, BatchLoadedFile, FullConversionResult, LyricFormat,
    ParsedSourceData,
};
use lyrics_helper_core::{SearchResult, model::track::FullLyricsResult};
use lyrics_helper_rs::LyricsHelperError;
use smtc_suite::{MediaCommand, NowPlayingInfo, SmtcSessionInfo, TextConversionMode};
use tokio::{
    sync::Mutex as TokioMutex,
    sync::mpsc::{Sender as TokioSender, channel as tokio_channel},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::amll_connector::types::UiUpdate;
use crate::types::{ProviderState, SettingsCategory};
use crate::{
    amll_connector::{AMLLConnectorConfig, ConnectorCommand, WebsocketStatus},
    app_actions::UserAction,
    app_settings::AppSettings,
    types::{AutoFetchResult, AutoSearchStatus, LocalLyricCacheEntry, LogEntry},
    utils,
};

pub type ConversionResultRx = StdReceiver<Result<FullConversionResult, LyricsHelperError>>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AppView {
    #[default]
    Editor,
    Downloader,
    BatchConverter,
}

#[derive(Debug, Clone, Default)]
pub enum SearchState {
    #[default]
    Idle,
    Searching,
    Success(Vec<SearchResult>),
    Error(String),
}

#[derive(Debug, Clone, Default)]
pub enum PreviewState {
    #[default]
    Idle,
    Loading,
    Success(String),
    Error(String),
}

#[derive(Debug, Clone, Default)]
pub struct DownloaderState {
    pub title_input: String,
    pub artist_input: String,
    pub album_input: String,
    pub duration_ms_input: u64,
    pub search_state: SearchState,
    pub selected_result_for_preview: Option<SearchResult>,
    pub preview_state: PreviewState,
    pub selected_full_lyrics: Option<FullLyricsResult>,
}

pub struct UiState {
    pub show_bottom_log_panel: bool,
    pub new_trigger_log_exists: bool,
    pub show_romanization_lrc_panel: bool,
    pub show_translation_lrc_panel: bool,
    pub wrap_text: bool,
    pub show_settings_window: bool,
    pub show_amll_connector_sidebar: bool,
    pub show_warnings_panel: bool,
    pub log_display_buffer: Vec<LogEntry>,
    pub temp_edit_settings: AppSettings,
    pub toasts: Toasts,
    pub available_system_fonts: Vec<String>,
    pub current_settings_category: SettingsCategory,
    pub current_view: AppView,
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
            show_warnings_panel: false,
            log_display_buffer: Vec::with_capacity(200),
            available_system_fonts: Vec::new(),
            current_settings_category: SettingsCategory::default(),
            current_view: AppView::default(),
        }
    }
}

pub struct LyricState {
    pub input_text: String,
    pub output_text: String,
    pub display_translation_lrc_output: String,
    pub display_romanization_lrc_output: String,
    pub parsed_lyric_data: Option<ParsedSourceData>,
    pub loaded_translation_lrc: Option<Vec<crate::types::DisplayLrcLine>>,
    pub loaded_romanization_lrc: Option<Vec<crate::types::DisplayLrcLine>>,
    pub metadata_source_is_download: bool,
    pub source_format: LyricFormat,
    pub target_format: LyricFormat,
    pub available_formats: Vec<LyricFormat>,
    pub last_opened_file_path: Option<std::path::PathBuf>,
    pub last_saved_file_path: Option<std::path::PathBuf>,
    pub conversion_in_progress: bool,
    pub conversion_result_rx: Option<ConversionResultRx>,
    pub current_warnings: Vec<String>,
}

pub struct LyricsHelperState {
    pub helper: Arc<TokioMutex<lyrics_helper_rs::LyricsHelper>>,
    pub provider_state: ProviderState,
    pub provider_load_result_rx: Option<StdReceiver<Result<(), String>>>,
}

impl LyricState {
    fn new() -> Self {
        Self {
            input_text: String::new(),
            output_text: String::new(),
            display_translation_lrc_output: String::new(),
            display_romanization_lrc_output: String::new(),
            parsed_lyric_data: None,
            loaded_translation_lrc: None,
            loaded_romanization_lrc: None,
            metadata_source_is_download: false,
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
            current_warnings: Vec::new(),
        }
    }
}

pub struct PlayerState {
    pub command_tx: Option<TokioSender<MediaCommand>>,
    pub current_now_playing: NowPlayingInfo,
    pub available_sessions: Vec<SmtcSessionInfo>,
    pub smtc_time_offset_ms: i64,
    pub last_requested_session_id: Option<String>,
}

impl PlayerState {
    fn new(settings: &AppSettings, command_tx: Option<TokioSender<MediaCommand>>) -> Self {
        Self {
            command_tx,
            current_now_playing: NowPlayingInfo::default(),
            available_sessions: Vec::new(),
            smtc_time_offset_ms: settings.smtc_time_offset_ms,
            last_requested_session_id: None,
        }
    }
}

pub struct AmllConnectorState {
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

pub struct AutoFetchState {
    pub result_rx: StdReceiver<AutoFetchResult>,
    pub result_tx: StdSender<AutoFetchResult>,

    pub current_fetch_cancellation_token: Option<CancellationToken>,
    pub current_ui_populated: bool,
    pub last_source_format: Option<LyricFormat>,
    pub local_cache_status: Arc<StdMutex<AutoSearchStatus>>,
    pub qqmusic_status: Arc<StdMutex<AutoSearchStatus>>,
    pub kugou_status: Arc<StdMutex<AutoSearchStatus>>,
    pub netease_status: Arc<StdMutex<AutoSearchStatus>>,
    pub amll_db_status: Arc<StdMutex<AutoSearchStatus>>,
    pub last_qq_result: Arc<StdMutex<Option<FullLyricsResult>>>,
    pub last_kugou_result: Arc<StdMutex<Option<FullLyricsResult>>>,
    pub last_netease_result: Arc<StdMutex<Option<FullLyricsResult>>>,
    pub last_amll_db_result: Arc<StdMutex<Option<FullLyricsResult>>>,
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
pub struct LocalCacheState {
    pub index: Arc<StdMutex<Vec<LocalLyricCacheEntry>>>,
    pub index_path: Option<std::path::PathBuf>,
    pub dir_path: Option<std::path::PathBuf>,
    pub cover_cache_dir: Option<std::path::PathBuf>,
}

pub struct UniLyricApp {
    // --- 状态模块 ---
    pub ui: UiState,
    pub lyrics: LyricState,
    pub player: PlayerState,
    pub fetcher: AutoFetchState,
    pub local_cache: LocalCacheState,
    pub amll_connector: AmllConnectorState,
    pub downloader: DownloaderState,
    pub batch_converter: BatchConverterState,

    // --- 核心依赖与配置 ---
    pub lyrics_helper_state: LyricsHelperState,
    pub app_settings: Arc<StdMutex<AppSettings>>,
    pub tokio_runtime: Arc<tokio::runtime::Runtime>,
    pub ui_log_receiver: StdReceiver<LogEntry>,

    // --- 事件系统 ---
    pub action_tx: StdSender<UserAction>,
    pub action_rx: StdReceiver<UserAction>,
    pub egui_ctx: egui::Context,
    pub actions_this_frame: Vec<UserAction>,

    // --- 标记 ---
    pub shutdown_initiated: bool,
    pub auto_fetch_trigger_time: Option<std::time::Instant>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BatchConverterStatus {
    #[default]
    Idle,
    Ready,
    Converting,
    Completed,
    Failed(String),
}

#[derive(Clone, Default)]
pub struct BatchConverterState {
    pub input_dir: Option<std::path::PathBuf>,
    pub output_dir: Option<std::path::PathBuf>,
    pub target_format: LyricFormat,
    pub tasks: Vec<BatchConversionConfig>,
    pub file_lookup: HashMap<BatchFileId, BatchLoadedFile>,
    pub status: BatchConverterStatus,
}

impl UniLyricApp {
    pub fn new(
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

        let lyric_state = LyricState::new();
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
                mode: settings.amll_connector_mode,
                server_port: settings.amll_connector_server_port,
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
            batch_converter: BatchConverterState::default(),
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

    pub fn send_shutdown_signals(&mut self) {
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
