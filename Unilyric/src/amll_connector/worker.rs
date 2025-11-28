use anyhow::anyhow;
use std::{
    pin::Pin,
    sync::mpsc::Sender as StdSender,
    time::{Duration, Instant},
};
use tracing::{debug, error, info, warn};

use smtc_suite::{MediaCommand, MediaUpdate, RepeatMode as SmtcRepeatMode, SmtcControlCommand};
use tokio::{
    sync::{
        mpsc::{
            Receiver as TokioReceiver, Sender as TokioSender, channel as tokio_channel,
            error::TrySendError,
        },
        oneshot,
    },
    task::{JoinError, JoinHandle},
    time::Sleep,
};

use crate::amll_connector::{
    protocol_v2::*,
    types::{ActorSettings, UiUpdate},
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

enum StateFutureResult {
    TaskFinished(Result<anyhow::Result<()>, JoinError>),
    RetryTimerFinished,
}

const CHANNEL_BUFFER_SIZE: usize = 32;

enum PostShutdownAction {
    DoNothing,
    Restart,
}

enum ConnectionState {
    Disconnected,
    WaitingToRetry(Pin<Box<Sleep>>),
    Running {
        tx: TokioSender<OutgoingMessage>,
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
    session_ready: bool,
    retry_attempts: u32,
    last_track_info: Option<smtc_suite::NowPlayingInfo>,
    last_audio_sent_time: Option<Instant>,
    last_lyric_sent: Option<LyricContent>,
}

fn handle_websocket_send_error<T>(result: Result<(), TrySendError<T>>, message_type: &str) {
    match result {
        Ok(_) => {}
        Err(TrySendError::Full(_)) => {
            warn!(
                "[AMLL Actor] WebSocket 发送队列已满，丢弃 {} 消息",
                message_type
            );
        }
        Err(TrySendError::Closed(_)) => {
            debug!(
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
        error!("[AMLL Actor] 向 SMTC 发送 {} 命令失败: {}", command_type, e);
    }
}

fn send_music_info_to_ws(tx: &TokioSender<OutgoingMessage>, info: &smtc_suite::NowPlayingInfo) {
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
    handle_websocket_send_error(tx.try_send(msg), "SetMusic");
}

fn send_cover_to_ws(tx: &TokioSender<OutgoingMessage>, cover_data: &[u8]) {
    if !cover_data.is_empty() {
        let bin_body = BinaryV2::SetCoverData {
            data: cover_data.to_vec(),
        };
        let msg = OutgoingMessage::Binary(bin_body);

        handle_websocket_send_error(tx.try_send(msg), "SetCoverData");
    }
}

fn send_play_state_to_ws(tx: &TokioSender<OutgoingMessage>, info: &smtc_suite::NowPlayingInfo) {
    if let Some(status) = info.playback_status {
        let state_update = match status {
            smtc_suite::PlaybackStatus::Playing => StateUpdate::Resumed,
            smtc_suite::PlaybackStatus::Paused | smtc_suite::PlaybackStatus::Stopped => {
                StateUpdate::Paused
            }
        };
        let payload = Payload::State(state_update);
        let msg = OutgoingMessage::Json(MessageV2 { payload });
        handle_websocket_send_error(tx.try_send(msg), "播放状态");
    }
}

fn send_progress_to_ws(tx: &TokioSender<OutgoingMessage>, info: &smtc_suite::NowPlayingInfo) {
    if let Some(progress) = info.position_ms {
        let payload = Payload::State(StateUpdate::Progress { progress });
        let msg = OutgoingMessage::Json(MessageV2 { payload });
        handle_websocket_send_error(tx.try_send(msg), "Progress");
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

    match start_websocket_client_task(&state.config, status_tx.clone(), media_cmd_tx.clone()) {
        Ok((tx, shutdown_tx, handle)) => {
            state.connection = ConnectionState::Running {
                tx,
                shutdown_tx,
                handle,
            };
            let _ = update_tx.send(UiUpdate {
                payload: ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::Connecting),
                repaint_needed: true,
            });
        }
        Err(e) => {
            error!("[AMLL Actor] 启动 WebSocket 客户端失败: {}", e);
            state.connection = ConnectionState::Disconnected;
            let _ = update_tx.send(UiUpdate {
                payload: ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::Error(format!(
                    "启动失败: {}",
                    e
                ))),
                repaint_needed: true,
            });
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

        if let (ConnectionState::Running { tx, .. }, true) =
            (&state.connection, state.session_ready)
        {
            let i16_byte_data = convert_f32_bytes_to_i16_bytes(&bytes);
            let msg = OutgoingMessage::Binary(BinaryV2::OnAudioData {
                data: i16_byte_data,
            });
            handle_websocket_send_error(tx.try_send(msg), "OnAudioData");
            state.last_audio_sent_time = Some(Instant::now());
        }
        return;
    }

    let mut repaint_needed = false;
    let payload = match update {
        MediaUpdate::TrackChanged(new_info) => {
            let is_new_song = state.last_track_info.as_ref().is_none_or(|cached| {
                cached.title != new_info.title
                    || cached.artist != new_info.artist
                    || cached.duration_ms != new_info.duration_ms
            });

            if let (ConnectionState::Running { tx, .. }, true) =
                (&state.connection, state.session_ready)
            {
                if is_new_song {
                    debug!("[AMLL Actor] 检测到新歌曲，发送元数据。");
                    send_music_info_to_ws(tx, &new_info);
                }

                if let Some(ref cover_data) = new_info.cover_data {
                    debug!("[AMLL Actor] 检测到封面更新，发送封面数据。");
                    send_cover_to_ws(tx, cover_data);
                }

                let modes_changed = state.last_track_info.as_ref().is_none_or(|cached| {
                    cached.repeat_mode != new_info.repeat_mode
                        || cached.is_shuffle_active != new_info.is_shuffle_active
                });

                if modes_changed {
                    debug!("[AMLL Actor] 检测到播放模式更新，发送更新。");
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

                    let payload = Payload::State(state_update);
                    let msg = OutgoingMessage::Json(MessageV2 { payload });
                    handle_websocket_send_error(tx.try_send(msg), "ModeChanged");
                }

                send_play_state_to_ws(tx, &new_info);
                send_progress_to_ws(tx, &new_info);
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
            if let (ConnectionState::Running { tx, .. }, true) =
                (&state.connection, state.session_ready)
            {
                debug!("[AMLL Actor] 检测到音量更新，发送更新。音量值: {volume:.2}",);
                let state_update = StateUpdate::Volume {
                    volume: volume as f64,
                };
                let payload = Payload::State(state_update);
                let msg = OutgoingMessage::Json(MessageV2 { payload });
                handle_websocket_send_error(tx.try_send(msg), "VolumeChanged");
            }
            ConnectorUpdate::SmtcUpdate(update)
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
    smtc_command_tx: &TokioSender<MediaCommand>,
) {
    match command {
        ConnectorCommand::Shutdown => {}
        ConnectorCommand::UpdateConfig(new_config) => {
            let should_be_running = new_config.enabled;
            let url_changed = state.config.websocket_url != new_config.websocket_url;
            state.config = new_config;

            let is_running = matches!(state.connection, ConnectionState::Running { .. });
            let is_waiting_to_retry =
                matches!(state.connection, ConnectionState::WaitingToRetry(_));

            if should_be_running && (!is_running || url_changed || is_waiting_to_retry) {
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
                } else {
                    state.retry_attempts = 0;
                    try_start_connection(state, status_tx, media_cmd_tx, update_tx);
                }
            } else if !should_be_running
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
            state.retry_attempts = 0;
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
            } else if matches!(state.connection, ConnectionState::WaitingToRetry(_)) {
                state.connection = ConnectionState::Disconnected;
                let _ = update_tx.send(UiUpdate {
                    payload: ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::Disconnected),
                    repaint_needed: true,
                });
            }
        }
        ConnectorCommand::SetProgress(progress) => {
            if let ConnectionState::Running { tx, .. } = &state.connection {
                let payload = Payload::State(StateUpdate::Progress { progress });
                let msg = OutgoingMessage::Json(MessageV2 { payload });
                handle_websocket_send_error(tx.try_send(msg), "SetProgress");
            }
        }
        ConnectorCommand::FlickerPlayPause => {
            handle_smtc_send_error(
                smtc_command_tx
                    .send(MediaCommand::Control(SmtcControlCommand::Pause))
                    .await,
                "FlickerPause",
            )
            .await;
            tokio::time::sleep(Duration::from_millis(250)).await;
            handle_smtc_send_error(
                smtc_command_tx
                    .send(MediaCommand::Control(SmtcControlCommand::Play))
                    .await,
                "FlickerPlay",
            )
            .await;
        }
        ConnectorCommand::SendLyric(parsed_data) => {
            let protocol_lyrics: Vec<LyricLine> = convert_to_protocol_lyrics(&parsed_data);
            let lyric_content = LyricContent::Structured {
                lines: protocol_lyrics,
            };

            if let ConnectionState::Running { tx, .. } = &state.connection {
                let payload = Payload::State(StateUpdate::SetLyric(lyric_content.clone()));
                let msg = OutgoingMessage::Json(MessageV2 { payload });
                handle_websocket_send_error(tx.try_send(msg), "SetLyric");
            }

            state.last_lyric_sent = Some(lyric_content);
        }
        ConnectorCommand::SendCover(cover_data) => {
            if let ConnectionState::Running { tx, .. } = &state.connection {
                send_cover_to_ws(tx, &cover_data);
            }
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
        session_ready: false,
        retry_attempts: 0,
        last_track_info: None,
        last_audio_sent_time: None,
        last_lyric_sent: None,
    };

    if state.config.enabled {
        try_start_connection(&mut state, &ws_status_tx, &media_cmd_tx, &update_tx);
    }

    loop {
        tokio::select! {
            biased;

            state_result = async {
                match &mut state.connection {
                    ConnectionState::Running { handle, .. } | ConnectionState::ShuttingDown { handle, .. } => {
                        StateFutureResult::TaskFinished(handle.await)
                    }
                    ConnectionState::WaitingToRetry(sleep) => {
                        sleep.await;
                        StateFutureResult::RetryTimerFinished
                    }
                    ConnectionState::Disconnected => {
                        std::future::pending().await
                    }
                }
            } => {
                match state_result {
                    StateFutureResult::TaskFinished(result) => {
                        state.session_ready = false;
                        let command = MediaCommand::SetHighFrequencyProgressUpdates(false);
                        handle_smtc_send_error(smtc_command_tx.send(command).await, "禁用高频更新").await;

                        let previous_state = std::mem::replace(&mut state.connection, ConnectionState::Disconnected);
                        let mut next_action = PostShutdownAction::DoNothing;
                        if let ConnectionState::ShuttingDown { next_action: action, .. } = previous_state {
                            next_action = action;
                        }

                        let was_successful_close = result.as_ref().is_ok_and(|res| res.is_ok());

                        if !matches!(next_action, PostShutdownAction::Restart) && !was_successful_close {
                            match &result {
                                Ok(Err(e)) => warn!("[AMLL Actor] WebSocket 客户端异常终止: {}", e),
                                Err(e) => error!("[AMLL Actor] WebSocket 任务 panicked: {}", e),
                                _ => {}
                            }

                            state.retry_attempts += 1;
                            const MAX_RETRIES: u32 = 1;

                            if state.retry_attempts > MAX_RETRIES {
                                error!("[AMLL Actor] 已达到最大重连次数 ({})，将停止自动重连。", MAX_RETRIES);
                                let _ = update_tx.send(UiUpdate {
                                    payload: ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::Error("已达到最大重连次数".to_string())),
                                    repaint_needed: true,
                                });
                            } else {
                                let base_delay_secs = 5;
                                let delay_secs = base_delay_secs * 2_u64.pow(state.retry_attempts - 1);
                                let reconnect_delay = Duration::from_secs(delay_secs);

                                let status_msg = format!("连接失败，正在重试 ({}/{})", state.retry_attempts, MAX_RETRIES);
                                info!("[AMLL Actor] {}将在 {:?} 后进行...", status_msg, reconnect_delay);

                                let _ = update_tx.send(UiUpdate {
                                    payload: ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::Error(status_msg)),
                                    repaint_needed: true,
                                });

                                state.connection = ConnectionState::WaitingToRetry(Box::pin(tokio::time::sleep(reconnect_delay)));
                            }
                        }else {
                            state.retry_attempts = 0;
                        }

                        match next_action {
                            PostShutdownAction::Restart => {
                                try_start_connection(&mut state, &ws_status_tx, &media_cmd_tx, &update_tx);
                            }
                            PostShutdownAction::DoNothing => {
                                let _ = update_tx.send(UiUpdate {
                                    payload: ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::Disconnected),
                                    repaint_needed: true,
                                });
                            }
                        }
                    }
                    StateFutureResult::RetryTimerFinished => {
                        state.connection = ConnectionState::Disconnected;
                        if state.config.enabled {
                            try_start_connection(&mut state, &ws_status_tx, &media_cmd_tx, &update_tx);
                        }
                    }
                }
            },

            Some(status) = ws_status_rx.recv() => {
                debug!("[AMLL Actor] 收到 WebSocket 状态更新: {:?}", status);

                if let (WebsocketStatus::Connected, ConnectionState::Running { tx, .. }) =
                    (&status, &state.connection)
                {
                    let command = MediaCommand::SetHighFrequencyProgressUpdates(true);
                    handle_smtc_send_error(smtc_command_tx.send(command).await, "启用高频更新").await;

                    let init_payload = Payload::Initialize;
                    let init_msg = OutgoingMessage::Json(MessageV2 { payload: init_payload });
                    handle_websocket_send_error(tx.try_send(init_msg), "Initialize");

                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    state.session_ready = true;

                    if let Some(track_info) = &state.last_track_info {
                        send_music_info_to_ws(tx, track_info);
                        if let Some(ref cover_data) = track_info.cover_data {
                            send_cover_to_ws(tx, cover_data);
                        }
                        send_play_state_to_ws(tx, track_info);
                        send_progress_to_ws(tx, track_info);
                        if let Some(ref lyric_content) = state.last_lyric_sent {
                            let payload = Payload::State(StateUpdate::SetLyric(lyric_content.clone()));
                            let msg = OutgoingMessage::Json(MessageV2 { payload });
                            handle_websocket_send_error(tx.try_send(msg), "SetLyric (重连)");
                        }
                    }
                } else if matches!(status, WebsocketStatus::Disconnected | WebsocketStatus::Error(_)) {
                    state.session_ready = false;
                    let command = MediaCommand::SetHighFrequencyProgressUpdates(false);
                    handle_smtc_send_error(smtc_command_tx.send(command).await, "禁用高频更新").await;
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
                handle_app_command(command, &mut state, &ws_status_tx, &media_cmd_tx, &update_tx, &smtc_command_tx).await;
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
