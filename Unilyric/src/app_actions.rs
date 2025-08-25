use crate::app_settings::AppSettings;
use crate::error::AppResult;
use crate::types::LrcContentType;
use lyrics_helper_core::FullConversionResult;
use lyrics_helper_core::LyricFormat;
use lyrics_helper_core::LyricsAndMetadata;
use lyrics_helper_core::SearchResult;
use lyrics_helper_core::model::track::FullLyricsResult;
use smtc_suite::SmtcControlCommand;

// 主事件枚举
#[derive(Debug, Clone)]
pub enum UserAction {
    File(FileAction),
    Lyrics(Box<LyricsAction>),
    Player(PlayerAction),
    UI(UIAction),
    Settings(SettingsAction),
    AmllConnector(AmllConnectorAction),
}

// 子事件枚举定义
#[derive(Debug, Clone)]
pub enum FileAction {
    Open,
    Save,
    LoadTranslationLrc,
    LoadRomanizationLrc,
}

#[derive(Debug, Clone)]
pub enum LyricsAction {
    Convert,
    ConvertCompleted(AppResult<FullConversionResult>),
    ConvertChinese(ferrous_opencc::config::BuiltinConfig),
    Search,
    SearchCompleted(AppResult<Vec<SearchResult>>),
    Download(SearchResult),
    DownloadCompleted(AppResult<FullLyricsResult>),
    SourceFormatChanged(LyricFormat),
    TargetFormatChanged(LyricFormat),
    MetadataChanged,                         // 元数据被用户编辑
    AddMetadata,                             // 添加新的元数据条目
    DeleteMetadata(usize),                   // 删除指定索引的元数据条目
    UpdateMetadataKey(usize, String),        // 更新指定索引的元数据键
    UpdateMetadataValue(usize, String),      // 更新指定索引的元数据值
    ToggleMetadataPinned(usize),             // 切换指定索引的元数据固定状态
    LrcInputChanged(String, LrcContentType), // 当LRC文本框内容改变时
    MainInputChanged(String),                // 当主输入文本框内容改变时
    ClearAllData,
    LoadFetchedResult(FullLyricsResult),
    ApplyFetchedLyrics(Box<LyricsAndMetadata>), // 应用获取到的歌词
    LoadFileContent(String, std::path::PathBuf),
    ApplyProcessor(ProcessorType),
}

#[derive(Debug, Clone)]
pub enum PlayerAction {
    /// 向 smtc-suite 发送一个媒体控制命令。
    Control(SmtcControlCommand),
    /// 让 smtc-suite 选择一个新的媒体会话。
    SelectSmtcSession(String),
    /// 保存当前歌词到本地缓存。
    SaveToLocalCache,
    /// 更新封面数据。
    UpdateCover(Option<Vec<u8>>),
    /// 控制 smtc-suite 的音频捕获功能
    ToggleAudioCapture(bool),
}

#[derive(Debug, Clone)]
pub enum PanelType {
    Settings,
    Metadata,
    Search,
    Log,
    Markers,
    Translation,
    Romanization,
    AmllConnector,
}

#[derive(Debug, Clone)]
pub enum UIAction {
    SetPanelVisibility(PanelType, bool),
    SetWrapText(bool),
    ShowPanel(PanelType),
    HidePanel(PanelType),
    ClearLogs,
    StopOtherSearches,
}

#[derive(Debug, Clone)]
pub enum SettingsAction {
    Save(Box<AppSettings>),
    Cancel,
    Reset,
}

#[derive(Debug, Clone)]
pub enum AmllConnectorAction {
    Connect,
    Disconnect,
    Retry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorType {
    MetadataStripper,
    SyllableSmoother,
    AgentRecognizer,
}
