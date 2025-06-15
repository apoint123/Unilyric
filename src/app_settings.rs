use directories::ProjectDirs;
use ini::Ini;
use log::{LevelFilter, log_enabled};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use crate::types::{AutoSearchSource, LyricFormat, search_order_to_string, string_to_search_order};

const PINNED_METADATA_SECTION: &str = "PinnedMetadata";
const LOGGING_SECTION: &str = "Logging";
const AMLL_CONNECTOR_SECTION: &str = "AmllConnector";
const GENERAL_SETTINGS_SECTION: &str = "GeneralSettings";
const AUTO_SEARCH_ORDER_KEY: &str = "AutoSearchSourceOrder";
const ALWAYS_SEARCH_ALL_SOURCES_KEY: &str = "AlwaysSearchAllSources";
const MULTI_VALUE_DELIMITER: &str = ";;;";
const UI_STATE_SECTION: &str = "UiState";
const LAST_SELECTED_SMTC_SESSION_KEY: &str = "LastSelectedSmtcSessionId";

const LAST_SOURCE_FORMAT_KEY: &str = "LastSourceFormat";
const LAST_TARGET_FORMAT_KEY: &str = "LastTargetFormat";

const LYRIC_STRIPPING_SECTION: &str = "LyricStripping";
const ENABLE_ONLINE_LYRIC_STRIPPING_KEY: &str = "EnableOnlineLyricStripping";
const STRIPPING_KEYWORDS_KEY: &str = "StrippingKeywords";
const STRIPPING_CASE_SENSITIVE_KEY: &str = "StrippingKeywordCaseSensitive";

const ENABLE_TTML_REGEX_STRIPPING_KEY: &str = "EnableTtmlRegexStripping";
const TTML_STRIPPING_REGEXES_KEY: &str = "TtmlStrippingRegexes";
const TTML_REGEX_STRIPPING_CASE_SENSITIVE_KEY: &str = "TtmlRegexStrippingCaseSensitive";

const WEBSOCKET_SERVER_SECTION: &str = "WebsocketServer";
const WEBSOCKET_SERVER_ENABLED_KEY: &str = "Enabled";
const WEBSOCKET_SERVER_PORT_KEY: &str = "Port";
const SEND_AUDIO_DATA_KEY: &str = "SendAudioDataToPlayer";

const BATCH_CONVERSION_SECTION: &str = "BatchConversion";
const BATCH_OUTPUT_DIRECTORY_KEY: &str = "OutputDirectory";
const BATCH_DEFAULT_TARGET_FORMAT_KEY: &str = "DefaultTargetFormat";
const BATCH_AUTO_PAIR_ENABLED_KEY: &str = "AutoPairEnabled";
const BATCH_TRANSLATION_SUFFIXES_KEY: &str = "TranslationSuffixes";
const BATCH_ROMANIZATION_SUFFIXES_KEY: &str = "RomanizationSuffixes";

#[derive(Debug, Clone)]
pub struct LogSettings {
    pub enable_file_log: bool,
    pub file_log_level: LevelFilter,
    pub console_log_level: LevelFilter,
}

impl Default for LogSettings {
    fn default() -> Self {
        LogSettings {
            enable_file_log: false,
            file_log_level: LevelFilter::Info,
            console_log_level: LevelFilter::Info,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WebsocketServerSettings {
    pub enabled: bool,
    pub port: u16,
}

impl Default for WebsocketServerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 10086,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppSettings {
    pub log_settings: LogSettings,
    pub pinned_metadata: HashMap<String, Vec<String>>,
    pub smtc_time_offset_ms: i64,
    pub amll_connector_enabled: bool,
    pub amll_connector_websocket_url: String,
    pub auto_search_source_order: Vec<AutoSearchSource>,
    pub always_search_all_sources: bool,
    pub last_selected_smtc_session_id: Option<String>,

    // --- 歌词清理相关设置字段 (关键词部分) ---
    /// 是否启用在线下载歌词的自动清理功能
    pub enable_online_lyric_stripping: bool,
    /// 用于识别描述性行的关键词列表
    pub stripping_keywords: Vec<String>,
    /// 关键词匹配时是否区分大小写
    pub stripping_keyword_case_sensitive: bool,

    // 正则表达式移除相关设置字段
    /// 是否启用基于正则表达式的TTML段落移除
    pub enable_ttml_regex_stripping: bool,
    /// 用户定义的正则表达式字符串列表
    pub ttml_stripping_regexes: Vec<String>,
    /// 正则表达式匹配时是否区分大小写 (注意：通常正则本身可以带flag)
    pub ttml_regex_stripping_case_sensitive: bool,

    pub websocket_server_settings: WebsocketServerSettings,

    pub last_known_amll_index_head: Option<String>,
    pub checked_amll_update_since_last_success: bool,
    pub auto_check_amll_index_update_on_startup: bool,
    pub last_source_format: LyricFormat,
    pub last_target_format: LyricFormat,
    pub send_audio_data_to_player: bool,

    pub batch_output_directory: Option<PathBuf>,
    pub batch_default_target_format: Option<LyricFormat>,
    pub batch_auto_pair_enabled: bool,
    pub batch_translation_suffixes: Vec<String>,
    pub batch_romanization_suffixes: Vec<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            log_settings: LogSettings::default(),
            pinned_metadata: HashMap::new(),
            smtc_time_offset_ms: 0,
            amll_connector_enabled: false,
            amll_connector_websocket_url: "ws://localhost:11444".to_string(),
            auto_search_source_order: AutoSearchSource::default_order(),
            always_search_all_sources: false,
            last_selected_smtc_session_id: None,
            enable_online_lyric_stripping: true,
            last_known_amll_index_head: None,
            checked_amll_update_since_last_success: false,
            auto_check_amll_index_update_on_startup: true,
            send_audio_data_to_player: true,
            stripping_keywords: vec![
    "作曲".to_string(),
    "作词".to_string(),
    "编曲".to_string(),
    "演唱".to_string(),
    "歌手".to_string(),
    "歌名".to_string(),
    "专辑".to_string(),
    "发行".to_string(),
    "出品".to_string(),
    "监制".to_string(),
    "录音".to_string(),
    "混音".to_string(),
    "母带".to_string(),
    "吉他".to_string(),
    "贝斯".to_string(),
    "鼓".to_string(),
    "键盘".to_string(),
    "弦乐".to_string(),
    "和声".to_string(),
    "版权".to_string(),
    "制作人".to_string(),
    "原唱".to_string(),
    "翻唱".to_string(),
    "词".to_string(),
    "曲".to_string(),
    "发行人".to_string(),
    "宣推".to_string(),
    "录音制作".to_string(),
    "制作发行".to_string(),
    "音乐制作".to_string(),
    "录音师".to_string(),
    "混音工程师".to_string(),
    "母带工程师".to_string(),
    "制作统筹".to_string(),
    "艺术指导".to_string(),
    "出品团队".to_string(),
    "发行方".to_string(),
    "和声编写".to_string(),
    "封面设计".to_string(),
    "策划".to_string(),
    "营销推广".to_string(),
    "总策划".to_string(),
    "特别鸣谢".to_string(),
    "出品人".to_string(),
    "出品公司".to_string(),
    "联合出品".to_string(),
    "词曲提供".to_string(),
    "制作公司".to_string(),
    "推广策划".to_string(),
    "乐器演奏".to_string(),
    "钢琴/合成器演奏".to_string(),
    "钢琴演奏".to_string(),
    "合成器演奏".to_string(),
    "弦乐编写".to_string(),
    "弦乐监制".to_string(),
    "第一小提琴".to_string(),
    "第二小提琴".to_string(),
    "中提琴".to_string(),
    "大提琴".to_string(),
    "弦乐录音师".to_string(),
    "弦乐录音室".to_string(),
    "和声演唱".to_string(),
    "录/混音".to_string(),
    "制作助理".to_string(),
    "和音".to_string(),
    "乐队统筹".to_string(),
    "维伴音乐".to_string(),
    "灯光设计".to_string(),
    "配唱制作人".to_string(),
    "文案".to_string(),
    "设计".to_string(),
    "策划统筹".to_string(),
    "企划宣传".to_string(),
    "企划营销".to_string(),
    "录音室".to_string(),
    "混音室".to_string(),
    "母带后期制作人".to_string(),
    "母带后期处理工程师".to_string(),
    "母带后期处理录音室".to_string(),
    "鸣谢".to_string(),

    // --- 纯英文关键字 ---
    "OP".to_string(),
    "SP".to_string(),
    "Lyrics by".to_string(),
    "Composed by".to_string(),
    "Produced by".to_string(),
    "Published by".to_string(),
    "Vocals by".to_string(),
    "Background Vocals by".to_string(),
    "Additional Vocal by".to_string(),
    "Mixing Engineer".to_string(),
    "Mastered by".to_string(),
    "Executive Producer".to_string(),
    "Vocal Engineer".to_string(),
    "Vocals Produced by".to_string(),
    "Recorded at".to_string(),
    "Repertoire Owner".to_string(),
    "Co-Producer".to_string(),
    "Mastering Engineer".to_string(),
    "Written by".to_string(),
    "Lyrics".to_string(),
    "Composer".to_string(),
    "Arranged By".to_string(),
    "Record Producer".to_string(),
    "Guitar".to_string(),
    "Music Production".to_string(),
    "Recording Engineer".to_string(),
    "Backing Vocal".to_string(),
    "Art Director".to_string(),
    "Chief Producer".to_string(),
    "Production Team".to_string(),
    "Publisher".to_string(),
    "Lyricist".to_string(),
    "Arranger".to_string(),
    "Producer".to_string(),
    "Backing Vocals".to_string(),
    "Backing Vocals Design".to_string(),
    "Cover Design".to_string(),
    "Planner".to_string(),
    "Marketing Promotion".to_string(),
    "Chref Planner".to_string(),
    "Acknowledgement".to_string(),
    "Production Company".to_string(),
    "Jointly Produced by".to_string(),
    "Co-production".to_string(),
    "Presenter".to_string(),
    "Presented by".to_string(),
    "Co-produced by".to_string(),
    "Lyrics and Composition Provided by".to_string(),
    "Music and Lyrics Provided by".to_string(),
    "Lyrics & Composition Provided by".to_string(),
    "Words and Music by".to_string(),
    "Distribution".to_string(),
    "Release".to_string(),
    "Distributed by".to_string(),
    "Released by".to_string(),
    "Produce Company".to_string(),
    "Promotion Planning".to_string(),
    "Marketing Strategy".to_string(),
    "Promotion Strategy".to_string(),
    "Strings".to_string(),
    "First Violin".to_string(),
    "Second Violin".to_string(),
    "Viola".to_string(),
    "Cello".to_string(),
    "Vocal Producer".to_string(),
    "Supervised production".to_string(),
    "Copywriting".to_string(),
    "Design".to_string(),
    "Planner and coordinator".to_string(),
    "Propaganda".to_string(),
    "Arrangement".to_string(),
    "Guitars".to_string(),
    "Bass".to_string(),
    "Drums".to_string(),
    "Backing Vocal Arrangement".to_string(),
    "Strings Arrangement".to_string(),
    "Recording Studio".to_string(),

    "OP/发行".to_string(),
    "混音/母带工程师".to_string(),
    "OP/SP".to_string(),
    "词Lyrics".to_string(),
    "曲Composer".to_string(),
    "编曲Arranged By".to_string(),
    "制作人Record Producer".to_string(),
    "吉他Guitar".to_string(),
    "音乐制作Music Production".to_string(),
    "录音师Recording Engineer".to_string(),
    "混音工程师Mixing Engineer".to_string(),
    "母带工程师Mastering Engineer".to_string(),
    "和声Backing Vocal".to_string(),
    "制作统筹Executive Producer".to_string(),
    "艺术指导Art Director".to_string(),
    "监制Chief Producer".to_string(),
    "出品团队Production Team".to_string(),
    "发行方Publisher".to_string(),
    "词Lyricist".to_string(),
    "编曲Arranger".to_string(),
    "制作人Producer".to_string(),
    "和声Backing Vocals".to_string(),
    "和声编写Backing Vocals Design".to_string(),
    "混音Mixing Engineer".to_string(),
    "封面设计Cover Design".to_string(),
    "策划Planner".to_string(),
    "营销推广Marketing Promotion".to_string(),
    "总策划Chref Planner".to_string(),
    "特别鸣谢Acknowledgement".to_string(),
    "出品人Chief Producer".to_string(),
    "出品公司Production Company".to_string(),
    "联合出品Co-produced by".to_string(),
    "联合出品Jointly Produced by".to_string(),
    "联合出品Co-production".to_string(),
    "出品方Presenter".to_string(),
    "出品方Presented by".to_string(),
    "词曲提供Lyrics and Composition Provided by".to_string(),
    "词曲提供Music and Lyrics Provided by".to_string(),
    "词曲提供Lyrics & Composition Provided by".to_string(),
    "词曲提供Words and Music by".to_string(),
    "发行Distribution".to_string(),
    "发行Release".to_string(),
    "发行Distributed by".to_string(),
    "发行Released by".to_string(),
    "制作公司Produce Company".to_string(),
    "推广策划Promotion Planning".to_string(),
    "推广策划Marketing Strategy".to_string(),
    "推广策划Promotion Strategy".to_string(),
    "弦乐 Strings".to_string(),
    "第一小提琴 First Violin".to_string(),
    "第二小提琴 Second Violin".to_string(),
    "中提琴 Viola".to_string(),
    "大提琴 Cello".to_string(),
    "配唱制作人Vocal Producer".to_string(),
    "监制Supervised production".to_string(),
    "文案Copywriting".to_string(),
    "设计Design".to_string(),
    "策划统筹Planner and coordinator".to_string(),
    "企划宣传Propaganda".to_string(),
    "编曲Arrangement".to_string(),
    "吉他Guitars".to_string(),
    "贝斯Bass".to_string(),
    "鼓Drums".to_string(),
    "和声编写Backing Vocal Arrangement".to_string(),
    "弦乐编写Strings Arrangement".to_string(),
    "录音室Recording Studio".to_string(),
    "混音室Mixing Studio".to_string(),
    "母带后期制作人Mastering Producer".to_string(),
    "母带后期处理工程师Mastering Engineer".to_string(),
    "母带后期处理录音室Mastering Studio".to_string(),            ],
            stripping_keyword_case_sensitive: false,
            enable_ttml_regex_stripping: true,
            ttml_stripping_regexes: vec![
                "(?:【.*?未经.*?】|\\(.*?未经.*?\\)|「.*?未经.*?」|（.*?未经.*?）|『.*?未经.*?』)".to_string(),
                "(?:【.*?音乐人.*?】|\\(.*?音乐人.*?\\)|「.*?音乐人.*?」|（.*?音乐人.*?）|『.*?音乐人.*?』)".to_string(),
                ".*?未经.*?许可.*?不得.*?使用.*? ".to_string(),
                ".*?未经.*?许可.*?不得.*?方式.*? ".to_string(),
                "未经著作权人书面许可，\\s*不得以任何方式\\s*[(\\u{FF08}]包括.*?等[)\\u{FF09}]\\s*使用".to_string(),
                ".*?发行方\\s*[：:].*?".to_string(),
                ".*?(?:工作室|特别企划).*?".to_string(),
                ],
            ttml_regex_stripping_case_sensitive: false,

            websocket_server_settings: WebsocketServerSettings::default(),
            last_source_format: LyricFormat::Ass,
            last_target_format: LyricFormat::Ttml,
            batch_output_directory: None,
            batch_default_target_format: None,
            batch_auto_pair_enabled: true,
            batch_translation_suffixes: vec![
                "_tr".to_string(), 
                "_translation".to_string(), 
                "_trans".to_string(),
                ".tr".to_string(),
                ".translation".to_string(),
            ],
            batch_romanization_suffixes: vec![
                "_romaji".to_string(), 
                "_romanization".to_string(), 
                "_roma".to_string(),
                ".romaji".to_string(),
                ".romanization".to_string(),
            ],

        }
    }
}

impl AppSettings {
    pub fn config_dir() -> Option<PathBuf> {
        if let Some(proj_dirs) = ProjectDirs::from("com", "Unilyric", "Unilyric") {
            let config_dir = proj_dirs.data_local_dir();
            if !config_dir.exists()
                && let Err(e) = fs::create_dir_all(config_dir)
            {
                log::error!("无法创建配置目录 {config_dir:?}: {e}");
                return None;
            }
            Some(config_dir.to_path_buf())
        } else {
            log::error!("无法获取项目配置目录路径。");
            None
        }
    }

    fn config_file_path() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join("unilyric.ini"))
    }

    pub fn load() -> Self {
        if let Some(path) = Self::config_file_path() {
            if path.exists() {
                log::info!("[Settings] 尝试从 {path:?} 加载配置文件。");
                match Ini::load_from_file(&path) {
                    Ok(conf) => {
                        // --- 初始化默认值，用于 fallback ---
                        let defaults = AppSettings::default();

                        // --- 加载日志设置 ---
                        let log_section_opt = conf.section(Some(LOGGING_SECTION));
                        let ls = LogSettings {
                            enable_file_log: log_section_opt
                                .and_then(|s| s.get("EnableFileLog"))
                                .and_then(|s_val| s_val.parse::<bool>().ok())
                                .unwrap_or(defaults.log_settings.enable_file_log),
                            file_log_level: log_section_opt
                                .and_then(|s| s.get("FileLogLevel"))
                                .and_then(|s_val| LevelFilter::from_str(s_val).ok())
                                .unwrap_or(defaults.log_settings.file_log_level),
                            console_log_level: log_section_opt
                                .and_then(|s| s.get("ConsoleLogLevel"))
                                .and_then(|s_val| LevelFilter::from_str(s_val).ok())
                                .unwrap_or(defaults.log_settings.console_log_level),
                        };

                        // --- 加载 PinnedMetadata ---
                        let mut loaded_pinned_metadata = HashMap::new();
                        if let Some(pinned_section) = conf.section(Some(PINNED_METADATA_SECTION)) {
                            for (key, single_value_str) in pinned_section.iter() {
                                let values_vec: Vec<String> = single_value_str
                                    .split(MULTI_VALUE_DELIMITER)
                                    .map(|s_val| s_val.to_string())
                                    .collect();
                                loaded_pinned_metadata.insert(key.to_string(), values_vec);
                            }
                        }

                        // --- 加载 AMLL Connector 设置 ---
                        let connector_section_opt = conf.section(Some(AMLL_CONNECTOR_SECTION));
                        let mc_enabled = connector_section_opt
                            .and_then(|s| s.get("Enabled"))
                            .and_then(|s_val| s_val.parse::<bool>().ok())
                            .unwrap_or(defaults.amll_connector_enabled);
                        let mc_url = connector_section_opt
                            .and_then(|s| s.get("WebSocketURL"))
                            .map(|s_val| s_val.to_string())
                            .unwrap_or(defaults.amll_connector_websocket_url.clone());
                        let smtc_offset = connector_section_opt
                            .and_then(|s| s.get("SmtcTimeOffsetMs"))
                            .and_then(|s_val| s_val.parse::<i64>().ok())
                            .unwrap_or(defaults.smtc_time_offset_ms);
                        let loaded_send_audio_data = connector_section_opt
                            .and_then(|s| s.get(SEND_AUDIO_DATA_KEY))
                            .and_then(|s_val| s_val.parse::<bool>().ok())
                            .unwrap_or(defaults.send_audio_data_to_player);

                        // --- 加载通用设置 ---
                        let general_section_opt = conf.section(Some(GENERAL_SETTINGS_SECTION));
                        let loaded_search_order = general_section_opt
                            .and_then(|s| s.get(AUTO_SEARCH_ORDER_KEY))
                            .map_or_else(
                                || defaults.auto_search_source_order.clone(),
                                |s_order_ref| {
                                    if s_order_ref.trim().is_empty() {
                                        defaults.auto_search_source_order.clone()
                                    } else {
                                        string_to_search_order(s_order_ref.trim())
                                    }
                                },
                            );
                        let loaded_always_search_all = general_section_opt
                            .and_then(|s| s.get(ALWAYS_SEARCH_ALL_SOURCES_KEY))
                            .and_then(|s_val| s_val.parse::<bool>().ok())
                            .unwrap_or(defaults.always_search_all_sources);

                        // --- 加载 UI 状态 ---
                        let ui_state_section_opt = conf.section(Some(UI_STATE_SECTION));
                        let loaded_last_selected_smtc_id = ui_state_section_opt
                            .and_then(|s| s.get(LAST_SELECTED_SMTC_SESSION_KEY))
                            .map(|s_val| s_val.to_string())
                            .filter(|s| !s.is_empty()); // 如果为空字符串，则视为 None

                        // --- 加载和合并歌词清理设置 ---
                        let stripping_section_opt = conf.section(Some(LYRIC_STRIPPING_SECTION));

                        let enable_keyword_stripping = stripping_section_opt
                            .and_then(|s| s.get(ENABLE_ONLINE_LYRIC_STRIPPING_KEY))
                            .and_then(|s_val| s_val.parse::<bool>().ok())
                            .unwrap_or(defaults.enable_online_lyric_stripping);

                        // 合并 stripping_keywords
                        let mut final_stripping_keywords: Vec<String> =
                            defaults.stripping_keywords.clone();
                        let mut seen_keywords = HashSet::new();
                        for kw in &final_stripping_keywords {
                            // 将默认项预先加入 seen 集合
                            seen_keywords.insert(kw.clone());
                        }
                        if let Some(keywords_ini_str) = stripping_section_opt
                            .as_ref()
                            .and_then(|s| s.get(STRIPPING_KEYWORDS_KEY))
                        {
                            let user_keywords: Vec<String> = keywords_ini_str
                                .split(';')
                                .map(|s_val| s_val.trim().to_string())
                                .filter(|s_val| !s_val.is_empty())
                                .collect();
                            for kw in user_keywords {
                                // 只添加用户列表中新的、不重复的项
                                if seen_keywords.insert(kw.clone()) {
                                    final_stripping_keywords.push(kw);
                                }
                            }
                        }

                        let keyword_case_sensitive = stripping_section_opt
                            .and_then(|s| s.get(STRIPPING_CASE_SENSITIVE_KEY))
                            .and_then(|s_val| s_val.parse::<bool>().ok())
                            .unwrap_or(defaults.stripping_keyword_case_sensitive);

                        let enable_regex_stripping = stripping_section_opt
                            .and_then(|s| s.get(ENABLE_TTML_REGEX_STRIPPING_KEY))
                            .and_then(|s_val| s_val.parse::<bool>().ok())
                            .unwrap_or(defaults.enable_ttml_regex_stripping);

                        // 合并 ttml_stripping_regexes
                        let mut final_ttml_stripping_regexes: Vec<String> =
                            defaults.ttml_stripping_regexes.clone();
                        let mut seen_regexes = HashSet::new();
                        for re_str in &final_ttml_stripping_regexes {
                            // 将默认项预先加入 seen 集合
                            seen_regexes.insert(re_str.clone());
                        }
                        if let Some(regexes_ini_str) = stripping_section_opt
                            .as_ref()
                            .and_then(|s| s.get(TTML_STRIPPING_REGEXES_KEY))
                        {
                            let user_regexes: Vec<String> = regexes_ini_str
                                .split(';')
                                .map(|s_val| s_val.trim().to_string())
                                .filter(|s_val| !s_val.is_empty())
                                .collect();
                            for re_str in user_regexes {
                                // 只添加用户列表中新的、不重复的项
                                if seen_regexes.insert(re_str.clone()) {
                                    final_ttml_stripping_regexes.push(re_str);
                                }
                            }
                        }

                        let regex_case_sensitive = stripping_section_opt
                            .and_then(|s| s.get(TTML_REGEX_STRIPPING_CASE_SENSITIVE_KEY))
                            .and_then(|s_val| s_val.parse::<bool>().ok())
                            .unwrap_or(defaults.ttml_regex_stripping_case_sensitive);

                        let ws_server_section_opt = conf.section(Some(WEBSOCKET_SERVER_SECTION));
                        let ws_server_settings = WebsocketServerSettings {
                            enabled: ws_server_section_opt
                                .and_then(|s| s.get(WEBSOCKET_SERVER_ENABLED_KEY))
                                .and_then(|s_val| s_val.parse::<bool>().ok())
                                .unwrap_or(defaults.websocket_server_settings.enabled),
                            port: ws_server_section_opt
                                .and_then(|s| s.get(WEBSOCKET_SERVER_PORT_KEY))
                                .and_then(|s_val| s_val.parse::<u16>().ok())
                                .unwrap_or(defaults.websocket_server_settings.port),
                        };

                        let loaded_last_known_head = general_section_opt
                            .and_then(|s| s.get("LastKnownAmllIndexHead"))
                            .map(|s_val| s_val.to_string())
                            .filter(|s| !s.is_empty());

                        let loaded_checked_update_flag = general_section_opt
                            .and_then(|s| s.get("CheckedAmllUpdateSinceSuccess"))
                            .and_then(|s_val| s_val.parse::<bool>().ok())
                            .unwrap_or(defaults.checked_amll_update_since_last_success);

                        let loaded_auto_check_startup_flag = general_section_opt
                            .and_then(|s| s.get("AutoCheckAmllUpdateOnStartup"))
                            .and_then(|s_val| s_val.parse::<bool>().ok())
                            .unwrap_or(defaults.auto_check_amll_index_update_on_startup);

                        let loaded_last_source_format = general_section_opt
                            .and_then(|s| s.get(LAST_SOURCE_FORMAT_KEY))
                            .and_then(|s_val| LyricFormat::from_str(s_val).ok())
                            .unwrap_or(defaults.last_source_format);

                        let loaded_last_target_format = general_section_opt
                            .and_then(|s| s.get(LAST_TARGET_FORMAT_KEY))
                            .and_then(|s_val| LyricFormat::from_str(s_val).ok())
                            .unwrap_or(defaults.last_target_format);

                        let batch_section_opt = conf.section(Some(BATCH_CONVERSION_SECTION));

                        let loaded_batch_output_dir = batch_section_opt
                            .and_then(|s| s.get(BATCH_OUTPUT_DIRECTORY_KEY))
                            .map(PathBuf::from)
                            .filter(|p| !p.as_os_str().is_empty());

                        let loaded_batch_default_format = batch_section_opt
                            .and_then(|s| s.get(BATCH_DEFAULT_TARGET_FORMAT_KEY))
                            .and_then(|s_val| LyricFormat::from_str(s_val).ok());

                        let loaded_batch_auto_pair = batch_section_opt
                            .and_then(|s| s.get(BATCH_AUTO_PAIR_ENABLED_KEY))
                            .and_then(|s_val| s_val.parse::<bool>().ok())
                            .unwrap_or(defaults.batch_auto_pair_enabled);

                        let loaded_batch_trans_suffixes = batch_section_opt
                            .and_then(|s| s.get(BATCH_TRANSLATION_SUFFIXES_KEY))
                            .map(|s_val| {
                                s_val
                                    .split(';')
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect()
                            })
                            .unwrap_or(defaults.batch_translation_suffixes.clone());

                        let loaded_batch_roma_suffixes = batch_section_opt
                            .and_then(|s| s.get(BATCH_ROMANIZATION_SUFFIXES_KEY))
                            .map(|s_val| {
                                s_val
                                    .split(';')
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect()
                            })
                            .unwrap_or(defaults.batch_romanization_suffixes.clone());

                        // --- 构建最终的 AppSettings 实例 ---
                        let final_settings = AppSettings {
                            log_settings: ls,
                            pinned_metadata: loaded_pinned_metadata,
                            smtc_time_offset_ms: smtc_offset,
                            amll_connector_enabled: mc_enabled,
                            amll_connector_websocket_url: mc_url,
                            auto_search_source_order: loaded_search_order,
                            always_search_all_sources: loaded_always_search_all,
                            last_selected_smtc_session_id: loaded_last_selected_smtc_id,

                            enable_online_lyric_stripping: enable_keyword_stripping,
                            stripping_keywords: final_stripping_keywords,
                            stripping_keyword_case_sensitive: keyword_case_sensitive,
                            enable_ttml_regex_stripping: enable_regex_stripping,
                            ttml_stripping_regexes: final_ttml_stripping_regexes,
                            ttml_regex_stripping_case_sensitive: regex_case_sensitive,

                            websocket_server_settings: ws_server_settings,

                            last_known_amll_index_head: loaded_last_known_head,
                            checked_amll_update_since_last_success: loaded_checked_update_flag,
                            auto_check_amll_index_update_on_startup: loaded_auto_check_startup_flag,
                            last_source_format: loaded_last_source_format,
                            last_target_format: loaded_last_target_format,
                            send_audio_data_to_player: loaded_send_audio_data,

                            batch_output_directory: loaded_batch_output_dir,
                            batch_default_target_format: loaded_batch_default_format,
                            batch_auto_pair_enabled: loaded_batch_auto_pair,
                            batch_translation_suffixes: loaded_batch_trans_suffixes,
                            batch_romanization_suffixes: loaded_batch_roma_suffixes,
                        };

                        if log_enabled!(log::Level::Debug) {
                            log::debug!(
                                "[Settings] 最终加载的 AppSettings: 搜索顺序: {:?}, 总是搜索所有源: {}, 关键词数量: {}, 正则表达式数量: {}",
                                final_settings
                                    .auto_search_source_order
                                    .iter()
                                    .map(|s| s.display_name())
                                    .collect::<Vec<_>>(),
                                final_settings.always_search_all_sources,
                                final_settings.stripping_keywords.len(),
                                final_settings.ttml_stripping_regexes.len()
                            );
                        }
                        return final_settings;
                    }
                    Err(e) => {
                        log::error!("[Settings] 加载配置文件 {path:?} 失败: {e}。将使用默认配置。");
                        // 如果加载失败，仍然可以考虑保存一次默认配置，以确保文件存在且格式正确
                        // 但这里遵循原逻辑，返回默认配置
                        let defaults_on_error = AppSettings::default();
                        if defaults_on_error.save().is_err() {
                            // 尝试保存默认配置，以备下次启动
                            log::error!("[Settings] 无法在加载错误后保存默认配置文件到 {path:?}。");
                        }
                        return defaults_on_error;
                    }
                }
            } else {
                log::info!("[Settings] 配置文件 {path:?} 未找到。将创建并使用默认配置。");
                // 配置文件不存在时，直接使用默认值，并尝试保存一次
                let default_settings = AppSettings::default();
                if default_settings.save().is_err() {
                    log::error!("[Settings] 无法保存初始默认配置文件到 {path:?}。");
                }
                return default_settings;
            }
        }
        log::warn!("[Settings] 无法确定配置文件路径。将使用运行时默认配置。");
        AppSettings::default()
    }

    pub fn save(&self) -> Result<(), ini::Error> {
        if let Some(path) = Self::config_file_path() {
            let mut conf = Ini::new();
            conf.with_section(Some(LOGGING_SECTION))
                .set(
                    "EnableFileLog",
                    self.log_settings.enable_file_log.to_string(),
                )
                .set("FileLogLevel", self.log_settings.file_log_level.to_string())
                .set(
                    "ConsoleLogLevel",
                    self.log_settings.console_log_level.to_string(),
                );

            conf.delete(Some(PINNED_METADATA_SECTION));
            if !self.pinned_metadata.is_empty() {
                let mut section = conf.with_section(Some(PINNED_METADATA_SECTION));
                for (key, values_vec) in &self.pinned_metadata {
                    if !values_vec.is_empty() {
                        let single_value_str = values_vec.join(MULTI_VALUE_DELIMITER);
                        section.set(key, single_value_str);
                    }
                }
            }

            conf.with_section(Some(AMLL_CONNECTOR_SECTION))
                .set("Enabled", self.amll_connector_enabled.to_string())
                .set("WebSocketURL", &self.amll_connector_websocket_url)
                .set(
                    SEND_AUDIO_DATA_KEY,
                    self.send_audio_data_to_player.to_string(),
                )
                .set("SmtcTimeOffsetMs", self.smtc_time_offset_ms.to_string());

            let search_order_str = search_order_to_string(&self.auto_search_source_order);
            conf.with_section(Some(GENERAL_SETTINGS_SECTION))
                .set(AUTO_SEARCH_ORDER_KEY, search_order_str);

            let mut general_section = conf.with_section(Some(GENERAL_SETTINGS_SECTION));
            general_section.set(
                ALWAYS_SEARCH_ALL_SOURCES_KEY,
                self.always_search_all_sources.to_string(),
            );

            general_section.set(LAST_SOURCE_FORMAT_KEY, self.last_source_format.to_string());
            general_section.set(LAST_TARGET_FORMAT_KEY, self.last_target_format.to_string());

            let mut ui_state_section = conf.with_section(Some(UI_STATE_SECTION));
            if let Some(ref session_id) = self.last_selected_smtc_session_id {
                ui_state_section.set(LAST_SELECTED_SMTC_SESSION_KEY, session_id);
            } else {
                // 如果是 None，可以写入空字符串或删除该键
                ui_state_section.set(LAST_SELECTED_SMTC_SESSION_KEY, ""); // 保存为空字符串
                // 或者: ui_state_section.delete(LAST_SELECTED_SMTC_SESSION_KEY);
            }

            let mut stripping_section = conf.with_section(Some(LYRIC_STRIPPING_SECTION));
            stripping_section.set(
                ENABLE_ONLINE_LYRIC_STRIPPING_KEY,
                self.enable_online_lyric_stripping.to_string(),
            );
            stripping_section.set(STRIPPING_KEYWORDS_KEY, self.stripping_keywords.join(";"));
            stripping_section.set(
                STRIPPING_CASE_SENSITIVE_KEY,
                self.stripping_keyword_case_sensitive.to_string(),
            );

            stripping_section.set(
                ENABLE_TTML_REGEX_STRIPPING_KEY,
                self.enable_ttml_regex_stripping.to_string(),
            );
            stripping_section.set(
                TTML_STRIPPING_REGEXES_KEY,
                self.ttml_stripping_regexes.join(";"),
            );
            stripping_section.set(
                TTML_REGEX_STRIPPING_CASE_SENSITIVE_KEY,
                self.ttml_regex_stripping_case_sensitive.to_string(),
            );

            conf.with_section(Some(WEBSOCKET_SERVER_SECTION))
                .set(
                    WEBSOCKET_SERVER_ENABLED_KEY,
                    self.websocket_server_settings.enabled.to_string(),
                )
                .set(
                    WEBSOCKET_SERVER_PORT_KEY,
                    self.websocket_server_settings.port.to_string(),
                );

            let mut batch_section = conf.with_section(Some(BATCH_CONVERSION_SECTION));

            if let Some(ref batch_dir) = self.batch_output_directory {
                batch_section.set(BATCH_OUTPUT_DIRECTORY_KEY, batch_dir.to_string_lossy());
            }

            if let Some(ref batch_format) = self.batch_default_target_format {
                batch_section.set(BATCH_DEFAULT_TARGET_FORMAT_KEY, batch_format.to_string());
            }

            batch_section.set(
                BATCH_AUTO_PAIR_ENABLED_KEY,
                self.batch_auto_pair_enabled.to_string(),
            );

            batch_section.set(
                BATCH_TRANSLATION_SUFFIXES_KEY,
                self.batch_translation_suffixes.join(";"),
            );

            batch_section.set(
                BATCH_ROMANIZATION_SUFFIXES_KEY,
                self.batch_romanization_suffixes.join(";"),
            );

            match conf.write_to_file(&path) {
                Ok(_) => Ok(()),
                Err(write_error) => {
                    log::error!("[Settings] 保存配置到 {path:?} 失败: {write_error}");
                    Err(ini::Error::Io(write_error))
                }
            }
        } else {
            let err_msg = "[Settings] 无法确定配置文件路径，保存失败。".to_string();
            log::error!("{err_msg}");
            Err(ini::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                err_msg,
            )))
        }
    }
}
