use crate::amll_connector::types::SmtcSessionInfo;
use crate::amll_connector::{NowPlayingInfo, WebsocketStatus};
use crate::app_settings::AppSettings;
use crate::types::LrcContentType;
use lyrics_helper_rs::SearchResult;
use lyrics_helper_rs::converter::LyricFormat;
use lyrics_helper_rs::model::track::FullLyricsResult;

// 主事件枚举
#[derive(Debug, Clone)]
pub enum UserAction {
    File(FileAction),
    Lyrics(LyricsAction),
    Player(PlayerAction),
    UI(UIAction),
    Settings(SettingsAction),
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
    ConvertCompleted(Result<lyrics_helper_rs::converter::types::FullConversionResult, String>), // 转换完成
    ConvertChinese(String),
    Search,
    SearchCompleted(Result<Vec<SearchResult>, String>), // 搜索完成
    Download(SearchResult),
    DownloadCompleted(Result<FullLyricsResult, String>), // 下载完成
    SourceFormatChanged(LyricFormat),
    TargetFormatChanged(LyricFormat),
    MetadataChanged,                                   // 元数据被用户编辑
    AddMetadata,                                       // 添加新的元数据条目
    DeleteMetadata(usize),                             // 删除指定索引的元数据条目
    UpdateMetadataKey(usize, String),                  // 更新指定索引的元数据键
    UpdateMetadataValue(usize, String),                // 更新指定索引的元数据值
    ToggleMetadataPinned(usize),                       // 切换指定索引的元数据固定状态
    AutoFetchCompleted(crate::types::AutoFetchResult), // 自动获取完成
    LrcInputChanged(String, LrcContentType),           // 当LRC文本框内容改变时
    MainInputChanged(String),                          // 当主输入文本框内容改变时
    ClearAllData,
}

#[derive(Debug, Clone)]
pub enum PlayerAction {
    WebsocketStatusChanged(WebsocketStatus),
    SmtcTrackChanged(NowPlayingInfo),
    SmtcSessionListChanged(Vec<SmtcSessionInfo>),
    SelectedSmtcSessionVanished(String),
    AudioVolumeChanged { volume: f32, is_muted: bool },
    SimulatedProgressUpdate(u64),

    // 播放器相关操作，暂时为空
    ConnectAmll,
    DisconnectAmll,
    SelectSmtcSession(String),
    SaveToLocalCache,
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
}

#[derive(Debug, Clone)]
pub enum SettingsAction {
    Save(AppSettings),
    Cancel,
    Reset,
}
