use chrono::{DateTime, Local};
use lyrics_helper_core::{LyricFormat, LyricLine, LyricsAndMetadata};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

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
    pub saved_timestamp: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LrcContentType {
    Translation,
    Romanization,
}

#[derive(Debug, Clone)]
pub enum DisplayLrcLine {
    Parsed(Box<LyricLine>),
    Raw { original_text: String },
}

#[derive(Debug, Clone)]
pub enum AutoFetchResult {
    LyricsReady {
        source: AutoSearchSource,
        lyrics_and_metadata: Box<LyricsAndMetadata>,
        output_text: String,
        title: String,
        artist: String,
    },
    CoverUpdate {
        title: String,
        artist: String,
        cover_data: Option<Vec<u8>>,
    },
    LyricsSuccess {
        source: AutoSearchSource,
        lyrics_and_metadata: Box<LyricsAndMetadata>,
        title: String,
        artist: String,
    },
    RequestCache,
    NotFound,
    FetchError(AppError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutoSearchSource {
    LocalCache,
    QqMusic,
    Kugou,
    Netease,
    AmllDb,
}

impl AutoSearchSource {
    pub fn display_name(&self) -> &'static str {
        match self {
            AutoSearchSource::LocalCache => "本地缓存",
            AutoSearchSource::QqMusic => "QQ音乐",
            AutoSearchSource::Kugou => "酷狗音乐",
            AutoSearchSource::Netease => "网易云音乐",
            AutoSearchSource::AmllDb => "AMLL-DB",
        }
    }

    pub fn default_order() -> Vec<Self> {
        vec![
            Self::LocalCache,
            Self::AmllDb,
            Self::Netease,
            Self::QqMusic,
            Self::Kugou,
        ]
    }
}

impl From<String> for AutoSearchSource {
    fn from(s: String) -> Self {
        match s.as_str() {
            "qq" => Self::QqMusic,
            "kugou" => Self::Kugou,
            "netease" => Self::Netease,
            "amll-ttml-database" => Self::AmllDb,
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
}

/// 歌词提供商的加载状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderState {
    /// 尚未初始化
    Uninitialized,
    /// 正在加载中
    Loading,
    /// 已就绪
    Ready,
    /// 加载失败
    Failed(String),
}
