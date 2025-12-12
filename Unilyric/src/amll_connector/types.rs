use lyrics_helper_core::converter::types::ParsedSourceData;
use serde::{Deserialize, Serialize};
use smtc_suite::MediaUpdate;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AMLLConnectorConfig {
    pub enabled: bool,
    pub websocket_url: String,
}

impl Default for AMLLConnectorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            websocket_url: "ws://localhost:11444".to_string(),
        }
    }
}

/// WebSocket 连接状态的枚举
#[derive(Debug, Clone, PartialEq, Default)]
pub enum WebsocketStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct ActorSettings {}

/// Unilyric 主应用发送给 amll_connector worker 的命令
#[derive(Debug, Clone)]
pub enum ConnectorCommand {
    UpdateConfig(AMLLConnectorConfig),
    UpdateActorSettings(ActorSettings),
    SendLyric(ParsedSourceData),
    SendCover(Vec<u8>),
    Shutdown,
    DisconnectWebsocket,
}

/// amll_connector worker 发送给 Unilyric 主应用的更新/事件
#[derive(Debug, Clone)]
pub enum ConnectorUpdate {
    WebsocketStatusChanged(WebsocketStatus),
    SmtcUpdate(MediaUpdate),
}

/// 发送给 UI 的更新包
#[derive(Debug, Clone)]
pub struct UiUpdate {
    pub payload: ConnectorUpdate,
    pub repaint_needed: bool,
}
