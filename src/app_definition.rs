use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::mpsc::channel as std_channel;
use std::sync::mpsc::{Receiver as StdReceiver, Sender as StdSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use egui_toast::Toasts;
use lyrics_helper_rs::SearchResult;
use lyrics_helper_rs::converter::LyricFormat;
use reqwest::Client;
use tokio::sync::Mutex as TokioMutex;

use crate::amll_connector::amll_connector_manager;
use crate::amll_connector::{
    AMLLConnectorConfig, ConnectorCommand, ConnectorUpdate, NowPlayingInfo, WebsocketStatus,
    types::SmtcSessionInfo,
};
use crate::app::TtmlDbUploadUserAction;
use crate::app_settings::AppSettings;
use crate::logger::LogEntry;
use crate::types::{
    AutoFetchResult, AutoSearchStatus, EditableMetadataEntry, LocalLyricCacheEntry,
};
use crate::websocket_server::ServerCommand;
use crate::{utils, websocket_server};
use lyrics_helper_rs::converter::types::FullConversionResult;
use lyrics_helper_rs::model::track::FullLyricsResult;
use tokio::sync::mpsc as tokio_mpsc;

use lyrics_helper_rs::error::LyricsHelperError;

pub(super) type SearchResultRx = StdReceiver<Result<Vec<SearchResult>, LyricsHelperError>>;
pub(super) type DownloadResultRx = StdReceiver<Result<FullLyricsResult, LyricsHelperError>>;

pub(super) type ConversionResultRx = StdReceiver<Result<FullConversionResult, LyricsHelperError>>;

pub(super) struct UniLyricApp {
    pub(super) lyrics_helper: Option<Arc<lyrics_helper_rs::LyricsHelper>>, // 歌词助手核心实例
    pub(super) lyrics_helper_rx: StdReceiver<Arc<lyrics_helper_rs::LyricsHelper>>, // 用于接收初始化完成的歌词助手
    pub(super) conversion_result_rx: Option<ConversionResultRx>,

    pub(super) current_auto_search_ui_populated: bool,

    // --- UI相关的文本输入输出区域 ---
    pub(super) input_text: String,                      // 主输入框文本
    pub(super) output_text: String,                     // 主输出框文本 (转换后的歌词)
    pub(super) display_translation_lrc_output: String,  // 翻译LRC预览面板的文本
    pub(super) display_romanization_lrc_output: String, // 罗马音LRC预览面板的文本

    // --- 格式选择与文件路径 ---
    pub(super) source_format: LyricFormat, // 当前选择的源歌词格式
    pub(super) target_format: LyricFormat, // 当前选择的目标歌词格式
    pub(super) available_formats: Vec<LyricFormat>, // 所有可用的歌词格式列表
    pub(super) last_opened_file_path: Option<PathBuf>, // 上次打开文件的路径
    pub(super) last_saved_file_path: Option<PathBuf>, // 上次保存文件的路径

    pub(super) show_metadata_panel: bool,
    pub(super) show_markers_panel: bool,
    pub(super) current_markers: Vec<(usize, String)>,

    // 为每个提供商添加搜索状态，供UI显示
    pub(super) qqmusic_auto_search_status: Arc<Mutex<AutoSearchStatus>>,
    pub(super) kugou_auto_search_status: Arc<Mutex<AutoSearchStatus>>,
    pub(super) netease_auto_search_status: Arc<Mutex<AutoSearchStatus>>,
    pub(super) amll_db_auto_search_status: Arc<Mutex<AutoSearchStatus>>,
    pub(super) musixmatch_auto_search_status: Arc<Mutex<AutoSearchStatus>>,
    pub(super) last_musixmatch_search_result: Arc<Mutex<Option<FullLyricsResult>>>,

    // 用于在侧边栏加载之前搜索结果的缓存
    pub(super) last_qq_search_result: Arc<Mutex<Option<FullLyricsResult>>>,
    pub(super) last_kugou_search_result: Arc<Mutex<Option<FullLyricsResult>>>,
    pub(super) last_netease_search_result: Arc<Mutex<Option<FullLyricsResult>>>,
    pub(super) last_amll_db_search_result: Arc<Mutex<Option<FullLyricsResult>>>,

    // --- 状态标志 (UI控制与内部逻辑) ---
    pub(super) conversion_in_progress: bool, // 标记歌词转换是否正在进行
    pub(super) show_bottom_log_panel: bool,  // 是否显示底部日志面板
    pub(super) new_trigger_log_exists: bool, // 是否有新的触发器日志（用于UI提示）
    pub(super) is_any_file_hovering_window: bool, // 标记是否有文件悬停在应用窗口上 (用于拖放提示)
    pub(super) show_romanization_lrc_panel: bool, // 是否显示罗马音LRC编辑/预览面板
    pub(super) show_translation_lrc_panel: bool, // 是否显示翻译LRC编辑/预览面板
    pub(super) wrap_text: bool,              // 文本框是否自动换行
    pub(super) show_settings_window: bool,   // 是否显示设置窗口
    pub(super) show_amll_connector_sidebar: bool, // 是否显示AMLL媒体连接器侧边栏

    // --- 核心数据存储 ---
    pub(super) parsed_lyric_data: Option<lyrics_helper_rs::converter::types::ParsedSourceData>, // 解析后的歌词数据
    pub(super) editable_metadata: Vec<EditableMetadataEntry>, // UI上可编辑的元数据列表

    // --- 网络下载通用 ---
    pub(super) http_client: Client, // 全局HTTP客户端实例

    pub(super) metadata_source_is_download: bool,

    // --- 网络搜索与下载 ---
    pub(super) search_query: String, // 通用搜索查询
    pub(super) search_results: Vec<lyrics_helper_rs::model::track::SearchResult>,

    pub(super) search_in_progress: bool,   // 标记搜索是否正在进行
    pub(super) download_in_progress: bool, // 标记下载是否正在进行
    pub(super) show_search_window: bool,   // 是否显示搜索/下载窗口

    pub(super) search_result_rx: Option<SearchResultRx>,
    pub(super) download_result_rx: Option<DownloadResultRx>,

    // --- 从文件加载的次要LRC数据 ---
    pub(super) loaded_translation_lrc: Option<Vec<crate::types::DisplayLrcLine>>, // 手动加载的翻译LRC行 (用于UI显示和处理)
    pub(super) loaded_romanization_lrc: Option<Vec<crate::types::DisplayLrcLine>>, // 手动加载的罗马音LRC行

    // --- 应用设置 ---
    pub(super) app_settings: Arc<Mutex<AppSettings>>, // 应用设置 (多线程安全，用于持久化)
    pub(super) temp_edit_settings: AppSettings,       // 用于在设置窗口中临时编辑的设置副本

    // --- 日志系统 ---
    pub(super) log_display_buffer: Vec<LogEntry>, // UI上显示的日志条目缓冲区
    pub(super) ui_log_receiver: StdReceiver<LogEntry>, // 从日志后端接收日志条目的通道

    // --- AMLL Connector (媒体播放器集成) ---
    pub(super) media_connector_config: Arc<Mutex<AMLLConnectorConfig>>, // AMLL连接器配置 (多线程安全)
    pub(super) media_connector_status: Arc<Mutex<WebsocketStatus>>, // AMLL连接器WebSocket状态 (多线程安全)
    pub(super) current_media_info: Arc<TokioMutex<Option<NowPlayingInfo>>>, // 当前播放的媒体信息 (异步多线程安全)
    pub(super) media_connector_command_tx: Option<StdSender<ConnectorCommand>>, // 发送命令到AMLL连接器工作线程的通道发送端
    pub(super) media_connector_update_rx: StdReceiver<ConnectorUpdate>, // 从AMLL连接器工作线程接收更新的通道接收端
    pub(super) media_connector_worker_handle: Option<thread::JoinHandle<()>>, // AMLL连接器工作线程的句柄
    pub(super) media_connector_update_tx_for_worker: StdSender<ConnectorUpdate>, // 用于连接器内部任务向主更新通道发送消息的克隆发送端
    pub(super) audio_visualization_is_active: bool,

    // --- SMTC 会话管理 (系统媒体传输控制) ---
    pub(super) available_smtc_sessions: Arc<Mutex<Vec<SmtcSessionInfo>>>, // 可用的SMTC会话列表 (多线程安全)
    pub(super) selected_smtc_session_id: Arc<Mutex<Option<String>>>, // 用户当前选择的SMTC会话ID (多线程安全)
    pub(super) initial_selected_smtc_session_id_from_settings: Option<String>, // 从设置加载的初始选定SMTC会话ID
    pub(super) last_smtc_position_ms: u64, // 上次从SMTC获取的播放位置 (毫秒)
    pub(super) last_smtc_position_report_time: Option<Instant>, // 上次报告SMTC播放位置的时间点
    pub(super) is_currently_playing_sensed_by_smtc: bool, // SMTC是否报告当前正在播放
    pub(super) current_song_duration_ms: u64, // 当前歌曲的总时长 (毫秒)
    pub(super) smtc_time_offset_ms: i64,   // SMTC时间偏移量 (毫秒，用于校准)
    pub(super) current_smtc_volume: Arc<Mutex<Option<(f32, bool)>>>, // 当前SMTC报告的音量 (值, 是否静音)
    pub(super) last_requested_volume_for_session: Arc<Mutex<Option<String>>>, // 上次为特定会话请求音量的时间戳或标识

    // --- 自动获取与异步处理 ---
    pub(super) auto_fetch_result_rx: StdReceiver<crate::types::AutoFetchResult>, // 接收自动获取歌词结果的通道
    pub(super) auto_fetch_result_tx: StdSender<crate::types::AutoFetchResult>, // 发送自动获取歌词结果的通道
    pub(super) tokio_runtime: Arc<tokio::runtime::Runtime>, // Tokio异步运行时实例 (Arc方便共享)
    pub(super) progress_simulation_interval: Duration,      // 播放进度模拟定时器的间隔
    pub(super) progress_timer_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>, // 关闭进度定时器的发送端
    pub(super) progress_timer_join_handle: Option<tokio::task::JoinHandle<()>>, // 进度定时器任务的句柄
    pub(super) last_auto_fetch_source_format: Option<LyricFormat>, // 上次自动获取的歌词的原始格式
    pub(super) last_auto_fetch_source_for_stripping_check: Option<crate::types::AutoSearchSource>, // 上次自动获取的来源 (用于判断是否需要清理)
    pub(super) manual_refetch_request: Option<crate::types::AutoSearchSource>, // 手动请求重新获取特定来源的标记

    // --- 本地歌词缓存 ---
    pub(super) local_lyrics_cache_index: Arc<Mutex<Vec<LocalLyricCacheEntry>>>, // 本地缓存歌词的索引 (多线程安全)
    pub(super) local_lyrics_cache_index_path: Option<PathBuf>, // 本地缓存索引文件的路径
    pub(super) local_lyrics_cache_dir_path: Option<PathBuf>,   // 本地缓存歌词文件的目录路径
    pub(super) local_cache_auto_search_status: Arc<Mutex<AutoSearchStatus>>, // 本地缓存自动搜索状态

    // --- UI 通知 ---
    pub(super) toasts: Toasts, // egui toast 通知管理器

    // --- WebSocket 服务器 (用于外部控制或歌词同步) ---
    pub(super) websocket_server_command_tx: Option<tokio_mpsc::Sender<ServerCommand>>, // 发送命令到WebSocket服务器任务的通道发送端
    pub(super) websocket_server_handle: Option<tokio::task::JoinHandle<()>>, // WebSocket服务器任务的句柄
    pub(super) websocket_server_enabled: bool, // WebSocket服务器是否启用
    pub(super) websocket_server_port: u16,     // WebSocket服务器监听端口
    pub(super) last_true_smtc_processed_info: Arc<TokioMutex<Option<NowPlayingInfo>>>, // 上次处理的真实SMTC播放信息 (异步多线程安全)
    pub(super) shutdown_initiated: bool, // 标记应用是否已启动关闭流程

    // --- TTML DB 上传功能相关 ---
    pub(super) ttml_db_upload_in_progress: bool, // 标记TTML DB上传是否正在进行
    pub(super) ttml_db_last_paste_url: Option<String>, // 上次成功上传到paste服务后获取的URL
    pub(super) ttml_db_upload_action_rx: StdReceiver<TtmlDbUploadUserAction>, // 接收TTML DB上传操作结果的通道
    pub(super) ttml_db_upload_action_tx: StdSender<TtmlDbUploadUserAction>, // 发送TTML DB上传操作命令的通道
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
        let (mc_update_tx, mc_update_rx) = std_channel::<ConnectorUpdate>();
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
            log::info!("[LyricsHelper] 开始异步初始化...");
            match lyrics_helper_rs::LyricsHelper::new().await {
                Ok(helper) => {
                    if lyrics_helper_tx.send(Arc::new(helper)).is_ok() {
                        log::info!("[LyricsHelper] 异步初始化成功并已发送。");
                    } else {
                        log::error!(
                            "[LyricsHelper] 异步初始化成功，但发送失败，UI线程可能已关闭。"
                        );
                    }
                }
                Err(e) => {
                    log::error!("[LyricsHelper] 异步初始化失败: {}", e);
                }
            }
        });

        // --- 初始化本地歌词缓存 ---
        let mut local_lyrics_cache_index_data: Vec<LocalLyricCacheEntry> = Vec::new();
        let mut local_cache_dir: Option<PathBuf> = None;
        let mut local_cache_index_file_path: Option<PathBuf> = None;

        if let Some(data_dir) = utils::get_app_data_dir() {
            let cache_dir = data_dir.join("local_lyrics_cache");
            if !cache_dir.exists() {
                if let Err(e) = std::fs::create_dir_all(&cache_dir) {
                    log::error!("[UniLyricApp] 无法创建本地歌词缓存目录 {cache_dir:?}: {e}");
                }
            }
            local_cache_dir = Some(cache_dir.clone());

            let index_file = cache_dir.join("local_lyrics_index.jsonl");
            if index_file.exists() {
                if let Ok(file) = File::open(&index_file) {
                    let reader = BufReader::new(file);
                    for line in reader.lines().flatten() {
                        if !line.trim().is_empty() {
                            if let Ok(entry) = serde_json::from_str::<LocalLyricCacheEntry>(&line) {
                                local_lyrics_cache_index_data.push(entry);
                            }
                        }
                    }
                    log::info!(
                        "[UniLyricApp] 从 {:?} 加载了 {} 条本地缓存歌词索引。",
                        index_file,
                        local_lyrics_cache_index_data.len()
                    );
                }
            }
            local_cache_index_file_path = Some(index_file);
        }

        // --- 初始化UI Toast通知 ---
        let toasts = Toasts::new()
            .anchor(egui::Align2::LEFT_TOP, (10.0, 10.0))
            .direction(egui::Direction::TopDown);

        // --- 初始化WebSocket服务器 ---
        let (ws_cmd_tx, ws_cmd_rx) = tokio_mpsc::channel(32);
        let websocket_server_handle: Option<tokio::task::JoinHandle<()>> = if settings
            .websocket_server_settings
            .enabled
        {
            let server_addr = format!("127.0.0.1:{}", settings.websocket_server_settings.port);
            let server_instance = websocket_server::WebsocketServer::new(ws_cmd_rx);
            log::info!("[UniLyricApp new] 准备启动 WebSocket 服务器, 服务器地址: {server_addr}");
            Some(runtime_instance.spawn(async move {
                server_instance.run(server_addr).await;
            }))
        } else {
            log::info!("[UniLyricApp new] WebSocket 服务器未启用。");
            None
        };

        // --- 初始化AMLL媒体连接器配置 ---
        let mc_config = AMLLConnectorConfig {
            enabled: settings.amll_connector_enabled,
            websocket_url: settings.amll_connector_websocket_url.clone(),
        };

        // --- 构建 UniLyricApp 实例 ---
        let mut app = Self {
            current_auto_search_ui_populated: false,

            lyrics_helper: None,
            lyrics_helper_rx,
            conversion_result_rx: None,
            input_text: String::new(),
            output_text: String::new(),
            display_translation_lrc_output: String::new(),
            display_romanization_lrc_output: String::new(),
            source_format: settings.last_source_format,
            target_format: settings.last_target_format,
            available_formats: LyricFormat::all().to_vec(),
            last_opened_file_path: None,
            last_saved_file_path: None,
            conversion_in_progress: false,
            show_bottom_log_panel: false,
            new_trigger_log_exists: false,
            is_any_file_hovering_window: false,
            show_romanization_lrc_panel: false,
            show_translation_lrc_panel: false,
            wrap_text: true,
            show_settings_window: false,
            show_amll_connector_sidebar: mc_config.enabled,
            parsed_lyric_data: None,
            editable_metadata: Vec::new(),
            http_client: async_http_client,
            search_query: String::new(),
            search_results: Vec::new(),
            search_in_progress: false,
            download_in_progress: false,
            search_result_rx: None,
            download_result_rx: None,

            show_metadata_panel: false,
            show_markers_panel: false,
            current_markers: Vec::new(),
            metadata_source_is_download: false,

            qqmusic_auto_search_status: Arc::new(Mutex::new(AutoSearchStatus::NotAttempted)),
            kugou_auto_search_status: Arc::new(Mutex::new(AutoSearchStatus::NotAttempted)),
            netease_auto_search_status: Arc::new(Mutex::new(AutoSearchStatus::NotAttempted)),
            amll_db_auto_search_status: Arc::new(Mutex::new(AutoSearchStatus::NotAttempted)),
            last_qq_search_result: Arc::new(Mutex::new(None)),
            last_kugou_search_result: Arc::new(Mutex::new(None)),
            last_netease_search_result: Arc::new(Mutex::new(None)),
            last_amll_db_search_result: Arc::new(Mutex::new(None)),
            musixmatch_auto_search_status: Arc::new(Mutex::new(AutoSearchStatus::NotAttempted)),
            last_musixmatch_search_result: Arc::new(Mutex::new(None)),

            show_search_window: false,
            loaded_translation_lrc: None,
            loaded_romanization_lrc: None,
            app_settings: Arc::new(Mutex::new(settings.clone())),
            temp_edit_settings: settings.clone(),
            log_display_buffer: Vec::with_capacity(200),
            ui_log_receiver,
            media_connector_config: Arc::new(Mutex::new(mc_config.clone())),
            media_connector_status: Arc::new(Mutex::new(WebsocketStatus::default())),
            current_media_info: Arc::new(TokioMutex::new(None)),
            media_connector_command_tx: None,
            media_connector_update_rx: mc_update_rx,
            media_connector_worker_handle: None,
            media_connector_update_tx_for_worker: mc_update_tx,
            audio_visualization_is_active: settings.send_audio_data_to_player,
            available_smtc_sessions: Arc::new(Mutex::new(Vec::new())),
            selected_smtc_session_id: Arc::new(Mutex::new(None)),
            initial_selected_smtc_session_id_from_settings: settings
                .last_selected_smtc_session_id
                .clone(),
            last_smtc_position_ms: 0,
            last_smtc_position_report_time: None,
            is_currently_playing_sensed_by_smtc: false,
            current_song_duration_ms: 0,
            smtc_time_offset_ms: settings.smtc_time_offset_ms,
            current_smtc_volume: Arc::new(Mutex::new(None)),
            last_requested_volume_for_session: Arc::new(Mutex::new(None)),
            auto_fetch_result_tx: auto_fetch_tx,
            auto_fetch_result_rx: auto_fetch_rx,
            tokio_runtime: runtime_instance,
            progress_simulation_interval: Duration::from_millis(100),
            progress_timer_shutdown_tx: None,
            progress_timer_join_handle: None,
            last_auto_fetch_source_format: None,
            last_auto_fetch_source_for_stripping_check: None,
            manual_refetch_request: None,
            local_lyrics_cache_index: Arc::new(Mutex::new(local_lyrics_cache_index_data)),
            local_lyrics_cache_index_path: local_cache_index_file_path,
            local_lyrics_cache_dir_path: local_cache_dir,
            local_cache_auto_search_status: Arc::new(Mutex::new(Default::default())),
            toasts,
            websocket_server_command_tx: if settings.websocket_server_settings.enabled {
                Some(ws_cmd_tx)
            } else {
                None
            },
            websocket_server_handle,
            websocket_server_enabled: settings.websocket_server_settings.enabled,
            websocket_server_port: settings.websocket_server_settings.port,
            last_true_smtc_processed_info: Arc::new(TokioMutex::new(None)),
            shutdown_initiated: false,
            ttml_db_upload_in_progress: false,
            ttml_db_last_paste_url: None,
            ttml_db_upload_action_rx: upload_action_rx,
            ttml_db_upload_action_tx: upload_action_tx,
        };

        // --- 启动AMLL媒体连接器 (如果启用) ---
        if mc_config.enabled {
            amll_connector_manager::ensure_running(&mut app);
            if let Some(ref initial_id) = app.initial_selected_smtc_session_id_from_settings {
                if let Some(ref tx) = app.media_connector_command_tx {
                    log::debug!("[UniLyricApp new] 尝试恢复上次选择的 SMTC 会话 ID: {initial_id}");
                    if tx
                        .send(ConnectorCommand::SelectSmtcSession(initial_id.clone()))
                        .is_err()
                    {
                        log::error!("[UniLyricApp new] 启动时发送 SelectSmtcSession 命令失败。");
                    }
                    // 同时更新运行时的 selected_smtc_session_id 状态
                    *app.selected_smtc_session_id.lock().unwrap() = Some(initial_id.clone());
                } else {
                    log::error!(
                        "[UniLyricApp new] 启动时无法应用上次选择的 SMTC 会话：command_tx 不可用。"
                    );
                }
            }
            if app.audio_visualization_is_active {
                if let Some(tx) = &app.media_connector_command_tx {
                    if tx.send(ConnectorCommand::StartAudioVisualization).is_err() {
                        log::error!(
                            "[UniLyricApp new] 启动时发送 StartAudioVisualization 命令失败。"
                        );
                    }
                } else {
                    log::warn!(
                        "[UniLyricApp new] 启动时 media_connector_command_tx 不可用，无法发送 StartAudioVisualization。"
                    );
                }
            }
        }

        app
    }
}
