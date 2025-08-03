use std::fs::File;
use std::io::{BufRead, BufReader};
use std::sync::{
    Arc, Mutex as StdMutex,
    mpsc::{Receiver as StdReceiver, Sender as StdSender, channel as std_channel},
};

use egui_toast::Toasts;
use lyrics_helper_rs::{
    SearchResult,
    converter::{LyricFormat, types::FullConversionResult},
    error::LyricsHelperError,
    model::track::FullLyricsResult,
};
use smtc_suite::{MediaCommand, NowPlayingInfo, SmtcSessionInfo};
use tokio::{
    sync::Mutex as TokioMutex,
    sync::mpsc::{Sender as TokioSender, channel as tokio_channel},
    task::JoinHandle,
};

use crate::amll_connector::types::UiUpdate;
use crate::types::ProviderState;
use crate::{
    amll_connector::{AMLLConnectorConfig, ConnectorCommand, WebsocketStatus},
    app::TtmlDbUploadUserAction,
    app_actions::UserAction,
    app_settings::AppSettings,
    types::{AutoFetchResult, AutoSearchStatus, LocalLyricCacheEntry, LogEntry},
    utils,
};

pub(super) type SearchResultRx = StdReceiver<Result<Vec<SearchResult>, LyricsHelperError>>;
pub(super) type DownloadResultRx = StdReceiver<Result<FullLyricsResult, LyricsHelperError>>;
pub(super) type ConversionResultRx = StdReceiver<Result<FullConversionResult, LyricsHelperError>>;

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
    pub(super) show_search_window: bool,
    pub(super) log_display_buffer: Vec<LogEntry>,
    pub(super) temp_edit_settings: AppSettings,
    pub(super) toasts: Toasts,
    pub(super) available_system_fonts: Vec<String>,
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
            show_search_window: false,
            log_display_buffer: Vec::with_capacity(200),
            available_system_fonts: Vec::new(),
        }
    }
}

pub(super) struct LyricState {
    pub(super) input_text: String,
    pub(super) output_text: String,
    pub(super) display_translation_lrc_output: String,
    pub(super) display_romanization_lrc_output: String,
    pub(super) parsed_lyric_data: Option<lyrics_helper_rs::converter::types::ParsedSourceData>,
    pub(super) loaded_translation_lrc: Option<Vec<crate::types::DisplayLrcLine>>,
    pub(super) loaded_romanization_lrc: Option<Vec<crate::types::DisplayLrcLine>>,
    pub(super) editable_metadata: Vec<crate::types::EditableMetadataEntry>,
    pub(super) metadata_is_user_edited: bool,
    pub(super) metadata_source_is_download: bool,
    pub(super) current_markers: Vec<(usize, String)>,
    pub(super) source_format: LyricFormat,
    pub(super) target_format: LyricFormat,
    pub(super) available_formats: Vec<LyricFormat>,
    pub(super) last_opened_file_path: Option<std::path::PathBuf>,
    pub(super) last_saved_file_path: Option<std::path::PathBuf>,
    pub(super) conversion_in_progress: bool,
    pub(super) conversion_result_rx: Option<ConversionResultRx>,
    pub(super) search_in_progress: bool,
    pub(super) search_query: String,
    pub(super) search_results: Vec<lyrics_helper_rs::model::track::SearchResult>,
    pub(super) search_result_rx: Option<SearchResultRx>,
    pub(super) download_in_progress: bool,
    pub(super) download_result_rx: Option<DownloadResultRx>,
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
            editable_metadata: Vec::new(),
            metadata_is_user_edited: false,
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
            search_in_progress: false,
            search_query: String::new(),
            search_results: Vec::new(),
            search_result_rx: None,
            download_in_progress: false,
            download_result_rx: None,
        }
    }
}

pub(super) struct PlayerState {
    pub(super) command_tx: Option<TokioSender<MediaCommand>>,
    pub(super) current_now_playing: NowPlayingInfo,
    pub(super) available_sessions: Vec<SmtcSessionInfo>,
    pub(super) smtc_time_offset_ms: i64,
    pub(super) last_requested_session_id: Option<String>,
}

impl PlayerState {
    fn new(_settings: &AppSettings, command_tx: TokioSender<MediaCommand>) -> Self {
        Self {
            command_tx: Some(command_tx),
            current_now_playing: NowPlayingInfo::default(),
            available_sessions: Vec::new(),
            smtc_time_offset_ms: 0,
            last_requested_session_id: None,
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
}

pub(super) struct AutoFetchState {
    pub(super) result_rx: StdReceiver<AutoFetchResult>,
    pub(super) result_tx: StdSender<AutoFetchResult>,

    pub(super) current_ui_populated: bool,
    pub(super) last_source_format: Option<LyricFormat>,
    pub(super) last_source_for_stripping_check: Option<crate::types::AutoSearchSource>,
    pub(super) manual_refetch_request: Option<crate::types::AutoSearchSource>,
    pub(super) local_cache_status: Arc<StdMutex<AutoSearchStatus>>,
    pub(super) qqmusic_status: Arc<StdMutex<AutoSearchStatus>>,
    pub(super) kugou_status: Arc<StdMutex<AutoSearchStatus>>,
    pub(super) netease_status: Arc<StdMutex<AutoSearchStatus>>,
    pub(super) amll_db_status: Arc<StdMutex<AutoSearchStatus>>,
    pub(super) last_qq_result:
        Arc<StdMutex<Option<lyrics_helper_rs::model::track::FullLyricsResult>>>,
    pub(super) last_kugou_result:
        Arc<StdMutex<Option<lyrics_helper_rs::model::track::FullLyricsResult>>>,
    pub(super) last_netease_result:
        Arc<StdMutex<Option<lyrics_helper_rs::model::track::FullLyricsResult>>>,
    pub(super) last_amll_db_result:
        Arc<StdMutex<Option<lyrics_helper_rs::model::track::FullLyricsResult>>>,
}

impl AutoFetchState {
    fn new(result_tx: StdSender<AutoFetchResult>, result_rx: StdReceiver<AutoFetchResult>) -> Self {
        Self {
            result_rx,
            result_tx,
            current_ui_populated: false,
            last_source_format: None,
            last_source_for_stripping_check: None,
            manual_refetch_request: None,
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
}

pub(super) struct TtmlDbUploadState {
    pub(super) in_progress: bool,
    pub(super) last_paste_url: Option<String>,
    pub(super) action_rx: StdReceiver<TtmlDbUploadUserAction>,
    pub(super) action_tx: StdSender<TtmlDbUploadUserAction>,
}

impl TtmlDbUploadState {
    fn new(
        action_tx: StdSender<TtmlDbUploadUserAction>,
        action_rx: StdReceiver<TtmlDbUploadUserAction>,
    ) -> Self {
        Self {
            in_progress: false,
            last_paste_url: None,
            action_rx,
            action_tx,
        }
    }
}

pub(super) struct UniLyricApp {
    // --- 状态模块 ---
    pub(super) ui: UiState,
    pub(super) lyrics: LyricState,
    pub(super) player: PlayerState,
    pub(super) fetcher: AutoFetchState,
    pub(super) local_cache: LocalCacheState,
    pub(super) ttml_db_upload: TtmlDbUploadState,
    pub(super) amll_connector: AmllConnectorState,

    // --- 核心依赖与配置 ---
    pub(super) lyrics_helper_state: LyricsHelperState,
    pub(super) app_settings: Arc<StdMutex<AppSettings>>,
    pub(super) tokio_runtime: Arc<tokio::runtime::Runtime>,
    pub(super) ui_log_receiver: StdReceiver<LogEntry>,

    // --- 事件系统 ---
    pub(super) egui_ctx: egui::Context,
    pub(super) actions_this_frame: Vec<UserAction>,

    // --- 标记 ---
    pub(super) shutdown_initiated: bool,
}

impl UniLyricApp {
    pub(super) fn new(
        cc: &eframe::CreationContext,
        settings: AppSettings,
        ui_log_receiver: StdReceiver<LogEntry>,
    ) -> Self {
        let egui_ctx = cc.egui_ctx.clone();
        Self::setup_fonts(&cc.egui_ctx, &settings);
        let tokio_runtime = Self::create_tokio_runtime();
        let (smtc_controller, smtc_update_rx) =
            smtc_suite::MediaManager::start().expect("smtc-suite 启动失败");

        let (auto_fetch_tx, auto_fetch_rx) = std_channel::<AutoFetchResult>();
        let (upload_action_tx, upload_action_rx) = std_channel::<TtmlDbUploadUserAction>();
        let (amll_update_tx, amll_update_rx) = std_channel::<UiUpdate>();
        let (amll_command_tx, amll_command_rx) = tokio_channel::<ConnectorCommand>(32);

        let lyric_state = LyricState::new(&settings);
        let player_state = PlayerState::new(&settings, smtc_controller.command_tx.clone());
        let auto_fetch_state = AutoFetchState::new(auto_fetch_tx, auto_fetch_rx);
        let ttml_db_upload_state = TtmlDbUploadState::new(upload_action_tx, upload_action_rx);
        let local_cache = LocalCacheState::default(); // 先用默认值，等下加载数据

        let mc_config = AMLLConnectorConfig {
            enabled: settings.amll_connector_enabled,
            websocket_url: settings.amll_connector_websocket_url.clone(),
        };

        let amll_actor_handle =
            tokio_runtime.spawn(crate::amll_connector::worker::amll_connector_actor(
                amll_command_rx,
                amll_update_tx,
                mc_config.clone(),
                smtc_controller.command_tx.clone(),
                smtc_update_rx,
            ));

        let amll_connector_state = AmllConnectorState::new(
            amll_command_tx,
            amll_update_rx,
            amll_actor_handle,
            mc_config,
        );

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
            ttml_db_upload: ttml_db_upload_state,
            lyrics_helper_state,
            app_settings: Arc::new(StdMutex::new(settings)),
            tokio_runtime,
            ui_log_receiver,
            egui_ctx,
            actions_this_frame: Vec::new(),
            shutdown_initiated: false,
        };

        app.load_local_cache();

        app.trigger_provider_loading();

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
                    tracing::error!("读取字体文件失败: {}", font_family_name);
                }
            } else {
                tracing::warn!("未找到上次选择的字体: {}", font_family_name);
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
                tracing::error!("[UniLyricApp] 无法创建本地歌词目录 {cache_dir:?}: {e}");
                return;
            }
            self.local_cache.dir_path = Some(cache_dir.clone());

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

                tracing::info!(
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
            tracing::debug!("[Shutdown] 正在发送 Shutdown 命令到 smtc-suite ...");
            let _ = tx.try_send(MediaCommand::Shutdown);
        }

        if let Some(tx) = &self.amll_connector.command_tx {
            tracing::debug!("[Shutdown] 正在发送 Shutdown 命令到 actor...");
            let _ = tx.try_send(ConnectorCommand::Shutdown);
        }
    }
}
