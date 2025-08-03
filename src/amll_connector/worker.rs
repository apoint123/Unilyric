use std::{sync::mpsc::Sender as StdSender, time::Duration};

use smtc_suite::{MediaCommand, MediaUpdate, SmtcControlCommand};
use tokio::sync::{
    mpsc::{
        Receiver as TokioReceiver, Sender as TokioSender, channel as tokio_channel,
        error::TrySendError,
    },
    oneshot,
};

use crate::amll_connector::types::{ActorSettings, UiUpdate};

use super::{
    protocol::{Artist, ClientMessage},
    translation::convert_to_protocol_lyrics,
    types::{AMLLConnectorConfig, ConnectorCommand, ConnectorUpdate, WebsocketStatus},
    websocket_client,
};

const CHANNEL_BUFFER_SIZE: usize = 32;
const WEBSOCKET_SHUTDOWN_TIMEOUT_MS: u64 = 100;

struct ActorState {
    config: AMLLConnectorConfig,
    actor_settings: ActorSettings,
    websocket_client: Option<WebSocketClientState>,
    last_track_info: Option<smtc_suite::NowPlayingInfo>,
}

/// WebSocket 客户端的运行时状态
/// 这三个字段总是同时为 Some 或 None，表示客户端是否正在运行
struct WebSocketClientState {
    /// 向 WebSocket 客户端发送消息的通道
    outgoing_tx: TokioSender<ClientMessage>,
    /// 用于关闭 WebSocket 客户端的信号发送器
    shutdown_signal_tx: oneshot::Sender<()>,
    /// WebSocket 客户端任务的句柄
    client_handle: tokio::task::JoinHandle<()>,
}

impl WebSocketClientState {
    /// 创建新的 WebSocket 客户端状态
    fn new(
        outgoing_tx: TokioSender<ClientMessage>,
        shutdown_signal_tx: oneshot::Sender<()>,
        client_handle: tokio::task::JoinHandle<()>,
    ) -> Self {
        Self {
            outgoing_tx,
            shutdown_signal_tx,
            client_handle,
        }
    }

    /// 检查客户端是否正在运行
    fn is_running(&self) -> bool {
        !self.client_handle.is_finished()
    }

    /// 获取发送通道的引用
    fn outgoing_tx(&self) -> &TokioSender<ClientMessage> {
        &self.outgoing_tx
    }

    /// 发送关闭信号给客户端
    fn send_shutdown_signal(self) -> Result<tokio::task::JoinHandle<()>, ()> {
        match self.shutdown_signal_tx.send(()) {
            Ok(_) => {
                tracing::debug!("[WebSocket Client] 关闭信号已发送");
                Ok(self.client_handle)
            }
            Err(_) => {
                tracing::debug!("[WebSocket Client] 关闭信号发送失败，客户端可能已关闭");
                Ok(self.client_handle)
            }
        }
    }

    /// 关闭客户端
    async fn shutdown(self) {
        match self.send_shutdown_signal() {
            Ok(handle) => {
                let timeout_future =
                    tokio::time::sleep(Duration::from_millis(WEBSOCKET_SHUTDOWN_TIMEOUT_MS));
                tokio::select! {
                    result = handle => {
                        match result {
                            Ok(_) => tracing::debug!("[WebSocket Client] 客户端已正常退出"),
                            Err(e) if e.is_cancelled() => tracing::debug!("[WebSocket Client] 客户端被取消"),
                            Err(e) => tracing::warn!("[WebSocket Client] 客户端异常退出: {}", e),
                        }
                    }
                    _ = timeout_future => {
                        tracing::warn!("[WebSocket Client] 客户端未在预期时间内退出");
                    }
                }
            }
            Err(_) => {
                tracing::error!("[WebSocket Client] 无法发送关闭信号");
            }
        }
    }
}

fn handle_websocket_send_error<T>(result: Result<(), TrySendError<T>>, message_type: &str) {
    match result {
        Ok(_) => {}
        Err(TrySendError::Full(_)) => {
            tracing::warn!(
                "[AMLL Actor] WebSocket 发送队列已满，丢弃 {} 消息",
                message_type
            );
        }
        Err(TrySendError::Closed(_)) => {
            tracing::error!(
                "[AMLL Actor] WebSocket 客户端通道已关闭，无法发送 {} 消息",
                message_type
            );
        }
    }
}

async fn handle_smtc_send_error<T>(
    result: Result<(), tokio::sync::mpsc::error::SendError<T>>,
    command_type: &str,
) {
    if let Err(e) = result {
        tracing::error!("[AMLL Actor] 向 SMTC 发送 {} 命令失败: {}", command_type, e);
    }
}

fn send_music_info_to_ws(client: &WebSocketClientState, info: &smtc_suite::NowPlayingInfo) {
    let artists_vec = info.artist.as_ref().map_or_else(Vec::new, |name| {
        vec![Artist {
            id: Default::default(),
            name: name.as_str().into(),
        }]
    });
    let msg = ClientMessage::SetMusicInfo {
        music_id: Default::default(),
        music_name: info
            .title
            .clone()
            .map_or(Default::default(), |s| s.as_str().into()),
        album_id: Default::default(),
        album_name: info
            .album_title
            .clone()
            .map_or(Default::default(), |s| s.as_str().into()),
        artists: artists_vec,
        duration: info.duration_ms.unwrap_or(0),
    };
    handle_websocket_send_error(client.outgoing_tx().try_send(msg), "SetMusicInfo");
}

fn send_cover_to_ws(client: &WebSocketClientState, cover_data: &[u8]) {
    if !cover_data.is_empty() {
        let msg = ClientMessage::SetMusicAlbumCoverImageData {
            data: cover_data.to_vec(),
        };
        handle_websocket_send_error(
            client.outgoing_tx().try_send(msg),
            "SetMusicAlbumCoverImageData",
        );
    }
}

fn send_play_state_to_ws(client: &WebSocketClientState, info: &smtc_suite::NowPlayingInfo) {
    if let Some(is_playing) = info.is_playing {
        let msg = if is_playing {
            ClientMessage::OnResumed
        } else {
            ClientMessage::OnPaused
        };
        handle_websocket_send_error(client.outgoing_tx().try_send(msg), "播放状态");
    }
}

fn send_progress_to_ws(client: &WebSocketClientState, info: &smtc_suite::NowPlayingInfo) {
    if let Some(progress) = info.position_ms {
        let msg = ClientMessage::OnPlayProgress { progress };
        handle_websocket_send_error(client.outgoing_tx().try_send(msg), "OnPlayProgress");
    }
}

pub async fn amll_connector_actor(
    mut command_rx: TokioReceiver<ConnectorCommand>,
    update_tx: StdSender<UiUpdate>,
    initial_config: AMLLConnectorConfig,
    smtc_command_tx: TokioSender<MediaCommand>,
    mut smtc_update_rx: TokioReceiver<MediaUpdate>,
) {
    tracing::debug!("[AMLL Actor] Actor 任务已启动。");

    let mut state = ActorState {
        config: initial_config,
        actor_settings: ActorSettings {},
        websocket_client: None,
        last_track_info: None,
    };

    let (ws_status_tx, mut ws_status_rx) = tokio_channel(CHANNEL_BUFFER_SIZE);
    let (media_cmd_tx, mut media_cmd_rx) = tokio_channel(CHANNEL_BUFFER_SIZE);

    if state.config.enabled {
        match start_websocket_client_task(&state.config, ws_status_tx.clone(), media_cmd_tx.clone())
        {
            Ok((outgoing_tx, client_handle, shutdown_signal_tx)) => {
                state.websocket_client = Some(WebSocketClientState::new(
                    outgoing_tx,
                    shutdown_signal_tx,
                    client_handle,
                ));
                tracing::debug!("[AMLL Actor] WebSocket 客户端初始化成功");
            }
            Err(e) => {
                tracing::error!("[AMLL Actor] 初始化 WebSocket 客户端失败: {}", e);

                let update = UiUpdate {
                    payload: ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::断开),
                    repaint_needed: true,
                };

                let _ = update_tx.send(update);
            }
        }
    }

    tracing::debug!("[AMLL Actor] 已启动并进入主事件循环。");

    loop {
        tokio::select! {
            Some(command) = command_rx.recv() => {
                if matches!(command, ConnectorCommand::Shutdown) {
                    if let Some(client) = state.websocket_client.take() {
                        client.shutdown().await;
                    }
                    break;
                }
                handle_app_command(command, &mut state, &ws_status_tx, &media_cmd_tx, &update_tx).await;
            },

            Some(status) = ws_status_rx.recv() => {
                handle_websocket_status(status, &update_tx, &smtc_command_tx).await;
            },

            Some(media_cmd) = media_cmd_rx.recv() => {
                handle_player_control_command(media_cmd, &smtc_command_tx).await;
            },

            Some(update) = smtc_update_rx.recv() => {
                handle_smtc_update(update, &mut state, &update_tx);
            },
        }
    }
}

fn handle_new_song(new_info: &smtc_suite::NowPlayingInfo, state: &ActorState) {
    tracing::debug!("[AMLL Actor] 检测到新歌曲，重置并发送元数据。");
    if let Some(client) = &state.websocket_client {
        send_music_info_to_ws(client, new_info);
        if let Some(ref cover_data) = new_info.cover_data {
            send_cover_to_ws(client, cover_data);
        }
    }
}

fn handle_progress_update(new_info: &smtc_suite::NowPlayingInfo, state: &ActorState) -> bool {
    let mut repaint_needed = false;
    let last_info = state.last_track_info.as_ref();

    if let Some(client) = &state.websocket_client {
        let cover_changed = new_info.cover_data.is_some()
            && new_info.cover_data != last_info.and_then(|i| i.cover_data.as_ref()).cloned();
        if cover_changed {
            if let Some(ref cover_data) = new_info.cover_data {
                send_cover_to_ws(client, cover_data);
            }
            repaint_needed = true;
        }

        let playing_state_changed = new_info.is_playing != last_info.and_then(|i| i.is_playing);
        if playing_state_changed {
            send_play_state_to_ws(client, new_info);
            repaint_needed = true;
        }

        send_progress_to_ws(client, new_info);
    }

    repaint_needed
}

fn handle_smtc_update(
    update: MediaUpdate,
    state: &mut ActorState,
    update_tx: &StdSender<UiUpdate>,
) -> bool {
    let mut repaint_needed = false;

    let payload = match update {
        MediaUpdate::TrackChanged(new_info) => {
            let is_new_song = match &state.last_track_info {
                Some(cached) => cached.title != new_info.title || cached.artist != new_info.artist,
                None => true,
            };

            if is_new_song {
                handle_new_song(&new_info, state);
                repaint_needed = true;
            } else {
                repaint_needed = handle_progress_update(&new_info, state);
            }
            state.last_track_info = Some(new_info.clone());
            ConnectorUpdate::SmtcUpdate(MediaUpdate::TrackChanged(new_info))
        }
        other_update => {
            if matches!(
                other_update,
                MediaUpdate::SessionsChanged(_) | MediaUpdate::SelectedSessionVanished(_)
            ) {
                state.last_track_info = None;
                repaint_needed = true;
            }
            ConnectorUpdate::SmtcUpdate(other_update)
        }
    };

    let ui_update = UiUpdate {
        payload,
        repaint_needed,
    };
    let _ = update_tx.send(ui_update);

    repaint_needed
}

async fn handle_app_command(
    command: ConnectorCommand,
    state: &mut ActorState,
    ws_status_tx: &TokioSender<WebsocketStatus>,
    media_cmd_tx: &TokioSender<SmtcControlCommand>,
    update_tx: &StdSender<UiUpdate>,
) {
    match command {
        ConnectorCommand::Shutdown => {}
        ConnectorCommand::UpdateConfig(new_config) => {
            let old_config = state.config.clone();
            state.config = new_config;
            let should_be_running = state.config.enabled;
            let is_running = state
                .websocket_client
                .as_ref()
                .is_some_and(|client| client.is_running());
            let url_changed = old_config.websocket_url != state.config.websocket_url;

            if should_be_running && (!is_running || url_changed) {
                tracing::info!("[AMLL Actor] 配置已更改，正在启动/重启...");
                if let Some(old_client) = state.websocket_client.take() {
                    old_client.shutdown().await;
                }
                match start_websocket_client_task(
                    &state.config,
                    ws_status_tx.clone(),
                    media_cmd_tx.clone(),
                ) {
                    Ok((outgoing_tx, client_handle, shutdown_signal_tx)) => {
                        state.websocket_client = Some(WebSocketClientState::new(
                            outgoing_tx,
                            shutdown_signal_tx,
                            client_handle,
                        ));
                        tracing::debug!("[AMLL Actor] WebSocket 客户端已成功启动");
                    }
                    Err(e) => {
                        tracing::error!("[AMLL Actor] 启动 WebSocket 客户端失败: {}", e);
                        let ui_update = UiUpdate {
                            payload: ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::断开),
                            repaint_needed: true,
                        };
                        let _ = update_tx.send(ui_update);
                    }
                }
            } else if !should_be_running && is_running {
                tracing::debug!("[AMLL Actor] 配置已禁用，正在停止客户端...");
                if let Some(client) = state.websocket_client.take() {
                    client.shutdown().await;
                }
            }
        }
        ConnectorCommand::UpdateActorSettings(new_settings) => {
            tracing::debug!("[AMLL Actor] 收到设置更新: {:?}", new_settings);
            state.actor_settings = new_settings;
        }
        ConnectorCommand::DisconnectWebsocket => {
            tracing::debug!("[AMLL Actor] 收到 Disconnect 命令，正在关闭 WebSocket 客户端...");
            if let Some(client) = state.websocket_client.take() {
                client.shutdown().await;
            }

            let update = UiUpdate {
                payload: ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::断开),
                repaint_needed: true,
            };

            if update_tx.send(update).is_err() {
                tracing::warn!(
                    "[AMLL Actor] 发送 WebSocket 断开状态更新到 UI 线程失败，UI 可能已关闭"
                );
            }
        }
        ConnectorCommand::SendLyric(parsed_data) => {
            if let Some(client) = &state.websocket_client {
                let protocol_lyrics = convert_to_protocol_lyrics(&parsed_data);
                let body = ClientMessage::SetLyric {
                    data: protocol_lyrics,
                };
                handle_websocket_send_error(client.outgoing_tx().try_send(body), "SetLyric");
            } else {
                tracing::debug!("[AMLL Actor] WebSocket 客户端未连接，忽略 SendLyric 命令");
            }
        }
        ConnectorCommand::SendClientMessage(message) => {
            if let Some(client) = &state.websocket_client {
                handle_websocket_send_error(
                    client.outgoing_tx().try_send(message),
                    "ClientMessage",
                );
            } else {
                tracing::debug!("[AMLL Actor] WebSocket 客户端未连接，忽略 SendClientMessage 命令");
            }
        }
        ConnectorCommand::SendCover(cover_data) => {
            if let Some(client) = &state.websocket_client {
                tracing::info!(
                    "[AMLL Actor] 发送封面到 WebSocket，大小: {} bytes",
                    cover_data.len()
                );
                send_cover_to_ws(client, &cover_data);
            } else {
                tracing::debug!("[AMLL Actor] WebSocket 客户端未连接，忽略 SendCover 命令");
            }
        }
    }
}

async fn handle_websocket_status(
    status: WebsocketStatus,
    update_tx: &StdSender<UiUpdate>,
    smtc_command_tx: &TokioSender<MediaCommand>,
) {
    tracing::debug!("[AMLL Actor] 收到 WebSocket 状态更新: {:?}", status);
    let enable_high_freq = matches!(status, WebsocketStatus::已连接);
    let command = MediaCommand::SetHighFrequencyProgressUpdates(enable_high_freq);
    handle_smtc_send_error(smtc_command_tx.send(command).await, "高频更新开关").await;
    let ui_update = UiUpdate {
        payload: ConnectorUpdate::WebsocketStatusChanged(status),
        repaint_needed: true,
    };
    let _ = update_tx.send(ui_update);
}

async fn handle_player_control_command(
    media_cmd: SmtcControlCommand,
    smtc_command_tx: &TokioSender<MediaCommand>,
) {
    tracing::info!("[AMLL Actor] 从客户端收到媒体命令: {:?}", media_cmd);
    let command_to_send = MediaCommand::Control(media_cmd);
    handle_smtc_send_error(
        smtc_command_tx.send(command_to_send).await,
        "来自WebSocket的控制命令",
    )
    .await;
}

type WebSocketClientTask = (
    TokioSender<ClientMessage>,
    tokio::task::JoinHandle<()>,
    oneshot::Sender<()>,
);

fn start_websocket_client_task(
    config: &AMLLConnectorConfig,
    status_tx: TokioSender<WebsocketStatus>,
    media_cmd_tx: TokioSender<SmtcControlCommand>,
) -> Result<WebSocketClientTask, Box<dyn std::error::Error + Send + Sync>> {
    if config.websocket_url.is_empty() {
        return Err("WebSocket URL 不能为空".into());
    }

    let (ws_outgoing_tx, ws_outgoing_rx) = tokio_channel(CHANNEL_BUFFER_SIZE);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let url = config.websocket_url.clone();

    if !url.starts_with("ws://") && !url.starts_with("wss://") {
        return Err(format!("无效的 WebSocket URL 格式: {}", url).into());
    }

    let handle = tokio::spawn(async move {
        websocket_client::run_websocket_client(
            url,
            ws_outgoing_rx,
            status_tx,
            media_cmd_tx,
            shutdown_rx,
        )
        .await;
    });

    Ok((ws_outgoing_tx, handle, shutdown_tx))
}
