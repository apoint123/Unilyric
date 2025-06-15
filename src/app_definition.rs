// 导入标准库和第三方库
use std::collections::{HashMap, HashSet}; // 哈希表和哈希集合
use std::fs::File; // 文件操作
use std::io::{BufRead, BufReader}; // 带缓冲的读写操作
use std::path::PathBuf; // 路径操作
use std::sync::mpsc::channel as std_channel;
use std::sync::mpsc::{
    Receiver as StdReceiver,
    Sender as StdSender, // 标准库多生产者单消费者通道
};
use std::sync::{Arc, Mutex}; // 原子引用计数和互斥锁
use std::thread; // 线程操作
use std::time::{Duration, Instant}; // 时间和即时操作 // 标准库通道创建函数别名

use egui_toast::Toasts; // egui 的 toast 通知组件
use reqwest::Client; // HTTP 客户端
use tokio::sync::Mutex as TokioMutex; // Tokio 的异步互斥锁

// 导入项目内部模块
use crate::amll_connector::amll_connector_manager; // AMLL 连接器管理器
use crate::amll_connector::{
    AMLLConnectorConfig,
    ConnectorCommand,
    ConnectorUpdate,
    NowPlayingInfo,
    WebsocketStatus,        // AMLL 连接器相关定义
    types::SmtcSessionInfo, // SMTC 会话信息
};
use crate::amll_lyrics_fetcher::{AmllIndexEntry, AmllSearchField}; // AMLL 歌词获取器相关定义
use crate::app::TtmlDbUploadUserAction;
use crate::app_settings::AppSettings; // 应用设置
use crate::logger::LogEntry; // 日志条目定义
use crate::metadata_processor::MetadataStore; // 元数据处理器
use crate::types::{
    // 应用中使用的各种数据类型定义
    AmllIndexDownloadState,
    AmllTtmlDownloadState,
    AutoFetchResult,
    AutoSearchStatus,
    CanonicalMetadataKey,
    EditableMetadataEntry,
    KrcDownloadState,
    LocalLyricCacheEntry,
    LyricFormat,
    MarkerInfo,
    NeteaseDownloadState,
    QqMusicDownloadState,
    TtmlParagraph,
};
use crate::websocket_server::ServerCommand; // WebSocket 服务器命令
use crate::{netease_lyrics_fetcher, utils, websocket_server}; // 其他模块
use tokio::sync::mpsc as tokio_mpsc; // Tokio 的多生产者单消费者通道 // TTML 数据库上传用户操作

// --- UniLyricApp 结构体定义 ---
// UniLyricApp 是应用的核心结构体，包含了应用的所有状态和数据。
pub struct UniLyricApp {
    // --- UI相关的文本输入输出区域 ---
    pub input_text: String,                      // 主输入框文本
    pub output_text: String,                     // 主输出框文本 (转换后的歌词)
    pub display_translation_lrc_output: String,  // 翻译LRC预览面板的文本
    pub display_romanization_lrc_output: String, // 罗马音LRC预览面板的文本

    // --- 格式选择与文件路径 ---
    pub source_format: LyricFormat,             // 当前选择的源歌词格式
    pub target_format: LyricFormat,             // 当前选择的目标歌词格式
    pub available_formats: Vec<LyricFormat>,    // 所有可用的歌词格式列表
    pub last_opened_file_path: Option<PathBuf>, // 上次打开文件的路径
    pub last_saved_file_path: Option<PathBuf>,  // 上次保存文件的路径

    // --- 状态标志 (UI控制与内部逻辑) ---
    pub conversion_in_progress: bool, // 标记歌词转换是否正在进行
    pub source_is_line_timed: bool,   // 标记源歌词是否为逐行定时 (如LRC)
    pub detected_formatted_ttml_source: bool, // 标记是否检测到源TTML是特定格式化过的 (如Apple Music)
    pub show_bottom_log_panel: bool,          // 是否显示底部日志面板
    pub new_trigger_log_exists: bool,         // 是否有新的触发器日志（用于UI提示）
    pub is_any_file_hovering_window: bool,    // 标记是否有文件悬停在应用窗口上 (用于拖放提示)
    pub show_markers_panel: bool,             // 是否显示标记信息面板
    pub show_romanization_lrc_panel: bool,    // 是否显示罗马音LRC编辑/预览面板
    pub show_translation_lrc_panel: bool,     // 是否显示翻译LRC编辑/预览面板
    pub wrap_text: bool,                      // 文本框是否自动换行
    pub show_metadata_panel: bool,            // 是否显示元数据编辑面板
    pub show_settings_window: bool,           // 是否显示设置窗口
    pub metadata_source_is_download: bool,    // 标记当前元数据是否主要来自网络下载
    pub show_amll_connector_sidebar: bool,    // 是否显示AMLL媒体连接器侧边栏
    pub current_auto_search_ui_populated: bool, // 标记自动搜索UI是否已被内容填充
    pub audio_visualization_enabled_by_ui: bool, // UI上是否启用了音频可视化数据发送

    // --- 核心数据存储 ---
    pub parsed_ttml_paragraphs: Option<Vec<TtmlParagraph>>, // 解析后的TTML歌词段落
    pub metadata_store: Arc<Mutex<MetadataStore>>,          // 元数据存储 (多线程安全)
    pub editable_metadata: Vec<EditableMetadataEntry>,      // UI上可编辑的元数据列表
    pub persistent_canonical_keys: HashSet<CanonicalMetadataKey>, // 用户固定的规范元数据键集合
    pub current_markers: Vec<MarkerInfo>,                   // 当前歌词中的标记信息 (如乐器段)
    pub current_raw_ttml_from_input: Option<String>, // 从输入缓存的原始TTML文本 (如果源是TTML)

    // --- 网络下载通用 ---
    pub http_client: Client, // 全局HTTP客户端实例

    // --- 网络下载相关 (QQ音乐) ---
    pub qqmusic_query: String, // QQ音乐搜索查询字符串
    pub qq_download_state: Arc<Mutex<QqMusicDownloadState>>, // QQ音乐下载状态 (多线程安全)
    pub show_qqmusic_download_window: bool, // 是否显示QQ音乐下载窗口
    pub qqmusic_auto_search_status: Arc<Mutex<AutoSearchStatus>>, // QQ音乐自动搜索状态
    pub last_qq_search_result: Arc<Mutex<Option<crate::types::ProcessedLyricsSourceData>>>, // 上次QQ音乐搜索结果缓存

    // --- 网络下载相关 (酷狗音乐) ---
    pub kugou_query: String, // 酷狗音乐搜索查询字符串
    pub kugou_download_state: Arc<Mutex<KrcDownloadState>>, // 酷狗音乐KRC下载状态 (多线程安全)
    pub show_kugou_download_window: bool, // 是否显示酷狗音乐下载窗口
    pub pending_krc_translation_lines: Option<Vec<String>>, // 从KRC文件解析出的待处理内嵌翻译行
    pub kugou_auto_search_status: Arc<Mutex<AutoSearchStatus>>, // 酷狗自动搜索状态
    pub last_kugou_search_result: Arc<Mutex<Option<crate::types::ProcessedLyricsSourceData>>>, // 上次酷狗搜索结果缓存

    // --- 网络下载相关 (网易云音乐) ---
    pub netease_query: String, // 网易云音乐搜索查询字符串
    pub netease_download_state: Arc<Mutex<NeteaseDownloadState>>, // 网易云音乐下载状态 (多线程安全)
    pub show_netease_download_window: bool, // 是否显示网易云音乐下载窗口
    pub netease_client: Arc<Mutex<Option<crate::netease_lyrics_fetcher::api::NeteaseClient>>>, // 网易云音乐API客户端实例 (多线程安全)
    pub direct_netease_main_lrc_content: Option<String>, // 直接从网易云获取的主LRC歌词内容 (特殊情况)
    pub netease_auto_search_status: Arc<Mutex<AutoSearchStatus>>, // 网易云自动搜索状态
    pub last_netease_search_result: Arc<Mutex<Option<crate::types::ProcessedLyricsSourceData>>>, // 上次网易云搜索结果缓存

    // --- 网络下载相关 (amll-ttml-db) ---
    pub amll_db_repo_url_base: String, // AMLL TTML数据库仓库的基础URL
    pub amll_index: Arc<Mutex<Vec<AmllIndexEntry>>>, // AMLL索引数据 (多线程安全)
    pub amll_index_download_state: Arc<Mutex<AmllIndexDownloadState>>, // AMLL索引下载状态 (多线程安全)
    pub amll_search_query: String,                                     // AMLL数据库搜索查询字符串
    pub amll_selected_search_field: AmllSearchField, // AMLL数据库当前选择的搜索字段
    pub amll_search_results: Arc<Mutex<Vec<AmllIndexEntry>>>, // AMLL数据库搜索结果 (多线程安全)
    pub amll_ttml_download_state: Arc<Mutex<AmllTtmlDownloadState>>, // AMLL TTML歌词下载状态 (多线程安全)
    pub show_amll_download_window: bool,                             // 是否显示AMLL数据库下载窗口
    pub amll_index_cache_path: Option<PathBuf>,                      // AMLL索引本地缓存文件路径
    pub amll_db_auto_search_status: Arc<Mutex<AutoSearchStatus>>,    // AMLL DB自动搜索状态
    pub last_amll_db_search_result: Arc<Mutex<Option<crate::types::ProcessedLyricsSourceData>>>, // 上次AMLL DB搜索结果缓存

    // --- 从文件加载的次要LRC数据 ---
    pub loaded_translation_lrc: Option<Vec<crate::types::DisplayLrcLine>>, // 手动加载的翻译LRC行 (用于UI显示和处理)
    pub loaded_romanization_lrc: Option<Vec<crate::types::DisplayLrcLine>>, // 手动加载的罗马音LRC行

    // --- 从网络下载的待处理次要歌词内容 ---
    pub session_platform_metadata: HashMap<String, String>, // 当前会话从平台获取的元数据 (如歌曲名、歌手)
    pub pending_translation_lrc_from_download: Option<String>, // 从网络下载的待处理翻译LRC文本
    pub pending_romanization_qrc_from_download: Option<String>, // 从网络下载的待处理罗马音QRC文本
    pub pending_romanization_lrc_from_download: Option<String>, // 从网络下载的待处理罗马音LRC文本

    // --- 应用设置 ---
    pub app_settings: Arc<Mutex<AppSettings>>, // 应用设置 (多线程安全，用于持久化)
    pub temp_edit_settings: AppSettings,       // 用于在设置窗口中临时编辑的设置副本

    // --- 日志系统 ---
    pub log_display_buffer: Vec<LogEntry>, // UI上显示的日志条目缓冲区
    pub ui_log_receiver: StdReceiver<LogEntry>, // 从日志后端接收日志条目的通道

    // --- AMLL Connector (媒体播放器集成) ---
    pub media_connector_config: Arc<Mutex<AMLLConnectorConfig>>, // AMLL连接器配置 (多线程安全)
    pub media_connector_status: Arc<Mutex<WebsocketStatus>>, // AMLL连接器WebSocket状态 (多线程安全)
    pub current_media_info: Arc<TokioMutex<Option<NowPlayingInfo>>>, // 当前播放的媒体信息 (异步多线程安全)
    pub media_connector_command_tx: Option<StdSender<ConnectorCommand>>, // 发送命令到AMLL连接器工作线程的通道发送端
    pub media_connector_update_rx: StdReceiver<ConnectorUpdate>, // 从AMLL连接器工作线程接收更新的通道接收端
    pub media_connector_worker_handle: Option<thread::JoinHandle<()>>, // AMLL连接器工作线程的句柄
    pub media_connector_update_tx_for_worker: StdSender<ConnectorUpdate>, // 用于连接器内部任务向主更新通道发送消息的克隆发送端

    // --- SMTC 会话管理 (系统媒体传输控制) ---
    pub available_smtc_sessions: Arc<Mutex<Vec<SmtcSessionInfo>>>, // 可用的SMTC会话列表 (多线程安全)
    pub selected_smtc_session_id: Arc<Mutex<Option<String>>>, // 用户当前选择的SMTC会话ID (多线程安全)
    pub initial_selected_smtc_session_id_from_settings: Option<String>, // 从设置加载的初始选定SMTC会话ID
    pub last_smtc_position_ms: u64, // 上次从SMTC获取的播放位置 (毫秒)
    pub last_smtc_position_report_time: Option<Instant>, // 上次报告SMTC播放位置的时间点
    pub is_currently_playing_sensed_by_smtc: bool, // SMTC是否报告当前正在播放
    pub current_song_duration_ms: u64, // 当前歌曲的总时长 (毫秒)
    pub smtc_time_offset_ms: i64,   // SMTC时间偏移量 (毫秒，用于校准)
    pub current_smtc_volume: Arc<Mutex<Option<(f32, bool)>>>, // 当前SMTC报告的音量 (值, 是否静音)
    pub last_requested_volume_for_session: Arc<Mutex<Option<String>>>, // 上次为特定会话请求音量的时间戳或标识

    // --- 自动获取与异步处理 ---
    pub auto_fetch_result_rx: StdReceiver<crate::types::AutoFetchResult>, // 接收自动获取歌词结果的通道
    pub auto_fetch_result_tx: StdSender<crate::types::AutoFetchResult>, // 发送自动获取歌词结果的通道
    pub tokio_runtime: Arc<tokio::runtime::Runtime>, // Tokio异步运行时实例 (Arc方便共享)
    pub progress_simulation_interval: Duration,      // 播放进度模拟定时器的间隔
    pub progress_timer_shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>, // 关闭进度定时器的发送端
    pub progress_timer_join_handle: Option<tokio::task::JoinHandle<()>>, // 进度定时器任务的句柄
    pub last_auto_fetch_source_format: Option<LyricFormat>, // 上次自动获取的歌词的原始格式
    pub last_auto_fetch_source_for_stripping_check: Option<crate::types::AutoSearchSource>, // 上次自动获取的来源 (用于判断是否需要清理)
    pub manual_refetch_request: Option<crate::types::AutoSearchSource>, // 手动请求重新获取特定来源的标记

    // --- 本地歌词缓存 ---
    pub local_lyrics_cache_index: Arc<Mutex<Vec<LocalLyricCacheEntry>>>, // 本地缓存歌词的索引 (多线程安全)
    pub local_lyrics_cache_index_path: Option<PathBuf>,                  // 本地缓存索引文件的路径
    pub local_lyrics_cache_dir_path: Option<PathBuf>, // 本地缓存歌词文件的目录路径
    pub local_cache_auto_search_status: Arc<Mutex<AutoSearchStatus>>, // 本地缓存自动搜索状态

    // --- UI 通知 ---
    pub toasts: Toasts, // egui toast 通知管理器

    // --- WebSocket 服务器 (用于外部控制或歌词同步) ---
    pub websocket_server_command_tx: Option<tokio_mpsc::Sender<ServerCommand>>, // 发送命令到WebSocket服务器任务的通道发送端
    pub websocket_server_handle: Option<tokio::task::JoinHandle<()>>, // WebSocket服务器任务的句柄
    pub websocket_server_enabled: bool,                               // WebSocket服务器是否启用
    pub websocket_server_port: u16,                                   // WebSocket服务器监听端口
    pub last_true_smtc_processed_info: Arc<TokioMutex<Option<NowPlayingInfo>>>, // 上次处理的真实SMTC播放信息 (异步多线程安全)
    pub shutdown_initiated: bool, // 标记应用是否已启动关闭流程

    // --- TTML DB 上传功能相关 ---
    pub ttml_db_upload_in_progress: bool, // 标记TTML DB上传是否正在进行
    pub ttml_db_last_paste_url: Option<String>, // 上次成功上传到paste服务后获取的URL
    pub ttml_db_upload_action_rx: StdReceiver<TtmlDbUploadUserAction>, // 接收TTML DB上传操作结果的通道
    pub ttml_db_upload_action_tx: StdSender<TtmlDbUploadUserAction>, // 发送TTML DB上传操作命令的通道
}

impl UniLyricApp {
    /// UniLyricApp的构造函数，用于创建应用实例。
    ///
    /// # 参数
    /// * `cc` - `&eframe::CreationContext`，eframe创建上下文，用于访问egui上下文等。
    /// * `settings` - `AppSettings`，从配置文件加载的应用设置。
    /// * `ui_log_receiver` - `Receiver<LogEntry>`，用于从日志后端接收日志条目并在UI上显示。
    ///
    /// # 返回
    /// `Self` - UniLyricApp 应用实例。
    pub fn new(
        cc: &eframe::CreationContext,           // eframe 创建上下文
        settings: AppSettings,                  // 应用设置实例
        ui_log_receiver: StdReceiver<LogEntry>, // UI日志接收器
    ) -> Self {
        // 设置自定义字体函数
        fn setup_custom_fonts(ctx: &egui::Context) {
            let mut fonts = egui::FontDefinitions::default();
            // 加载思源更纱黑体作为自定义字体
            fonts.font_data.insert(
                "SarasaUiSC".to_owned(), // 字体名称
                egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/SarasaUiSC-Regular.ttf" // 从静态字节码加载字体文件
                ))
                .into(),
            );
            // 将自定义字体添加到 proportional (比例) 字体族的首位
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "SarasaUiSC".to_owned());
            // 将自定义字体添加到 monospace (等宽) 字体族
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("SarasaUiSC".to_owned());
            ctx.set_fonts(fonts); // 应用字体定义到egui上下文
        }

        setup_custom_fonts(&cc.egui_ctx); // 调用字体设置函数
        egui_extras::install_image_loaders(&cc.egui_ctx); // 安装egui的图像加载器

        // 创建异步HTTP客户端实例
        let async_http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(30)) // 设置请求超时为30秒
            .build()
            .expect("构建HTTP客户端失败"); // 如果构建失败则panic

        // 初始化网易云音乐API客户端实例
        let netease_api_client_instance = match netease_lyrics_fetcher::api::NeteaseClient::new() {
            Ok(client) => Some(client), // 成功则Some(client)
            Err(e) => {
                log::error!("[Unilyric] 初始化网易云API客户端失败: {e}");
                None // 失败则None
            }
        };

        // 创建用于TTML DB上传操作的通道
        let (upload_action_tx, upload_action_rx) = std_channel::<TtmlDbUploadUserAction>();

        // --- 初始化固定的元数据和元数据存储 ---
        let mut initial_persistent_canonical_keys = HashSet::new(); // 初始空的固定规范键集合
        let mut initial_metadata_store = MetadataStore::new(); // 初始空的元数据存储
        // 从应用设置中加载固定的元数据
        for (display_key, values_vec) in &settings.pinned_metadata {
            match display_key.trim().parse::<CanonicalMetadataKey>() {
                // 尝试解析为规范键
                Ok(canonical_key) => {
                    initial_persistent_canonical_keys.insert(canonical_key.clone()); // 添加到固定键集合
                    for v_str in values_vec {
                        // 遍历该键的所有固定值
                        if let Err(e) = initial_metadata_store.add(display_key, v_str.clone()) {
                            log::error!(
                                "[Unilyric] 从设置加载固定元数据 '{display_key}' (值: '{v_str}') 到Store失败: {e}"
                            );
                        }
                    }
                }
                Err(_) => {
                    // 如果不是规范键，则视为自定义键
                    let custom_key = CanonicalMetadataKey::Custom(display_key.trim().to_string());
                    initial_persistent_canonical_keys.insert(custom_key.clone());
                    for v_str in values_vec {
                        if let Err(e) = initial_metadata_store.add(display_key, v_str.clone()) {
                            log::error!(
                                "[Unilyric] 从设置加载固定自定义元数据 '{display_key}' (值: '{v_str}') 到Store失败: {e}"
                            );
                        }
                    }
                }
            }
        }

        // --- 初始化AMLL TTML数据库相关状态 ---
        // AMLL TTML DB仓库的基础URL (使用GitHub代理加速访问raw文件)
        let amll_repo_base = "https://github.moeyy.xyz/https://raw.githubusercontent.com/Steve-xmh/amll-ttml-db/main".to_string();
        let initial_amll_index_state; // AMLL索引的初始下载状态
        let mut initial_amll_index_data: Vec<AmllIndexEntry> = Vec::new(); // 初始AMLL索引数据
        // AMLL索引缓存文件路径 (通常在应用数据目录下)
        let amll_cache_path: Option<PathBuf> =
            utils::get_app_data_dir().map(|dir| dir.join("amll_index_cache.jsonl"));

        let mut loaded_cached_head: Option<String> = None; // 从缓存加载的索引HEAD提交哈希

        // 尝试从本地缓存加载AMLL索引
        if let Some(ref cache_p) = amll_cache_path {
            // 尝试加载缓存的HEAD信息
            match crate::amll_lyrics_fetcher::amll_fetcher::load_cached_index_head(cache_p) {
                Ok(Some(head)) => {
                    loaded_cached_head = Some(head); // 成功加载HEAD
                }
                Ok(None) => {
                    log::info!("[UniLyricApp new] 未找到缓存的 AMLL 索引 HEAD。");
                }
                Err(e) => {
                    log::warn!("[UniLyricApp new] 从缓存加载 AMLL 索引 HEAD 失败: {e}。");
                }
            }

            if cache_p.exists() {
                // 如果缓存文件实际存在
                // 尝试加载索引文件内容
                match crate::amll_lyrics_fetcher::amll_fetcher::load_index_from_cache(cache_p) {
                    Ok(parsed_cached_entries) => {
                        if !parsed_cached_entries.is_empty() {
                            // 如果解析出有效条目
                            initial_amll_index_data = parsed_cached_entries; // 应用缓存数据
                            if let Some(head_str) = loaded_cached_head.clone() {
                                // 如果有HEAD信息，则状态为成功加载了该版本的缓存
                                initial_amll_index_state =
                                    AmllIndexDownloadState::Success(head_str.clone());
                                log::info!(
                                    "[UniLyricApp new] AMLL 索引从缓存加载成功 (HEAD: {}), 待检查更新。",
                                    head_str.chars().take(7).collect::<String>() // 日志中只显示HEAD前7位
                                );
                            } else {
                                // 没有HEAD信息，状态设为空闲，后续会尝试下载
                                initial_amll_index_state = AmllIndexDownloadState::Idle;
                                log::info!(
                                    "[UniLyricApp new] AMLL 索引从缓存加载，但无缓存 HEAD，状态设为 Idle。"
                                );
                            }
                        } else {
                            // 缓存文件存在但解析后为空或无效
                            log::info!(
                                "[UniLyricApp new] AMLL 索引缓存文件存在但解析后为空或无效。"
                            );
                            if let Some(head_str) = loaded_cached_head.clone() {
                                // 即使内容为空，但有HEAD，也标记为Success，让后续更新逻辑处理
                                initial_amll_index_state =
                                    AmllIndexDownloadState::Success(head_str.clone());
                                log::warn!(
                                    "[UniLyricApp new] 索引缓存为空，但有缓存HEAD ({})，设为Success待更新。",
                                    head_str.chars().take(7).collect::<String>()
                                );
                            } else {
                                initial_amll_index_state = AmllIndexDownloadState::Idle;
                            }
                        }
                    }
                    Err(e) => {
                        // 从缓存加载索引内容失败
                        log::warn!("[UniLyricApp new] 从缓存加载 AMLL 索引内容失败: {e}。");
                        if let Some(head_str) = loaded_cached_head.clone() {
                            initial_amll_index_state = AmllIndexDownloadState::Error(format!(
                                "缓存索引内容加载失败 (HEAD: {}): {}",
                                head_str.chars().take(7).collect::<String>(),
                                e
                            ));
                        } else {
                            initial_amll_index_state =
                                AmllIndexDownloadState::Error(format!("缓存索引内容加载失败: {e}"));
                        }
                    }
                }
            } else {
                // 缓存文件不存在
                log::info!("[UniLyricApp new] AMLL 索引缓存文件 {cache_p:?} 不存在。");
                initial_amll_index_state = AmllIndexDownloadState::Idle;
            }
        } else {
            // 获取应用数据目录失败，无法使用缓存
            initial_amll_index_state = AmllIndexDownloadState::Idle;
        }

        // --- 初始化AMLL媒体连接器配置 ---
        let mc_config = AMLLConnectorConfig {
            enabled: settings.amll_connector_enabled, // 是否启用连接器
            websocket_url: settings.amll_connector_websocket_url.clone(), // WebSocket URL
        };

        // --- 初始化通道 ---
        let (auto_fetch_tx, auto_fetch_rx) = std_channel::<AutoFetchResult>(); // 自动获取歌词结果通道
        let (mc_update_tx, mc_update_rx) = std_channel::<ConnectorUpdate>(); // AMLL连接器更新通道

        // --- 初始化本地歌词缓存 ---
        let mut local_lyrics_cache_index_data: Vec<LocalLyricCacheEntry> = Vec::new(); // 本地缓存索引数据
        let mut local_cache_dir: Option<PathBuf> = None; // 本地缓存目录路径
        let mut local_cache_index_file_path: Option<PathBuf> = None; // 本地缓存索引文件路径

        if let Some(data_dir) = utils::get_app_data_dir() {
            // 获取应用数据目录
            let cache_dir = data_dir.join("local_lyrics_cache"); // 缓存子目录
            if !cache_dir.exists() {
                // 如果目录不存在则创建
                if let Err(e) = std::fs::create_dir_all(&cache_dir) {
                    log::error!("[UniLyricApp] 无法创建本地歌词缓存目录 {cache_dir:?}: {e}");
                }
            }
            local_cache_dir = Some(cache_dir.clone()); // 保存缓存目录路径

            let index_file = cache_dir.join("local_lyrics_index.jsonl"); // 索引文件名
            if index_file.exists() {
                // 如果索引文件存在
                match File::open(&index_file) {
                    // 打开文件
                    Ok(file) => {
                        let reader = BufReader::new(file); // 使用带缓冲的读取器
                        for line_result in reader.lines() {
                            //逐行读取
                            match line_result {
                                Ok(line) => {
                                    if !line.trim().is_empty() {
                                        // 忽略空行
                                        // 尝试将行JSON反序列化为缓存条目
                                        match serde_json::from_str::<LocalLyricCacheEntry>(&line) {
                                            Ok(entry) => local_lyrics_cache_index_data.push(entry),
                                            Err(e) => log::warn!(
                                                "[UniLyricApp] 解析本地缓存索引行 '{line}' 失败: {e}"
                                            ),
                                        }
                                    }
                                }
                                Err(e) => {
                                    // 读取行失败
                                    log::error!("[UniLyricApp] 读取本地缓存索引文件行失败: {e}");
                                    break; // 停止读取
                                }
                            }
                        }
                        log::info!(
                            "[UniLyricApp] 从 {:?} 加载了 {} 条本地缓存歌词索引。",
                            index_file,
                            local_lyrics_cache_index_data.len()
                        );
                    }
                    Err(e) => log::error!(
                        // 打开索引文件失败
                        "[UniLyricApp] 打开本地缓存索引文件 {index_file:?} 失败: {e}"
                    ),
                }
            }
            local_cache_index_file_path = Some(index_file); // 保存索引文件路径
        }

        // --- 创建Tokio异步运行时 ---
        let runtime_instance = tokio::runtime::Builder::new_multi_thread() // 多线程运行时
            .enable_all() // 启用所有Tokio功能
            .worker_threads(2) // 设置工作线程数为2 (可根据需要调整)
            .thread_name("unilyric-app-tokio") // 设置线程名称前缀
            .build()
            .expect("无法为应用创建 Tokio 运行时");

        // --- 初始化UI Toast通知 ---
        let toasts = Toasts::new()
            .anchor(egui::Align2::LEFT_TOP, (10.0, 10.0)) // 设置Toast锚点在左上角，带偏移
            .direction(egui::Direction::TopDown); // Toast从上往下排列

        // --- 初始化WebSocket服务器相关通道 ---
        let (ws_cmd_tx, ws_cmd_rx) = tokio_mpsc::channel(32); // Tokio mpsc通道，缓冲区大小32

        // --- 构建 UniLyricApp 实例 ---
        let mut app = Self {
            // --- UI相关的文本输入输出区域 ---
            input_text: String::new(),
            output_text: String::new(),
            display_translation_lrc_output: String::new(),
            display_romanization_lrc_output: String::new(),

            // --- 格式选择与文件路径 ---
            source_format: settings.last_source_format, // 从设置恢复上次源格式
            target_format: settings.last_target_format, // 从设置恢复上次目标格式
            available_formats: LyricFormat::all(),      // 获取所有可用格式
            last_opened_file_path: None,
            last_saved_file_path: None,

            // --- 状态标志 (UI控制与内部逻辑) ---
            conversion_in_progress: false,
            source_is_line_timed: false,
            detected_formatted_ttml_source: false,
            show_bottom_log_panel: false,
            new_trigger_log_exists: false,
            is_any_file_hovering_window: false,
            show_markers_panel: false,
            show_romanization_lrc_panel: false,
            show_translation_lrc_panel: false,
            wrap_text: true, // 默认启用文本换行
            show_metadata_panel: false,
            show_settings_window: false,
            metadata_source_is_download: false,
            show_amll_connector_sidebar: mc_config.enabled, // 根据配置决定是否显示连接器侧边栏
            current_auto_search_ui_populated: false,
            audio_visualization_enabled_by_ui: settings.send_audio_data_to_player, // 从设置恢复音频可视化启用状态

            // --- 核心数据存储 ---
            parsed_ttml_paragraphs: None,
            metadata_store: Arc::new(Mutex::new(initial_metadata_store)), // 使用初始化的元数据存储
            editable_metadata: Vec::new(), // UI可编辑元数据列表初始为空，后续会从store重建
            persistent_canonical_keys: initial_persistent_canonical_keys, // 使用初始化的固定键集合
            current_markers: Vec::new(),
            current_raw_ttml_from_input: None,

            // --- 网络下载通用 ---
            http_client: async_http_client, // 使用创建的HTTP客户端

            // --- 网络下载相关 (QQ音乐) ---
            qqmusic_query: String::new(),
            qq_download_state: Arc::new(Mutex::new(QqMusicDownloadState::Idle)),
            show_qqmusic_download_window: false,
            qqmusic_auto_search_status: Arc::new(Mutex::new(AutoSearchStatus::default())),
            last_qq_search_result: Arc::new(Mutex::new(None)),

            // --- 网络下载相关 (酷狗音乐) ---
            kugou_query: String::new(),
            kugou_download_state: Arc::new(Mutex::new(KrcDownloadState::Idle)),
            show_kugou_download_window: false,
            pending_krc_translation_lines: None,
            kugou_auto_search_status: Arc::new(Mutex::new(AutoSearchStatus::default())),
            last_kugou_search_result: Arc::new(Mutex::new(None)),

            // --- 网络下载相关 (网易云音乐) ---
            netease_query: String::new(),
            netease_download_state: Arc::new(Mutex::new(NeteaseDownloadState::Idle)),
            show_netease_download_window: false,
            netease_client: Arc::new(Mutex::new(netease_api_client_instance)), // 使用初始化的网易云客户端
            direct_netease_main_lrc_content: None,
            netease_auto_search_status: Arc::new(Mutex::new(AutoSearchStatus::default())),
            last_netease_search_result: Arc::new(Mutex::new(None)),

            // --- 网络下载相关 (amll-ttml-db) ---
            amll_db_repo_url_base: amll_repo_base, // AMLL DB仓库URL
            amll_index: Arc::new(Mutex::new(initial_amll_index_data)), // 使用初始化的AMLL索引数据
            amll_index_download_state: Arc::new(Mutex::new(initial_amll_index_state)), // 使用初始化的AMLL索引状态
            amll_search_query: String::new(),
            amll_selected_search_field: AmllSearchField::default(), // 默认搜索字段
            amll_search_results: Arc::new(Mutex::new(Vec::new())),
            amll_ttml_download_state: Arc::new(Mutex::new(AmllTtmlDownloadState::Idle)),
            show_amll_download_window: false,
            amll_index_cache_path: amll_cache_path, // AMLL索引缓存路径
            amll_db_auto_search_status: Arc::new(Mutex::new(AutoSearchStatus::default())),
            last_amll_db_search_result: Arc::new(Mutex::new(None)),

            // --- 从文件加载的次要LRC数据 ---
            loaded_translation_lrc: None,
            loaded_romanization_lrc: None,

            // --- 从网络下载的待处理次要歌词内容 ---
            session_platform_metadata: HashMap::new(),
            pending_translation_lrc_from_download: None,
            pending_romanization_qrc_from_download: None,
            pending_romanization_lrc_from_download: None,

            // --- 应用设置 ---
            app_settings: Arc::new(Mutex::new(settings.clone())), // 共享的应用设置
            temp_edit_settings: settings.clone(),                 // 用于编辑的临时设置副本

            // --- 日志系统 ---
            log_display_buffer: Vec::with_capacity(200), // 日志显示缓冲区，预分配容量
            ui_log_receiver,                             // 从构造函数传入的UI日志接收器

            // --- AMLL Connector (媒体播放器集成) ---
            media_connector_config: Arc::new(Mutex::new(mc_config.clone())), // 使用初始化的连接器配置
            media_connector_status: Arc::new(Mutex::new(WebsocketStatus::default())),
            current_media_info: Arc::new(TokioMutex::new(None)),
            media_connector_command_tx: None, // 命令发送端初始为None，在启动worker时设置
            media_connector_update_rx: mc_update_rx, // 连接器更新接收端
            media_connector_worker_handle: None, // 工作线程句柄初始为None
            media_connector_update_tx_for_worker: mc_update_tx, // 克隆的更新发送端，用于worker内部

            // --- SMTC 会话管理 ---
            available_smtc_sessions: Arc::new(Mutex::new(Vec::new())),
            selected_smtc_session_id: Arc::new(Mutex::new(None)),
            initial_selected_smtc_session_id_from_settings:
                settings // 从设置恢复上次选择的SMTC会话ID
                    .last_selected_smtc_session_id
                    .clone(),
            last_smtc_position_ms: 0,
            last_smtc_position_report_time: None,
            is_currently_playing_sensed_by_smtc: false,
            current_song_duration_ms: 0,
            smtc_time_offset_ms: settings.smtc_time_offset_ms, // 从设置恢复SMTC时间偏移
            current_smtc_volume: Arc::new(Mutex::new(None)),
            last_requested_volume_for_session: Arc::new(Mutex::new(None)),

            // --- 自动获取与异步处理 ---
            auto_fetch_result_tx: auto_fetch_tx, // 自动获取结果发送端
            auto_fetch_result_rx: auto_fetch_rx, // 自动获取结果接收端
            tokio_runtime: Arc::new(runtime_instance), // 使用创建的Tokio运行时
            progress_simulation_interval: Duration::from_millis(100), // 进度模拟间隔100ms
            progress_timer_shutdown_tx: None,
            progress_timer_join_handle: None,
            last_auto_fetch_source_format: None,
            last_auto_fetch_source_for_stripping_check: None,
            manual_refetch_request: None,

            // --- 本地歌词缓存 ---
            local_lyrics_cache_index: Arc::new(Mutex::new(local_lyrics_cache_index_data)), // 使用初始化的本地缓存索引
            local_lyrics_cache_index_path: local_cache_index_file_path, // 本地缓存索引文件路径
            local_lyrics_cache_dir_path: local_cache_dir,               // 本地缓存目录路径
            local_cache_auto_search_status: Arc::new(Mutex::new(AutoSearchStatus::default())),

            // --- UI 通知 ---
            toasts, // 使用创建的Toast管理器

            // --- WebSocket 服务器 ---
            websocket_server_enabled: settings.websocket_server_settings.enabled, // 从设置恢复WebSocket服务器启用状态
            websocket_server_port: settings.websocket_server_settings.port, // 从设置恢复WebSocket服务器端口
            websocket_server_command_tx: Some(ws_cmd_tx),                   // 存储命令发送端
            websocket_server_handle: None, // 服务器任务句柄初始为None

            last_true_smtc_processed_info: Arc::new(TokioMutex::new(None)), // 上次处理的真实SMTC信息

            // --- TTML DB 上传 ---
            ttml_db_upload_in_progress: false,
            ttml_db_last_paste_url: None,
            ttml_db_upload_action_rx: upload_action_rx, // TTML DB上传操作接收端
            ttml_db_upload_action_tx: upload_action_tx, // TTML DB上传操作发送端

            shutdown_initiated: false, // 应用关闭流程是否已启动
        };

        // --- 启动WebSocket服务器 (如果启用) ---
        if app.websocket_server_enabled {
            let server_addr = format!("127.0.0.1:{}", settings.websocket_server_settings.port); // 构建服务器地址
            let server_instance = websocket_server::WebsocketServer::new(ws_cmd_rx); // 创建服务器实例，传入命令接收端
            log::info!("[UniLyricApp new] 准备启动 WebSocket 服务器, 服务器地址: {server_addr}");
            // 在Tokio运行时中启动服务器任务
            let server_task_handle = app.tokio_runtime.spawn(async move {
                server_instance.run(server_addr).await; // 运行服务器
            });
            app.websocket_server_handle = Some(server_task_handle); // 保存服务器任务句柄
        } else {
            app.websocket_server_command_tx = None; // 如果未启用，则清空命令发送端
            log::info!("[UniLyricApp new] WebSocket 服务器未启用。");
        }

        // --- 从元数据存储重建UI可编辑元数据列表 ---
        app.rebuild_editable_metadata_from_store();

        // --- 启动AMLL媒体连接器 (如果启用) ---
        if mc_config.enabled {
            amll_connector_manager::ensure_running(&mut app); // 确保连接器工作线程正在运行
            // 尝试恢复上次选择的SMTC会话
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
            // 如果UI上启用了音频可视化数据发送
            if app.audio_visualization_enabled_by_ui {
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

        // --- 启动时检查AMLL索引更新 (如果设置启用) ---
        if settings.auto_check_amll_index_update_on_startup {
            app.check_for_amll_index_update();
        }

        app // 返回创建的应用实例
    }
}
