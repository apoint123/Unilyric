use std::{
    sync::mpsc::Sender as StdSender,
    time::{Duration, Instant},
};

use anyhow::anyhow;
use futures_util::FutureExt;
use smtc_suite::{MediaCommand, MediaUpdate, RepeatMode as SmtcRepeatMode, SmtcControlCommand};
use tokio::{
    sync::{
        broadcast,
        mpsc::{
            Receiver as TokioReceiver, Sender as TokioSender, channel as tokio_channel,
            error::TrySendError,
        },
        oneshot,
    },
    task::{JoinError, JoinHandle},
};
use tracing::{debug, error, info, warn};

use crate::amll_connector::{
    protocol_v2::*,
    types::{ActorSettings, ConnectorMode, UiUpdate},
    websocket_server,
};

use super::{
    translation::convert_to_protocol_lyrics,
    types::{AMLLConnectorConfig, ConnectorCommand, ConnectorUpdate, WebsocketStatus},
    websocket_client,
};

type ClientTaskComponents = (
    TokioSender<OutgoingMessage>,
    oneshot::Sender<()>,
    JoinHandle<anyhow::Result<()>>,
);

type ServerTaskComponents = (
    broadcast::Sender<OutgoingMessage>,
    TokioReceiver<()>,
    oneshot::Sender<()>,
    JoinHandle<anyhow::Result<()>>,
);

enum StateFutureResult {
    TaskFinished(Result<anyhow::Result<()>, JoinError>),
}

const CHANNEL_BUFFER_SIZE: usize = 32;

enum PostShutdownAction {
    DoNothing,
    Restart,
}

enum RunningConnection {
    Client {
        tx: TokioSender<OutgoingMessage>,
    },
    Server {
        broadcast_tx: broadcast::Sender<OutgoingMessage>,
    },
}

enum ConnectionState {
    Disconnected,
    Running {
        conn_type: RunningConnection,
        shutdown_tx: oneshot::Sender<()>,
        handle: JoinHandle<anyhow::Result<()>>,
    },
    ShuttingDown {
        handle: JoinHandle<anyhow::Result<()>>,
        next_action: PostShutdownAction,
    },
}

struct ActorState {
    config: AMLLConnectorConfig,
    actor_settings: ActorSettings,
    connection: ConnectionState,
    server_new_conn_rx: Option<TokioReceiver<()>>,
    session_ready: bool,
    last_track_info: Option<smtc_suite::NowPlayingInfo>,
    last_audio_sent_time: Option<Instant>,
    last_lyric_sent: Option<LyricContent>,
}

fn send_outgoing_message(state: &mut ActorState, msg: OutgoingMessage, msg_type_log: &str) {
    if let ConnectionState::Running { conn_type, .. } = &mut state.connection {
        match conn_type {
            RunningConnection::Client { tx } => match tx.try_send(msg) {
                Ok(_) => {}
                Err(TrySendError::Full(_)) => {
                    warn!("[AMLL Actor] 客户端发送队列已满，丢弃 {msg_type_log}",)
                }
                Err(TrySendError::Closed(_)) => debug!("[AMLL Actor] 客户端通道已关闭"),
            },
            RunningConnection::Server { broadcast_tx, .. } => {
                if let Err(_e) = broadcast_tx.send(msg) {
                    // trace!("[AMLL Actor] 广播消息无人接收: {}", _e);
                }
            }
        }
    }
}

async fn handle_smtc_send_error<T>(
    result: Result<(), tokio::sync::mpsc::error::SendError<T>>,
    command_type: &str,
) {
    if let Err(e) = result {
        error!("[AMLL Actor] 向 SMTC 发送 {} 命令失败: {}", command_type, e);
    }
}

fn send_music_info_to_ws(state: &mut ActorState, info: &smtc_suite::NowPlayingInfo) {
    let artists_vec = info.artist.as_ref().map_or_else(Vec::new, |name| {
        vec![Artist {
            id: Default::default(),
            name: name.as_str().into(),
        }]
    });
    let music_info = MusicInfo {
        music_id: Default::default(),
        music_name: info.title.clone().unwrap_or_default(),
        album_id: Default::default(),
        album_name: info.album_title.clone().unwrap_or_default(),
        artists: artists_vec,
        duration: info.duration_ms.unwrap_or(0),
    };

    let payload = Payload::State(StateUpdate::SetMusic(music_info));
    let msg = OutgoingMessage::Json(MessageV2 { payload });
    send_outgoing_message(state, msg, "SetMusic");
}

fn send_cover_to_ws(state: &mut ActorState, cover_data: &[u8]) {
    if !cover_data.is_empty() {
        let bin_body = BinaryV2::SetCoverData {
            data: cover_data.to_vec(),
        };
        let msg = OutgoingMessage::Binary(bin_body);
        send_outgoing_message(state, msg, "SetCoverData");
    }
}

fn send_play_state_to_ws(state: &mut ActorState, info: &smtc_suite::NowPlayingInfo) {
    if let Some(status) = info.playback_status {
        let state_update = match status {
            smtc_suite::PlaybackStatus::Playing => StateUpdate::Resumed,
            smtc_suite::PlaybackStatus::Paused | smtc_suite::PlaybackStatus::Stopped => {
                StateUpdate::Paused
            }
        };
        let payload = Payload::State(state_update);
        let msg = OutgoingMessage::Json(MessageV2 { payload });
        send_outgoing_message(state, msg, "播放状态");
    }
}

fn send_progress_to_ws(state: &mut ActorState, info: &smtc_suite::NowPlayingInfo) {
    if let Some(progress) = info.position_ms {
        let payload = Payload::State(StateUpdate::Progress { progress });
        let msg = OutgoingMessage::Json(MessageV2 { payload });
        send_outgoing_message(state, msg, "Progress");
    }
}

fn start_websocket_client_task(
    config: &AMLLConnectorConfig,
    status_tx: TokioSender<WebsocketStatus>,
    media_cmd_tx: TokioSender<SmtcControlCommand>,
) -> Result<ClientTaskComponents, anyhow::Error> {
    if config.websocket_url.is_empty() {
        return Err(anyhow!("WebSocket URL 不能为空"));
    }
    if !config.websocket_url.starts_with("ws://") && !config.websocket_url.starts_with("wss://") {
        return Err(anyhow!(
            "无效的 WebSocket URL 格式: {}",
            config.websocket_url
        ));
    }

    let (ws_outgoing_tx, ws_outgoing_rx) = tokio_channel(CHANNEL_BUFFER_SIZE);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let url = config.websocket_url.clone();

    let handle = tokio::spawn(async move {
        websocket_client::run_websocket_client(
            url,
            ws_outgoing_rx,
            status_tx,
            media_cmd_tx,
            shutdown_rx,
        )
        .await
    });

    Ok((ws_outgoing_tx, shutdown_tx, handle))
}

fn start_websocket_server_task(
    config: &AMLLConnectorConfig,
    media_cmd_tx: TokioSender<SmtcControlCommand>,
) -> Result<ServerTaskComponents, anyhow::Error> {
    let port = config.server_port;
    if port == 0 {
        return Err(anyhow!("无效的服务端端口"));
    }

    let (broadcast_tx, broadcast_rx) = broadcast::channel(64);
    let (new_conn_tx, new_conn_rx) = tokio_channel(16);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let handle = tokio::spawn(async move {
        websocket_server::run_websocket_server(
            port,
            broadcast_rx,
            media_cmd_tx,
            new_conn_tx,
            shutdown_rx,
        )
        .await
    });

    Ok((broadcast_tx, new_conn_rx, shutdown_tx, handle))
}

fn try_start_connection(
    state: &mut ActorState,
    status_tx: &TokioSender<WebsocketStatus>,
    media_cmd_tx: &TokioSender<SmtcControlCommand>,
    update_tx: &StdSender<UiUpdate>,
) {
    if !state.config.enabled {
        return;
    }
    if matches!(state.connection, ConnectionState::Running { .. }) {
        return;
    }

    match state.config.mode {
        ConnectorMode::Client => {
            match start_websocket_client_task(
                &state.config,
                status_tx.clone(),
                media_cmd_tx.clone(),
            ) {
                Ok((tx, shutdown_tx, handle)) => {
                    state.connection = ConnectionState::Running {
                        conn_type: RunningConnection::Client { tx },
                        shutdown_tx,
                        handle,
                    };
                    let _ = update_tx.send(UiUpdate {
                        payload: ConnectorUpdate::WebsocketStatusChanged(
                            WebsocketStatus::Connecting,
                        ),
                        repaint_needed: true,
                    });
                }
                Err(e) => {
                    handle_start_error(e, state, update_tx);
                }
            }
        }
        ConnectorMode::Server => {
            match start_websocket_server_task(&state.config, media_cmd_tx.clone()) {
                Ok((broadcast_tx, new_conn_rx, shutdown_tx, handle)) => {
                    state.server_new_conn_rx = Some(new_conn_rx);

                    state.connection = ConnectionState::Running {
                        conn_type: RunningConnection::Server { broadcast_tx },
                        shutdown_tx,
                        handle,
                    };
                    state.session_ready = true;
                    let _ = update_tx.send(UiUpdate {
                        payload: ConnectorUpdate::WebsocketStatusChanged(
                            WebsocketStatus::Connected,
                        ),
                        repaint_needed: true,
                    });
                }
                Err(e) => {
                    handle_start_error(e, state, update_tx);
                }
            }
        }
    }
}

fn handle_start_error(e: anyhow::Error, state: &mut ActorState, update_tx: &StdSender<UiUpdate>) {
    error!("[AMLL Actor] 启动连接任务失败: {e}");
    state.connection = ConnectionState::Disconnected;
    let _ = update_tx.send(UiUpdate {
        payload: ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::Error(format!(
            "启动失败: {}",
            e
        ))),
        repaint_needed: true,
    });
}

fn push_full_state_to_current_connection(state: &mut ActorState) {
    if let Some(track_info) = &state.last_track_info {
        let info = track_info.clone();

        send_music_info_to_ws(state, &info);
        if let Some(ref cover_data) = info.cover_data {
            send_cover_to_ws(state, cover_data);
        }
        send_play_state_to_ws(state, &info);
        send_progress_to_ws(state, &info);

        if let Some(ref lyric_content) = state.last_lyric_sent {
            let payload = Payload::State(StateUpdate::SetLyric(lyric_content.clone()));
            let msg = OutgoingMessage::Json(MessageV2 { payload });
            send_outgoing_message(state, msg, "SetLyric (全量更新)");
        }
    }
}

fn handle_smtc_update(
    update: MediaUpdate,
    state: &mut ActorState,
    update_tx: &StdSender<UiUpdate>,
) {
    if let MediaUpdate::AudioData(bytes) = update {
        const AUDIO_SEND_INTERVAL: Duration = Duration::from_millis(10);
        if let Some(last_sent) = state.last_audio_sent_time
            && last_sent.elapsed() < AUDIO_SEND_INTERVAL
        {
            return;
        }

        if let (ConnectionState::Running { .. }, true) = (&state.connection, state.session_ready) {
            let i16_byte_data = convert_f32_bytes_to_i16_bytes(&bytes);
            let msg = OutgoingMessage::Binary(BinaryV2::OnAudioData {
                data: i16_byte_data,
            });
            send_outgoing_message(state, msg, "OnAudioData");
            state.last_audio_sent_time = Some(Instant::now());
        }
        return;
    }

    let mut repaint_needed = false;
    let payload = match update {
        MediaUpdate::TrackChanged(new_info) => {
            let is_new_song = state.last_track_info.as_ref().is_none_or(|cached| {
                cached.title != new_info.title || cached.artist != new_info.artist
            });

            if let (ConnectionState::Running { .. }, true) =
                (&state.connection, state.session_ready)
            {
                if is_new_song {
                    send_music_info_to_ws(state, &new_info);
                }
                if let Some(ref cover_data) = new_info.cover_data {
                    send_cover_to_ws(state, cover_data);
                }

                let modes_changed = state.last_track_info.as_ref().is_none_or(|cached| {
                    cached.repeat_mode != new_info.repeat_mode
                        || cached.is_shuffle_active != new_info.is_shuffle_active
                });

                if modes_changed {
                    let shuffle_state = new_info.is_shuffle_active.unwrap_or(false);
                    let smtc_repeat_mode = new_info.repeat_mode.unwrap_or_default();
                    let protocol_repeat_mode = match smtc_repeat_mode {
                        SmtcRepeatMode::Off => RepeatMode::Off,
                        SmtcRepeatMode::One => RepeatMode::One,
                        SmtcRepeatMode::All => RepeatMode::All,
                    };

                    let state_update = StateUpdate::ModeChanged {
                        repeat: protocol_repeat_mode,
                        shuffle: shuffle_state,
                    };
                    let msg = OutgoingMessage::Json(MessageV2 {
                        payload: Payload::State(state_update),
                    });
                    send_outgoing_message(state, msg, "ModeChanged");
                }

                let status_changed = state
                    .last_track_info
                    .as_ref()
                    .is_none_or(|cached| cached.playback_status != new_info.playback_status);

                if status_changed {
                    send_play_state_to_ws(state, &new_info);
                }

                send_progress_to_ws(state, &new_info);
            }
            let final_info = if let Some(cached_info) = state.last_track_info.take() {
                if is_new_song {
                    *new_info.clone()
                } else {
                    let mut merged_info = *new_info.clone();
                    if merged_info.cover_data.is_none() && cached_info.cover_data.is_some() {
                        merged_info.cover_data = cached_info.cover_data;
                    }
                    merged_info
                }
            } else {
                *new_info.clone()
            };
            state.last_track_info = Some(final_info);
            repaint_needed = true;
            ConnectorUpdate::SmtcUpdate(MediaUpdate::TrackChanged(new_info))
        }
        MediaUpdate::VolumeChanged { volume, .. } => {
            if let (ConnectionState::Running { .. }, true) =
                (&state.connection, state.session_ready)
            {
                let state_update = StateUpdate::Volume {
                    volume: volume as f64,
                };
                let msg = OutgoingMessage::Json(MessageV2 {
                    payload: Payload::State(state_update),
                });
                send_outgoing_message(state, msg, "VolumeChanged");
            }
            ConnectorUpdate::SmtcUpdate(update)
        }
        other_update => {
            if matches!(
                other_update,
                MediaUpdate::SessionsChanged(_) | MediaUpdate::SelectedSessionVanished(_)
            ) {
                repaint_needed = true;
            }
            ConnectorUpdate::SmtcUpdate(other_update)
        }
    };

    let _ = update_tx.send(UiUpdate {
        payload,
        repaint_needed,
    });
}

async fn handle_app_command(
    command: ConnectorCommand,
    state: &mut ActorState,
    status_tx: &TokioSender<WebsocketStatus>,
    media_cmd_tx: &TokioSender<SmtcControlCommand>,
    update_tx: &StdSender<UiUpdate>,
) {
    match command {
        ConnectorCommand::Shutdown => {}
        ConnectorCommand::StartConnection => {
            if matches!(state.connection, ConnectionState::Disconnected) {
                try_start_connection(state, status_tx, media_cmd_tx, update_tx);
            } else {
                warn!("[AMLL Actor] 收到连接指令，但当前已在运行中");
            }
        }
        ConnectorCommand::UpdateConfig(new_config) => {
            let mode_changed = state.config.mode != new_config.mode;
            let url_changed = state.config.websocket_url != new_config.websocket_url;
            let port_changed = state.config.server_port != new_config.server_port;

            state.config = new_config;

            let is_running = matches!(state.connection, ConnectionState::Running { .. });

            if is_running
                && (mode_changed
                    || (state.config.mode == ConnectorMode::Client && url_changed)
                    || (state.config.mode == ConnectorMode::Server && port_changed))
            {
                info!("[AMLL Actor] 配置变更，正在重启...");
                if let ConnectionState::Running {
                    shutdown_tx,
                    handle,
                    ..
                } = std::mem::replace(&mut state.connection, ConnectionState::Disconnected)
                {
                    let _ = shutdown_tx.send(());
                    state.connection = ConnectionState::ShuttingDown {
                        handle,
                        next_action: PostShutdownAction::Restart,
                    };
                }
            } else if !state.config.enabled
                && is_running
                && let ConnectionState::Running {
                    shutdown_tx,
                    handle,
                    ..
                } = std::mem::replace(&mut state.connection, ConnectionState::Disconnected)
            {
                let _ = shutdown_tx.send(());
                state.connection = ConnectionState::ShuttingDown {
                    handle,
                    next_action: PostShutdownAction::DoNothing,
                };
            }
        }
        ConnectorCommand::UpdateActorSettings(new_settings) => {
            debug!("[AMLL Actor] 收到设置更新: {:?}", new_settings);
            state.actor_settings = new_settings;
        }
        ConnectorCommand::DisconnectWebsocket => {
            if let ConnectionState::Running {
                shutdown_tx,
                handle,
                ..
            } = std::mem::replace(&mut state.connection, ConnectionState::Disconnected)
            {
                let _ = shutdown_tx.send(());
                state.connection = ConnectionState::ShuttingDown {
                    handle,
                    next_action: PostShutdownAction::DoNothing,
                };
            } else {
                state.connection = ConnectionState::Disconnected;
                let _ = update_tx.send(UiUpdate {
                    payload: ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::Disconnected),
                    repaint_needed: true,
                });
            }
        }
        ConnectorCommand::SendLyric(parsed_data) => {
            let protocol_lyrics: Vec<LyricLine> = convert_to_protocol_lyrics(&parsed_data);
            let lyric_content = LyricContent::Structured {
                lines: protocol_lyrics,
            };

            let payload = Payload::State(StateUpdate::SetLyric(lyric_content.clone()));
            let msg = OutgoingMessage::Json(MessageV2 { payload });
            send_outgoing_message(state, msg, "SetLyric");

            state.last_lyric_sent = Some(lyric_content);
        }
        ConnectorCommand::SendCover(cover_data) => {
            send_cover_to_ws(state, &cover_data);
        }
    }
}

async fn handle_player_control_command(
    media_cmd: SmtcControlCommand,
    smtc_command_tx: &TokioSender<MediaCommand>,
) {
    info!("[AMLL Actor] 从客户端收到媒体命令: {:?}", media_cmd);
    let command_to_send = MediaCommand::Control(media_cmd);
    handle_smtc_send_error(
        smtc_command_tx.send(command_to_send).await,
        "来自WebSocket的控制命令",
    )
    .await;
}

pub async fn amll_connector_actor(
    mut command_rx: TokioReceiver<ConnectorCommand>,
    update_tx: StdSender<UiUpdate>,
    initial_config: AMLLConnectorConfig,
    smtc_command_tx: TokioSender<MediaCommand>,
    mut smtc_update_rx: TokioReceiver<MediaUpdate>,
) {
    let (ws_status_tx, mut ws_status_rx) = tokio_channel(CHANNEL_BUFFER_SIZE);
    let (media_cmd_tx, mut media_cmd_rx) = tokio_channel(CHANNEL_BUFFER_SIZE);

    let mut state = ActorState {
        config: initial_config,
        actor_settings: ActorSettings {},
        connection: ConnectionState::Disconnected,
        server_new_conn_rx: None,
        session_ready: false,
        last_track_info: None,
        last_audio_sent_time: None,
        last_lyric_sent: None,
    };

    loop {
        let mut server_new_conn_future = std::future::pending::<Option<()>>().left_future();

        if let Some(rx) = &mut state.server_new_conn_rx {
            server_new_conn_future = rx.recv().right_future();
        }

        tokio::select! {
            biased;

            state_result = async {
                match &mut state.connection {
                    ConnectionState::Running { handle, .. } | ConnectionState::ShuttingDown { handle, .. } => {
                        StateFutureResult::TaskFinished(handle.await)
                    }
                    ConnectionState::Disconnected => {
                        std::future::pending().await
                    }
                }
            } => {
                match state_result {
                    StateFutureResult::TaskFinished(result) => {
                        state.session_ready = false;
                        state.server_new_conn_rx = None;
                        if state.config.mode == ConnectorMode::Client {
                            let _ = smtc_command_tx.send(MediaCommand::SetHighFrequencyProgressUpdates(false)).await;
                        }

                        let previous_state = std::mem::replace(&mut state.connection, ConnectionState::Disconnected);
                        let mut next_action = PostShutdownAction::DoNothing;
                        if let ConnectionState::ShuttingDown { next_action: action, .. } = previous_state {
                            next_action = action;
                        }

                        let was_successful_close = result.as_ref().is_ok_and(|res| res.is_ok());

                        if !matches!(next_action, PostShutdownAction::Restart) && !was_successful_close {
                             match &result {
                                Ok(Err(e)) => warn!("[AMLL Actor] 连接意外断开: {e}"),
                                Err(e) => error!("[AMLL Actor] 任务 panic: {e}"),
                                _ => {}
                            }
                            let error_msg = if let Ok(Err(e)) = result { format!("连接断开: {e}") } else { "连接意外终止".to_string() };

                            let _ = update_tx.send(UiUpdate {
                                payload: ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::Error(error_msg)),
                                repaint_needed: true,
                            });
                        }

                        if let PostShutdownAction::Restart = next_action {
                            try_start_connection(&mut state, &ws_status_tx, &media_cmd_tx, &update_tx);
                        } else if was_successful_close {
                             let _ = update_tx.send(UiUpdate {
                                payload: ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::Disconnected),
                                repaint_needed: true,
                            });
                        }
                    }
                }
            },

            Some(_) = server_new_conn_future => {
                push_full_state_to_current_connection(&mut state);
            },

            Some(status) = ws_status_rx.recv() => {
                if state.config.mode == ConnectorMode::Client {
                    if let (WebsocketStatus::Connected, ConnectionState::Running { conn_type: RunningConnection::Client { tx }, .. }) = (&status, &state.connection) {
                        let _ = smtc_command_tx.send(MediaCommand::SetHighFrequencyProgressUpdates(true)).await;

                        let init_msg = OutgoingMessage::Json(MessageV2 { payload: Payload::Initialize });
                        let _ = tx.try_send(init_msg);

                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        state.session_ready = true;

                        push_full_state_to_current_connection(&mut state);
                    } else {
                        state.session_ready = false;
                         let _ = smtc_command_tx.send(MediaCommand::SetHighFrequencyProgressUpdates(false)).await;
                    }
                }

                let _ = update_tx.send(UiUpdate {
                    payload: ConnectorUpdate::WebsocketStatusChanged(status),
                    repaint_needed: true,
                });
            },

            Some(command) = command_rx.recv() => {
                if matches!(command, ConnectorCommand::Shutdown) {
                     if let ConnectionState::Running { shutdown_tx, handle, .. } = state.connection {
                        let _ = shutdown_tx.send(());
                        handle.await.ok();
                    }
                    break;
                }
                handle_app_command(command, &mut state, &ws_status_tx, &media_cmd_tx, &update_tx).await;
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

fn convert_f32_bytes_to_i16_bytes(f32_bytes: &[u8]) -> Vec<u8> {
    let spectrum_f32: Vec<f32> = f32_bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap_or_default()))
        .collect();

    let mut i16_byte_vec = Vec::with_capacity(spectrum_f32.len() * 2);

    for &sample_f32 in &spectrum_f32 {
        let i16_sample = (sample_f32.clamp(-1.0, 1.0) * 32767.0) as i16;

        i16_byte_vec.extend_from_slice(&i16_sample.to_le_bytes());
    }

    i16_byte_vec
}
