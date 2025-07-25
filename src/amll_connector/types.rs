// 导入 serde 库，用于配置的序列化和反序列化
use serde::{Deserialize, Serialize};
use smtc_suite::{NowPlayingInfo, SmtcControlCommand};
// 导入标准库的 Instant，用于 SharedPlayerState 中的时间戳
use std::time::Instant;
// 导入 ws_protocol 库中的 Body 枚举，作为 WebSocket 消息的协议体
use ws_protocol::Body as ProtocolBody;
// 如果 Unilyric 的主日志系统有特定的 LogEntry 类型，并且希望通过 channel 传递，则需要导入
// use crate::logger::LogEntry as UnilyricLogEntry; // 假设路径

/// AMLL AMLL Connector的配置信息
/// 包含AMLL Connector的启用状态和 WebSocket URL。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AMLLConnectorConfig {
    /// AMLL Connector是否启用。如果为 false，则AMLL Connector不会尝试建立 WebSocket 连接或监听 SMTC 事件。
    pub enabled: bool,
    /// AMLL Player WebSocket 服务器的 URL。
    pub websocket_url: String,
}

impl Default for AMLLConnectorConfig {
    /// 为 `AMLLConnectorConfig` 提供默认实现。
    fn default() -> Self {
        Self {
            enabled: false,                                    // 默认不启用AMLL Connector
            websocket_url: "ws://localhost:11444".to_string(), // AMLL Player 默认的 WebSocket 地址
        }
    }
}

/// WebSocket 连接状态的枚举
#[derive(Debug, Clone, PartialEq)]
pub enum WebsocketStatus {
    /// 未连接状态。
    断开,
    /// 正在尝试连接状态。
    连接中,
    /// 已成功连接状态。
    已连接,
    /// 连接出现错误，包含具体的错误信息。
    错误(String),
}

impl Default for WebsocketStatus {
    /// 为 `WebsocketStatus` 提供默认实现，默认为 `Disconnected`。
    fn default() -> Self {
        WebsocketStatus::断开
    }
}

/// Unilyric 主应用发送给 amll_connector worker 的命令
#[derive(Debug, Clone)]
pub enum ConnectorCommand {
    UpdateConfig(AMLLConnectorConfig),
    SendLyricTtml(String),
    SendProtocolBody(ProtocolBody), // 可以用来发送任何 ws_protocol::Body
    Shutdown,
    SmtcTrackChanged(NowPlayingInfo),

    SelectSmtcSession(String),
    MediaControl(SmtcControlCommand),
    AdjustAudioSessionVolume {
        target_identifier: String,
        volume: Option<f32>,
        mute: Option<bool>,
    },
    RequestAudioSessionVolume(String),

    StartAudioVisualization,
    StopAudioVisualization,
    DisconnectWebsocket,
}

/// amll_connector worker 发送给 Unilyric 主应用的更新/事件
#[derive(Debug, Clone)]
pub enum ConnectorUpdate {
    WebsocketStatusChanged(WebsocketStatus),
    MediaCommand(SmtcControlCommand),
    NowPlayingTrackChanged(NowPlayingInfo),
    SmtcSessionListChanged(Vec<SmtcSessionInfo>),
    SelectedSmtcSessionVanished(String),
    AudioSessionVolumeChanged {
        session_id: String,
        volume: f32,
        is_muted: bool,
    },
    AudioDataPacket(Vec<u8>),
    SimulatedProgressUpdate(u64),
}

/// SMTC 共享播放器状态 (主要由 smtc_handler 内部管理)
/// 外部可能只需要读取部分状态，或者通过 NowPlayingInfo 获取。
/// 包含了当前播放媒体的详细信息和播放器的控制能力。
#[derive(Debug, Clone)]
pub struct SharedPlayerState {
    /// 歌曲标题。
    pub title: String,
    /// 艺术家名称。
    pub artist: String,
    /// 专辑名称。
    pub album: String,
    /// 专辑封面的原始二进制数据。
    pub cover_data: Option<Vec<u8>>,
    /// 专辑封面数据的哈希值。
    pub cover_data_hash: Option<u64>,
    /// 最后已知的播放位置（毫秒）。
    pub last_known_position_ms: u64,
    /// 报告 `last_known_position_ms` 的时间戳。
    pub last_known_position_report_time: Option<Instant>,
    /// 指示当前是否正在播放。
    pub is_playing: bool,
    /// 歌曲的总时长（毫秒）。
    pub song_duration_ms: u64,
    /// 播放器是否支持暂停操作。
    pub can_pause: bool,
    /// 播放器是否支持播放操作。
    pub can_play: bool,
    /// 播放器是否支持跳到下一首。
    pub can_skip_next: bool,
    /// 播放器是否支持跳到上一首。
    pub can_skip_previous: bool,
    /// 播放器是否支持跳转到指定位置。
    pub can_seek: bool,
}

impl SharedPlayerState {
    pub fn get_estimated_current_position_ms(&self) -> u64 {
        if self.is_playing {
            // 如果正在播放，根据上次报告时间和流逝的时间计算当前位置
            if let Some(report_time) = self.last_known_position_report_time {
                let elapsed_ms = report_time.elapsed().as_millis() as u64;
                let estimated_pos = self.last_known_position_ms + elapsed_ms;
                // 确保推算的位置不超过歌曲总时长
                if self.song_duration_ms > 0 {
                    std::cmp::min(estimated_pos, self.song_duration_ms)
                } else {
                    estimated_pos
                }
            } else {
                // 如果没有报告时间，只能返回已知位置
                self.last_known_position_ms
            }
        } else {
            // 如果是暂停状态，位置就是上次已知的位置
            self.last_known_position_ms
        }
    }

    pub fn reset_to_empty(&mut self) {
        *self = Self::default();
    }
}

impl Default for SharedPlayerState {
    /// 为 `SharedPlayerState` 提供默认实现。
    fn default() -> Self {
        Self {
            title: "无歌曲".to_string(), // 默认标题
            artist: String::new(),
            album: String::new(),
            cover_data: None, // 默认无封面数据
            cover_data_hash: None,
            last_known_position_ms: 0,
            last_known_position_report_time: None,
            is_playing: false, // 默认不播放
            song_duration_ms: 0,
            can_pause: false,
            can_play: false,
            can_skip_next: false,
            can_skip_previous: false,
            can_seek: false,
        }
    }
}

/// 单个 SMTC 会话的标识信息
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SmtcSessionInfo {
    /// Windows 内部的会话 ID，用于唯一标识
    pub session_id: String,
    /// 源应用的 AppUserModelId (例如 "Spotify.exe")
    pub source_app_user_model_id: String,
    /// 用于向用户显示的友好名称
    pub display_name: String,
    // 可以考虑添加一个 is_current_system_default: bool 字段，
    // 如果 smtc_handler 能够区分哪个是系统当前的默认会话。
}
