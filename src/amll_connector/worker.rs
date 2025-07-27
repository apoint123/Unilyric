use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender as StdSender,
    },
    time::Duration,
};

use crossbeam_channel::{
    Receiver as CrossbeamReceiver, RecvTimeoutError, Sender as CrossbeamSender,
};
use smtc_suite::{MediaCommand, MediaUpdate, SmtcControlCommand};
use tokio::sync::{
    mpsc::{
        Receiver as TokioReceiver, Sender as TokioSender, channel as tokio_channel,
        error::TrySendError,
    },
    oneshot,
};

use super::{
    protocol::{Artist, ClientMessage},
    translation::convert_to_protocol_lyrics,
    types::{AMLLConnectorConfig, ConnectorCommand, ConnectorUpdate, WebsocketStatus},
    websocket_client,
};

const SMTC_BRIDGE_TIMEOUT_MS: u64 = 100;
const CHANNEL_BUFFER_SIZE: usize = 32;
const SMTC_UPDATE_BUFFER_SIZE: usize = 128;
const WEBSOCKET_SHUTDOWN_TIMEOUT_MS: u64 = 100;

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

fn handle_smtc_send_error<T>(
    result: Result<(), crossbeam_channel::SendError<T>>,
    command_type: &str,
) {
    if let Err(e) = result {
        tracing::error!("[AMLL Actor] 向 SMTC 发送 {} 命令失败: {}", command_type, e);
    }
}

fn handle_update_send_error<T>(
    result: Result<(), std::sync::mpsc::SendError<T>>,
    update_type: &str,
) {
    if let Err(e) = result {
        tracing::warn!(
            "[AMLL Actor] 发送 {} 更新到 UI 线程失败: {}，UI 可能已关闭",
            update_type,
            e
        );
    }
}

pub async fn amll_connector_actor(
    mut command_rx: TokioReceiver<ConnectorCommand>,
    update_tx: StdSender<ConnectorUpdate>,
    initial_config: AMLLConnectorConfig,
    smtc_command_tx: CrossbeamSender<MediaCommand>,
    smtc_update_rx: CrossbeamReceiver<MediaUpdate>,
) {
    tracing::debug!("[AMLL Actor] Actor 任务已启动。");
    let mut config = initial_config;
    let mut websocket_client: Option<WebSocketClientState> = None;
    let mut last_sent_title: Option<String> = None;

    let (ws_status_tx, mut ws_status_rx) = tokio_channel(CHANNEL_BUFFER_SIZE);
    let (media_cmd_tx, mut media_cmd_rx) = tokio_channel(CHANNEL_BUFFER_SIZE);

    if config.enabled {
        match start_websocket_client_task(&config, ws_status_tx.clone(), media_cmd_tx.clone()) {
            Ok((outgoing_tx, client_handle, shutdown_signal_tx)) => {
                websocket_client = Some(WebSocketClientState::new(
                    outgoing_tx,
                    shutdown_signal_tx,
                    client_handle,
                ));
                tracing::debug!("[AMLL Actor] WebSocket 客户端初始化成功");
            }
            Err(e) => {
                tracing::error!("[AMLL Actor] 初始化 WebSocket 客户端失败: {}", e);
                let _ = update_tx.send(ConnectorUpdate::WebsocketStatusChanged(
                    WebsocketStatus::断开,
                ));
            }
        }
    }

    let bridge_shutdown_signal = Arc::new(AtomicBool::new(false));

    let (smtc_update_tx_async, mut smtc_update_rx_async) = tokio_channel(SMTC_UPDATE_BUFFER_SIZE);
    let smtc_bridge_handle = {
        let signal = Arc::clone(&bridge_shutdown_signal);
        let smtc_update_rx_clone = smtc_update_rx.clone();

        tokio::spawn(async move {
            tracing::debug!("[SMTC Bridge] 桥接任务已启动。");

            loop {
                if signal.load(Ordering::Relaxed) {
                    tracing::debug!("[SMTC Bridge] 收到关闭信号，正在退出循环。");
                    break;
                }

                let recv_result = tokio::task::spawn_blocking({
                    let rx = smtc_update_rx_clone.clone();
                    move || rx.recv_timeout(Duration::from_millis(SMTC_BRIDGE_TIMEOUT_MS))
                })
                .await;

                // 回到异步上下文处理结果
                match recv_result {
                    Ok(Ok(update)) => {
                        // JoinHandle 成功, recv_timeout 成功
                        if smtc_update_tx_async.send(update).await.is_err() {
                            tracing::error!("[SMTC Bridge] 异步通道已关闭，桥接任务退出");
                            break;
                        }
                    }
                    Ok(Err(RecvTimeoutError::Timeout)) => {
                        continue;
                    }
                    Ok(Err(RecvTimeoutError::Disconnected)) => {
                        tracing::debug!("[SMTC Bridge] SMTC 更新通道已断开，桥接任务退出");
                        break;
                    }
                    Err(join_err) => {
                        // spawn_blocking 任务本身失败 (e.g., cancelled)
                        tracing::warn!(
                            "[SMTC Bridge] 桥接的阻塞任务失败: {}，桥接任务退出",
                            join_err
                        );
                        break;
                    }
                }
            }
            tracing::debug!("[SMTC Bridge] 桥接任务已完成。");
        })
    };

    tracing::debug!("[AMLL Actor] 已启动并进入主事件循环。");

    loop {
        tokio::select! {
            Some(command) = command_rx.recv() => {
                match command {
                    ConnectorCommand::Shutdown => {
                        tracing::debug!("[AMLL Actor] 已收到 Shutdown 命令。");

                        bridge_shutdown_signal.store(true, Ordering::Relaxed);

                        match smtc_bridge_handle.await {
                            Ok(_) => tracing::debug!("[AMLL Actor] SMTC 桥接任务已正常完成"),
                            Err(e) if e.is_cancelled() => tracing::debug!("[AMLL Actor] SMTC 桥接任务被取消"),
                            Err(e) => tracing::warn!("[AMLL Actor] SMTC 桥接任务异常结束: {}", e),
                        }

                        if let Some(client) = websocket_client.take() {
                            client.shutdown().await;
                        }
                        break;
                    },
                    ConnectorCommand::UpdateConfig(new_config) => {
                        let old_config = config.clone();
                        config = new_config;
                        let should_be_running = config.enabled;
                        let is_running = websocket_client
                            .as_ref()
                            .map_or(false, |client| client.is_running());
                        let url_changed = old_config.websocket_url != config.websocket_url;

                        if should_be_running && (!is_running || url_changed) {
                            tracing::info!("[AMLL Actor] 配置已更改，正在启动/重启...");

                            if let Some(old_client) = websocket_client.take() {
                                if let Ok(handle) = old_client.send_shutdown_signal() {
                                    handle.abort();
                                }
                            }

                            match start_websocket_client_task(&config, ws_status_tx.clone(), media_cmd_tx.clone()) {
                                Ok((outgoing_tx, client_handle, shutdown_signal_tx)) => {
                                    websocket_client = Some(WebSocketClientState::new(
                                        outgoing_tx,
                                        shutdown_signal_tx,
                                        client_handle,
                                    ));
                                    tracing::debug!("[AMLL Actor] WebSocket 客户端已成功启动");
                                }
                                Err(e) => {
                                    tracing::error!("[AMLL Actor] 启动 WebSocket 客户端失败: {}", e);
                                    let _ = update_tx.send(ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::断开));
                                }
                            }
                        } else if !should_be_running && is_running {
                            tracing::debug!("[AMLL Actor] 配置已禁用，正在停止客户端...");
                            if let Some(client) = websocket_client.take() {
                                if let Ok(handle) = client.send_shutdown_signal() {
                                    handle.abort();
                                }
                            }
                        }
                    },
                    ConnectorCommand::DisconnectWebsocket => {
                        tracing::debug!("[AMLL Actor] 收到 Disconnect 命令，正在关闭 WebSocket 客户端...");
                        if let Some(client) = websocket_client.take() {
                            if let Ok(handle) = client.send_shutdown_signal() {
                                handle.abort();
                            }
                        }
                        handle_update_send_error(
                            update_tx.send(ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::断开)),
                            "WebSocket断开状态"
                        );
                    },
                    ConnectorCommand::SendLyric(parsed_data) => {
                        if let Some(client) = &websocket_client {
                            let protocol_lyrics = convert_to_protocol_lyrics(&parsed_data);
                            let body = ClientMessage::SetLyric { data: protocol_lyrics };
                            handle_websocket_send_error(client.outgoing_tx().try_send(body), "SetLyric");
                        } else {
                            tracing::debug!("[AMLL Actor] WebSocket 客户端未连接，忽略 SendLyric 命令");
                        }
                    },
                    ConnectorCommand::SendClientMessage(message) => {
                        if let Some(client) = &websocket_client {
                            handle_websocket_send_error(client.outgoing_tx().try_send(message), "ClientMessage");
                        } else {
                            tracing::debug!("[AMLL Actor] WebSocket 客户端未连接，忽略 SendClientMessage 命令");
                        }
                    },
                }
            },

            Some(status) = ws_status_rx.recv() => {
                tracing::debug!("[AMLL Actor] 收到 WebSocket 状态更新: {:?}", status);
                let enable_high_freq = matches!(status, WebsocketStatus::已连接);
                let command = MediaCommand::SetHighFrequencyProgressUpdates(enable_high_freq);
                handle_smtc_send_error(smtc_command_tx.send(command), "高频更新开关");
                handle_update_send_error(
                    update_tx.send(ConnectorUpdate::WebsocketStatusChanged(status)),
                    "WebSocket状态"
                );
            },

            Some(media_cmd) = media_cmd_rx.recv() => {
                tracing::debug!("[AMLL Actor] 从客户端收到媒体命令: {:?}", media_cmd);
                handle_update_send_error(
                    update_tx.send(ConnectorUpdate::MediaCommand(media_cmd)),
                    "媒体命令"
                );
            },

            Some(update) = smtc_update_rx_async.recv() => {
                handle_update_send_error(
                    update_tx.send(ConnectorUpdate::SmtcUpdate(update.clone())),
                    "SMTC更新"
                );

                if let MediaUpdate::TrackChanged(track_info) = update {
                    tracing::trace!("[AMLL Actor] 收到 SmtcTrackChanged 更新，直接处理...");

                    let is_new_song = track_info.title.as_deref() != last_sent_title.as_deref();
                    if is_new_song {
                        last_sent_title = track_info.title.clone();
                    }

                    if let Some(client) = &websocket_client {
                        let tx = client.outgoing_tx();
                        if is_new_song {
                            let artists_vec = track_info.artist.as_ref().map_or_else(Vec::new, |name| {
                                vec![Artist { id: Default::default(), name: name.as_str().into() }]
                            });
                            let set_music_info_body = ClientMessage::SetMusicInfo {
                                music_id: Default::default(),
                                music_name: track_info.title.clone().map_or(Default::default(), |s| s.as_str().into()),
                                album_id: Default::default(),
                                album_name: track_info.album_title.clone().map_or(Default::default(), |s| s.as_str().into()),
                                artists: artists_vec,
                                duration: track_info.duration_ms.unwrap_or(0),
                            };
                            handle_websocket_send_error(tx.try_send(set_music_info_body), "SetMusicInfo");

                            if let Some(ref cover_data) = track_info.cover_data
                                && !cover_data.is_empty() {
                                    let cover_message = ClientMessage::SetMusicAlbumCoverImageData {
                                        data: cover_data.to_vec()
                                    };
                                    handle_websocket_send_error(tx.try_send(cover_message), "SetMusicAlbumCoverImageData");
                                }
                        }

                        if let Some(is_playing) = track_info.is_playing {
                            let play_state_message = if is_playing {
                                ClientMessage::OnResumed
                            } else {
                                ClientMessage::OnPaused
                            };
                            handle_websocket_send_error(tx.try_send(play_state_message), "播放状态");
                        }

                        if let Some(progress) = track_info.position_ms {
                            let progress_message = ClientMessage::OnPlayProgress { progress };
                            handle_websocket_send_error(tx.try_send(progress_message), "OnPlayProgress");
                        }
                    }
                }
            },
        }
    }
    tracing::trace!("[AMLL Actor] 主事件循环已结束，Actor 任务即将完成。");
}

fn start_websocket_client_task(
    config: &AMLLConnectorConfig,
    status_tx: TokioSender<WebsocketStatus>,
    media_cmd_tx: TokioSender<SmtcControlCommand>,
) -> Result<
    (
        TokioSender<ClientMessage>,
        tokio::task::JoinHandle<()>,
        oneshot::Sender<()>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
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
