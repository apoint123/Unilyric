use chrono::{DateTime, Local};
use lyrics_helper_rs::converter::types::{LyricFormat, LyricLine};
use lyrics_helper_rs::model::track::FullLyricsResult;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq)]
pub struct EditableMetadataEntry {
    pub key: String,
    pub value: String,
    pub is_pinned: bool,
    pub is_from_file: bool,
    pub id: egui::Id,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalLyricCacheEntry {
    pub smtc_title: String,
    pub smtc_artists: Vec<String>,
    pub ttml_filename: String,
    pub original_source_format: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LrcContentType {
    Translation,
    Romanization,
}

#[derive(Debug, Clone)]
pub enum DisplayLrcLine {
    Parsed(LyricLine),
    Raw { original_text: String },
}

#[derive(Debug, Clone)]
pub enum AutoFetchResult {
    Success {
        source: AutoSearchSource,
        full_lyrics_result: FullLyricsResult,
    },
    NotFound,
    FetchError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutoSearchSource {
    LocalCache,
    QqMusic,
    Kugou,
    Netease,
    AmllDb,
    Musixmatch,
}

impl AutoSearchSource {
    pub fn display_name(&self) -> &'static str {
        match self {
            AutoSearchSource::LocalCache => "本地缓存",
            AutoSearchSource::QqMusic => "QQ音乐",
            AutoSearchSource::Kugou => "酷狗音乐",
            AutoSearchSource::Netease => "网易云音乐",
            AutoSearchSource::AmllDb => "AMLL-DB",
            AutoSearchSource::Musixmatch => "Musixmatch",
        }
    }

    pub fn default_order() -> Vec<Self> {
        vec![
            Self::LocalCache,
            Self::AmllDb,
            Self::Netease,
            Self::QqMusic,
            Self::Kugou,
            Self::Musixmatch,
        ]
    }
}

pub fn search_order_to_string(order: &[AutoSearchSource]) -> String {
    order
        .iter()
        .map(|s| s.display_name())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn string_to_search_order(s: &str) -> Vec<AutoSearchSource> {
    let mut order = Vec::new();
    for name in s.split(',') {
        match name {
            "本地缓存" => order.push(AutoSearchSource::LocalCache),
            "QQ音乐" => order.push(AutoSearchSource::QqMusic),
            "酷狗音乐" => order.push(AutoSearchSource::Kugou),
            "网易云音乐" => order.push(AutoSearchSource::Netease),
            "AMLL-DB" => order.push(AutoSearchSource::AmllDb),
            _ => {}
        }
    }
    order
}

impl From<String> for AutoSearchSource {
    fn from(s: String) -> Self {
        match s.as_str() {
            "qq" => Self::QqMusic,
            "kugou" => Self::Kugou,
            "netease" => Self::Netease,
            "amll-ttml-database" => Self::AmllDb,
            "musixmatch" => Self::Musixmatch,
            _ => {
                tracing::warn!("未知的提供商名称 '{s}'，无法转换为 AutoSearchSource");
                Self::QqMusic
            }
        }
    }
}

impl From<AutoSearchSource> for &'static str {
    fn from(val: AutoSearchSource) -> Self {
        match val {
            AutoSearchSource::QqMusic => "qq",
            AutoSearchSource::Kugou => "kugou",
            AutoSearchSource::Netease => "netease",
            AutoSearchSource::AmllDb => "amll-ttml-database",
            AutoSearchSource::Musixmatch => "musixmatch",
            AutoSearchSource::LocalCache => "local",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoSearchStatus {
    NotAttempted,
    Searching,
    Success(LyricFormat),
    NotFound,
    Error(String),
}

impl Default for AutoSearchStatus {
    fn default() -> Self {
        Self::NotAttempted
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<&tracing::Level> for LogLevel {
    fn from(level: &tracing::Level) -> Self {
        match *level {
            tracing::Level::ERROR => LogLevel::Error,
            tracing::Level::WARN => LogLevel::Warn,
            tracing::Level::INFO => LogLevel::Info,
            tracing::Level::DEBUG => LogLevel::Debug,
            tracing::Level::TRACE => LogLevel::Trace,
        }
    }
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Info => "提示",
            LogLevel::Warn => "警告",
            LogLevel::Error => "错误",
            LogLevel::Debug => "调试",
            LogLevel::Trace => "追溯",
        }
    }

    pub fn color(&self) -> egui::Color32 {
        match self {
            LogLevel::Error => egui::Color32::from_rgb(255, 100, 100),
            LogLevel::Warn => egui::Color32::from_rgb(255, 200, 0),
            LogLevel::Info => egui::Color32::from_rgb(100, 180, 255),
            LogLevel::Debug => egui::Color32::from_gray(150),
            LogLevel::Trace => egui::Color32::from_gray(100),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
    pub timestamp: DateTime<Local>,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChineseConversionVariant {
    // 通用转换
    S2T, // 简体到繁体
    T2S, // 繁体到简体
    // 地区性转换
    S2TWP, // 简体到台湾正体 (含用语)
    S2HK,  // 简体到香港繁体
    TW2SP, // 繁体(台湾)到简体 (含用语)
    TW2S,  // 繁体(台湾)到简体 (仅文字)
    // 繁体互转
    TW2T, // 台湾繁体到香港繁体 (t2tw.json 的逆操作)
    HK2T, // 香港繁体到台湾繁体
    // 其他转换
    S2TW, // 简体到台湾繁体 (仅文字)
    T2TW, // 繁体到台湾繁体 (异体字)
    T2HK, // 繁体到香港繁体 (异体字)
    HK2S, // 香港繁体到简体
    // 日语汉字
    JP2T, // 日语新字体到繁体旧字体
    T2JP, // 繁体旧字体到日语新字体
}

impl ChineseConversionVariant {
    pub fn to_filename(&self) -> &'static str {
        match self {
            Self::S2T => "s2t.json",
            Self::T2S => "t2s.json",
            Self::S2TWP => "s2twp.json",
            Self::S2HK => "s2hk.json",
            Self::TW2SP => "tw2sp.json",
            Self::TW2S => "tw2s.json",
            Self::TW2T => "tw2t.json",
            Self::HK2T => "hk2t.json",
            Self::S2TW => "s2tw.json",
            Self::T2TW => "t2tw.json",
            Self::T2HK => "t2hk.json",
            Self::HK2S => "hk2s.json",
            Self::JP2T => "jp2t.json",
            Self::T2JP => "t2jp.json",
        }
    }
}
