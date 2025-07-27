use lyrics_helper_rs::converter::types::ParsedSourceData;
use serde::{Deserialize, Serialize};
use smtc_suite::{MediaUpdate, SmtcControlCommand};

use crate::amll_connector::protocol::ClientMessage;

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
    SendLyric(ParsedSourceData),

    SendClientMessage(ClientMessage),
    Shutdown,
    DisconnectWebsocket,
}

/// amll_connector worker 发送给 Unilyric 主应用的更新/事件
#[derive(Debug, Clone)]
pub enum ConnectorUpdate {
    WebsocketStatusChanged(WebsocketStatus),
    MediaCommand(SmtcControlCommand),
    SmtcUpdate(MediaUpdate),
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
