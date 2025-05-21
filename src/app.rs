// 导入项目内模块和外部库
use crate::app_settings::AppSettings; // 应用设置模块
use eframe::egui::{self, Pos2}; // egui UI库
use egui::Color32; // egui 颜色类型
use log::{error, info}; // 日志库
use reqwest::Client; // HTTP客户端，用于网络请求
use std::collections::{HashMap, HashSet}; // 标准库集合类型
use std::fmt::Write as FmtWrite; // 格式化写入 trait
use std::path::PathBuf; // 路径处理
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex}; // 原子引用计数和互斥锁，用于多线程共享数据 // 多生产者单消费者通道，用于接收日志

// 导入项目中定义的各种类型，如歌词格式、错误类型、元数据结构等
use crate::types::{
    AssMetadata, ConvertError, LrcContentType, LrcLine, LyricFormat, LysSyllable, MarkerInfo,
    ParsedSourceData, ProcessedAssData, TtmlParagraph, TtmlSyllable,
};

// 导入元数据处理器和规范化的元数据键类型
use crate::metadata_processor::MetadataStore;
use crate::types::CanonicalMetadataKey;

// 导入各个歌词格式的解析器和转换器模块
use crate::json_parser;
use crate::krc_parser;
use crate::logger::LogEntry; // 日志条目结构
use crate::lrc_parser;
use crate::lyricify_lines_parser;
use crate::lyricify_lines_to_ttml_data;
use crate::lys_parser;
use crate::lys_to_ttml_data;
use crate::qrc_parser;
use crate::qrc_to_ttml_data;
use crate::spl_parser;
use crate::ttml_generator;
use crate::ttml_parser;
use crate::yrc_parser;
use crate::yrc_to_ttml_data;
// 导入ASS解析器和各个在线歌词获取模块
use crate::{ass_parser, kugou_lyrics_fetcher, netease_lyrics_fetcher, qq_lyrics_fetcher};

/// 代表元数据编辑器中的一个可编辑条目。
#[derive(Clone, Debug)]
pub struct EditableMetadataEntry {
    pub key: String,        // 元数据键名 (显示用)
    pub value: String,      // 元数据值
    pub is_pinned: bool,    // 此条目是否被用户标记为“固定”
    pub is_from_file: bool, // 此条目是否来自当前加载的文件 (或为固定项的初始状态)
    pub id: egui::Id,       // egui 用于追踪UI元素的唯一ID
}

/// 表示QQ音乐下载状态的枚举。
#[derive(Debug, Clone)]
pub enum QqMusicDownloadState {
    Idle,                                                                // 空闲状态
    Downloading,                                                         // 下载中
    Success(crate::qq_lyrics_fetcher::qqlyricsfetcher::FetchedQqLyrics), // 下载成功，包含获取到的歌词数据
    Error(String),                                                       // 下载失败，包含错误信息
}

/// 表示酷狗音乐KRC歌词下载状态的枚举。
#[derive(Debug, Clone)]
pub enum KrcDownloadState {
    Idle,                                                   // 空闲状态
    Downloading,                                            // 下载中
    Success(crate::kugou_lyrics_fetcher::FetchedKrcLyrics), // 下载成功
    Error(String),                                          // 下载失败
}

/// 表示网易云音乐歌词下载状态的枚举。
#[derive(Debug, Clone)]
pub enum NeteaseDownloadState {
    Idle,                                                         // 空闲状态
    InitializingClient,                                           // 正在初始化API客户端
    Downloading,                                                  // 下载中
    Success(crate::netease_lyrics_fetcher::FetchedNeteaseLyrics), // 下载成功
    Error(String),                                                // 下载失败
}

/// UniLyricApp 结构体，代表整个应用程序的状态。
pub struct UniLyricApp {
    // UI相关的文本输入输出区域
    pub input_text: String,                      // 左侧输入框的文本内容
    pub output_text: String,                     // 中间输出框的文本内容
    pub display_translation_lrc_output: String,  // 右侧翻译LRC预览面板的文本内容
    pub display_romanization_lrc_output: String, // 右侧罗马音LRC预览面板的文本内容

    // 格式选择与文件路径
    pub source_format: LyricFormat,             // 当前选择的源歌词格式
    pub target_format: LyricFormat,             // 当前选择的目标歌词格式
    pub available_formats: Vec<LyricFormat>,    // 可用的歌词格式列表
    pub last_opened_file_path: Option<PathBuf>, // 上次打开文件的路径
    pub last_saved_file_path: Option<PathBuf>,  // 上次保存文件的路径

    // 状态标志
    pub conversion_in_progress: bool, // 标记转换过程是否正在进行
    pub source_is_line_timed: bool,   // 标记源文件是否为逐行歌词 (如LRC, LYL)
    pub detected_formatted_ttml_source: bool, // 标记源TTML是否是格式化的
    pub show_bottom_log_panel: bool,  // 是否显示底部日志面板
    pub new_trigger_log_exists: bool, // 是否有新的触发性日志 (如错误、警告) 尚未被用户通过打开日志面板查看
    pub is_any_file_hovering_window: bool, // 标记是否有文件正悬停在窗口上 (用于拖放提示)
    pub show_markers_panel: bool,     // 是否显示标记面板
    pub show_romanization_lrc_panel: bool, // 是否显示罗马音LRC预览面板
    pub show_translation_lrc_panel: bool, // 是否显示翻译LRC预览面板
    pub wrap_text: bool,              // 文本框是否自动换行
    pub show_metadata_panel: bool,    // 是否显示元数据编辑窗口
    pub show_settings_window: bool,   // 是否显示设置窗口
    pub metadata_source_is_download: bool, // 标记当前元数据是否主要来源于网络下载 (影响元数据合并策略)

    // 核心数据存储
    pub parsed_ttml_paragraphs: Option<Vec<TtmlParagraph>>, // 解析输入后得到的TTML段落数据 (中间格式)
    pub metadata_store: Arc<Mutex<MetadataStore>>,          // 存储所有元数据的地方，线程安全
    pub editable_metadata: Vec<EditableMetadataEntry>,      // UI上可编辑的元数据列表
    pub persistent_canonical_keys: HashSet<CanonicalMetadataKey>, // 用户希望固定的元数据类型的规范化键集合
    pub current_markers: Vec<MarkerInfo>,                         // 当前解析出的标记列表
    pub current_raw_ttml_from_input: Option<String>, // 如果源是TTML或JSON，这里存储原始的TTML字符串内容

    // 拖放相关
    pub last_known_pointer_pos_while_dragging: Option<Pos2>, // 文件拖放时最后已知的鼠标指针位置

    // 网络下载相关 (QQ音乐)
    pub qqmusic_query: String, // QQ音乐搜索查询词
    pub download_state: Arc<Mutex<QqMusicDownloadState>>, // QQ音乐下载状态，线程安全
    pub http_client: Client,   // 全局HTTP客户端实例，用于所有网络请求
    pub show_qqmusic_download_window: bool, // 是否显示QQ音乐下载模态窗口

    // 网络下载相关 (酷狗音乐)
    pub kugou_query: String, // 酷狗音乐搜索查询词
    pub kugou_download_state: Arc<Mutex<KrcDownloadState>>, // 酷狗音乐下载状态，线程安全
    pub show_kugou_download_window: bool, // 是否显示酷狗音乐下载模态窗口
    pub pending_krc_translation_lines: Option<Vec<String>>, // 从KRC文件内嵌翻译解析出的待合并翻译行

    // 网络下载相关 (网易云音乐)
    pub netease_query: String, // 网易云音乐搜索查询词
    pub netease_download_state: Arc<Mutex<NeteaseDownloadState>>, // 网易云音乐下载状态，线程安全
    pub show_netease_download_window: bool, // 是否显示网易云音乐下载模态窗口
    pub netease_client: Arc<Mutex<Option<netease_lyrics_fetcher::api::NeteaseClient>>>, // 网易云API客户端实例，线程安全

    // 从文件加载的次要LRC数据
    pub loaded_translation_lrc: Option<Vec<LrcLine>>, // 从文件加载的翻译LRC行
    pub loaded_romanization_lrc: Option<Vec<LrcLine>>, // 从文件加载的罗马音LRC行

    // 应用设置
    pub app_settings: Arc<Mutex<AppSettings>>, // 应用设置，线程安全
    pub temp_edit_settings: AppSettings,       // 在设置窗口中临时编辑的设置副本

    // 日志系统
    pub log_display_buffer: Vec<LogEntry>, // 存储在UI上显示的日志条目
    pub ui_log_receiver: Receiver<LogEntry>, // 从日志后端接收日志条目的通道

    // 从网络下载的待处理次要歌词内容
    pub session_platform_metadata: HashMap<String, String>, // 从下载平台获取的当次会话元数据 (如歌曲ID, 平台特定信息)
    pub pending_translation_lrc_from_download: Option<String>, // 从下载获取的待合并翻译LRC文本
    pub pending_romanization_qrc_from_download: Option<String>, // 从下载获取的待合并罗马音QRC文本
    pub pending_romanization_lrc_from_download: Option<String>, // 从下载获取的待合并罗马音LRC文本

    // 特殊情况：网易云直接下载的LRC主歌词（当没有YRC时）
    pub direct_netease_main_lrc_content: Option<String>,
}

// UniLyricApp 的实现块
impl UniLyricApp {
    /// UniLyricApp的构造函数，用于创建应用实例。
    ///
    /// # Arguments
    /// * `cc` - `&eframe::CreationContext`，eframe创建上下文，用于访问egui上下文等。
    /// * `settings` - `AppSettings`，从配置文件加载的应用设置。
    /// * `ui_log_receiver` - `Receiver<LogEntry>`，用于从日志后端接收日志条目并在UI上显示。
    ///
    /// # Returns
    /// `Self` - UniLyricApp 应用实例。
    pub fn new(
        cc: &eframe::CreationContext, // eframe 创建上下文，可以访问 egui::Context 等
        settings: AppSettings,        // 从配置文件加载的应用设置
        ui_log_receiver: Receiver<LogEntry>, // 用于从日志后端接收日志条目的通道
    ) -> Self {
        /// 内部辅助函数：设置自定义字体。
        /// 这是因为egui内置的字体无法显示中文
        fn setup_custom_fonts(ctx: &egui::Context) {
            let mut fonts = egui::FontDefinitions::default(); // 获取默认字体定义
            // 插入自定义字体数据 "SarasaUiSC"
            fonts.font_data.insert(
                "SarasaUiSC".to_owned(), // 字体名称
                egui::FontData::from_static(include_bytes!(
                    // 从静态字节数组加载字体文件
                    "../assets/fonts/SarasaUiSC-Regular.ttf" // 字体文件路径 (相对于项目根目录)
                ))
                .into(), // 转换为 egui::FontData
            );
            // 将 "SarasaUiSC" 添加到 proportional (比例) 字体家族的首选列表
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "SarasaUiSC".to_owned());
            // 将 "SarasaUiSC" 添加到 monospace (等宽) 字体家族的列表
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("SarasaUiSC".to_owned());
            ctx.set_fonts(fonts); // 应用新的字体定义到egui上下文
        }

        setup_custom_fonts(&cc.egui_ctx); // 调用字体设置函数

        // 初始化异步HTTP客户端，设置超时时间为30秒。
        let async_http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("构建HTTP客户端失败"); // 如果构建失败则panic

        // 初始化网易云音乐API客户端实例。
        let netease_api_client_instance = match netease_lyrics_fetcher::api::NeteaseClient::new() {
            Ok(client) => Some(client), // 初始化成功
            Err(e) => {
                log::error!("[Unilyric] 初始化网易云API客户端失败: {}", e);
                None // 初始化失败
            }
        };

        // 初始化 persistent_canonical_keys 集合和 MetadataStore。
        // persistent_canonical_keys 存储用户希望固定的元数据类型的规范化键。
        // MetadataStore 存储当前会话的元数据，会先被设置中的固定元数据填充。
        let mut initial_persistent_canonical_keys = HashSet::new();
        let mut initial_metadata_store = MetadataStore::new();

        // 从 app_settings.pinned_metadata (类型 HashMap<String, Vec<String>>) 初始化。
        // pinned_metadata 存储的是上次保存时UI上被固定的条目的显示键和对应的值列表。
        for (display_key, values_vec) in &settings.pinned_metadata {
            // 尝试将INI中存储的显示键解析为规范键 (CanonicalMetadataKey)
            match display_key.trim().parse::<CanonicalMetadataKey>() {
                Ok(canonical_key) => {
                    // 如果解析成功，将这个规范化的键添加到 persistent_canonical_keys 集合中，
                    // 这表示用户 *意图* 固定这种类型的元数据。
                    initial_persistent_canonical_keys.insert(canonical_key.clone());
                    // 将从设置中加载的固定元数据值添加到初始的 MetadataStore。
                    for v_str in values_vec {
                        // values_vec 是 &Vec<String>
                        if let Err(e) = initial_metadata_store.add(display_key, v_str.clone()) {
                            log::error!(
                                "[Unilyric] 从设置加载固定元数据 '{}' (值: '{}') 到Store失败: {}",
                                display_key,
                                v_str,
                                e
                            );
                        }
                    }
                }
                Err(_) => {
                    // 如果无法解析为标准键，则作为自定义(Custom)键处理。
                    let custom_key = CanonicalMetadataKey::Custom(display_key.trim().to_string());
                    initial_persistent_canonical_keys.insert(custom_key.clone());
                    for v_str in values_vec {
                        if let Err(e) = initial_metadata_store.add(display_key, v_str.clone()) {
                            log::error!(
                                "[Unilyric] 从设置加载固定自定义元数据 '{}' (值: '{}') 到Store失败: {}",
                                display_key,
                                v_str,
                                e
                            );
                        }
                    }
                }
            }
        }
        log::info!(
            "[Unilyric] 从设置加载了 {} 个固定元数据键的类型。",
            initial_persistent_canonical_keys.len()
        );
        log::info!(
            "[Unilyric] 已填充 {} 条来自设置的固定元数据到初始存储。",
            initial_metadata_store.iter_all().count()
        );

        // 初始化应用状态结构体 Self (UniLyricApp) 的所有字段。
        let mut app = Self {
            input_text: String::new(),                      // 输入框文本，初始为空
            output_text: String::new(),                     // 输出框文本，初始为空
            display_translation_lrc_output: String::new(),  // 翻译LRC预览，初始为空
            display_romanization_lrc_output: String::new(), // 罗马音LRC预览，初始为空
            source_format: LyricFormat::Ass,                // 默认源格式为ASS
            target_format: LyricFormat::Ttml,               // 默认目标格式为TTML
            available_formats: LyricFormat::all(),          // 获取所有支持的歌词格式列表
            last_opened_file_path: None,                    // 初始化时无已打开文件
            last_saved_file_path: None,                     // 初始化时无已保存文件
            conversion_in_progress: false,                  // 初始化时无转换任务
            parsed_ttml_paragraphs: None,                   // 初始化时无已解析的TTML段落
            metadata_store: Arc::new(Mutex::new(initial_metadata_store)), // 使用上面初始化的元数据存储
            editable_metadata: Vec::new(), // 可编辑元数据列表，初始为空 (稍后从store重建)
            persistent_canonical_keys: initial_persistent_canonical_keys, // 使用上面初始化的固定键集合
            current_markers: Vec::new(),                                  // 标记列表，初始为空
            source_is_line_timed: false, // 源是否逐行歌词，初始为false
            current_raw_ttml_from_input: None, // 原始TTML输入，初始为None
            show_bottom_log_panel: false, // 底部日志面板，初始隐藏
            new_trigger_log_exists: false, // 无新触发性日志
            is_any_file_hovering_window: false, // 无文件悬停
            last_known_pointer_pos_while_dragging: None, // 无拖放指针位置
            show_markers_panel: false,   // 标记点面板，初始隐藏
            show_romanization_lrc_panel: false, // 罗马音LRC面板，初始隐藏
            show_translation_lrc_panel: false, // 翻译LRC面板，初始隐藏
            wrap_text: true,             // 文本框默认自动换行
            qqmusic_query: String::new(), // QQ音乐查询词，初始为空
            download_state: Arc::new(Mutex::new(QqMusicDownloadState::Idle)), // QQ音乐下载状态，初始为空闲
            http_client: async_http_client, // 使用上面创建的HTTP客户端
            show_qqmusic_download_window: false, // QQ音乐下载窗口，初始隐藏
            kugou_query: String::new(),     // 酷狗查询词，初始为空
            kugou_download_state: Arc::new(Mutex::new(KrcDownloadState::Idle)), // 酷狗下载状态，初始为空闲
            show_kugou_download_window: false, // 酷狗下载窗口，初始隐藏
            pending_krc_translation_lines: None, // KRC内嵌翻译，初始为None
            netease_query: String::new(),      // 网易云查询词，初始为空
            netease_download_state: Arc::new(Mutex::new(NeteaseDownloadState::Idle)), // 网易云下载状态，初始为空闲
            show_netease_download_window: false, // 网易云下载窗口，初始隐藏
            netease_client: Arc::new(Mutex::new(netease_api_client_instance)), // 使用上面创建的网易云API客户端
            loaded_translation_lrc: None,  // 已加载翻译LRC，初始为None
            loaded_romanization_lrc: None, // 已加载罗马音LRC，初始为None
            show_metadata_panel: false,    // 元数据编辑面板，初始隐藏
            detected_formatted_ttml_source: false, // 是否检测到格式化TTML，初始为false
            app_settings: Arc::new(Mutex::new(settings.clone())), // 应用设置的Arc<Mutex>副本
            show_settings_window: false,   // 设置窗口，初始隐藏
            temp_edit_settings: settings,  // 用于设置窗口编辑的临时设置副本
            log_display_buffer: Vec::with_capacity(200), // 日志显示缓冲区，预分配容量
            session_platform_metadata: HashMap::new(), // 会话平台元数据，初始为空
            metadata_source_is_download: false, // 元数据是否来自下载，初始为false
            ui_log_receiver,               // 从参数传入的日志接收器
            pending_romanization_qrc_from_download: None, // 待处理罗马音QRC，初始为None
            pending_translation_lrc_from_download: None, // 待处理翻译LRC，初始为None
            pending_romanization_lrc_from_download: None, // 待处理罗马音LRC，初始为None
            direct_netease_main_lrc_content: None, // 网易云直接主LRC内容，初始为None
        };

        // 在所有字段初始化之后，根据初始的 MetadataStore (已包含固定项) 重建UI的可编辑元数据列表。
        app.rebuild_editable_metadata_from_store();

        app // 返回初始化完成的 UniLyricApp 实例
    }

    /// 将UI元数据编辑器中的当前状态同步回内部的 `MetadataStore`，
    /// 更新 `persistent_canonical_keys` 集合，并将固定的元数据保存到应用设置中，
    /// 最后触发目标格式歌词的重新生成。
    ///
    /// 这个函数通常在元数据编辑器中的内容发生更改并需要应用时调用，
    /// 例如用户点击“应用”按钮或关闭元数据编辑窗口时。
    pub fn sync_store_from_editable_list_and_trigger_conversion(&mut self) {
        // 限制 MutexGuard 的生命周期
        {
            // 获取 MetadataStore 的锁，以便进行修改
            let mut store = self.metadata_store.lock().unwrap();
            store.clear(); // 清空当前的 MetadataStore，准备从UI状态完全重建

            // --- 同步UI固定状态到内部状态并保存到设置 ---
            self.persistent_canonical_keys.clear(); // 清空当前的固定键集合，将根据UI重新填充
            // `current_pinned_for_settings` 用于收集当前在UI上被标记为“固定”的元数据，
            // 其键是规范化的显示键 (canonical_key.to_display_key())，值是原始用户输入的值列表。
            let mut current_pinned_for_settings: HashMap<String, Vec<String>> = HashMap::new();

            // 遍历UI元数据编辑器中的每一个条目 (self.editable_metadata)
            for entry_ui in &self.editable_metadata {
                // 只有当键名非空时才处理
                if !entry_ui.key.trim().is_empty() {
                    // 1. 将UI条目添加到 MetadataStore (无论它是否被固定)
                    //    MetadataStore 内部会处理键的规范化和多值存储。
                    if let Err(e) = store.add(&entry_ui.key, entry_ui.value.clone()) {
                        log::warn!(
                            "[Unilyric UI同步] 添加元数据 '{}' 到Store失败: {}",
                            entry_ui.key,
                            e
                        );
                    }

                    // 2. 如果此UI条目被用户标记为“固定”(is_pinned)
                    if entry_ui.is_pinned {
                        let value_to_pin = entry_ui.value.clone(); // 获取要固定的值

                        // 尝试将UI上显示的键名解析为其规范化的 CanonicalMetadataKey
                        match entry_ui.key.trim().parse::<CanonicalMetadataKey>() {
                            Ok(canonical_key) => {
                                // 解析成功 (是标准元数据类型)
                                // 获取该规范键对应的、用于存储到设置文件中的唯一显示键。
                                // 例如，即使用户输入 "Title" 或 "TITLE"，这里都应得到如 "Title"。
                                let key_for_settings = canonical_key.to_display_key();

                                // 将此固定项添加到 current_pinned_for_settings 中，
                                // 使用规范化的显示键作为 map 的键。
                                current_pinned_for_settings
                                    .entry(key_for_settings)
                                    .or_default() // 如果键不存在则插入默认值 (空Vec)
                                    .push(value_to_pin); // 添加值

                                // 将此规范键添加到 self.persistent_canonical_keys 集合中，
                                // 表示这种类型的元数据是用户希望固定的。
                                self.persistent_canonical_keys.insert(canonical_key);
                            }
                            Err(_) => {
                                // 解析失败 (是自定义元数据类型)
                                // 对于自定义键，直接使用用户在UI上输入的键名（去除首尾空格）。
                                let custom_key_for_settings = entry_ui.key.trim().to_string();
                                current_pinned_for_settings
                                    .entry(custom_key_for_settings.clone())
                                    .or_default()
                                    .push(value_to_pin);

                                // 将自定义键的规范形式 (CanonicalMetadataKey::Custom) 添加到固定键集合。
                                self.persistent_canonical_keys
                                    .insert(CanonicalMetadataKey::Custom(custom_key_for_settings));
                            }
                        }
                    } // 结束处理固定条目
                } // 结束处理非空键名条目
            } // 结束遍历 editable_metadata

            // --- 更新 AppSettings 并保存到INI文件 ---
            {
                // AppSettings 锁作用域开始
                let mut app_settings_locked = self.app_settings.lock().unwrap();
                // 将从UI收集到的、当前所有被固定的元数据更新到 app_settings 中。
                app_settings_locked.pinned_metadata = current_pinned_for_settings;
                // 保存更新后的设置到配置文件 (如 INI 文件)。
                if let Err(e) = app_settings_locked.save() {
                    log::error!("[Unilyric UI同步] 保存固定元数据到设置文件失败: {}", e);
                } else {
                    log::info!(
                        "[Unilyric UI同步] 已将 {} 个键的固定元数据保存到设置文件。",
                        app_settings_locked.pinned_metadata.len()
                    );
                }
            } // AppSettings 锁释放
        } // MetadataStore 锁作用域结束（store 在这里被丢弃，锁自动释放）

        log::info!(
            "[Unilyric UI同步] MetadataStore已从UI编辑器同步。固定键类型数量: {}. 总元数据条目数量: {}",
            self.persistent_canonical_keys.len(),
            self.metadata_store.lock().unwrap().iter_all().count()
        );
        log::trace!(
            "[Unilyric UI同步] 当前固定的元数据键类型列表: {:?}",
            self.persistent_canonical_keys
        );

        // --- 触发歌词转换 ---
        // 检查 MetadataStore 是否为空（即使没有歌词段落，仅有元数据也可能需要生成输出，例如LRC头部）
        let store_is_empty = self.metadata_store.lock().unwrap().is_empty();
        // 如果存在已解析的歌词段落 (self.parsed_ttml_paragraphs)，或者元数据存储不为空，
        // 则调用 generate_target_format_output() 来重新生成目标格式的歌词输出。
        if self.parsed_ttml_paragraphs.is_some() || !store_is_empty {
            self.generate_target_format_output(); // 生成目标格式的输出文本
        }
    }

    /// 根据当前的 `MetadataStore` 重建UI上显示的元数据编辑列表 (`self.editable_metadata`)。
    ///
    /// 此函数在以下情况被调用：
    /// 1. 应用初始化时 (`new` 函数末尾)。
    /// 2. 从文件加载或网络下载歌词并解析元数据后 (`update_app_state_from_parsed_data` 函数末尾)。
    /// 3. 用户在元数据编辑器中手动添加/删除条目后，或更改固定状态后，可能需要刷新列表以正确排序和显示状态。
    ///
    /// 主要逻辑：
    /// - 遍历 `self.metadata_store` 中的所有元数据项。
    /// - 为每个存储的元数据项创建一个 `EditableMetadataEntry` 对象。
    ///   - `key`: 使用 `CanonicalMetadataKey::to_display_key()` 获取规范的显示键名。
    ///   - `value`: 存储的值。
    ///   - `is_pinned`: 通过检查 `canonical_key_from_store` 是否存在于 `self.persistent_canonical_keys` 集合中来确定。
    ///     这确保了UI上的“固定”状态与内部的持久化意图一致。
    ///   - `is_from_file`: 初始标记为 `true`，表示这些条目是直接从当前的权威数据源（`MetadataStore`）加载的。
    ///   - `id`: 为每个条目生成一个唯一的 `egui::Id`，用于UI元素的追踪。
    /// - 对新生成的 `EditableMetadataEntry` 列表进行排序：
    ///   - 固定项 (`is_pinned == true`) 排在非固定项之前。
    ///   - 同为固定项或同为非固定项时，按键名（不区分大小写）的字母顺序排序。
    /// - 最后，用这个新生成的、排序后的列表替换 `self.editable_metadata`。
    pub fn rebuild_editable_metadata_from_store(&mut self) {
        // 获取 MetadataStore 的只读锁
        let store_guard = self.metadata_store.lock().unwrap();
        // 初始化一个新的可编辑元数据列表
        let mut new_editable_list: Vec<EditableMetadataEntry> = Vec::new();
        // 用于为 egui::Id 生成唯一后缀的计数器
        let mut id_seed_counter = 0;

        // 遍历 MetadataStore 中的所有元数据项。
        // store_guard.iter_all() 返回一个迭代器，其元素是 (CanonicalMetadataKey, &Vec<String>)
        // 即每个规范化的键及其对应的值列表。
        for (canonical_key_from_store, values_vec) in store_guard.iter_all() {
            // 获取该规范键对应的用户友好显示名称 (例如，CanonicalMetadataKey::Title -> "Title")
            let display_key_name = canonical_key_from_store.to_display_key();

            // MetadataStore 可能为一个键存储多个值，所以遍历值列表
            for value_str in values_vec {
                id_seed_counter += 1; // 增加ID种子，确保每个条目的ID唯一
                // 为UI条目创建一个唯一的egui ID，这对于egui正确处理用户交互（如文本框编辑）很重要。
                // ID基于显示键名和计数器生成，替换特殊字符以避免ID格式问题。
                let new_id = egui::Id::new(format!(
                    "editable_meta_{}_{}",
                    display_key_name.replace([':', '/', ' '], "_"), // 替换键名中的特殊字符
                    id_seed_counter
                ));

                // 创建一个新的 EditableMetadataEntry 实例
                new_editable_list.push(EditableMetadataEntry {
                    key: display_key_name.clone(), // UI上显示的键名
                    value: value_str.clone(),      // UI上显示/编辑的值
                    // 核心逻辑：判断此条目在UI上是否应显示为“固定”。
                    // 这是通过检查其规范键是否存在于 self.persistent_canonical_keys 集合中来决定的。
                    // self.persistent_canonical_keys 反映了用户当前希望哪些类型的元数据是固定的。
                    is_pinned: self
                        .persistent_canonical_keys
                        .contains(canonical_key_from_store),
                    is_from_file: true, // 标记此条目是直接从 MetadataStore 加载的
                    id: new_id,         // egui 用的唯一ID
                });
            }
        }

        // 对新构建的可编辑元数据列表进行排序。
        // 排序规则：
        // 1. “固定”的条目 (is_pinned == true) 排在前面。
        // 2. 在“固定”或“非固定”的组内，按键名 (key) 的字母顺序（不区分大小写）排序。
        new_editable_list.sort_by(|a, b| {
            match (a.is_pinned, b.is_pinned) {
                (true, false) => std::cmp::Ordering::Less, // a是固定的，b不是，则a排在前面
                (false, true) => std::cmp::Ordering::Greater, // a不是固定的，b是，则b排在前面
                _ => a.key.to_lowercase().cmp(&b.key.to_lowercase()), // 两者固定状态相同，按键名排序
            }
        });

        // 用新生成的、排序好的列表替换 self.editable_metadata
        self.editable_metadata = new_editable_list;
        log::info!(
            "[Unilyric] 已从内部存储重建UI元数据列表。共 {} 个条目。",
            self.editable_metadata.len()
        );
    }

    /// 触发QQ音乐歌词的下载流程。
    ///
    /// 此函数在用户输入查询词并点击“下载”按钮时被调用。
    /// 它会启动一个新的线程来执行异步的网络请求和歌词解析，以避免阻塞UI。
    pub fn trigger_qqmusic_download(&mut self) {
        // 获取用户输入的查询词，并去除首尾空白。
        let query = self.qqmusic_query.trim().to_string();
        // 如果查询词为空，则记录错误并提前返回。
        if query.is_empty() {
            log::error!("[Unilyric] QQ音乐下载：请输入有效的搜索内容。");
            // 如果当前状态是下载中，也将其重置为空闲，以避免UI卡在下载状态。
            let mut download_status_locked = self.download_state.lock().unwrap();
            if matches!(*download_status_locked, QqMusicDownloadState::Downloading) {
                *download_status_locked = QqMusicDownloadState::Idle;
            }
            return;
        }

        // 更新下载状态为 "Downloading"。
        // 使用代码块限制 MutexGuard 的生命周期。
        {
            let mut download_status_locked = self.download_state.lock().unwrap();
            *download_status_locked = QqMusicDownloadState::Downloading;
        }

        // 克隆需要在新线程中使用的共享状态和HTTP客户端。
        // Arc (原子引用计数) 使得这些资源可以被安全地跨线程共享。
        let state_clone = Arc::clone(&self.download_state); // 下载状态的Arc副本
        let client_clone = self.http_client.clone(); // HTTP客户端的副本

        // 启动一个新的系统线程来执行耗时的网络操作，避免阻塞UI线程。
        std::thread::spawn(move || {
            // 在新线程中创建一个Tokio运行时，用于执行异步代码。
            // `Builder::new_current_thread()` 创建一个单线程运行时。
            // `enable_all()` 启用所有Tokio特性（如IO, time）。
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r, // 运行时创建成功
                Err(e) => {
                    // 运行时创建失败
                    log::error!("[Unilyric] QQ音乐下载：创建Tokio运行时失败: {}", e);
                    // 更新下载状态为错误状态。
                    let mut status_lock = state_clone.lock().unwrap();
                    *status_lock =
                        QqMusicDownloadState::Error(format!("创建异步运行时失败: {}", e));
                    return; // 线程结束
                }
            };

            // 使用 `rt.block_on` 在当前线程（即新创建的这个std::thread）上阻塞式地运行异步代码块。
            rt.block_on(async {
                log::info!("[Unilyric] QQ音乐下载：正在获取: '{}'", query);
                // 调用实际的歌词下载和解析函数。
                // `download_lyrics_by_query_first_match` 是一个异步函数。
                match qq_lyrics_fetcher::qqlyricsfetcher::download_lyrics_by_query_first_match(
                    &client_clone, // 传入HTTP客户端引用
                    &query,        // 传入查询词引用
                )
                .await // 等待异步操作完成
                {
                    Ok(data) => { // 下载和解析成功
                        info!(
                            "[Unilyric] 下载成功： {} - {}",
                            data.song_name.as_deref().unwrap_or("未知歌名"), 
                            data.artists_name.join("/") // 将歌手名列表用 "/" 连接
                        );
                        // 更新下载状态为成功，并附带获取到的歌词数据。
                        let mut status_lock = state_clone.lock().unwrap();
                        *status_lock = QqMusicDownloadState::Success(data);
                    }
                    Err(e) => { // 下载或解析过程中发生错误
                        log::error!("[Unilyric] QQ音乐歌词下载失败: {}", e);
                        // 更新下载状态为错误，并附带错误信息。
                        let mut status_lock = state_clone.lock().unwrap();
                        *status_lock = QqMusicDownloadState::Error(e.to_string());
                    }
                }
            }); // Tokio运行时 block_on 结束
        }); // 新线程结束
    }

    /// 触发网易云音乐歌词的下载流程。
    pub fn trigger_netease_download(&mut self) {
        let query = self.netease_query.trim().to_string();
        if query.is_empty() {
            log::error!("[Unilyric] 网易云音乐下载：查询内容为空，无法开始下载。");
            let mut ds_lock = self.netease_download_state.lock().unwrap();
            *ds_lock = NeteaseDownloadState::Idle; // 重置状态
            return;
        }

        let download_state_clone = Arc::clone(&self.netease_download_state);
        let client_mutex_arc_clone = Arc::clone(&self.netease_client);

        // 更新下载状态：如果客户端未初始化，则为InitializingClient，否则为Downloading
        {
            let mut ds_lock = download_state_clone.lock().unwrap();
            let client_guard = client_mutex_arc_clone.lock().unwrap();
            if client_guard.is_none() {
                *ds_lock = NeteaseDownloadState::InitializingClient;
            } else {
                *ds_lock = NeteaseDownloadState::Downloading;
            }
        }

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    log::error!("[Unilyric 网易云下载线程] 创建Tokio运行时失败: {}", e);
                    let mut status_lock = download_state_clone.lock().unwrap();
                    *status_lock =
                        NeteaseDownloadState::Error(format!("创建异步运行时失败: {}", e));
                    return;
                }
            };

            rt.block_on(async move {
                let maybe_client_instance: Option<netease_lyrics_fetcher::api::NeteaseClient>;
                // 确保客户端已初始化
                {
                    let mut client_option_guard = client_mutex_arc_clone.lock().unwrap();
                    if client_option_guard.is_none() {
                        match netease_lyrics_fetcher::api::NeteaseClient::new() {
                            Ok(new_client) => {
                                *client_option_guard = Some(new_client);
                            }
                            Err(e) => {
                                let mut status_lock = download_state_clone.lock().unwrap();
                                *status_lock =
                                    NeteaseDownloadState::Error(format!("客户端初始化失败: {}", e));
                                return;
                            }
                        }
                    }
                    // 克隆客户端实例以在异步块中使用
                    maybe_client_instance = (*client_option_guard).clone();
                }

                if let Some(netease_api_client) = maybe_client_instance {
                    // 再次检查并设置下载状态为Downloading (如果之前是InitializingClient)
                    {
                        let mut ds_lock = download_state_clone.lock().unwrap();
                        if matches!(*ds_lock, NeteaseDownloadState::InitializingClient) {
                            *ds_lock = NeteaseDownloadState::Downloading;
                        }
                    }

                    match netease_lyrics_fetcher::search_and_fetch_first_netease_lyrics(
                        &netease_api_client,
                        &query,
                    )
                    .await
                    {
                        Ok(data) => {
                            log::info!(
                                "[Unilyric] 网易云音乐下载成功：已获取 {} - {}",
                                data.song_name.as_deref().unwrap_or("未知歌名"),
                                data.artists_name.join("/")
                            );
                            let mut status_lock = download_state_clone.lock().unwrap();
                            *status_lock = NeteaseDownloadState::Success(data);
                        }
                        Err(e) => {
                            log::error!("[Unilyric] 网易云歌词下载失败: {}", e);
                            let mut status_lock = download_state_clone.lock().unwrap();
                            *status_lock = NeteaseDownloadState::Error(e.to_string());
                        }
                    }
                } else {
                    log::error!("[Unilyric] 出现了一个意外的错误");
                    let mut status_lock = download_state_clone.lock().unwrap();
                    *status_lock = NeteaseDownloadState::Error("客户端创建失败".to_string());
                }
            });
        });
    }

    /// 辅助方法：将LRC格式的次要歌词内容逐行合并到主歌词段落中。
    /// 适用于主歌词是YRC（或其他逐行格式，如LRC），而次要歌词是LRC的情况。
    /// 这种合并方式不依赖时间戳匹配，而是简单地将LRC的第N行赋给主歌词的第N个段落。
    ///
    /// # Arguments
    /// * `primary_paragraphs` - 可变的主歌词段落列表 (`Vec<TtmlParagraph>`)。
    /// * `lrc_content_str` - 包含次要歌词的完整LRC文本字符串。
    ///   对于来自QQ音乐的翻译，此字符串应已通过 `preprocess_qq_translation_lrc_content` 处理。
    /// * `content_type` - 指示LRC内容是翻译还是罗马音。
    /// * `language_code` - 可选的语言代码 (主要用于翻译)。
    ///
    /// # Returns
    /// `Result<(), ConvertError>` - 如果解析LRC内容时发生错误，则返回Err。
    fn merge_lrc_content_line_by_line_with_primary_paragraphs(
        primary_paragraphs: &mut [TtmlParagraph],
        lrc_content_str: &str,
        content_type: LrcContentType,
        language_code: Option<String>,
    ) -> Result<(), ConvertError> {
        if lrc_content_str.is_empty() || primary_paragraphs.is_empty() {
            return Ok(());
        }

        // 解析LRC字符串为LrcLine列表
        let (lrc_lines, _parsed_lrc_meta) = // _parsed_lrc_meta 在此函数中暂不使用
            match lrc_parser::parse_lrc_text_to_lines(lrc_content_str) {
                Ok(result) => result,
                Err(e) => {
                    log::error!("[Unilyric] 逐行合并时解析LRC内容失败: {}", e);
                    return Err(e); // 将解析错误向上传播
                }
            };

        if lrc_lines.is_empty() {
            return Ok(());
        }

        // 逐行合并：将LRC的第N行文本赋给主歌词的第N个段落
        for (para_idx, primary_para) in primary_paragraphs.iter_mut().enumerate() {
            if let Some(lrc_line) = lrc_lines.get(para_idx) {
                // 获取LRC行的文本。如果来自QQ音乐的翻译且原为"//"，
                // preprocess_qq_translation_lrc_content 已将其文本变为空字符串。
                // lrc_parser 解析后，LrcLine.text 会是这个空字符串。
                let text_to_set = lrc_line.text.clone(); // 直接使用LrcLine的文本

                match content_type {
                    LrcContentType::Romanization => {
                        primary_para.romanization = Some(text_to_set);
                    }
                    LrcContentType::Translation => {
                        primary_para.translation = Some((text_to_set, language_code.clone()));
                    }
                }
            } else {
                // 如果LRC的行数少于主歌词段落数，则后续主段落没有对应的次要歌词
                log::warn!(
                    "[Unilyric] 逐行合并：LRC行数 ({}) 少于主歌词段落数 ({})，段落 #{} 及之后无匹配。",
                    lrc_lines.len(),
                    primary_paragraphs.len(),
                    para_idx
                );
                break; // 停止合并，因为LRC行已用尽
            }
        }
        Ok(())
    }

    /// 合并从网络下载获取的次要歌词（如翻译LRC、罗马音QRC/LRC）到主歌词段落中。
    /// 此函数处理不同主歌词格式（如YRC vs 非YRC）与不同次要歌词格式的合并策略。
    fn merge_downloaded_secondary_lyrics(&mut self) {
        // 检查主歌词段落是否存在且非空
        let primary_paragraphs_are_empty_or_none = self
            .parsed_ttml_paragraphs
            .as_ref()
            .is_none_or(|p| p.is_empty());
        // 检查主歌词格式是否为YRC或LRC (当网易云只有LRC时)
        // 注意：如果网易云下载的是LRC作为主歌词，source_format 会被设为 LyricFormat::Lrc
        let main_lyrics_format_is_line_oriented =
            matches!(self.source_format, LyricFormat::Yrc | LyricFormat::Lrc);

        // --- 处理翻译 ---
        if let Some(trans_lrc_content_str) = self.pending_translation_lrc_from_download.take() {
            if primary_paragraphs_are_empty_or_none {
                // 主段落为空，尝试独立加载LRC翻译
                match crate::lrc_parser::parse_lrc_text_to_lines(&trans_lrc_content_str) {
                    Ok((lines, _meta)) => {
                        if !lines.is_empty() {
                            self.loaded_translation_lrc = Some(lines);
                        }
                        log::info!(
                            "[Unilyric] 主段落为空，独立加载了翻译LRC ({}行)。",
                            self.loaded_translation_lrc.as_ref().map_or(0, |v| v.len())
                        );
                    }
                    Err(e) => log::error!("[Unilyric] 主段落为空时，解析独立翻译LRC失败: {}", e),
                }
            } else if let Some(ref mut primary_paragraphs) = self.parsed_ttml_paragraphs {
                // 确定用于合并的语言代码
                let lang_code_for_merge: Option<String> = self
                    .session_platform_metadata
                    .get("language") // 首先尝试会话元数据
                    .cloned()
                    .or_else(|| {
                        // 然后尝试全局元数据存储
                        self.metadata_store
                            .lock()
                            .unwrap()
                            .get_single_value(&CanonicalMetadataKey::Language)
                            .cloned()
                    });

                if main_lyrics_format_is_line_oriented {
                    // 主歌词是YRC或LRC，辅助歌词是LRC字符串 -> 使用逐行LRC合并逻辑
                    log::info!("[Unilyric] 正在逐行合并LRC格式的翻译...");
                    if let Err(e) = Self::merge_lrc_content_line_by_line_with_primary_paragraphs(
                        primary_paragraphs,
                        &trans_lrc_content_str,
                        LrcContentType::Translation,
                        lang_code_for_merge,
                    ) {
                        error!("[Unilyric] 逐行合并LRC翻译到主歌词失败: {}", e);
                    }
                } else {
                    // 主歌词不是YRC/LRC -> 使用基于时间戳的LRC合并逻辑
                    log::info!("[Unilyric] 主歌词非逐行格式，正在按时间戳合并下载的LRC翻译...");
                    if let Err(e) = Self::merge_lrc_lines_into_paragraphs_internal(
                        primary_paragraphs,
                        &trans_lrc_content_str,
                        LrcContentType::Translation,
                        lang_code_for_merge,
                    ) {
                        error!("[Unilyric] 按时间戳合并下载的LRC翻译失败: {}", e);
                    }
                }
            }
        }

        // --- 处理罗马音 ---
        // 首先处理来自QQ音乐的QRC格式罗马音 (如果存在)
        if let Some(roma_qrc_content_str) = self.pending_romanization_qrc_from_download.take() {
            if primary_paragraphs_are_empty_or_none {
            } else if let Some(ref mut primary_paragraphs) = self.parsed_ttml_paragraphs {
                log::info!("[Unilyric] 正在按时间戳合并下载的QRC罗马音...");
                if let Err(e) = Self::merge_secondary_qrc_into_paragraphs_internal(
                    primary_paragraphs,
                    &roma_qrc_content_str,
                    LrcContentType::Romanization,
                ) {
                    error!("[Unilyric] 合并下载的QRC罗马音失败: {}", e);
                }
            }
        }
        // 然后处理来自网易云等平台的LRC格式罗马音 (如果存在且未被QRC罗马音覆盖)
        else if let Some(roma_lrc_content_str) =
            self.pending_romanization_lrc_from_download.take()
        {
            if primary_paragraphs_are_empty_or_none {
                match crate::lrc_parser::parse_lrc_text_to_lines(&roma_lrc_content_str) {
                    Ok((lines, _meta)) => {
                        if !lines.is_empty() {
                            self.loaded_romanization_lrc = Some(lines);
                        }
                        log::info!(
                            "[Unilyric] 主段落为空，独立加载了罗马音LRC ({}行)。",
                            self.loaded_romanization_lrc.as_ref().map_or(0, |v| v.len())
                        );
                    }
                    Err(e) => log::error!("[Unilyric] 主段落为空时，解析独立罗马音LRC失败: {}", e),
                }
            } else if let Some(ref mut primary_paragraphs) = self.parsed_ttml_paragraphs {
                if main_lyrics_format_is_line_oriented {
                    // 主歌词是YRC或LRC，辅助歌词是LRC字符串 -> 使用逐行LRC合并逻辑
                    log::info!("[Unilyric] 正在逐行合并LRC格式的罗马音...");
                    if let Err(e) = Self::merge_lrc_content_line_by_line_with_primary_paragraphs(
                        primary_paragraphs,
                        &roma_lrc_content_str,
                        LrcContentType::Romanization,
                        None, // 罗马音通常无特定语言代码
                    ) {
                        error!("[Unilyric] 逐行合并LRC罗马音到主歌词失败: {}", e);
                    }
                } else {
                    // 主歌词不是YRC/LRC -> 使用基于时间戳的LRC合并逻辑
                    log::info!("[Unilyric] 主歌词非逐行格式，正在按时间戳合并下载的LRC罗马音...");
                    if let Err(e) = Self::merge_lrc_lines_into_paragraphs_internal(
                        primary_paragraphs,
                        &roma_lrc_content_str,
                        LrcContentType::Romanization,
                        None,
                    ) {
                        error!("[Unilyric] 按时间戳合并下载的LRC罗马音失败: {}", e);
                    }
                }
            }
        }

        // KRC内嵌翻译的处理逻辑
        if let Some(trans_lines) = self.pending_krc_translation_lines.take() {
            if let Some(ref mut paragraphs) = self.parsed_ttml_paragraphs {
                if !paragraphs.is_empty() && !trans_lines.is_empty() {
                    log::info!(
                        "[Unilyric] 正在应用KRC内嵌翻译 (共 {} 行翻译到 {} 个段落)",
                        trans_lines.len(),
                        paragraphs.len()
                    );
                    for (i, para_line) in paragraphs.iter_mut().enumerate() {
                        if let Some(trans_text) = trans_lines.get(i) {
                            let text_to_use = if trans_text == " " {
                                ""
                            } else {
                                trans_text.as_str()
                            };
                            // 只有当段落尚无翻译，或翻译为空时，才使用KRC内嵌翻译填充
                            if para_line.translation.is_none()
                                || para_line
                                    .translation
                                    .as_ref()
                                    .is_some_and(|(t, _)| t.is_empty())
                            {
                                para_line.translation = Some((text_to_use.to_string(), None)); // KRC内嵌翻译通常无明确语言代码
                            }
                        }
                    }
                }
            } else {
                log::warn!(
                    "[Unilyric] KRC内嵌翻译存在，但无主歌词段落可合并。暂存的翻译将被丢弃。"
                );
            }
        }
    }

    /// 处理QQ音乐歌词下载完成后的逻辑。
    /// 包括清理旧数据、设置新歌词内容、处理元数据、暂存次要歌词，并触发转换。
    fn handle_qq_download_completion(&mut self) {
        let mut fetched_lyrics_to_process: Option<
            crate::qq_lyrics_fetcher::qqlyricsfetcher::FetchedQqLyrics,
        > = None;
        let mut error_to_report: Option<String> = None;
        let mut should_close_window = false; // 标志，用于决定是否关闭下载窗口

        // 检查下载状态，获取数据或错误信息
        {
            // 锁的作用域开始
            let mut download_status_locked = self.download_state.lock().unwrap();
            match &*download_status_locked {
                QqMusicDownloadState::Success(data) => {
                    fetched_lyrics_to_process = Some(data.clone());
                    should_close_window = true; // 下载成功，准备关闭窗口
                }
                QqMusicDownloadState::Error(msg) => {
                    error_to_report = Some(msg.clone());
                    should_close_window = true; // 下载错误，准备关闭窗口
                }
                _ => {} // Downloading 或 Idle 状态，不处理
            }
            // 只有在处理完 Success 或 Error 后才重置状态为 Idle
            if should_close_window {
                *download_status_locked = QqMusicDownloadState::Idle;
            }
        } // 锁的作用域结束

        if let Some(fetched_data) = fetched_lyrics_to_process {
            self.clear_all_data(); // 清理之前的所有数据
            self.metadata_source_is_download = true; // 标记元数据来源于网络下载

            // 填充会话相关的平台元数据
            self.session_platform_metadata.clear();
            if let Some(s_name) = fetched_data.song_name.as_ref().filter(|s| !s.is_empty()) {
                self.session_platform_metadata
                    .insert("musicName".to_string(), s_name.clone());
            }
            if let Some(a_name) = fetched_data.album_name.as_ref().filter(|s| !s.is_empty()) {
                self.session_platform_metadata
                    .insert("album".to_string(), a_name.clone());
            }
            if let Some(s_id) = fetched_data.song_id.as_ref().filter(|s| !s.is_empty()) {
                self.session_platform_metadata
                    .insert("qqMusicId".to_string(), s_id.clone());
            }
            // 设置主歌词内容
            match &fetched_data.main_lyrics_qrc {
                Some(qrc_content) if !qrc_content.trim().is_empty() => {
                    self.input_text = qrc_content.clone();
                    self.source_format = LyricFormat::Qrc; // 源格式设为QRC
                    log::info!("[Unilyric] QQ音乐：已加载QRC歌词。");
                }
                _ => {
                    log::error!("[Unilyric] QQ音乐：未找到有效的QRC歌词。");
                    self.input_text.clear(); // 清空输入框
                }
            }
            // 预处理并暂存翻译LRC (移除 "//" 行)
            let processed_translation_lrc = fetched_data
                .translation_lrc
                .filter(|s| !s.trim().is_empty()) // 确保翻译LRC非空
                .map(Self::preprocess_qq_translation_lrc_content); // 调用预处理函数

            self.pending_translation_lrc_from_download = processed_translation_lrc;
            // 暂存罗马音QRC
            self.pending_romanization_qrc_from_download = fetched_data
                .romanization_qrc
                .filter(|s| !s.trim().is_empty());

            // 清除之前通过文件加载的次要LRC，因为现在要用下载的
            self.loaded_translation_lrc = None;
            self.loaded_romanization_lrc = None;

            self.handle_convert(); // 触发歌词转换和后续处理流程
        } else if let Some(err_msg) = error_to_report {
            log::error!("[Unilyric] QQ音乐下载失败: {}", err_msg);
        }

        // 在所有处理完成后，根据标志关闭窗口
        if should_close_window {
            self.show_qqmusic_download_window = false; // 关闭下载窗口
        }
    }

    /// 处理酷狗音乐KRC歌词下载完成后的逻辑。
    fn handle_kugou_download_completion(&mut self) {
        let mut fetched_krc_to_process: Option<crate::kugou_lyrics_fetcher::FetchedKrcLyrics> =
            None;
        let mut error_to_report: Option<String> = None;
        let mut should_close_window = false; // 标志，用于决定是否关闭下载窗口

        // 检查下载状态
        {
            let mut download_status_locked = self.kugou_download_state.lock().unwrap();
            match &*download_status_locked {
                KrcDownloadState::Success(data) => {
                    fetched_krc_to_process = Some(data.clone());
                    should_close_window = true;
                }
                KrcDownloadState::Error(msg) => {
                    error_to_report = Some(msg.clone());
                    should_close_window = true;
                }
                _ => {} // Downloading 或 Idle
            }
            if should_close_window {
                *download_status_locked = KrcDownloadState::Idle; // 重置状态
            }
        }

        if let Some(fetched_data) = fetched_krc_to_process {
            self.clear_all_data(); // 清理旧数据
            self.metadata_source_is_download = true; // 标记元数据来源

            // 填充会话平台元数据
            self.session_platform_metadata.clear();
            if let Some(s_name) = fetched_data.song_name.as_ref().filter(|s| !s.is_empty()) {
                self.session_platform_metadata
                    .insert("musicName".to_string(), s_name.clone());
            }
            if let Some(a_name) = fetched_data.album_name.as_ref().filter(|s| !s.is_empty()) {
                self.session_platform_metadata
                    .insert("album".to_string(), a_name.clone());
            }
            // 将KRC内嵌元数据也放入session_platform_metadata，让 update_app_state_from_parsed_data 统一处理优先级
            for meta_item in &fetched_data.krc_embedded_metadata {
                self.session_platform_metadata
                    .insert(meta_item.key.clone(), meta_item.value.clone());
            }

            self.input_text = fetched_data.krc_content; // 加载KRC内容到输入框
            self.source_format = LyricFormat::Krc; // 源格式设为KRC
            log::info!("[Unilyric] 酷狗音乐：已加载KRC歌词");

            // 暂存KRC内嵌的翻译行 (如果有)
            if let Some(translation_lines) = fetched_data.translation_lines {
                if !translation_lines.is_empty() {
                    self.pending_krc_translation_lines = Some(translation_lines);
                }
            }
            self.handle_convert(); // 触发转换流程
        } else if let Some(err_msg) = error_to_report {
            log::error!("[Unilyric] 酷狗歌词下载失败: {}", err_msg);
        }

        if should_close_window {
            self.show_kugou_download_window = false; // 关闭下载窗口
        }
    }

    /// 处理网易云音乐歌词下载完成后的逻辑。
    fn handle_netease_download_completion(&mut self) {
        let mut fetched_data_to_process: Option<
            crate::netease_lyrics_fetcher::FetchedNeteaseLyrics,
        > = None;
        let mut error_to_report: Option<String> = None;
        let mut should_close_window = false; // 标志，用于决定是否关闭下载窗口

        // 检查下载状态
        {
            let mut download_status_locked = self.netease_download_state.lock().unwrap();
            match &*download_status_locked {
                NeteaseDownloadState::Success(data) => {
                    fetched_data_to_process = Some(data.clone());
                    should_close_window = true;
                }
                NeteaseDownloadState::Error(msg) => {
                    error_to_report = Some(msg.clone());
                    should_close_window = true;
                }
                _ => {} // InitializingClient, Downloading 或 Idle
            }
            if should_close_window {
                *download_status_locked = NeteaseDownloadState::Idle; // 重置状态
            }
        }

        if let Some(fetched_data) = fetched_data_to_process {
            self.clear_all_data(); // 清理旧数据
            self.metadata_source_is_download = true; // 标记元数据来源
            self.session_platform_metadata.clear(); // 清理会话元数据

            // 填充会话平台元数据 (歌曲名、歌手、专辑、歌曲ID)
            if let Some(s_name) = fetched_data.song_name.as_ref().filter(|s| !s.is_empty()) {
                self.session_platform_metadata
                    .insert("musicName".to_string(), s_name.clone());
            }
            if let Some(a_name) = fetched_data.album_name.as_ref().filter(|s| !s.is_empty()) {
                self.session_platform_metadata
                    .insert("album".to_string(), a_name.clone());
            }
            if let Some(s_id) = fetched_data.song_id.as_ref().filter(|s| !s.is_empty()) {
                self.session_platform_metadata
                    .insert("ncmMusicId".to_string(), s_id.clone());
            }

            let mut main_lyric_content_set = false; // 标记主歌词是否已设置

            // 优先处理主歌词的 YRC/KLyric 内容 (来自 fetched_data.karaoke_lrc)
            if let Some(karaoke_text) = fetched_data
                .karaoke_lrc
                .as_ref()
                .filter(|s| !s.trim().is_empty())
            {
                self.input_text = karaoke_text.clone();
                self.source_format = LyricFormat::Yrc; // 源格式设为YRC
                main_lyric_content_set = true;
            }

            // 如果主歌词不是YRC，则尝试使用LRC
            if !main_lyric_content_set {
                if let Some(lrc_text) = fetched_data
                    .main_lrc
                    .as_ref()
                    .filter(|s| !s.trim().is_empty())
                {
                    self.input_text = lrc_text.clone();
                    self.source_format = LyricFormat::Lrc; // 源格式设为LRC
                    main_lyric_content_set = true;
                    // 如果这是主歌词，也存入 direct_netease_main_lrc_content 以便LQE使用
                    self.direct_netease_main_lrc_content = Some(lrc_text.clone());
                }
            }

            if !main_lyric_content_set {
                log::error!(
                    "[Unilyric] 网易云音乐歌词下载成功，但未找到有效的逐字 (YRC) 或逐行 (LRC) 主歌词内容。"
                );
                self.input_text.clear(); // 清空输入框
            }

            // 暂存翻译LRC
            self.pending_translation_lrc_from_download = fetched_data
                .translation_lrc
                .filter(|s| !s.trim().is_empty());
            if self.pending_translation_lrc_from_download.is_some() {
                log::info!("[Unilyric] 网易云音乐：已暂存翻译 (LRC格式)。");
            }

            // 暂存罗马音LRC
            self.pending_romanization_lrc_from_download = fetched_data
                .romanization_lrc
                .filter(|s| !s.trim().is_empty());
            if self.pending_romanization_lrc_from_download.is_some() {
                log::info!("[Unilyric] 网易云音乐：已暂存罗马音 (LRC格式)。");
            }

            // 清除之前通过“文件”菜单加载的LRC，因为现在要用下载的
            self.loaded_translation_lrc = None;
            self.loaded_romanization_lrc = None;

            // 目标格式自动切换逻辑: 如果源是LRC (例如网易云只提供了LRC主歌词)，
            if self.source_format == LyricFormat::Lrc
                && !matches!(
                    self.target_format,
                    LyricFormat::Lqe | LyricFormat::Spl | LyricFormat::Lrc
                )
            {
                log::info!(
                    "[Unilyric] 网易云下载：源为主LRC，目标格式从 {:?} 自动切换为LQE。",
                    self.target_format
                );
                self.target_format = LyricFormat::Lqe;
            }

            self.handle_convert(); // 触发转换和合并流程
        } else if let Some(err_msg) = error_to_report {
            log::error!("[Unilyric] 网易云音乐歌词下载失败: {}", err_msg);
        }

        if should_close_window {
            self.show_netease_download_window = false; // 关闭下载窗口
            log::debug!("[Unilyric] 网易云下载窗口已关闭 (因下载完成或错误)。");
        }
    }

    /// 清理所有派生数据，例如输出文本、解析后的段落、标记等。
    /// 通常在加载新输入前或清除所有数据时调用。
    pub fn clear_derived_data(&mut self) {
        log::info!("[Unilyric] 正在清理输出文本、已解析段落、标记等...");
        self.output_text.clear(); // 清空主输出框
        self.display_translation_lrc_output.clear(); // 清空翻译LRC预览
        self.display_romanization_lrc_output.clear(); // 清空罗马音LRC预览
        self.parsed_ttml_paragraphs = None; // 清除已解析的TTML段落
        self.current_markers.clear(); // 清除当前标记
        self.source_is_line_timed = false; // 重置是否逐行歌词的标志
        self.current_raw_ttml_from_input = None; // 清除原始TTML输入缓存
        // 注意： self.metadata_store 和 self.editable_metadata 不在此处清理，
        // 它们有独立的管理逻辑，尤其是在 clear_all_data 中。
    }

    /// 清理应用中的所有数据，包括输入、输出、已解析内容、元数据（除用户固定的）、待处理下载等。
    /// 目的是将应用恢复到一个相对干净的状态，准备加载新内容。
    pub fn clear_all_data(&mut self) {
        log::info!("[Unilyric] 正在清理所有数据");
        self.input_text.clear(); // 清空输入框文本
        self.clear_derived_data(); // 调用派生数据清理

        // 清理待处理的KRC内嵌翻译和从文件加载的LRC
        self.pending_krc_translation_lines = None;
        self.loaded_translation_lrc = None;
        self.loaded_romanization_lrc = None;

        // 清理从网络下载待合并的次要歌词内容
        self.pending_translation_lrc_from_download = None;
        self.pending_romanization_qrc_from_download = None;
        self.pending_romanization_lrc_from_download = None;
        self.direct_netease_main_lrc_content = None; // 清理网易云直接LRC内容

        // 清理会话平台元数据和下载来源标记
        self.session_platform_metadata.clear();
        self.metadata_source_is_download = false;

        // --- 元数据存储和UI编辑列表的特殊处理 ---
        {
            // MetadataStore 作用域开始
            let mut store = self.metadata_store.lock().unwrap();
            store.clear(); // 1. 完全清空内部的 MetadataStore

            // 2. 从应用设置 (app_settings.pinned_metadata) 中重新加载用户标记为“固定”的元数据到 store。
            //    这些是用户上次通过UI操作并保存到INI文件的固定项。
            //    这一步确保即使用户清除了当前加载文件的元数据，他们希望持久化的固定项仍然保留在store中。
            let app_settings_locked = self.app_settings.lock().unwrap();
            for (display_key_from_settings, values_vec_from_settings) in
                &app_settings_locked.pinned_metadata
            {
                // 关键检查：确保这个从设置中读取的固定项的键，确实也存在于 `self.persistent_canonical_keys` 中。
                // `self.persistent_canonical_keys` 反映了当前UI上用户实际希望固定的键类型。
                // 这可以防止旧的、在设置中但用户已在UI取消固定的项被错误地重新加载。
                let canonical_key_to_check = match display_key_from_settings
                    .trim()
                    .parse::<CanonicalMetadataKey>()
                {
                    Ok(ck) => ck,
                    Err(_) => {
                        CanonicalMetadataKey::Custom(display_key_from_settings.trim().to_string())
                    }
                };

                if self
                    .persistent_canonical_keys
                    .contains(&canonical_key_to_check)
                {
                    for v_str in values_vec_from_settings {
                        if let Err(e) = store.add(display_key_from_settings, v_str.clone()) {
                            log::warn!(
                                "[Unilyric 清理所有数据] 从设置重载固定元数据 '{}' (值: '{}') 到Store失败: {}",
                                display_key_from_settings,
                                v_str,
                                e
                            );
                        }
                    }
                }
            }
        } // MetadataStore 锁释放

        // 3. 更新UI的可编辑元数据列表 (self.editable_metadata):
        //    - 只保留那些在UI上被标记为 is_pinned 的条目。
        //    - 将这些保留条目的 is_from_file 标记为 false，因为它们现在不代表文件内容，而是用户持久化的选择。
        //    - 清空 self.persistent_canonical_keys，然后根据清理后 editable_metadata 中剩余的固定项重新构建它。
        //      (或者，更简单的方式是，在上面从设置加载到store后，直接调用 rebuild_editable_metadata_from_store，
        //       它会基于store和现有的 persistent_canonical_keys 重建UI列表。当前代码是先操作UI列表再同步store/keys)

        // 当前代码逻辑：保留UI上的固定项，然后用它们重建 persistent_canonical_keys，并确保它们在 store 中。
        self.editable_metadata.retain(|entry| entry.is_pinned); // 只保留UI上已固定的
        for entry in self.editable_metadata.iter_mut() {
            entry.is_from_file = false; // 这些不再代表文件内容
            // 确保这些UI上的固定项也存在于（空的）store中，但这似乎与上面从设置加载到store的逻辑有些重叠或冲突。
            // 推荐做法：在上面用设置填充store后，调用 rebuild_editable_metadata_from_store() 来统一UI和store。
            // 为了保持原逻辑，这里暂时保留：
            if let Err(e) = self
                .metadata_store
                .lock()
                .unwrap()
                .add(&entry.key, entry.value.clone())
            {
                log::warn!(
                    "[Unilyric] 将UI固定元数据 '{}' 添加回 store 失败: {}",
                    entry.key,
                    e
                );
            }
        }
        // 重建 persistent_canonical_keys 集合，使其与清理后UI上仍然存在的固定项一致。
        self.persistent_canonical_keys.clear();
        for entry in &self.editable_metadata {
            // 此时 editable_metadata 只包含 is_pinned=true 的项
            if let Ok(canonical_key) = entry.key.trim().parse::<CanonicalMetadataKey>() {
                self.persistent_canonical_keys.insert(canonical_key);
            } else {
                // 自定义键
                self.persistent_canonical_keys
                    .insert(CanonicalMetadataKey::Custom(entry.key.trim().to_string()));
            }
        }

        // 最后，基于可能已更新的 store 和 persistent_canonical_keys，重建UI元数据列表以确保一致性。
        self.rebuild_editable_metadata_from_store();
        log::info!(
            "[Unilyric] 已清除输入、输出和大部分已加载数据。元数据已重置（仅保留用户固定的项）。"
        );
    }

    /// 预处理LRC内容字符串，主要用于处理QQ音乐下载的翻译LRC。
    /// 将文本内容为 "//" 的LRC行转换为空文本行（只保留时间戳部分）。
    ///
    /// # Arguments
    /// * `lrc_content` - 原始的LRC多行文本字符串。
    ///
    /// # Returns
    /// `String` - 处理后的LRC多行文本字符串。
    fn preprocess_qq_translation_lrc_content(lrc_content: String) -> String {
        lrc_content
            .lines() // 将输入字符串按行分割成迭代器
            .map(|line_str| {
                // 对每一行进行处理
                // 尝试找到最后一个 ']' 字符，这通常是LRC时间戳的结束位置
                if let Some(text_start_idx) = line_str.rfind(']') {
                    let timestamp_part = &line_str[..=text_start_idx]; // 提取时间戳部分 (包括 ']')
                    let text_part = line_str[text_start_idx + 1..].trim(); // 提取文本部分并去除首尾空格

                    if text_part == "//" {
                        // 如果文本部分正好是 "//"，则只返回时间戳部分（即文本变为空）
                        timestamp_part.to_string()
                    } else {
                        // 否则，返回原始行字符串
                        line_str.to_string()
                    }
                } else {
                    // 如果行不包含 ']'，说明它可能不是标准的LRC行
                    String::new()
                }
            })
            .collect::<Vec<String>>() // 将处理过的所有行收集到一个Vec<String>
            .join("\n") // 再用换行符将它们连接回一个多行字符串
    }

    /// 解析输入框中的文本到内部的中间数据结构 (`ParsedSourceData`)。
    /// 这个中间数据结构通常包含TTML格式的段落列表和各种元数据。
    ///
    /// # Returns
    /// `Result<ParsedSourceData, ConvertError>` - 成功则返回解析后的数据，失败则返回转换错误。
    fn parse_input_to_intermediate_data(&self) -> Result<ParsedSourceData, ConvertError> {
        // 如果输入文本去除首尾空格后为空，则直接返回默认的（空的）ParsedSourceData
        if self.input_text.trim().is_empty() {
            log::warn!("[Unilyric 解析输入] 输入文本为空，返回默认的空 ParsedSourceData。");
            return Ok(Default::default());
        }

        // 根据当前选择的源格式 (self.source_format) 调用相应的解析逻辑
        match self.source_format {
            LyricFormat::Ass => {
                // 处理ASS格式
                // 从字符串加载并处理ASS数据
                let ass_data: ProcessedAssData =
                    ass_parser::load_and_process_ass_from_string(&self.input_text)?;
                // 将处理后的ASS数据生成为中间TTML字符串
                let internal_ttml_str =
                    ttml_generator::generate_intermediate_ttml_from_ass(&ass_data, true)?; // true 表示生成用于内部处理的TTML
                // 解析这个内部TTML字符串
                let (
                    paragraphs,
                    _ttml_derived_meta,
                    is_line_timed_val,
                    detected_formatted_ttml,
                    _detected_ass_ttml_trans_lang,
                ) = ttml_parser::parse_ttml_from_string(&internal_ttml_str)?;
                // 构建 ParsedSourceData
                Ok(ParsedSourceData {
                    paragraphs,
                    language_code: ass_data.language_code.clone(),
                    songwriters: ass_data.songwriters.clone(),
                    agent_names: ass_data.agent_names.clone(),
                    apple_music_id: ass_data.apple_music_id.clone(),
                    general_metadata: ass_data.metadata.clone(),
                    markers: ass_data.markers.clone(),
                    is_line_timed_source: is_line_timed_val,
                    raw_ttml_from_input: Some(internal_ttml_str),
                    detected_formatted_input: Some(detected_formatted_ttml),
                    _source_translation_language: ass_data.detected_translation_language.clone(),
                    ..Default::default()
                })
            }
            LyricFormat::Ttml => {
                // 处理TTML格式
                // 直接从输入文本解析TTML
                let (
                    paragraphs,
                    meta,
                    is_line_timed_val,
                    detected_formatted,
                    detected_ttml_trans_lang,
                ) = ttml_parser::parse_ttml_from_string(&self.input_text)?;
                let mut psd = ParsedSourceData {
                    paragraphs,
                    is_line_timed_source: is_line_timed_val,
                    raw_ttml_from_input: Some(self.input_text.clone()), // 缓存原始TTML输入
                    detected_formatted_input: Some(detected_formatted),
                    general_metadata: meta, // 从TTML解析出的元数据
                    _source_translation_language: detected_ttml_trans_lang, // 使用从 TTML 解析器直接获取的值
                    ..Default::default()
                };
                // 从 general_metadata 中提取特定类型的元数据到 ParsedSourceData 的专用字段
                let mut remaining_general_meta = Vec::new();
                for m in &psd.general_metadata {
                    match m.key.parse::<CanonicalMetadataKey>() {
                        Ok(CanonicalMetadataKey::Language) => {
                            psd.language_code = Some(m.value.clone())
                        }
                        Ok(CanonicalMetadataKey::AppleMusicId) => {
                            psd.apple_music_id = m.value.clone()
                        }
                        Ok(CanonicalMetadataKey::Songwriter) => {
                            psd.songwriters.push(m.value.clone())
                        }
                        Ok(_) => {
                            // 其他标准键，但未在此处特定处理的，保留在通用元数据中
                            remaining_general_meta.push(m.clone());
                        }
                        Err(_) => {
                            // 自定义键或无法解析的键，保留在通用元数据中
                            remaining_general_meta.push(m.clone());
                            log::info!(
                                "[Unilyric] 元数据键 '{}' 无法解析为标准键，将保留在通用元数据中。",
                                m.key
                            );
                        }
                    }
                }
                psd.general_metadata = remaining_general_meta; // 更新通用元数据列表
                psd.songwriters.sort_unstable();
                psd.songwriters.dedup(); // 对词曲作者去重排序
                Ok(psd)
            }
            LyricFormat::Json => {
                // 处理JSON格式 (通常是Apple Music的JSON)
                // 从字符串加载JSON数据
                let bundle = json_parser::load_from_string(&self.input_text)?;
                // JSON解析器内部已将数据转换为类似 ParsedSourceData 的结构
                Ok(ParsedSourceData {
                    paragraphs: bundle.paragraphs,
                    language_code: bundle.language_code,
                    songwriters: bundle.songwriters,
                    agent_names: bundle.agent_names,
                    apple_music_id: bundle.apple_music_id,
                    general_metadata: bundle.general_metadata,
                    is_line_timed_source: bundle.is_line_timed,
                    raw_ttml_from_input: Some(bundle.raw_ttml_string), // JSON中内嵌的TTML字符串
                    detected_formatted_input: Some(bundle.detected_formatted_ttml),
                    ..Default::default()
                })
            }
            LyricFormat::Krc => {
                // 处理KRC格式
                // 从字符串加载KRC数据
                let (krc_lines, mut krc_meta_from_parser) =
                    krc_parser::load_krc_from_string(&self.input_text)?;
                // KRC内嵌翻译特殊处理：从元数据中移除，其值通常是Base64编码，不直接用于显示
                let mut _krc_internal_translation_base64: Option<String> = None; // 未使用
                krc_meta_from_parser.retain(|item| {
                    if item.key == "KrcInternalTranslation" {
                        // 自定义的键名，用于标记内部翻译数据
                        _krc_internal_translation_base64 = Some(item.value.clone());
                        false // 从元数据列表中移除此项
                    } else {
                        true // 保留其他元数据项
                    }
                });
                // 将KRC行和处理后的元数据转换为TTML段落和元数据
                let (paragraphs, meta_from_converter) =
                    qrc_to_ttml_data::convert_qrc_to_ttml_data(&krc_lines, krc_meta_from_parser)?; // KRC和QRC共用转换逻辑
                let mut psd = ParsedSourceData {
                    paragraphs,
                    general_metadata: meta_from_converter,
                    is_line_timed_source: false, // KRC是逐字歌词
                    ..Default::default()
                };
                // 从转换后的元数据中提取特定类型
                let mut final_general_meta = Vec::new();
                for item in psd.general_metadata.iter().cloned() {
                    match item.key.parse::<CanonicalMetadataKey>() {
                        Ok(CanonicalMetadataKey::Language) => psd.language_code = Some(item.value),
                        Ok(CanonicalMetadataKey::Songwriter) => psd.songwriters.push(item.value),
                        _ => final_general_meta.push(item), // 其他或无法解析的保留
                    }
                }
                psd.general_metadata = final_general_meta;
                psd.songwriters.sort_unstable();
                psd.songwriters.dedup();
                Ok(psd)
            }
            LyricFormat::Qrc => {
                // 处理QRC格式
                // 从字符串加载QRC数据
                let (qrc_lines, qrc_meta_from_parser) =
                    qrc_parser::load_qrc_from_string(&self.input_text)?;

                // 将QRC行和元数据转换为TTML段落和元数据
                let (paragraphs, meta_from_converter) =
                    qrc_to_ttml_data::convert_qrc_to_ttml_data(&qrc_lines, qrc_meta_from_parser)?;

                let mut psd = ParsedSourceData {
                    paragraphs,
                    general_metadata: meta_from_converter,
                    is_line_timed_source: false, // QRC是逐字歌词
                    ..Default::default()
                };
                // 从转换后的元数据中提取特定类型
                let mut final_general_meta = Vec::new();
                for item in psd.general_metadata.iter().cloned() {
                    match item.key.parse::<CanonicalMetadataKey>() {
                        Ok(CanonicalMetadataKey::Language) => psd.language_code = Some(item.value),
                        Ok(CanonicalMetadataKey::Songwriter) => psd.songwriters.push(item.value),
                        _ => final_general_meta.push(item),
                    }
                }
                psd.general_metadata = final_general_meta;
                psd.songwriters.sort_unstable();
                psd.songwriters.dedup();
                Ok(psd)
            }
            LyricFormat::Lys | LyricFormat::Spl | LyricFormat::Yrc => {
                // 处理 LYS, SPL, YRC
                // SPL 格式的特殊处理逻辑
                if self.source_format == LyricFormat::Spl {
                    let (spl_blocks_from_parser, _spl_meta) = // SPL无元数据
                        spl_parser::load_spl_from_string(&self.input_text)?;

                    if spl_blocks_from_parser.is_empty() {
                        log::info!("[UniLyric] SPL解析器未返回任何歌词数据。");
                        return Ok(Default::default()); // 返回空数据
                    }

                    let mut initial_ttml_paragraphs: Vec<TtmlParagraph> = Vec::new();

                    // 遍历SPL解析出的每个歌词块
                    for (block_idx, spl_block) in spl_blocks_from_parser.iter().enumerate() {
                        // 获取当前块的主歌词起始时间
                        let primary_start_time_for_block =
                            spl_block.start_times_ms.first().cloned().unwrap_or(0);
                        // 计算块的实际结束时间 (优先使用显式结束时间，否则尝试用下一块开始时间或默认时长)
                        let block_actual_end_ms = match spl_block.explicit_block_end_ms {
                            Some(explicit_end) => explicit_end,
                            None => {
                                if block_idx + 1 < spl_blocks_from_parser.len() {
                                    // 如果有下一块
                                    spl_blocks_from_parser[block_idx + 1]
                                        .start_times_ms
                                        .first()
                                        .cloned()
                                        .unwrap_or(primary_start_time_for_block + 3000) // 默认下一块开始，或当前块+3秒
                                } else {
                                    // 如果是最后一块
                                    primary_start_time_for_block + 3000 // 默认当前块+3秒
                                }
                            }
                        };
                        // 解析SPL块中的主文本（带内联时间戳）为音节列表 (LysSyllable)
                        let main_syllables_lys: Vec<LysSyllable> =
                            match spl_parser::parse_spl_main_text_to_syllables(
                                &spl_block.main_text_with_inline_ts, // 主文本内容
                                primary_start_time_for_block,        // 块起始时间
                                block_actual_end_ms,                 // 块结束时间
                                block_idx + 1,                       // 块索引 (用于日志)
                            ) {
                                Ok(syls) => syls,
                                Err(e) => {
                                    log::error!(
                                        "[UniLyric] 解析块 #{} ('{}') 的主文本音节失败: {}",
                                        block_idx,
                                        spl_block.main_text_with_inline_ts,
                                        e
                                    );
                                    continue; // 跳过此块
                                }
                            };
                        // 将 LysSyllable 转换为 TtmlSyllable
                        let processed_main_syllables: Vec<TtmlSyllable> =
                            crate::utils::process_parsed_syllables_to_ttml(
                                &main_syllables_lys,
                                "SPL",
                            ); // "SPL" 作为来源标记
                        // 获取块中的翻译文本 (可能多行，用 / 连接)
                        let translation_string_from_block: Option<String> =
                            if !spl_block.all_translation_lines.is_empty() {
                                Some(spl_block.all_translation_lines.join("/"))
                            } else {
                                None
                            };
                        let translation_tuple = translation_string_from_block.map(|t| (t, None)); // (文本, 语言代码=None)

                        // SPL块可能有多个起始时间 (start_times_ms)，每个时间点生成一个TTML段落
                        for &line_start_ms_for_para in &spl_block.start_times_ms {
                            // 计算该TTML段落的结束时间
                            let p_end_ms_for_para: u64 = if let Some(last_syl) =
                                processed_main_syllables.last()
                            {
                                let end_based_on_syl = last_syl.end_ms.max(line_start_ms_for_para); // 基于最后一个音节结束时间
                                end_based_on_syl.min(block_actual_end_ms) // 不超过块的实际结束时间
                            } else if translation_tuple.is_some() {
                                // 如果没有音节但有翻译
                                block_actual_end_ms // 段落结束时间为块结束时间
                            } else {
                                // 如果既无音节也无翻译
                                line_start_ms_for_para // 段落结束时间等于开始时间 (空行)
                            };

                            // 确保段落结束时间不早于开始时间
                            let final_p_end_ms_for_para =
                                if p_end_ms_for_para < line_start_ms_for_para {
                                    line_start_ms_for_para
                                } else {
                                    p_end_ms_for_para
                                };

                            // 只有当主歌词有内容或翻译有内容时，才创建TTML段落
                            let main_line_has_content = !processed_main_syllables.is_empty()
                                || !spl_block.main_text_with_inline_ts.trim().is_empty();
                            if main_line_has_content || translation_tuple.is_some() {
                                initial_ttml_paragraphs.push(TtmlParagraph {
                                    p_start_ms: line_start_ms_for_para,
                                    p_end_ms: final_p_end_ms_for_para,
                                    main_syllables: processed_main_syllables.clone(), // 克隆音节列表
                                    translation: translation_tuple.clone(), // 克隆翻译元组
                                    agent: "v1".to_string(),                // SPL不支持演唱者
                                    ..Default::default()
                                });
                            } else {
                                log::debug!(
                                    "[UniLyric] 跳过创建空的TTML段落，块起始时间: {:?}，主文本: '{}'",
                                    spl_block.start_times_ms,
                                    spl_block.main_text_with_inline_ts
                                );
                            }
                        }
                    } // 遍历SPL块结束

                    // SPL 后处理：合并在相同起始时间生成的多个TTML段落的翻译
                    let mut final_ttml_paragraphs: Vec<TtmlParagraph> = Vec::new();
                    if !initial_ttml_paragraphs.is_empty() {
                        let mut temp_iter = initial_ttml_paragraphs.into_iter().peekable();
                        while let Some(mut current_para) = temp_iter.next() {
                            let mut collected_additional_translations_for_current_para: Vec<
                                String,
                            > = Vec::new();
                            // 查看后续是否有相同起始时间的段落
                            while let Some(next_para_peek) = temp_iter.peek() {
                                if next_para_peek.p_start_ms == current_para.p_start_ms {
                                    // 如果起始时间相同
                                    let next_para = temp_iter.next().unwrap(); // 取出这个段落
                                    // 如果这个后续段落的主音节部分有文本，也作为翻译的一部分
                                    if !next_para.main_syllables.is_empty() {
                                        let trans_text_from_next_main = next_para
                                            .main_syllables
                                            .iter()
                                            .map(|s| {
                                                s.text.clone()
                                                    + if s.ends_with_space { " " } else { "" }
                                            })
                                            .collect::<String>()
                                            .trim()
                                            .to_string();
                                        if !trans_text_from_next_main.is_empty() {
                                            collected_additional_translations_for_current_para
                                                .push(trans_text_from_next_main);
                                        }
                                    }
                                    // 如果这个后续段落本身也有翻译，也加入
                                    if let Some((next_trans_text, _)) = next_para.translation {
                                        if !next_trans_text.is_empty() {
                                            collected_additional_translations_for_current_para
                                                .push(next_trans_text);
                                        }
                                    }
                                } else {
                                    break;
                                } // 起始时间不同，停止合并
                            }
                            // 将收集到的额外翻译合并到当前段落的翻译中
                            if !collected_additional_translations_for_current_para.is_empty() {
                                let combined_additional_trans =
                                    collected_additional_translations_for_current_para.join("/");
                                if let Some((ref mut existing_trans, _)) = current_para.translation
                                {
                                    if !existing_trans.is_empty() {
                                        existing_trans.push('/');
                                        existing_trans.push_str(&combined_additional_trans);
                                    } else {
                                        *existing_trans = combined_additional_trans;
                                    }
                                } else {
                                    current_para.translation =
                                        Some((combined_additional_trans, None));
                                }
                            }
                            final_ttml_paragraphs.push(current_para); // 添加处理后的段落到最终列表
                        }
                    }
                    log::info!(
                        "[Unilyric 解析输入 SPL] SPL转换完成，生成 {} 个TTML段落。",
                        final_ttml_paragraphs.len()
                    );
                    // 判断SPL是否为逐行格式 (如果每个段落最多只有一个音节)
                    let is_spl_line_timed = final_ttml_paragraphs
                        .iter()
                        .all(|p| p.main_syllables.len() <= 1);
                    return Ok(ParsedSourceData {
                        paragraphs: final_ttml_paragraphs,
                        general_metadata: Vec::new(), // SPL不支持元数据
                        is_line_timed_source: is_spl_line_timed,
                        ..Default::default()
                    }); // 注意这里是 return，因为SPL的处理已完成
                } // SPL 的 if 结束

                // LYS 和 YRC 的通用处理 (如果不是 SPL)
                let (paragraphs, general_metadata_from_parser, is_line_timed) =
                    match self.source_format {
                        LyricFormat::Lys => {
                            // 处理LYS格式
                            let (lys_lines, lys_meta) =
                                lys_parser::load_lys_from_string(&self.input_text)?;
                            let (ps, _meta_from_lys_converter) =
                                lys_to_ttml_data::convert_lys_to_ttml_data(&lys_lines)?;
                            (ps, lys_meta, false) // LYS是逐字格式
                        }
                        LyricFormat::Yrc => {
                            // 处理YRC格式 (网易云)
                            let (yrc_lines, yrc_meta) =
                                yrc_parser::load_yrc_from_string(&self.input_text)?;
                            let (ps, _meta_from_yrc_converter) =
                                yrc_to_ttml_data::convert_yrc_to_ttml_data(
                                    &yrc_lines,
                                    yrc_meta.clone(),
                                )?;
                            (ps, yrc_meta, false) // YRC是逐字格式
                        }
                        _ => unreachable!(), // 因为 SPL 已被上面处理，不应到达此分支
                    };
                let mut psd = ParsedSourceData {
                    paragraphs,
                    general_metadata: general_metadata_from_parser,
                    is_line_timed_source: is_line_timed,
                    ..Default::default()
                };
                // 从元数据中提取特定类型
                let mut final_general_meta = Vec::new();
                for item in psd.general_metadata.iter().cloned() {
                    match item.key.parse::<CanonicalMetadataKey>() {
                        Ok(CanonicalMetadataKey::Language) => psd.language_code = Some(item.value),
                        Ok(CanonicalMetadataKey::Songwriter) => psd.songwriters.push(item.value),
                        _ => final_general_meta.push(item),
                    }
                }
                psd.general_metadata = final_general_meta;
                psd.songwriters.sort_unstable();
                psd.songwriters.dedup();
                Ok(psd)
            }
            LyricFormat::Lyl => {
                // 处理LYL格式 (Lyricify Lines)
                // 解析Lyricify行
                let parsed_lines = lyricify_lines_parser::parse_lyricify_lines(&self.input_text)?;
                // 将解析后的行转换为TTML段落和元数据
                let (paragraphs, metadata) =
                    lyricify_lines_to_ttml_data::convert_lyricify_to_ttml_data(&parsed_lines)?;
                Ok(ParsedSourceData {
                    paragraphs,
                    general_metadata: metadata,
                    is_line_timed_source: true, // LYL是逐行格式
                    ..Default::default()
                })
            }
            LyricFormat::Lqe => {
                // 处理LQE格式
                // 从字符串加载LQE数据
                let lqe_parsed_data = crate::lqe_parser::load_lqe_from_string(&self.input_text)?;
                // 将LQE解析数据转换为内部的 ParsedSourceData 结构
                let mut intermediate_result =
                    crate::lqe_to_ttml_data::convert_lqe_to_intermediate_data(&lqe_parsed_data)?;
                // 从通用元数据中提取特定类型
                let mut final_general_meta_lqe: Vec<AssMetadata> = Vec::new();
                for meta_item in intermediate_result.general_metadata.iter().cloned() {
                    match meta_item.key.parse::<CanonicalMetadataKey>() {
                        Ok(CanonicalMetadataKey::Songwriter) => {
                            intermediate_result.songwriters.push(meta_item.value)
                        }
                        _ => final_general_meta_lqe.push(meta_item),
                    }
                }
                intermediate_result.general_metadata = final_general_meta_lqe;
                intermediate_result.songwriters.sort_unstable();
                intermediate_result.songwriters.dedup();
                Ok(intermediate_result)
            }
            LyricFormat::Lrc => {
                // 处理LRC格式
                log::info!(
                    "[Unilyric 解析输入] LRC作为主输入源，将尝试解析为行并计算精确结束时间。"
                );
                let (lrc_lines, lrc_meta) = lrc_parser::parse_lrc_text_to_lines(&self.input_text)?;

                let mut paragraphs: Vec<TtmlParagraph> = Vec::with_capacity(lrc_lines.len());

                // 遍历LRC行，为每行创建TTML段落
                for (i, current_lrc_line) in lrc_lines.iter().enumerate() {
                    let p_start_ms = current_lrc_line.timestamp_ms; // 段落开始时间
                    // 段落结束时间

                    let p_end_ms: u64 = if i + 1 < lrc_lines.len() {
                        // 如果有下一行
                        // 当前行的结束时间是下一行的开始时间
                        let next_line_start_ms = lrc_lines[i + 1].timestamp_ms;
                        // 确保结束时间不早于开始时间
                        if next_line_start_ms > p_start_ms {
                            next_line_start_ms
                        } else {
                            // 如果下一行时间戳异常（例如等于或早于当前行），则给一个小的默认时长
                            p_start_ms.saturating_add(1000) // 例如1000毫秒
                        }
                    } else {
                        // 如果是最后一行
                        // 结束时间是开始时间加上一个默认时长 (例如1分钟)
                        p_start_ms.saturating_add(60000)
                    };

                    // 创建TTML段落，LRC的一行对应一个TTML段落，该段落只包含一个音节，音节内容是整行文本
                    paragraphs.push(TtmlParagraph {
                        p_start_ms,
                        p_end_ms,
                        main_syllables: vec![TtmlSyllable {
                            text: current_lrc_line.text.clone(),
                            start_ms: p_start_ms,
                            end_ms: p_end_ms, // 音节的结束时间也使用计算出的行结束时间
                            ends_with_space: false, // LRC行通常不包含需要特殊处理的尾随空格
                        }],
                        agent: "v1".to_string(), // 默认 agent
                        ..Default::default()
                    });
                }

                let mut psd = ParsedSourceData {
                    paragraphs,
                    general_metadata: lrc_meta,   // 从LRC解析的元数据
                    is_line_timed_source: true,   // LRC 是逐行格式
                    lqe_main_lyrics_as_lrc: true, // 标记主歌词是LRC (因为源就是LRC)，用于LQE生成
                    ..Default::default()
                };

                // 从 general_metadata 填充 psd 特定字段
                let mut final_general_meta = Vec::new();
                for item in psd.general_metadata.iter().cloned() {
                    match item.key.parse::<CanonicalMetadataKey>() {
                        Ok(CanonicalMetadataKey::Language) => psd.language_code = Some(item.value),
                        // 特殊处理LRC中的 "[by:]" 标签，它通常指LRC制作者，对应 Author
                        Ok(CanonicalMetadataKey::Author) if item.key.eq_ignore_ascii_case("by") => {
                            final_general_meta.push(item);
                        }
                        _ => final_general_meta.push(item), // 其他或无法解析的保留
                    }
                }
                psd.general_metadata = final_general_meta;
                Ok(psd)
            }
        }
    }

    /// 根据解析后的源数据 (`ParsedSourceData`) 更新应用的核心状态，
    /// 包括主歌词段落、标记、元数据存储等。
    /// 此方法在加载新文件或从网络下载歌词后被调用。
    /// 它处理新元数据与用户已固定的元数据之间的合并逻辑。
    fn update_app_state_from_parsed_data(&mut self, data: ParsedSourceData) {
        log::debug!(
            "[Unilyric] 开始更新应用状态。当前已解析 {} 个段落。",
            data.paragraphs.len()
        );

        // 1. 更新主歌词段落和相关标记信息
        self.parsed_ttml_paragraphs = Some(data.paragraphs.clone()); // 更新TTML段落
        self.current_markers = data.markers.clone(); // 更新标记
        self.source_is_line_timed = data.is_line_timed_source; // 更新源是否逐行
        self.current_raw_ttml_from_input = data.raw_ttml_from_input.clone(); // 更新原始TTML缓存
        self.detected_formatted_ttml_source = data.detected_formatted_input.unwrap_or(false); // 更新是否检测到格式化TTML

        // 2. 元数据处理核心逻辑
        {
            // MetadataStore 锁作用域开始
            let mut store = self.metadata_store.lock().unwrap();
            store.clear(); // 2a. 清空当前的 MetadataStore，准备从新源和固定项重新填充

            // 2b. 添加来自新源的元数据
            // 首先添加会话/平台元数据 (通常来自网络下载，这些应该优先于文件内嵌的元数据，
            // 但当前 add 逻辑是追加，固定值逻辑会在后面覆盖)
            if !self.session_platform_metadata.is_empty() {
                for (key_str, value_str) in &self.session_platform_metadata {
                    if let Err(_e) = store.add(key_str, value_str.clone()) {}
                }
            }

            // 然后添加来自文件内嵌的通用元数据 (data.general_metadata)
            if !data.general_metadata.is_empty() {
                for meta_item in &data.general_metadata {
                    if let Err(_e) = store.add(&meta_item.key, meta_item.value.clone()) {}
                }
            }

            // 添加从 ParsedSourceData 特定字段提取的元数据
            if let Some(lang) = &data.language_code {
                let _ = store.add("language", lang.clone());
            }
            if !data.songwriters.is_empty() {
                for sw in &data.songwriters {
                    let _ = store.add("songwriters", sw.clone());
                }
            }
            if !data.apple_music_id.is_empty() {
                let _ = store.add("appleMusicId", data.apple_music_id.clone()); // "appleMusicId" -> CanonicalMetadataKey::AppleMusicId
            }
            if !data.agent_names.is_empty() {
                for (agent_id, agent_name) in &data.agent_names {
                    // agent_id (如 "v1", "v2") 通常是唯一的，但如果解析出多个同名agent，add会保留它们
                    // 这些通常是自定义键 CanonicalMetadataKey::Custom(agent_id)
                    let _ = store.add(agent_id, agent_name.clone());
                }
            }

            // 2c. 应用/覆盖用户通过UI标记为“固定”并已保存到设置的元数据值
            //     `self.app_settings.pinned_metadata` 存储的是上次保存到INI的固定项（显示键->值列表）。
            //     `self.persistent_canonical_keys` 存储的是用户当前在UI上希望固定的元数据类型的规范化键。
            let settings_pinned_map = self.app_settings.lock().unwrap().pinned_metadata.clone();
            if !settings_pinned_map.is_empty() {
                log::debug!(
                    "[Unilyric 更新应用状态] 应用 {} 个来自设置的固定元数据键。",
                    settings_pinned_map.len()
                );
            }

            for (pinned_display_key, pinned_values_vec) in settings_pinned_map {
                // pinned_values_vec 是 Vec<String>
                // 将设置中存储的显示键解析为其规范化形式
                let canonical_key_of_pinned_item = match pinned_display_key
                    .trim()
                    .parse::<CanonicalMetadataKey>()
                {
                    Ok(ck) => ck,
                    Err(_) => CanonicalMetadataKey::Custom(pinned_display_key.trim().to_string()),
                };

                // 关键检查：只有当这个规范化的键确实在 self.persistent_canonical_keys 集合中
                // (即用户当前仍然希望固定这种类型的元数据)，才应用设置中的固定值。
                if self
                    .persistent_canonical_keys
                    .contains(&canonical_key_of_pinned_item)
                {
                    // 从Store中移除由新文件/下载加载的、与此固定键对应的所有值
                    store.remove(&canonical_key_of_pinned_item);
                    // 使用设置中存储的显示键和值（可能是多个）重新添加到Store。
                    // store.add 会再次将其键名规范化。
                    for pinned_value_str in pinned_values_vec {
                        // 遍历值列表
                        if let Err(e) = store.add(&pinned_display_key, pinned_value_str.clone()) {
                            log::warn!(
                                "[Unilyric 更新应用状态] 应用设置中的固定元数据 '{}' (值: '{}') 到Store失败: {}",
                                pinned_display_key,
                                pinned_value_str,
                                e
                            );
                        }
                    }
                }
            }

            // 2d. 显式移除 KRC 内部语言 Base64 值 (如果它因任何原因进入了Store)
            //     这一步作为最后防线，确保它不会出现在最终的元数据列表中。
            store.remove(&CanonicalMetadataKey::KrcInternalTranslation);

            // 2e. 对元数据存储进行去重，确保值的唯一性
            //     去重操作会保留每个键下唯一的、非空的值。
            //     如果一个键之前通过多次 add 累积了相同的值，去重后只会保留一个。
            store.deduplicate_values();
        } // MetadataStore 锁释放

        // 3. 根据更新后的 MetadataStore 重建UI的可编辑元数据列表
        self.rebuild_editable_metadata_from_store();
    }

    /// 将通过“加载翻译/罗马音LRC”菜单加载的LRC行合并到当前的主歌词段落中，
    /// 并更新右侧的翻译/罗马音LRC预览面板。
    pub fn merge_lrc_into_paragraphs(&mut self) {
        let translation_lrc_header = self.generate_specific_lrc_header(LrcContentType::Translation);
        let romanization_lrc_header =
            self.generate_specific_lrc_header(LrcContentType::Romanization);

        if self.parsed_ttml_paragraphs.is_none() {
            // --- 无主段落时，仅更新LRC预览面板的逻辑 ---
            if self.loaded_translation_lrc.is_some() {
                self.display_translation_lrc_output =
                    self.loaded_translation_lrc
                        .as_ref()
                        .map_or(String::new(), |lines| {
                            let mut temp_output = translation_lrc_header.clone();
                            for l in lines {
                                temp_output.push_str(&format!(
                                    "{}{}\n",
                                    crate::utils::format_lrc_time_ms(l.timestamp_ms),
                                    l.text
                                ));
                            }
                            let final_output = temp_output.trim_end().to_string();
                            if final_output.is_empty() && translation_lrc_header.trim().is_empty() {
                                String::new()
                            } else {
                                format!("{}\n", final_output)
                            }
                        });
            } else {
                self.display_translation_lrc_output.clear();
            }

            if self.loaded_romanization_lrc.is_some() {
                self.display_romanization_lrc_output = self
                    .loaded_romanization_lrc
                    .as_ref()
                    .map_or(String::new(), |lines| {
                        let mut temp_output = romanization_lrc_header.clone();
                        for l in lines {
                            temp_output.push_str(&format!(
                                "{}{}\n",
                                crate::utils::format_lrc_time_ms(l.timestamp_ms),
                                l.text
                            ));
                        }
                        let final_output = temp_output.trim_end().to_string();
                        if final_output.is_empty() && romanization_lrc_header.trim().is_empty() {
                            String::new()
                        } else {
                            format!("{}\n", final_output)
                        }
                    });
            } else {
                self.display_romanization_lrc_output.clear();
            }
            return;
        }

        let paragraphs = self.parsed_ttml_paragraphs.as_mut().unwrap();
        const LRC_MATCH_TOLERANCE_MS: u64 = 15; // 匹配容差

        // --- 确定翻译语言代码 ---
        let specific_translation_lang_for_para: Option<String>;
        {
            let store = self.metadata_store.lock().unwrap();
            specific_translation_lang_for_para = store
                .get_single_value_by_str("translation_language")
                .cloned()
                .or_else(|| {
                    store
                        .get_single_value(&crate::types::CanonicalMetadataKey::Language)
                        .cloned()
                });
        }

        // --- 合并翻译LRC ---
        if let Some(ref lrc_trans_lines_vec) = self.loaded_translation_lrc {
            log::info!(
                "[Unilyric 合并LRC] 正在合并 {} 行已加载的翻译LRC到 {} 个主歌词段落。",
                lrc_trans_lines_vec.len(),
                paragraphs.len()
            );

            let mut available_lrc_lines: Vec<(&crate::types::LrcLine, bool)> = lrc_trans_lines_vec
                .iter()
                .map(|line| (line, false))
                .collect();

            for paragraph in paragraphs.iter_mut() {
                // 1. 为主歌词部分匹配翻译
                let para_start_ms = paragraph.p_start_ms;
                let para_end_ms = paragraph.p_end_ms;
                let mut best_match_main_idx: Option<usize> = None;
                let mut smallest_diff_main = u64::MAX;

                for (current_lrc_idx, (lrc_line, used)) in available_lrc_lines.iter().enumerate() {
                    if *used {
                        continue;
                    }
                    let diff = (lrc_line.timestamp_ms as i64 - para_start_ms as i64).unsigned_abs();
                    if diff <= LRC_MATCH_TOLERANCE_MS {
                        if diff < smallest_diff_main {
                            smallest_diff_main = diff;
                            best_match_main_idx = Some(current_lrc_idx);
                        }
                    } else if lrc_line.timestamp_ms > para_start_ms + LRC_MATCH_TOLERANCE_MS
                        && best_match_main_idx.is_some()
                    {
                        break;
                    }
                    if lrc_line.timestamp_ms > para_end_ms + LRC_MATCH_TOLERANCE_MS * 2 {
                        break;
                    }
                }

                if let Some(matched_idx) = best_match_main_idx {
                    let (matched_lrc, _) = available_lrc_lines[matched_idx];
                    if !matched_lrc.text.trim().is_empty() {
                        // 只有当LRC行文本非空时才更新
                        paragraph.translation = Some((
                            matched_lrc.text.clone(),
                            specific_translation_lang_for_para.clone(),
                        ));
                        // 标记该LRC行已被使用
                        available_lrc_lines[matched_idx].1 = true;
                    }
                }

                // 2. 为背景人声匹配翻译 (如果存在背景人声且有音节)
                if let Some(bg_section_ref) = paragraph.background_section.as_ref() {
                    // 使用不可变引用检查
                    if !bg_section_ref.syllables.is_empty() {
                        let bg_start_ms = bg_section_ref.start_ms;
                        let bg_end_ms = bg_section_ref.end_ms;
                        let mut best_match_bg_idx: Option<usize> = None;
                        let mut smallest_diff_bg = u64::MAX;

                        for (current_lrc_idx, (lrc_line, used)) in
                            available_lrc_lines.iter().enumerate()
                        {
                            if *used {
                                continue;
                            }
                            let diff =
                                (lrc_line.timestamp_ms as i64 - bg_start_ms as i64).unsigned_abs();
                            if diff <= LRC_MATCH_TOLERANCE_MS {
                                if diff < smallest_diff_bg {
                                    smallest_diff_bg = diff;
                                    best_match_bg_idx = Some(current_lrc_idx);
                                }
                            } else if lrc_line.timestamp_ms > bg_start_ms + LRC_MATCH_TOLERANCE_MS
                                && best_match_bg_idx.is_some()
                            {
                                break;
                            }
                            if lrc_line.timestamp_ms > bg_end_ms + LRC_MATCH_TOLERANCE_MS * 2 {
                                break;
                            }
                        }

                        if let Some(matched_idx) = best_match_bg_idx {
                            let (matched_lrc, _) = available_lrc_lines[matched_idx];
                            if !matched_lrc.text.trim().is_empty() {
                                // 只有当LRC行文本非空时才更新
                                // 获取可变引用来更新
                                if let Some(bg_section_mut) = paragraph.background_section.as_mut()
                                {
                                    bg_section_mut.translation = Some((
                                        matched_lrc.text.clone(),
                                        specific_translation_lang_for_para.clone(),
                                    ));
                                    available_lrc_lines[matched_idx].1 = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        // 合并后，重新生成翻译LRC预览面板的内容
        match crate::lrc_generator::generate_lrc_from_paragraphs(
            paragraphs,
            LrcContentType::Translation,
        ) {
            Ok(text) => {
                self.display_translation_lrc_output = translation_lrc_header + &text;
            }
            Err(e) => {
                log::error!("[Unilyric 合并LRC] 生成翻译LRC预览失败：{}", e);
                self.display_translation_lrc_output.clear();
            }
        }

        // --- 合并罗马音LRC (仅在LRC提供新内容时覆盖) ---
        if let Some(ref lrc_roma_lines_vec) = self.loaded_romanization_lrc {
            log::info!(
                "[Unilyric 合并LRC] 正在合并 {} 行已加载的罗马音LRC到 {} 个主歌词段落。",
                lrc_roma_lines_vec.len(),
                paragraphs.len()
            );

            let mut available_lrc_lines: Vec<(&crate::types::LrcLine, bool)> = lrc_roma_lines_vec
                .iter()
                .map(|line| (line, false))
                .collect();

            for paragraph in paragraphs.iter_mut() {
                // 1. 为主歌词部分匹配罗马音
                let para_start_ms = paragraph.p_start_ms;
                let para_end_ms = paragraph.p_end_ms;
                let mut best_match_main_idx: Option<usize> = None;
                let mut smallest_diff_main = u64::MAX;

                for (current_lrc_idx, (lrc_line, used)) in available_lrc_lines.iter().enumerate() {
                    if *used {
                        continue;
                    }
                    let diff = (lrc_line.timestamp_ms as i64 - para_start_ms as i64).unsigned_abs();
                    if diff <= LRC_MATCH_TOLERANCE_MS {
                        if diff < smallest_diff_main {
                            smallest_diff_main = diff;
                            best_match_main_idx = Some(current_lrc_idx);
                        }
                    } else if lrc_line.timestamp_ms > para_start_ms + LRC_MATCH_TOLERANCE_MS
                        && best_match_main_idx.is_some()
                    {
                        break;
                    }
                    if lrc_line.timestamp_ms > para_end_ms + LRC_MATCH_TOLERANCE_MS * 2 {
                        break;
                    }
                }

                if let Some(matched_idx) = best_match_main_idx {
                    let (matched_lrc, _) = available_lrc_lines[matched_idx];
                    if !matched_lrc.text.trim().is_empty() {
                        // 只有当LRC行文本非空时才更新
                        paragraph.romanization = Some(matched_lrc.text.clone());
                        available_lrc_lines[matched_idx].1 = true;
                    }
                }

                // 2. 为背景人声匹配罗马音
                if let Some(bg_section_ref) = paragraph.background_section.as_ref() {
                    // 使用不可变引用检查
                    if !bg_section_ref.syllables.is_empty() {
                        let bg_start_ms = bg_section_ref.start_ms;
                        let bg_end_ms = bg_section_ref.end_ms;
                        let mut best_match_bg_idx: Option<usize> = None;
                        let mut smallest_diff_bg = u64::MAX;

                        for (current_lrc_idx, (lrc_line, used)) in
                            available_lrc_lines.iter().enumerate()
                        {
                            if *used {
                                continue;
                            }
                            let diff =
                                (lrc_line.timestamp_ms as i64 - bg_start_ms as i64).unsigned_abs();
                            if diff <= LRC_MATCH_TOLERANCE_MS {
                                if diff < smallest_diff_bg {
                                    smallest_diff_bg = diff;
                                    best_match_bg_idx = Some(current_lrc_idx);
                                }
                            } else if lrc_line.timestamp_ms > bg_start_ms + LRC_MATCH_TOLERANCE_MS
                                && best_match_bg_idx.is_some()
                            {
                                break;
                            }
                            if lrc_line.timestamp_ms > bg_end_ms + LRC_MATCH_TOLERANCE_MS * 2 {
                                break;
                            }
                        }

                        if let Some(matched_idx) = best_match_bg_idx {
                            let (matched_lrc, _) = available_lrc_lines[matched_idx];
                            if !matched_lrc.text.trim().is_empty() {
                                // 只有当LRC行文本非空时才更新
                                if let Some(bg_section_mut) = paragraph.background_section.as_mut()
                                {
                                    bg_section_mut.romanization = Some(matched_lrc.text.clone());
                                    available_lrc_lines[matched_idx].1 = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        // 合并后，重新生成罗马音LRC预览面板的内容
        match crate::lrc_generator::generate_lrc_from_paragraphs(
            paragraphs,
            LrcContentType::Romanization,
        ) {
            Ok(text) => {
                self.display_romanization_lrc_output = romanization_lrc_header + &text;
            }
            Err(e) => {
                log::error!("[Unilyric 合并LRC] 生成罗马音LRC预览失败：{}", e);
                self.display_romanization_lrc_output.clear();
            }
        }
    }

    /// 处理歌词转换的核心函数。
    /// 1. 解析输入文本到中间数据结构 (`ParsedSourceData`)。
    /// 2. 更新应用状态 (元数据、标记等)。
    /// 3. 合并KRC内嵌翻译、网络下载的次要歌词、手动加载的LRC。
    /// 4. 生成目标格式的输出文本。
    pub fn handle_convert(&mut self) {
        log::info!(
            "[Unilyric 处理转换] 开始转换流程。输入文本是否为空: {}",
            self.input_text.is_empty()
        );
        self.output_text.clear(); // 清空上一次的输出结果
        self.conversion_in_progress = true; // 标记转换正在进行
        self.new_trigger_log_exists = false; // 重置新日志触发标志

        let mut parsed_data_for_update: ParsedSourceData = Default::default(); // 初始化为空的解析数据

        // 只有当输入文本非空时才尝试解析
        if !self.input_text.trim().is_empty() {
            match self.parse_input_to_intermediate_data() {
                Ok(parsed_data_bundle) => {
                    // 解析成功，使用解析结果
                    parsed_data_for_update = parsed_data_bundle;
                }
                Err(e) => {
                    // 解析失败，记录错误。parsed_data_for_update 保持默认（空）。
                    // 即使解析失败，后续仍会尝试应用会话元数据和持久化元数据。
                    log::error!(
                        "[Unilyric 处理转换] 解析源数据失败: {}. 仍将尝试应用元数据。",
                        e
                    );
                    // 清理可能存在的旧解析数据，因为源已更改或解析失败
                    self.parsed_ttml_paragraphs = None;
                    self.current_markers.clear();
                    self.source_is_line_timed = false;
                    self.current_raw_ttml_from_input = None;
                }
            }
        } else if self.parsed_ttml_paragraphs.is_none() {
            // 输入文本为空，并且之前也没有已解析的段落，无需做特别处理。
            // parsed_data_for_update 保持默认（空）。
            log::info!("[Unilyric 处理转换] 输入文本为空且无已解析段落。");
        } else {
            // 输入文本为空，但之前存在已解析的段落 (例如用户清空了输入框)。
            // 当前逻辑：如果输入文本为空，则清除已解析的段落。
            // parsed_data_for_update 保持默认（空），这将导致 update_app_state_from_parsed_data 清除旧段落。
            log::info!("[Unilyric 处理转换] 输入文本为空，将清除之前已解析的段落（如果存在）。");
        }

        // 无论解析是否成功，都调用 update_app_state_from_parsed_data。
        // 如果解析失败或输入为空，parsed_data_for_update 是空的，
        // 这会有效地清除旧的段落数据，但会正确处理元数据（如应用固定项）。
        self.update_app_state_from_parsed_data(parsed_data_for_update);

        // 再次确保KRC内部翻译相关的元数据（通常是base64编码的）从最终的元数据存储中移除。
        // 这是因为这类内部数据不应暴露给用户或包含在输出文件中。
        {
            // 加锁 MetadataStore
            let mut store = self.metadata_store.lock().unwrap();
            store.remove(&CanonicalMetadataKey::KrcInternalTranslation);
            // 也尝试移除可能存在的自定义键形式 (以防万一)
            store.remove(&CanonicalMetadataKey::Custom(
                "krcinternaltranslation".to_string(),
            ));
            store.remove(&CanonicalMetadataKey::Custom(
                "krc_internal_language_base64_value".to_string(),
            ));
        } // MetadataStore 锁释放
        // 确保UI元数据列表也得到更新，以反映这个移除
        self.rebuild_editable_metadata_from_store();

        // 处理KRC内嵌翻译 (如果存在于 pending_krc_translation_lines)
        // 这部分逻辑之前在 update_app_state_from_parsed_data 中被注释掉了，移到这里更合适，
        // 因为它是在主歌词段落和元数据都已初步建立之后进行的。
        if let Some(trans_lines) = self.pending_krc_translation_lines.take() {
            // .take() 会消耗掉数据
            if !trans_lines.is_empty() {
                if let Some(ref mut paragraphs) = self.parsed_ttml_paragraphs {
                    if !paragraphs.is_empty() {
                        log::info!(
                            "[Unilyric 处理转换] 正在应用KRC内嵌翻译 (共 {} 行)",
                            trans_lines.len()
                        );
                        for (i, para_line) in paragraphs.iter_mut().enumerate() {
                            if let Some(trans_text) = trans_lines.get(i) {
                                let text_to_use = if trans_text == "//" {
                                    ""
                                } else {
                                    trans_text.as_str()
                                };
                                // 只有当段落尚无翻译，或翻译为空时，才使用KRC内嵌翻译填充
                                if para_line.translation.is_none()
                                    || para_line
                                        .translation
                                        .as_ref()
                                        .is_some_and(|(t, _)| t.is_empty())
                                {
                                    para_line.translation = Some((text_to_use.to_string(), None)); // KRC内嵌翻译通常无明确语言代码
                                }
                            }
                        }
                    } else {
                        // 如果没有主段落，但有待处理的KRC翻译，将其放回以便后续处理（如果适用）
                        // 或者记录警告并丢弃。当前选择放回。
                        log::warn!(
                            "[Unilyric 处理转换] KRC内嵌翻译存在，但无主歌词段落可合并。将暂存的翻译重新放回。"
                        );
                        self.pending_krc_translation_lines = Some(trans_lines);
                    }
                } else {
                    log::warn!(
                        "[Unilyric 处理转换] KRC内嵌翻译存在，但 parsed_ttml_paragraphs 为 None。将暂存的翻译重新放回。"
                    );
                    self.pending_krc_translation_lines = Some(trans_lines);
                }
            }
        }

        // 合并从网络下载的次要歌词 (翻译LRC, 罗马音QRC/LRC)
        self.merge_downloaded_secondary_lyrics();
        // 合并用户通过菜单加载的次要LRC文件
        self.merge_lrc_into_paragraphs();

        // 生成最终的目标格式输出文本
        self.generate_target_format_output();

        self.conversion_in_progress = false; // 标记转换结束
        log::info!("[Unilyric 处理转换] 转换流程执行完毕。");
    }

    /// 判断给定的歌词格式是否为逐行格式。
    /// 逐行格式指的是歌词的每一行对应一个时间戳，而不是每个字或词。
    /// 例如：LRC, LYL。
    pub fn source_format_is_line_timed(format: LyricFormat) -> bool {
        matches!(format, LyricFormat::Lrc | LyricFormat::Lyl)
    }

    /// 触发酷狗音乐KRC歌词的下载流程。
    pub fn trigger_kugou_download(&mut self) {
        let query = self.kugou_query.trim().to_string();
        if query.is_empty() {
            log::error!("[Unilyric] 酷狗音乐下载：请输入有效的搜索内容。");
            let mut download_status_locked = self.kugou_download_state.lock().unwrap();
            // 如果正在下载中，也重置为空闲，避免UI卡顿
            if matches!(*download_status_locked, KrcDownloadState::Downloading) {
                *download_status_locked = KrcDownloadState::Idle;
            }
            return;
        }

        // 设置下载状态为Downloading
        {
            let mut download_status_locked = self.kugou_download_state.lock().unwrap();
            *download_status_locked = KrcDownloadState::Downloading;
        }

        // 克隆共享状态和HTTP客户端，用于新线程
        let state_clone = Arc::clone(&self.kugou_download_state);
        let client_clone = self.http_client.clone();

        // 启动新线程执行网络请求
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    log::error!("[Unilyric 酷狗下载线程] 创建Tokio运行时失败: {}", e);
                    let mut status_lock = state_clone.lock().unwrap();
                    *status_lock = KrcDownloadState::Error(format!("创建异步运行时失败: {}", e));
                    return;
                }
            };

            rt.block_on(async {
                log::info!("[Unilyric 酷狗下载线程] 正在获取 '{}' 的KRC歌词...", query);
                match kugou_lyrics_fetcher::fetch_lyrics_for_song_async(&client_clone, &query).await
                {
                    Ok(fetched_data) => {
                        log::info!(
                            "[Unilyric] 酷狗音乐下载成功：已获取 {} - {}",
                            fetched_data.song_name.as_deref().unwrap_or("未知歌名"),
                            fetched_data.artists_name.join("/")
                        );
                        let mut status_lock = state_clone.lock().unwrap();
                        *status_lock = KrcDownloadState::Success(fetched_data);
                    }
                    Err(e) => {
                        let error_message = format!("下载失败: {}", e);
                        log::error!("[Unilyric] 酷狗歌词下载失败: {}", error_message);
                        let mut status_lock = state_clone.lock().unwrap();
                        *status_lock = KrcDownloadState::Error(error_message);
                    }
                }
            });
        });
    }

    /// 为特定类型的LRC内容（翻译或罗马音）生成LRC文件头部元数据字符串。
    /// 例如 `[ti:歌名]\n[ar:歌手]\n[language:zh]\n`
    ///
    /// # Arguments
    /// * `content_type` - `LrcContentType`，指示是为翻译还是罗马音生成头部。
    ///
    /// # Returns
    /// `String` - 生成的LRC头部字符串，每条元数据占一行。
    fn generate_specific_lrc_header(&self, content_type: LrcContentType) -> String {
        let mut header = String::new(); // 初始化空字符串用于构建头部
        let store = self.metadata_store.lock().unwrap(); // 获取元数据存储的锁
        let mut lang_to_use: Option<String> = None; // 用于存储最终要使用的语言代码

        // 特别处理翻译类型的语言代码
        if content_type == LrcContentType::Translation {
            // 优先级1: 尝试从已解析的主歌词段落的翻译信息中提取语言代码。
            // 这适用于当翻译已合并到主歌词，并且主歌词段落中记录了翻译语言的情况。
            if let Some(paragraphs) = &self.parsed_ttml_paragraphs {
                for p in paragraphs {
                    if let Some((_text, Some(lang_code))) = &p.translation {
                        if !lang_code.is_empty() {
                            lang_to_use = Some(lang_code.clone());
                            break; // 找到第一个非空语言代码即可
                        }
                    }
                }
            }
            // 优先级2: 如果段落中未找到，尝试从元数据存储中获取专门为“翻译”指定的语言代码。
            // (假设存在一个如 "translation_language" 的自定义元数据键)
            if lang_to_use.is_none() {
                lang_to_use = store
                    .get_single_value_by_str("translation_language")
                    .cloned();
            }
        }
        // 优先级3 (或罗马音的默认逻辑): 如果上述都未找到语言代码，或者内容类型不是翻译，
        // 则尝试使用全局的元数据存储中的 "language" 标签。
        if lang_to_use.is_none() {
            lang_to_use = store
                .get_single_value(&CanonicalMetadataKey::Language)
                .cloned();
        }
        // 如果最终确定了语言代码，则将其添加到头部
        if let Some(lang) = lang_to_use {
            if !lang.is_empty() {
                // 使用 writeln! 宏格式化并写入，它会自动添加换行符
                let _ = writeln!(header, "[language:{}]", lang.trim()); // trim() 以防语言代码前后有意外空格
            }
        }

        // 定义标准LRC标签与程序内部元数据规范键的映射关系
        let lrc_tags_map = [
            (CanonicalMetadataKey::Title, "ti"),      // 歌名
            (CanonicalMetadataKey::Artist, "ar"),     // 歌手
            (CanonicalMetadataKey::Album, "al"),      // 专辑
            (CanonicalMetadataKey::Author, "by"),     // LRC制作者 (对应 Author)
            (CanonicalMetadataKey::Offset, "offset"), // 时间偏移
            (CanonicalMetadataKey::Length, "length"), // 歌曲长度
            (CanonicalMetadataKey::Editor, "re"),     // 使用的编辑器或程序
            (CanonicalMetadataKey::Version, "ve"),    // 程序版本
                                                      // 可以根据需要添加更多映射，例如自定义元数据到特定LRC标签
        ];

        // 遍历映射表，从元数据存储中获取值并添加到头部
        for (canonical_key, lrc_tag_name) in lrc_tags_map.iter() {
            // 尝试获取该规范键对应的所有值 (元数据存储可能为一个键存储多个值)
            if let Some(values_vec) = store.get_multiple_values(canonical_key) {
                if !values_vec.is_empty() {
                    // 将所有非空值用 "/" 连接起来
                    let combined_value = values_vec
                        .iter()
                        .map(|s| s.trim()) // 去除每个值的前后空格
                        .filter(|s| !s.is_empty()) // 过滤掉处理后的空值
                        .collect::<Vec<&str>>()
                        .join("/"); // 用斜杠连接多个值
                    if !combined_value.is_empty() {
                        let _ = writeln!(header, "[{}:{}]", lrc_tag_name, combined_value);
                    }
                }
            }
        }
        header // 返回构建好的LRC头部字符串
    }

    /// 根据当前的应用状态（已解析的TTML段落、元数据等）生成目标格式的歌词文本，
    /// 并更新主输出框和相关的LRC预览面板。
    pub fn generate_target_format_output(&mut self) {
        // 防止在下载过程中重复调用此函数 (如果输出框已显示下载提示)
        if self.conversion_in_progress && self.output_text.contains("正在下载") {
            log::warn!("[Unilyric 生成目标输出] 检测到正在下载，已跳过重复调用生成函数。");
            return;
        }
        // conversion_in_progress 应该在 handle_convert 的开始和结束处管理，
        // 这里不再重复设置，假设调用此函数时，状态是合适的。

        // 获取用于生成的TTML段落列表的克隆
        let paragraphs_for_gen: Vec<TtmlParagraph>;
        if let Some(ref paras_vec) = self.parsed_ttml_paragraphs {
            paragraphs_for_gen = paras_vec.clone();
            log::info!(
                "[Unilyric 生成目标输出] 准备从 {} 个已解析段落生成输出。",
                paragraphs_for_gen.len()
            );
        } else {
            // 如果没有已解析的段落 (例如，只有元数据，或者清空了输入)
            log::warn!(
                "[Unilyric 生成目标输出] self.parsed_ttml_paragraphs 为 None。将使用空段落列表生成。"
            );
            paragraphs_for_gen = Vec::new(); // 使用空列表，某些格式可能仍能生成元数据部分
        };

        // 为翻译和罗马音LRC预览生成头部元数据
        let translation_lrc_header = self.generate_specific_lrc_header(LrcContentType::Translation);
        let romanization_lrc_header =
            self.generate_specific_lrc_header(LrcContentType::Romanization);
        // 获取元数据存储的锁，供后续所有生成器使用，避免多次加锁
        let store_guard_for_lrc_gen = self.metadata_store.lock().unwrap();

        // --- 更新LRC预览面板的逻辑 ---

        // 1. 生成翻译LRC预览 (self.display_translation_lrc_output)
        if let Some(loaded_lines) = &self.loaded_translation_lrc {
            // 如果用户手动加载了翻译LRC文件
            let mut temp_output = translation_lrc_header.clone();
            for l in loaded_lines {
                let _ = writeln!(
                    temp_output,
                    "{}{}",
                    crate::utils::format_lrc_time_ms(l.timestamp_ms),
                    l.text.trim()
                );
            }
            let trimmed_preview = temp_output.trim_end_matches('\n'); // 去除末尾可能的多余换行
            // 只有当预览内容或头部非空时，才添加最后的换行符，保持格式整洁
            self.display_translation_lrc_output =
                if trimmed_preview.is_empty() && translation_lrc_header.trim().is_empty() {
                    String::new()
                } else {
                    format!("{}\n", trimmed_preview)
                };
        } else if !paragraphs_for_gen.is_empty() {
            // 如果没有手动加载的，但有主歌词段落，则尝试从主段落生成翻译LRC
            match crate::lrc_generator::generate_lrc_from_paragraphs(
                &paragraphs_for_gen,
                LrcContentType::Translation,
            ) {
                Ok(text) => self.display_translation_lrc_output = translation_lrc_header + &text, // 拼接头部和内容
                Err(e) => {
                    self.display_translation_lrc_output.clear();
                    log::error!("生成翻译LRC预览失败: {}", e);
                }
            }
        } else {
            // 既无手动加载，也无主段落，则预览只显示头部（如果头部有内容）
            self.display_translation_lrc_output = if !translation_lrc_header.trim().is_empty() {
                translation_lrc_header.trim_end_matches('\n').to_string() + "\n"
            } else {
                String::new()
            };
        }

        // 2. 生成罗马音LRC预览 (self.display_romanization_lrc_output) - 逻辑与翻译LRC类似
        if let Some(loaded_lines) = &self.loaded_romanization_lrc {
            let mut temp_output = romanization_lrc_header.clone();
            for l in loaded_lines {
                let _ = writeln!(
                    temp_output,
                    "{}{}",
                    crate::utils::format_lrc_time_ms(l.timestamp_ms),
                    l.text.trim()
                );
            }
            let trimmed_preview = temp_output.trim_end_matches('\n');
            self.display_romanization_lrc_output =
                if trimmed_preview.is_empty() && romanization_lrc_header.trim().is_empty() {
                    String::new()
                } else {
                    format!("{}\n", trimmed_preview)
                };
        } else if !paragraphs_for_gen.is_empty() {
            match crate::lrc_generator::generate_lrc_from_paragraphs(
                &paragraphs_for_gen,
                LrcContentType::Romanization,
            ) {
                Ok(text) => self.display_romanization_lrc_output = romanization_lrc_header + &text,
                Err(e) => {
                    self.display_romanization_lrc_output.clear();
                    log::error!("生成罗马音LRC预览失败: {}", e);
                }
            }
        } else {
            self.display_romanization_lrc_output = if !romanization_lrc_header.trim().is_empty() {
                romanization_lrc_header.trim_end_matches('\n').to_string() + "\n"
            } else {
                String::new()
            };
        }

        // --- 主输出生成逻辑 ---
        // 根据选择的目标格式 (self.target_format) 调用相应的生成器函数
        // store_guard_for_lrc_gen (元数据存储的锁) 已在上面获取，传递给生成器
        let result: Result<String, ConvertError> = match self.target_format {
            LyricFormat::Lrc => {
                // 生成LRC格式
                crate::lrc_generator::generate_main_lrc_from_paragraphs(
                    &paragraphs_for_gen,
                    &store_guard_for_lrc_gen,
                )
            }
            LyricFormat::Ttml => {
                // 生成TTML格式
                // 参数: 段落, 元数据, 时间模式("Line"或"Word"), 是否格式化, 是否用于JSON内嵌
                crate::ttml_generator::generate_ttml_from_paragraphs(
                    &paragraphs_for_gen,
                    &store_guard_for_lrc_gen,
                    if self.source_is_line_timed {
                        "Line"
                    } else {
                        "Word"
                    }, // 根据源是否逐行决定TTML时间模式
                    Some(
                        self.detected_formatted_ttml_source
                            && (self.source_format == LyricFormat::Ttml
                                || self.source_format == LyricFormat::Json),
                    ), // 如果源是格式化的TTML/JSON，则目标TTML也格式化
                    false, // false表示不用于JSON内嵌，是独立的TTML文件
                )
            }
            LyricFormat::Ass => {
                // 生成ASS格式
                crate::ass_generator::generate_ass(
                    paragraphs_for_gen.to_vec(),
                    &store_guard_for_lrc_gen,
                )
            }
            LyricFormat::Json => {
                // 生成JSON格式 (Apple Music)
                let output_timing_mode_for_json_ttml = if self.source_is_line_timed {
                    "Line"
                } else {
                    "Word"
                };
                // 首先生成内嵌的TTML字符串
                crate::ttml_generator::generate_ttml_from_paragraphs(
                    &paragraphs_for_gen,
                    &store_guard_for_lrc_gen,
                    output_timing_mode_for_json_ttml,
                    Some(
                        self.detected_formatted_ttml_source
                            && (self.source_format == LyricFormat::Ttml
                                || self.source_format == LyricFormat::Json),
                    ),
                    true, // true表示此TTML用于JSON内嵌，不包含翻译和罗马音
                )
                .and_then(|ttml_json_content| {
                    // 如果TTML生成成功
                    // 从元数据获取Apple Music ID，如果找不到则使用默认值
                    let apple_music_id_from_store = store_guard_for_lrc_gen
                        .get_single_value(&CanonicalMetadataKey::AppleMusicId)
                        .cloned()
                        .unwrap_or_else(|| "unknown_id".to_string());
                    // 构建Apple Music JSON所需的结构体
                    let play_params = crate::types::AppleMusicPlayParams {
                        id: apple_music_id_from_store.clone(),
                        kind: "lyric".to_string(),
                        catalog_id: apple_music_id_from_store.clone(),
                        display_type: 2,
                    };
                    let attributes = crate::types::AppleMusicAttributes {
                        ttml: ttml_json_content,
                        play_params,
                    };
                    let data_object = crate::types::AppleMusicDataObject {
                        id: apple_music_id_from_store,
                        data_type: "syllable-lyrics".to_string(),
                        attributes,
                    };
                    let root = crate::types::AppleMusicRoot {
                        data: vec![data_object],
                    };
                    // 将结构体序列化为JSON字符串
                    serde_json::to_string(&root).map_err(ConvertError::JsonParse) // 映射serde错误到自定义错误类型
                })
            }
            LyricFormat::Lys => {
                // 生成LYS格式
                crate::lys_generator::generate_lys_from_ttml_data(
                    &paragraphs_for_gen,
                    &store_guard_for_lrc_gen,
                    true,
                ) // true表示包含头部元数据
            }
            LyricFormat::Qrc => {
                // 生成QRC格式
                crate::qrc_generator::generate_qrc_from_ttml_data(
                    &paragraphs_for_gen,
                    &store_guard_for_lrc_gen,
                )
            }
            LyricFormat::Yrc => {
                // 生成YRC格式
                crate::yrc_generator::generate_yrc_from_ttml_data(
                    &paragraphs_for_gen,
                    &store_guard_for_lrc_gen,
                )
            }
            LyricFormat::Lyl => {
                // 生成LYL格式
                crate::lyricify_lines_generator::generate_from_ttml_data(&paragraphs_for_gen)
            }
            LyricFormat::Spl => {
                // 生成SPL格式
                crate::spl_generator::generate_spl_from_ttml_data(
                    &paragraphs_for_gen,
                    &store_guard_for_lrc_gen,
                )
            }
            LyricFormat::Lqe => {
                // 生成LQE格式
                // LQE格式需要主歌词、翻译LRC、罗马音LRC等多种信息，以及元数据。
                // 下面准备这些信息传递给LQE生成器。
                let mut lqe_extracted_translation_lrc_content: Option<String> = None;
                let mut lqe_translation_language: Option<String> = None;
                let mut lqe_extracted_romanization_lrc_content: Option<String> = None;
                let mut lqe_romanization_language: Option<String> = None;

                // --- 准备翻译LRC内容和语言代码 ---
                if let Some(loaded_lines) = &self.loaded_translation_lrc {
                    // 如果用户手动加载了翻译LRC
                    let mut lrc_text_body = String::new();
                    for l_line in loaded_lines {
                        let _ = writeln!(
                            lrc_text_body,
                            "{}{}",
                            crate::utils::format_lrc_time_ms(l_line.timestamp_ms),
                            l_line.text.trim()
                        );
                    }
                    if !lrc_text_body.trim().is_empty() {
                        lqe_extracted_translation_lrc_content =
                            Some(lrc_text_body.trim_end_matches('\n').to_string() + "\n");
                    }
                    // 尝试从元数据存储中获取翻译语言代码
                    lqe_translation_language = store_guard_for_lrc_gen
                        .get_single_value_by_str("translation_language")
                        .cloned()
                        .or_else(|| {
                            store_guard_for_lrc_gen
                                .get_single_value(&CanonicalMetadataKey::Language)
                                .cloned()
                        });
                    log::info!(
                        "[LQE生成准备] 使用手动加载的翻译LRC。语言: {:?}",
                        lqe_translation_language
                    );
                } else {
                    // 没有手动加载的翻译LRC，尝试从主歌词段落 (paragraphs_for_gen) 生成
                    if paragraphs_for_gen.iter().any(|p| {
                        p.translation.is_some()
                            && p.translation
                                .as_ref()
                                .is_some_and(|(t, _)| !t.trim().is_empty())
                    }) {
                        match crate::lrc_generator::generate_lrc_from_paragraphs(
                            &paragraphs_for_gen,
                            LrcContentType::Translation,
                        ) {
                            Ok(generated_lrc) if !generated_lrc.trim().is_empty() => {
                                lqe_extracted_translation_lrc_content = Some(generated_lrc);
                                // 尝试从段落的翻译信息中或元数据存储中获取语言代码
                                lqe_translation_language = paragraphs_for_gen
                                    .iter()
                                    .find_map(|p| {
                                        p.translation
                                            .as_ref()
                                            .and_then(|(_, lang_opt)| lang_opt.clone())
                                            .filter(|s| !s.is_empty())
                                    })
                                    .or_else(|| {
                                        store_guard_for_lrc_gen
                                            .get_single_value_by_str("translation_language")
                                            .cloned()
                                    })
                                    .or_else(|| {
                                        store_guard_for_lrc_gen
                                            .get_single_value(&CanonicalMetadataKey::Language)
                                            .cloned()
                                    });
                                log::info!(
                                    "[LQE生成准备] 从TTML段落生成翻译LRC。语言: {:?}",
                                    lqe_translation_language
                                );
                            }
                            Ok(_) => log::info!("[LQE生成准备] 从TTML段落生成的翻译LRC为空。"),
                            Err(e) => log::error!("[LQE生成准备] 从TTML段落生成翻译LRC失败: {}", e),
                        }
                    } else {
                        log::info!("[LQE生成准备] TTML段落中无翻译信息，翻译LRC部分将为空。");
                    }
                }

                // --- 准备罗马音LRC内容和语言代码 (逻辑与翻译LRC类似) ---
                if let Some(loaded_lines) = &self.loaded_romanization_lrc {
                    let mut lrc_text_body = String::new();
                    for l_line in loaded_lines {
                        let _ = writeln!(
                            lrc_text_body,
                            "{}{}",
                            crate::utils::format_lrc_time_ms(l_line.timestamp_ms),
                            l_line.text.trim()
                        );
                    }
                    if !lrc_text_body.trim().is_empty() {
                        lqe_extracted_romanization_lrc_content =
                            Some(lrc_text_body.trim_end_matches('\n').to_string() + "\n");
                    }
                    lqe_romanization_language = store_guard_for_lrc_gen
                        .get_single_value_by_str("romanization_language")
                        .cloned();
                    log::info!(
                        "[LQE生成准备] 使用手动加载的罗马音LRC。语言: {:?}",
                        lqe_romanization_language
                    );
                } else if paragraphs_for_gen.iter().any(|p| {
                    p.romanization.is_some()
                        && p.romanization
                            .as_ref()
                            .is_some_and(|r| !r.trim().is_empty())
                }) {
                    match crate::lrc_generator::generate_lrc_from_paragraphs(
                        &paragraphs_for_gen,
                        LrcContentType::Romanization,
                    ) {
                        Ok(generated_lrc) if !generated_lrc.trim().is_empty() => {
                            lqe_extracted_romanization_lrc_content = Some(generated_lrc);
                            lqe_romanization_language = store_guard_for_lrc_gen
                                .get_single_value_by_str("romanization_language")
                                .cloned();
                            log::info!(
                                "[LQE生成准备] 从TTML段落生成罗马音LRC。语言: {:?}",
                                lqe_romanization_language
                            );
                        }
                        Ok(_) => log::info!("[LQE生成准备] 从TTML段落生成的罗马音LRC为空。"),
                        Err(e) => {
                            log::error!("[LQE生成准备] 从TTML段落生成罗马音LRC失败: {}", e)
                        }
                    }
                } else {
                    log::info!("[LQE生成准备] TTML段落中无罗马音信息，罗马音LRC部分将为空。");
                }

                // 构建 ParsedSourceData 实例，作为传递给 LQE 生成器的数据包
                let source_data_for_lqe_gen = ParsedSourceData {
                    paragraphs: paragraphs_for_gen.to_vec(), // 主歌词段落
                    language_code: store_guard_for_lrc_gen
                        .get_single_value(&CanonicalMetadataKey::Language)
                        .cloned(), // 全局语言代码
                    songwriters: store_guard_for_lrc_gen
                        .get_multiple_values(&CanonicalMetadataKey::Songwriter)
                        .cloned()
                        .unwrap_or_default(), // 词曲作者
                    agent_names: self
                        .session_platform_metadata // 从会话元数据中提取演唱者信息 (v1, v2等)
                        .iter()
                        .filter(|(k, _)| {
                            k.starts_with('v')
                                && (k.len() == 2 || k.len() == 5)
                                && k[1..].chars().all(char::is_numeric)
                        })
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    apple_music_id: store_guard_for_lrc_gen
                        .get_single_value(&CanonicalMetadataKey::AppleMusicId)
                        .cloned()
                        .unwrap_or_default(), // Apple Music ID
                    general_metadata: vec![], // LQE的全局元数据由metadata_store直接提供给生成器，这里不重复传递
                    markers: self.current_markers.clone(), // 标记
                    is_line_timed_source: self.source_is_line_timed, // 是否逐行格式
                    raw_ttml_from_input: self.current_raw_ttml_from_input.clone(), // 原始TTML输入 (如果适用)
                    detected_formatted_input: Some(self.detected_formatted_ttml_source), // 是否检测到格式化
                    _source_translation_language: None, // LQE不直接使用这个字段

                    // 填充准备好的翻译和罗马音LRC内容及语言
                    lqe_extracted_translation_lrc_content,
                    lqe_translation_language,
                    lqe_extracted_romanization_lrc_content,
                    lqe_romanization_language,

                    // 根据源格式决定LQE主歌词部分的格式 (是LRC还是逐字)
                    lqe_main_lyrics_as_lrc: self.source_format == LyricFormat::Lrc,
                    // 如果源是LRC，并且有直接的LRC主歌词内容 (例如从网易云下载的)，则使用它
                    lqe_direct_main_lrc_content: if self.source_format == LyricFormat::Lrc
                        && self.direct_netease_main_lrc_content.is_some()
                    {
                        self.direct_netease_main_lrc_content.clone()
                    } else if self.source_format == LyricFormat::Lrc
                        && !self.input_text.is_empty()
                        && self
                            .parsed_ttml_paragraphs
                            .as_ref()
                            .is_some_and(|p| p.is_empty())
                    {
                        // 边缘情况：源是LRC，输入框有内容，但未解析出段落（可能是纯元数据LRC）。
                        // 此时，如果希望LQE的主歌词部分是这个原始LRC，可以在这里设置。
                        // 当前选择不使用，优先从段落生成或使用 direct_netease_main_lrc_content。
                        None
                    } else {
                        None
                    },
                };

                // 调用LQE生成器
                crate::lqe_generator::generate_lqe_from_intermediate_data(
                    &source_data_for_lqe_gen,
                    &store_guard_for_lrc_gen,
                )
            }
            LyricFormat::Krc => {
                // 生成KRC格式
                crate::krc_generator::generate_krc_from_ttml_data(
                    &paragraphs_for_gen,
                    &store_guard_for_lrc_gen,
                )
            }
        };

        // 处理生成结果
        match result {
            Ok(text) => {
                // 生成成功
                self.output_text = text; // 更新主输出框内容
                log::info!(
                    "[Unilyric 生成目标输出] 已成功生成目标格式 {:?} 的输出。",
                    self.target_format.to_string()
                );
            }
            Err(e) => {
                // 生成失败
                log::error!(
                    "[Unilyric 生成目标输出] 生成目标格式 {:?} 失败: {}",
                    self.target_format.to_string(),
                    e
                );
                self.output_text.clear(); // 清空输出框
                // 可以在这里向用户显示更友好的错误提示，例如通过一个状态栏或对话框
            }
        }
        // 确保在函数结束前释放元数据存储的锁 (如果 store_guard_for_lrc_gen 在这里仍然存活)
        // 由于 store_guard_for_lrc_gen 是在函数开始时获取的，它会在函数结束时自动释放。
    }

    /// 辅助方法：将QRC格式的次要歌词内容（主要是罗马音）合并到主歌词段落中。
    /// 匹配逻辑：对于每个主歌词段落，查找第一个开始时间与之在指定容差内匹配的QRC行。
    /// 一旦QRC行被匹配，则不再用于后续主歌词段落。
    ///
    /// # Arguments
    /// * `primary_paragraphs` - 可变的主歌词段落列表 (`Vec<TtmlParagraph>`)。
    /// * `qrc_content` - 包含次要歌词的完整QRC文本字符串。
    /// * `content_type` - 指示QRC内容是翻译还是罗马音 (主要用于日志和潜在的未来扩展，目前QRC主要用于罗马音)。
    ///
    /// # Returns
    /// `Result<(), ConvertError>` - 如果解析QRC内容时发生错误，则返回Err。
    fn merge_secondary_qrc_into_paragraphs_internal(
        primary_paragraphs: &mut [TtmlParagraph],
        qrc_content: &str,
        content_type: LrcContentType, // 虽然QRC主要用于罗马音，但保留类型以便日志区分
    ) -> Result<(), ConvertError> {
        // 如果没有QRC内容或没有主歌词段落，则无需操作
        if qrc_content.is_empty() || primary_paragraphs.is_empty() {
            return Ok(());
        }
        log::debug!(
            "[Unilyric 合并次要QRC] 类型 {:?}，QRC内容预览: {:.100}...", // 截断预览
            content_type,
            qrc_content
        );

        // 1. 解析传入的QRC文本内容
        let (secondary_qrc_lines, _secondary_qrc_meta) = // _secondary_qrc_meta (QRC元数据) 暂不使用
            match qrc_parser::load_qrc_from_string(qrc_content) {
                Ok(result) => result,
                Err(e) => {
                    log::error!("[Unilyric 合并次要QRC] 解析次要QRC内容失败: {}", e);
                    return Err(e); // 将解析错误向上传播
                }
            };

        if secondary_qrc_lines.is_empty() {
            log::debug!("[Unilyric 合并次要QRC] 次要QRC内容解析后为空行，不执行合并。");
            return Ok(());
        }

        // 2. 定义时间戳匹配容差
        const QRC_MATCH_TOLERANCE_MS: u64 = 15; // 15毫秒的容差，与LRC合并逻辑保持一致

        // 3. 迭代主歌词段落，为每个段落查找并合并匹配的QRC行
        let mut qrc_search_start_idx = 0; // 用于跟踪QRC行的消耗，避免重复匹配已使用的QRC行

        for primary_paragraph in &mut *primary_paragraphs {
            let para_start_ms = primary_paragraph.p_start_ms; // 当前主段落的开始时间

            // 从 qrc_search_start_idx 开始遍历次要QRC行
            for (current_qrc_idx, sec_qrc_line) in secondary_qrc_lines
                .iter()
                .enumerate()
                .skip(qrc_search_start_idx)
            {
                let sec_qrc_line_start_ms = sec_qrc_line.line_start_ms; // QRC行的开始时间

                // 计算当前QRC行开始时间与主歌词段落开始时间的绝对差值
                let diff_ms = (sec_qrc_line_start_ms as i64 - para_start_ms as i64).unsigned_abs();

                // 如果差值在容差范围内，则认为匹配成功
                if diff_ms <= QRC_MATCH_TOLERANCE_MS {
                    // 将QRC行的所有音节文本连接起来作为该行的完整文本内容。
                    // QRC本身是逐字的，但在这里我们将其视为一整行内容来匹配主歌词的行。
                    let mut combined_text_for_line = String::new();
                    if !sec_qrc_line.syllables.is_empty() {
                        let mut line_text_parts: Vec<String> = Vec::new();
                        for syl in sec_qrc_line.syllables.iter() {
                            line_text_parts.push(syl.text.clone());
                            // 注意：QRC的 LysSyllable 没有 ends_with_space 标志。
                            // 这里的简单连接假设 qrc_parser 输出的 LysSyllable.text 已包含必要的空格。
                        }
                        combined_text_for_line = line_text_parts.join("").trim().to_string(); // 连接并去除首尾多余空格
                    }

                    // 只有当连接后的文本非空时才进行设置
                    if !combined_text_for_line.is_empty() {
                        match content_type {
                            LrcContentType::Romanization => {
                                primary_paragraph.romanization =
                                    Some(combined_text_for_line.clone());
                                log::trace!(
                                    "[Unilyric 合并次要QRC] 段落 [{}ms] 匹配到罗马音QRC行 [{}ms]: '{}'",
                                    para_start_ms,
                                    sec_qrc_line_start_ms,
                                    combined_text_for_line
                                );
                            }
                            LrcContentType::Translation => {
                                // QRC通常不用于存储独立翻译行，但为保持接口一致性，也处理此情况
                                primary_paragraph.translation =
                                    Some((combined_text_for_line.clone(), None)); // QRC不携带语言代码
                                log::trace!(
                                    "[Unilyric 合并次要QRC] 段落 [{}ms] 匹配到翻译QRC行 [{}ms]: '{}'",
                                    para_start_ms,
                                    sec_qrc_line_start_ms,
                                    combined_text_for_line
                                );
                            }
                        }
                    } else if combined_text_for_line.is_empty() && sec_qrc_line.line_duration_ms > 0
                    {
                        // 如果QRC行文本为空但有持续时间（例如，一个空的QRC行 [1000,500]），
                        // 则也视为空匹配，以消耗掉这个时间点的QRC行，并设置为空字符串。
                        match content_type {
                            LrcContentType::Romanization => {
                                primary_paragraph.romanization = Some(String::new()); // 设置为空串
                                log::trace!(
                                    "[Unilyric 合并次要QRC] 段落 [{}ms] 匹配到空的罗马音QRC行 [{}ms]",
                                    para_start_ms,
                                    sec_qrc_line_start_ms
                                );
                            }
                            LrcContentType::Translation => {
                                primary_paragraph.translation = Some((String::new(), None)); // 设置为空串
                                log::trace!(
                                    "[Unilyric 合并次要QRC] 段落 [{}ms] 匹配到空的翻译QRC行 [{}ms]",
                                    para_start_ms,
                                    sec_qrc_line_start_ms
                                );
                            }
                        }
                    }

                    // "消耗"掉这条QRC行：更新下次QRC搜索的起始索引
                    qrc_search_start_idx = current_qrc_idx + 1;
                    break; // 找到匹配后，处理下一个主歌词段落
                }

                // 如果当前QRC行的时间戳已经远超当前主歌词段落开始时间+容差，并且尚未为该主段落找到匹配，
                // 那么后续的QRC行（时间更晚）也不太可能匹配当前主段落的开始时间。
                if sec_qrc_line_start_ms > para_start_ms + QRC_MATCH_TOLERANCE_MS {
                    break; // 提前结束对当前主段落的QRC行搜索
                }
            } // 内层 for current_qrc_idx 循环结束
        } // 外层 for para_idx 循环结束
        Ok(())
    }

    /// 辅助方法：将LRC格式的次要歌词内容（翻译或罗马音）合并到主歌词段落中。
    /// 匹配逻辑：对于每个主歌词段落，查找第一个开始时间与之在指定容差内匹配的LRC行。
    /// 一旦LRC行被匹配，则不再用于后续主歌词段落。
    ///
    /// # Arguments
    /// * `primary_paragraphs` - 可变的主歌词段落列表 (`Vec<TtmlParagraph>`)。
    /// * `lrc_content` - 包含次要歌词的完整LRC文本字符串。
    ///   对于来自QQ音乐的翻译，此字符串应已通过 `preprocess_qq_translation_lrc_content` 处理。
    /// * `content_type` - 指示LRC内容是翻译 (`LrcContentType::Translation`) 还是罗马音 (`LrcContentType::Romanization`)。
    /// * `language_code_from_lrc_meta` - 从LRC文件头部元数据中解析出的可选语言代码 (主要用于翻译)。
    ///
    /// # Returns
    /// `Result<(), ConvertError>` - 如果解析LRC内容时发生错误，则返回Err。
    fn merge_lrc_lines_into_paragraphs_internal(
        primary_paragraphs: &mut [TtmlParagraph],
        lrc_content: &str,
        content_type: LrcContentType,
        language_code_from_lrc_meta: Option<String>, // 从LRC元数据（如[language:xx]）传入的语言代码
    ) -> Result<(), ConvertError> {
        // 如果没有LRC内容或没有主歌词段落，则无需操作
        if lrc_content.is_empty() || primary_paragraphs.is_empty() {
            return Ok(());
        }

        // 1. 解析传入的LRC文本内容
        // lrc_parser 会返回解析出的LrcLine列表和从LRC头部提取的元数据
        let (lrc_lines, parsed_lrc_meta) = // parsed_lrc_meta 是 Vec<AssMetadata>
            match lrc_parser::parse_lrc_text_to_lines(lrc_content) {
                Ok(result) => result,
                Err(e) => {
                    log::error!("[Unilyric 合并次要LRC] 解析次要LRC内容失败: {}", e);
                    return Err(e); // 将解析错误向上传播
                }
            };

        if lrc_lines.is_empty() {
            log::debug!("[Unilyric 合并次要LRC] 次要LRC内容解析后为空行，不执行合并。");
            return Ok(());
        }

        // 2. 确定用于翻译的最终语言代码
        // 优先级：函数参数传入的 (`language_code_from_lrc_meta`) > LRC文件头部自动解析的
        let final_language_code_for_translation: Option<String> = if content_type
            == LrcContentType::Translation
        {
            language_code_from_lrc_meta.or_else(|| {
                // 如果参数中没有，则尝试从解析出的LRC元数据中查找 "language" 或 "lang" 标签
                parsed_lrc_meta
                    .iter()
                    .find(|m| {
                        m.key.eq_ignore_ascii_case("language") || m.key.eq_ignore_ascii_case("lang")
                    })
                    .map(|m| m.value.clone())
            })
        } else {
            None // 罗马音通常不携带语言代码，或由LQE生成器等后续步骤处理默认值
        };

        // 3. 定义时间戳匹配容差
        const LRC_MATCH_TOLERANCE_MS: u64 = 15; // 15毫秒的容差

        // 4. 迭代主歌词段落，为每个段落查找并合并匹配的LRC行
        let mut lrc_search_start_idx = 0; // 用于跟踪LRC行的消耗，避免重复匹配已使用的LRC行

        for primary_paragraph in primary_paragraphs {
            let para_start_ms = primary_paragraph.p_start_ms; // 当前主段落的开始时间

            // 从 lrc_search_start_idx 开始遍历LRC行
            for (current_lrc_idx, lrc_line) in
                lrc_lines.iter().enumerate().skip(lrc_search_start_idx)
            {
                let lrc_ts = lrc_line.timestamp_ms; // 当前LRC行的时间戳

                // 计算当前LRC行时间戳与主歌词段落开始时间的绝对差值
                let diff_ms = (lrc_ts as i64 - para_start_ms as i64).unsigned_abs();

                // 如果差值在容差范围内，则认为匹配成功
                if diff_ms <= LRC_MATCH_TOLERANCE_MS {
                    // 获取LRC行的文本。如果此LRC内容来自QQ音乐的翻译且原为"//"，
                    // `preprocess_qq_translation_lrc_content` 应已将其转换为空字符串。
                    let text_to_set = lrc_line.text.clone();

                    // 根据内容类型（翻译或罗马音）更新主歌词段落
                    match content_type {
                        LrcContentType::Romanization => {
                            primary_paragraph.romanization = Some(text_to_set);
                            log::trace!(
                                "[Unilyric 合并次要LRC] 段落 [{}ms] 匹配到罗马音LRC行 [{}ms]: '{}'",
                                para_start_ms,
                                lrc_ts,
                                primary_paragraph.romanization.as_deref().unwrap_or("")
                            );
                        }
                        LrcContentType::Translation => {
                            primary_paragraph.translation =
                                Some((text_to_set, final_language_code_for_translation.clone()));
                            log::trace!(
                                "[Unilyric 合并次要LRC] 段落 [{}ms] 匹配到翻译LRC行 [{}ms]: '{}' (语言: {:?})",
                                para_start_ms,
                                lrc_ts,
                                primary_paragraph
                                    .translation
                                    .as_ref()
                                    .map_or("", |(t, _)| t),
                                final_language_code_for_translation
                            );
                        }
                    }
                    // "消耗"掉这条LRC行：更新下次LRC搜索的起始索引
                    lrc_search_start_idx = current_lrc_idx + 1;
                    break; // 找到匹配后，停止为当前主歌词段落搜索LRC行，移至下一个主段落
                }

                // 优化：由于LRC行已按时间排序，如果当前LRC行的时间戳已经
                // 远超当前主歌词段落开始时间+容差，并且尚未为该主段落找到匹配，
                // 那么后续的LRC行（时间更晚）也不可能匹配当前主段落的开始时间。
                // 因此，可以提前结束对当前主段落的LRC行搜索。
                if lrc_ts > para_start_ms + LRC_MATCH_TOLERANCE_MS {
                    break;
                }
            } // 内层 for current_lrc_idx 循环结束
        } // 外层 for para_idx 循环结束
        Ok(())
    }
} // impl UniLyricApp 结束

// 实现 eframe::App trait，使 UniLyricApp 能够作为 egui 应用程序运行。
impl eframe::App for UniLyricApp {
    /// `update` 方法是 eframe 应用的核心，每一帧都会调用此方法来处理事件、更新状态和绘制UI。
    ///
    /// # Arguments
    /// * `ctx` - `&egui::Context`，egui的上下文，用于UI绘制和事件处理。
    /// * `_frame` - `&mut eframe::Frame`，eframe的窗口框架，可用于控制窗口属性等 (此处未使用，故用_标记)。
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // --- 处理从日志后端接收到的日志 ---
        let mut new_logs_received_this_frame = false; // 标记本帧是否收到新日志
        let mut has_warn_or_higher = false; // 标记本帧收到的日志中是否有警告或更高级别
        while let Ok(log_entry) = self.ui_log_receiver.try_recv() {
            // 非阻塞地尝试接收日志
            if self.log_display_buffer.len() >= 200 {
                // 限制日志缓冲区大小
                self.log_display_buffer.remove(0); // 移除最旧的日志
            }
            // 检查日志等级，如果达到警告级别，则标记
            if log_entry.level >= crate::logger::LogLevel::Warn {
                has_warn_or_higher = true;
            }
            self.log_display_buffer.push(log_entry); // 将新日志添加到显示缓冲区
            new_logs_received_this_frame = true;
        }
        // 如果收到警告或更高级别的日志，则自动显示日志面板
        if has_warn_or_higher {
            self.show_bottom_log_panel = true;
        } else if new_logs_received_this_frame && !self.show_bottom_log_panel {
            // 如果收到新日志但日志面板未显示，则标记有新的触发性日志 (例如，用于在按钮上显示提示)
            self.new_trigger_log_exists = true;
        }

        // --- 处理网络下载完成事件 ---
        self.handle_qq_download_completion(); // 检查并处理QQ音乐下载完成
        self.handle_kugou_download_completion(); // 检查并处理酷狗音乐下载完成
        self.handle_netease_download_completion(); // 检查并处理网易云音乐下载完成

        // (下面这行似乎是之前尝试自动打开日志面板的逻辑，当前行为是警告级别才自动打开)
        // if self.new_trigger_log_exists && !self.show_bottom_log_panel {
        //     //self.show_bottom_log_panel = true;
        // }

        // --- 更新UI面板的显示状态 ---
        // 如果当前有标记点数据，则显示标记点面板
        self.show_markers_panel = !self.current_markers.is_empty();
        // 如果加载了翻译LRC文件或翻译LRC预览内容非空，则显示翻译LRC面板
        self.show_translation_lrc_panel = self.loaded_translation_lrc.is_some()
            || !self.display_translation_lrc_output.is_empty();
        // 如果加载了罗马音LRC文件或罗马音LRC预览内容非空，则显示罗马音LRC面板
        self.show_romanization_lrc_panel = self.loaded_romanization_lrc.is_some()
            || !self.display_romanization_lrc_output.is_empty();

        // --- 处理文件拖放相关的状态更新 ---
        // 获取鼠标指针当前悬停位置 (如果正在拖动文件)
        if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
            self.last_known_pointer_pos_while_dragging = Some(pos);
        }
        // 检查是否有文件正悬停在窗口上
        let files_are_hovering_window_now = !ctx.input(|i| i.raw.hovered_files.is_empty());
        // 检查本帧是否有文件被放下 (拖放操作完成)
        let files_were_dropped_this_frame = !ctx.input(|i| i.raw.dropped_files.is_empty());

        if files_are_hovering_window_now && !files_were_dropped_this_frame {
            // 如果有文件悬停但尚未放下，则标记窗口正被文件悬停
            self.is_any_file_hovering_window = true;
        } else if !files_are_hovering_window_now && !files_were_dropped_this_frame {
            // 如果既无文件悬停也无文件放下 (例如，拖动结束在窗口外，或从未开始拖动)
            self.is_any_file_hovering_window = false;
            // 如果鼠标指针已离开窗口，清除最后已知的拖动指针位置
            if ctx.input(|i| i.pointer.hover_pos().is_none()) {
                self.last_known_pointer_pos_while_dragging = None;
            }
        }

        // --- 绘制UI主要面板 ---
        // 绘制顶部工具栏
        egui::TopBottomPanel::top("top_panel_id").show(ctx, |ui| {
            self.draw_toolbar(ui); // 调用工具栏绘制函数 (定义在 ui_toolbar.rs)
        });
        // 绘制底部日志面板 (如果 self.show_bottom_log_panel 为 true)
        self.draw_log_panel(ctx); // 调用日志面板绘制函数 (定义在 ui_log_panel.rs)

        // 计算各个侧边面板的推荐宽度，基于屏幕宽度动态调整
        let available_width = ctx.screen_rect().width();
        let input_panel_width = (available_width * 0.25).clamp(200.0, 400.0); // 输入面板宽度
        let lrc_panel_width = (available_width * 0.20).clamp(150.0, 350.0); // LRC预览面板宽度
        let markers_panel_width = (available_width * 0.18).clamp(120.0, 300.0); // 标记面板宽度

        // 绘制左侧输入面板
        egui::SidePanel::left("input_panel")
            .default_width(input_panel_width)
            .show(ctx, |ui| {
                self.draw_input_panel_contents(ui); // 调用输入面板内容绘制函数 (定义在 ui_input_panel.rs)
            });

        // 根据状态绘制右侧的各种可选面板
        if self.show_markers_panel {
            // 如果显示标记点面板
            egui::SidePanel::right("markers_panel")
                .default_width(markers_panel_width)
                .show(ctx, |ui| {
                    self.draw_markers_panel_contents(ui, self.wrap_text);
                });
        }
        if self.show_translation_lrc_panel {
            // 如果显示翻译LRC面板
            egui::SidePanel::right("translation_lrc_panel")
                .default_width(lrc_panel_width)
                .show(ctx, |ui| {
                    self.draw_translation_lrc_panel_contents(ui);
                });
        }
        if self.show_romanization_lrc_panel {
            // 如果显示罗马音LRC面板
            egui::SidePanel::right("romanization_lrc_panel")
                .default_width(lrc_panel_width)
                .show(ctx, |ui| {
                    self.draw_romanization_lrc_panel_contents(ui);
                });
        }

        // 绘制中央输出面板 (占据剩余空间)
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_output_panel_contents(ui); // 调用输出面板内容绘制函数 (定义在 ui_output_panel.rs)
        });

        // --- 绘制模态窗口和覆盖层 ---
        // 绘制元数据编辑窗口 (如果 self.show_metadata_panel 为 true)
        if self.show_metadata_panel {
            let mut show_metadata_panel_mut = self.show_metadata_panel; // 可变绑定，用于窗口的 open 参数
            let mut window_is_still_open = show_metadata_panel_mut; // 跟踪窗口是否仍然打开
            egui::Window::new("编辑元数据") // 窗口标题
                .open(&mut window_is_still_open) // 绑定到窗口的打开/关闭状态
                .default_width(450.0) // 默认宽度
                .default_height(400.0) // 默认高度
                .resizable(true) // 可调整大小
                .collapsible(true) // 可折叠
                .max_height(600.0) // 最大高度
                .show(ctx, |ui| {
                    // 调用元数据编辑器窗口内容绘制函数 (定义在 ui_metadata_editor.rs)
                    self.draw_metadata_editor_window_contents(ui, &mut show_metadata_panel_mut);
                });
            // 如果用户关闭了窗口 (window_is_still_open 变为 false)，则更新状态
            if !window_is_still_open {
                show_metadata_panel_mut = false;
            }
            self.show_metadata_panel = show_metadata_panel_mut; // 将窗口状态同步回应用状态
        }

        // --- 处理文件拖放逻辑 ---
        let files_are_hovered = !ctx.input(|i| i.raw.hovered_files.is_empty()); // 是否有文件悬停
        let mut is_dropping_file_this_frame = false; // 标记本帧是否有文件被放下

        if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
            // 如果本帧有文件被放下
            is_dropping_file_this_frame = true; // 标记正在放下文件
            let files = ctx.input(|i| i.raw.dropped_files.clone()); // 获取被放下的文件列表
            if let Some(file) = files.first() {
                // 只处理第一个拖放的文件
                if let Some(path) = &file.path {
                    // 如果文件有路径 (来自文件系统)
                    // 调用文件加载和转换函数 (定义在 io.rs)
                    crate::io::load_file_and_convert(self, path.clone())
                } else if let Some(bytes) = &file.bytes {
                    // 如果文件是字节数据 (例如从浏览器拖放的文本片段)
                    if let Ok(text_content) = String::from_utf8(bytes.to_vec()) {
                        //尝试从字节解码为UTF-8文本
                        self.clear_all_data(); // 清理旧数据
                        self.input_text = text_content; // 设置新内容到输入框
                        // 对于拖放的文本片段，可能难以确定其原始格式。
                        // 这里简单地使用当前选择的源格式，并触发转换。
                        self.metadata_source_is_download = false; // 标记为本地内容，非网络下载
                        self.handle_convert(); // 触发转换流程
                    } else {
                        log::warn!("[Unilyric] 拖放的字节数据不是有效的UTF-8文本，无法处理。");
                    }
                }
            }
        }

        // 绘制设置窗口 (如果 self.show_settings_window 为 true)
        if self.show_settings_window {
            self.draw_settings_window(ctx);
        }

        // 绘制各个网络下载的模态窗口 (如果其显示标志为 true)
        self.draw_qqmusic_download_modal_window(ctx);
        self.draw_kugou_download_modal_window(ctx);
        self.draw_netease_download_modal_window(ctx);

        // 如果有文件悬停在窗口上，并且本帧没有文件被放下，则显示拖放覆盖提示
        if files_are_hovered && !is_dropping_file_this_frame {
            egui::Area::new("drag_drop_overlay_area".into()) // 创建一个覆盖整个屏幕的区域
                .fixed_pos(egui::Pos2::ZERO) // 位置从 (0,0) 开始
                .order(egui::Order::Foreground) // 确保在最顶层绘制
                .show(ctx, |ui_overlay| {
                    let screen_rect = ui_overlay.ctx().screen_rect(); // 获取屏幕矩形
                    ui_overlay.set_clip_rect(screen_rect); // 设置裁剪区域为整个屏幕

                    // 绘制半透明背景
                    ui_overlay.painter().rect_filled(
                        screen_rect,
                        0.0,                                              // 圆角半径
                        Color32::from_rgba_unmultiplied(20, 20, 20, 190), // 深灰色半透明
                    );

                    // 在屏幕中央绘制提示文本
                    ui_overlay.painter().text(
                        screen_rect.center(),             // 文本位置
                        egui::Align2::CENTER_CENTER,      // 对齐方式
                        "拖放到此处以加载",               // 提示文本
                        egui::FontId::proportional(50.0), // 字体大小
                        Color32::WHITE,                   // 文本颜色
                    );
                });
        }
    } // update 方法结束
} // impl eframe::App 结束
