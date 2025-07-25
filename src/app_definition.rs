use std::fs::File;
use std::io::{BufRead, BufReader};
use std::sync::{Arc, Mutex};

use crate::amll_connector::{
    AMLLConnectorConfig, ConnectorCommand, ConnectorUpdate, WebsocketStatus,
};
use crate::app::TtmlDbUploadUserAction;
use crate::app_settings::AppSettings;
use crate::types::{AutoFetchResult, AutoSearchStatus, LocalLyricCacheEntry, LogEntry};
use crate::utils;
use egui_toast::Toasts;
use lyrics_helper_rs::converter::LyricFormat;
use reqwest::Client;
use smtc_suite::{MediaCommand, MediaUpdate, NowPlayingInfo, SmtcSessionInfo};
use std::sync::mpsc::{Receiver as StdReceiver, Sender as StdSender, channel as std_channel};
use tokio::sync::mpsc::Sender as TokioSender;
use tokio::sync::mpsc::channel as tokio_channel;
use tokio::task::JoinHandle;

use crate::app_actions::UserAction;

pub(super) type SearchResultRx = StdReceiver<
    Result<Vec<lyrics_helper_rs::SearchResult>, lyrics_helper_rs::error::LyricsHelperError>,
>;
pub(super) type DownloadResultRx = StdReceiver<
    Result<
        lyrics_helper_rs::model::track::FullLyricsResult,
        lyrics_helper_rs::error::LyricsHelperError,
    >,
>;
pub(super) type ConversionResultRx = StdReceiver<
    Result<
        lyrics_helper_rs::converter::types::FullConversionResult,
        lyrics_helper_rs::error::LyricsHelperError,
    >,
>;

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

pub(super) struct PlayerState {
    pub(super) command_tx: Option<crossbeam_channel::Sender<MediaCommand>>,
    pub(super) update_rx: Option<crossbeam_channel::Receiver<MediaUpdate>>,
    pub(super) current_now_playing: NowPlayingInfo,
    pub(super) available_sessions: Vec<SmtcSessionInfo>,
    pub(super) smtc_time_offset_ms: i64,
    pub(super) last_requested_session_id: Option<String>,
}

pub(super) struct AmllConnectorState {
    pub command_tx: Option<TokioSender<ConnectorCommand>>,
    pub actor_handle: Option<JoinHandle<()>>,
    pub status: Arc<Mutex<WebsocketStatus>>,
    pub config: Arc<Mutex<AMLLConnectorConfig>>,
    pub update_rx: std::sync::mpsc::Receiver<ConnectorUpdate>,
}

pub(super) struct AutoFetchState {
    pub(super) result_rx: StdReceiver<AutoFetchResult>,
    pub(super) result_tx: StdSender<AutoFetchResult>,
    pub(super) current_ui_populated: bool,
    pub(super) last_source_format: Option<LyricFormat>,
    pub(super) last_source_for_stripping_check: Option<crate::types::AutoSearchSource>,
    pub(super) manual_refetch_request: Option<crate::types::AutoSearchSource>,
    pub(super) local_cache_status: Arc<Mutex<AutoSearchStatus>>,
    pub(super) qqmusic_status: Arc<Mutex<AutoSearchStatus>>,
    pub(super) kugou_status: Arc<Mutex<AutoSearchStatus>>,
    pub(super) netease_status: Arc<Mutex<AutoSearchStatus>>,
    pub(super) amll_db_status: Arc<Mutex<AutoSearchStatus>>,
    pub(super) musixmatch_status: Arc<Mutex<AutoSearchStatus>>,
    pub(super) last_qq_result: Arc<Mutex<Option<lyrics_helper_rs::model::track::FullLyricsResult>>>,
    pub(super) last_kugou_result:
        Arc<Mutex<Option<lyrics_helper_rs::model::track::FullLyricsResult>>>,
    pub(super) last_netease_result:
        Arc<Mutex<Option<lyrics_helper_rs::model::track::FullLyricsResult>>>,
    pub(super) last_amll_db_result:
        Arc<Mutex<Option<lyrics_helper_rs::model::track::FullLyricsResult>>>,
    pub(super) last_musixmatch_result:
        Arc<Mutex<Option<lyrics_helper_rs::model::track::FullLyricsResult>>>,
}

pub(super) struct LocalCacheState {
    pub(super) index: Arc<Mutex<Vec<LocalLyricCacheEntry>>>,
    pub(super) index_path: Option<std::path::PathBuf>,
    pub(super) dir_path: Option<std::path::PathBuf>,
}

pub(super) struct TtmlDbUploadState {
    pub(super) in_progress: bool,
    pub(super) last_paste_url: Option<String>,
    pub(super) action_rx: StdReceiver<TtmlDbUploadUserAction>,
    pub(super) action_tx: StdSender<TtmlDbUploadUserAction>,
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
    pub(super) lyrics_helper: Option<Arc<lyrics_helper_rs::LyricsHelper>>,
    pub(super) lyrics_helper_rx: StdReceiver<Arc<lyrics_helper_rs::LyricsHelper>>,
    pub(super) http_client: Client,
    pub(super) app_settings: Arc<Mutex<AppSettings>>,
    pub(super) tokio_runtime: Arc<tokio::runtime::Runtime>,
    pub(super) ui_log_receiver: StdReceiver<LogEntry>,

    // --- 事件系统 ---
    pub(super) actions_this_frame: Vec<UserAction>,

    // --- 标记 ---
    pub(super) shutdown_initiated: bool,
    pub(super) is_any_file_hovering_window: bool,
}

impl UniLyricApp {
    /// UniLyricApp的构造函数，用于创建应用实例。
    pub(super) fn new(
        cc: &eframe::CreationContext,           // eframe 创建上下文
        settings: AppSettings,                  // 应用设置实例
        ui_log_receiver: StdReceiver<LogEntry>, // UI日志接收器
    ) -> Self {
        // 设置自定义字体函数
        fn setup_custom_fonts(ctx: &egui::Context) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "SarasaUiSC".to_owned(),
                egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/SarasaUiSC-Regular.ttf"
                ))
                .into(),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "SarasaUiSC".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("SarasaUiSC".to_owned());
            ctx.set_fonts(fonts);
        }

        setup_custom_fonts(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);

        // 创建异步HTTP客户端实例
        let async_http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("构建HTTP客户端失败");

        // --- 初始化通道 ---
        let (lyrics_helper_tx, lyrics_helper_rx) =
            std_channel::<Arc<lyrics_helper_rs::LyricsHelper>>();
        let (auto_fetch_tx, auto_fetch_rx) = std_channel::<AutoFetchResult>();
        let (upload_action_tx, upload_action_rx) = std_channel::<TtmlDbUploadUserAction>();

        // --- 创建Tokio异步运行时 ---
        let runtime_instance = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .thread_name("unilyric-app-tokio")
                .build()
                .expect("无法为应用创建 Tokio 运行时"),
        );

        // --- 异步初始化 LyricsHelper ---
        let rt_clone = runtime_instance.clone();
        rt_clone.spawn(async move {
            tracing::info!("[LyricsHelper] 开始异步初始化...");
            match lyrics_helper_rs::LyricsHelper::new().await {
                Ok(helper) => {
                    if lyrics_helper_tx.send(Arc::new(helper)).is_ok() {
                        tracing::info!("[LyricsHelper] 异步初始化成功并已发送。");
                    } else {
                        tracing::error!(
                            "[LyricsHelper] 异步初始化成功，但发送失败，UI线程可能已关闭。"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!("[LyricsHelper] 异步初始化失败: {e}");
                }
            }
        });

        // --- 初始化本地歌词缓存 ---

        let local_cache = LocalCacheState {
            index: Arc::new(Mutex::new(Vec::new())), // 先创建空的，稍后填充
            index_path: None,
            dir_path: None,
        };

        // --- 初始化UI Toast通知 ---
        let toasts = Toasts::new()
            .anchor(egui::Align2::LEFT_TOP, (10.0, 10.0))
            .direction(egui::Direction::TopDown);

        let mc_config = AMLLConnectorConfig {
            enabled: settings.amll_connector_enabled,
            websocket_url: settings.amll_connector_websocket_url.clone(),
        };

        let ui_state = UiState {
            show_bottom_log_panel: false,
            new_trigger_log_exists: false,
            show_romanization_lrc_panel: false,
            show_translation_lrc_panel: false,
            wrap_text: true,
            show_settings_window: false,
            show_amll_connector_sidebar: mc_config.enabled,
            show_metadata_panel: false,
            show_markers_panel: false,
            show_search_window: false,
            log_display_buffer: Vec::with_capacity(200),
            temp_edit_settings: settings.clone(),
            toasts,
        };

        let lyric_state = LyricState {
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
            source_format: settings.last_source_format,
            target_format: settings.last_target_format,
            available_formats: LyricFormat::all().to_vec(),
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
        };

        let (mut smtc_command_tx, mut smtc_update_rx) = (None, None);
        match smtc_suite::MediaManager::start() {
            Ok(controller) => {
                tracing::info!("[UniLyricApp new] smtc-suite 媒体服务已成功启动。");
                let smtc_suite::MediaController {
                    command_tx,
                    update_rx,
                } = controller;
                smtc_command_tx = Some(command_tx);
                smtc_update_rx = Some(update_rx);
            }
            Err(e) => {
                tracing::error!("[UniLyricApp new] 启动 smtc-suite 媒体服务失败: {e}");
            }
        };

        let smtc_controller = smtc_suite::MediaManager::start().expect("smtc-suite 启动失败");
        let smtc_cmd_tx = smtc_controller.command_tx;
        let smtc_update_rx_for_ui = smtc_controller.update_rx.clone(); // UI 保留一个克隆用于显示
        let smtc_update_rx_for_actor = smtc_controller.update_rx; // actor 拿走原始的

        let (amll_update_tx, amll_update_rx) = std::sync::mpsc::channel::<ConnectorUpdate>();
        let (amll_command_tx, amll_command_rx) = tokio_channel::<ConnectorCommand>(32);

        let actor_handle =
            runtime_instance.spawn(crate::amll_connector::worker::amll_connector_actor(
                amll_command_rx,
                amll_update_tx,
                mc_config.clone(),
                smtc_cmd_tx,
                smtc_update_rx_for_actor,
            ));

        let player_state = PlayerState {
            command_tx: smtc_command_tx,
            update_rx: smtc_update_rx,
            current_now_playing: NowPlayingInfo::default(),
            available_sessions: Vec::new(),
            smtc_time_offset_ms: settings.smtc_time_offset_ms,
            last_requested_session_id: settings.last_selected_smtc_session_id.clone(),
        };

        let amll_connector_state = AmllConnectorState {
            command_tx: Some(amll_command_tx),
            actor_handle: Some(actor_handle),
            status: Arc::new(Mutex::new(WebsocketStatus::default())),
            config: Arc::new(Mutex::new(mc_config)),
            update_rx: amll_update_rx,
        };

        let auto_fetch_state = AutoFetchState {
            result_rx: auto_fetch_rx,
            result_tx: auto_fetch_tx,
            current_ui_populated: false,
            last_source_format: None,
            last_source_for_stripping_check: None,
            manual_refetch_request: None,
            local_cache_status: Arc::new(Mutex::new(Default::default())),
            qqmusic_status: Arc::new(Mutex::new(AutoSearchStatus::NotAttempted)),
            kugou_status: Arc::new(Mutex::new(AutoSearchStatus::NotAttempted)),
            netease_status: Arc::new(Mutex::new(AutoSearchStatus::NotAttempted)),
            amll_db_status: Arc::new(Mutex::new(AutoSearchStatus::NotAttempted)),
            musixmatch_status: Arc::new(Mutex::new(AutoSearchStatus::NotAttempted)),
            last_qq_result: Arc::new(Mutex::new(None)),
            last_kugou_result: Arc::new(Mutex::new(None)),
            last_netease_result: Arc::new(Mutex::new(None)),
            last_amll_db_result: Arc::new(Mutex::new(None)),
            last_musixmatch_result: Arc::new(Mutex::new(None)),
        };

        let ttml_db_upload_state = TtmlDbUploadState {
            in_progress: false,
            last_paste_url: None,
            action_rx: upload_action_rx,
            action_tx: upload_action_tx,
        };

        // --- 构建 UniLyricApp 实例 ---
        let mut app = Self {
            ui: ui_state,
            lyrics: lyric_state,
            player: player_state,
            amll_connector: amll_connector_state,
            fetcher: auto_fetch_state,
            local_cache,
            ttml_db_upload: ttml_db_upload_state,
            lyrics_helper: None,
            lyrics_helper_rx,
            http_client: async_http_client,
            app_settings: Arc::new(Mutex::new(settings.clone())),
            tokio_runtime: runtime_instance,
            ui_log_receiver,
            actions_this_frame: Vec::new(),
            shutdown_initiated: false,
            is_any_file_hovering_window: false,
        };

        // --- 初始化本地歌词缓存 ---
        if let Some(data_dir) = utils::get_app_data_dir() {
            let cache_dir = data_dir.join("local_lyrics_cache");
            if !cache_dir.exists()
                && let Err(e) = std::fs::create_dir_all(&cache_dir)
            {
                tracing::error!("[UniLyricApp] 无法创建本地歌词缓存目录 {cache_dir:?}: {e}");
            }
            app.local_cache.dir_path = Some(cache_dir.clone());

            let index_file = cache_dir.join("local_lyrics_index.jsonl");
            if index_file.exists()
                && let Ok(file) = File::open(&index_file)
            {
                let reader = BufReader::new(file);
                let mut cache_entries = Vec::new();
                for line in reader.lines().flatten() {
                    if !line.trim().is_empty()
                        && let Ok(entry) = serde_json::from_str::<LocalLyricCacheEntry>(&line)
                    {
                        cache_entries.push(entry);
                    }
                }
                tracing::info!(
                    "[UniLyricApp] 从 {:?} 加载了 {} 条本地缓存歌词索引。",
                    index_file,
                    cache_entries.len()
                );
                *app.local_cache.index.lock().unwrap() = cache_entries;
            }
            app.local_cache.index_path = Some(index_file);
        }

        app
    }

    pub(crate) fn shutdown_amll_actor(&mut self) {
        if let Some(tx) = &self.amll_connector.command_tx {
            if tx.try_send(ConnectorCommand::Shutdown).is_err() {
                tracing::warn!("[Shutdown] 发送 Shutdown 命令到 actor 失败 (可能已关闭)。");
            }
        }
        if let Some(handle) = self.amll_connector.actor_handle.take() {
            handle.abort();
        }
    }
}
