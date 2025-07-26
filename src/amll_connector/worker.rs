use super::{
    types::{AMLLConnectorConfig, ConnectorCommand, ConnectorUpdate, WebsocketStatus},
    websocket_client,
};
use crossbeam_channel::{
    Receiver as CrossbeamReceiver, RecvTimeoutError, Sender as CrossbeamSender,
};
use smtc_suite::SmtcControlCommand;
use smtc_suite::{MediaCommand, MediaUpdate};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::Sender as StdSender,
};
use std::time::Duration;
use tokio::sync::mpsc::{
    Receiver as TokioReceiver, Sender as TokioSender, channel as tokio_channel,
};
use tokio::sync::oneshot;
use ws_protocol::Body as ProtocolBody;

pub async fn amll_connector_actor(
    mut command_rx: TokioReceiver<ConnectorCommand>,
    update_tx: StdSender<ConnectorUpdate>,
    initial_config: AMLLConnectorConfig,
    smtc_command_tx: CrossbeamSender<MediaCommand>,
    smtc_update_rx: CrossbeamReceiver<MediaUpdate>,
) {
    tracing::info!("[AMLL Actor] Actor 任务已启动。");
    let mut config = initial_config;
    let mut ws_outgoing_tx: Option<TokioSender<ProtocolBody>> = None;
    let mut ws_shutdown_signal_tx: Option<oneshot::Sender<()>> = None;
    let mut ws_client_handle: Option<tokio::task::JoinHandle<()>> = None;
    let mut last_sent_title: Option<String> = None;

    let (ws_status_tx, mut ws_status_rx) = tokio_channel(32);
    let (media_cmd_tx, mut media_cmd_rx) = tokio_channel(32);

    if config.enabled {
        let (new_tx, new_handle, new_shutdown_tx) =
            start_websocket_client_task(&config, ws_status_tx.clone(), media_cmd_tx.clone());
        ws_outgoing_tx = Some(new_tx);
        ws_client_handle = Some(new_handle);
        ws_shutdown_signal_tx = Some(new_shutdown_tx);
    }

    let bridge_shutdown_signal = Arc::new(AtomicBool::new(false));

    let (smtc_update_tx_async, mut smtc_update_rx_async) = tokio_channel(128);
    let smtc_bridge_handle = {
        let signal = Arc::clone(&bridge_shutdown_signal);
        tokio::spawn(async move {
            tracing::debug!("[SMTC Bridge] 桥接任务已启动。");
            tokio::task::spawn_blocking(move || {
                loop {
                    if signal.load(Ordering::Relaxed) {
                        tracing::debug!("[SMTC Bridge] 收到关闭信号，正在退出循环。");
                        break;
                    }

                    // 防止无限阻塞
                    match smtc_update_rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(update) => {
                            if smtc_update_tx_async.blocking_send(update).is_err() {
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => continue, // 超时是正常情况
                        Err(RecvTimeoutError::Disconnected) => break, // 通道关闭
                    }
                }
                tracing::debug!("[SMTC Bridge] 桥接任务正常退出。");
            })
            .await
            .unwrap();
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

                        if let Err(e) = smtc_bridge_handle.await {
                             tracing::warn!("[AMLL Actor] 等待 SMTC 桥接任务完成时出错: {}", e);
                        }

                        if let Some(tx) = ws_shutdown_signal_tx.take() { let _ = tx.send(()); }
                        if let Some(handle) = ws_client_handle.take() { handle.abort(); }
                        break;
                    },
                    ConnectorCommand::UpdateConfig(new_config) => {
                        let old_config = config.clone();
                        config = new_config;
                        let should_be_running = config.enabled;
                        let is_running = ws_client_handle.as_ref().is_some_and(|h| !h.is_finished());
                        let url_changed = old_config.websocket_url != config.websocket_url;
                        if should_be_running && (!is_running || url_changed) {
                            tracing::info!("[AMLL Actor] 配置要求客户端运行，正在启动/重启...");
                            if let Some(tx) = ws_shutdown_signal_tx.take() { let _ = tx.send(()); }
                            let (new_tx, new_handle, new_shutdown_tx) =
                                start_websocket_client_task(&config, ws_status_tx.clone(), media_cmd_tx.clone());
                            ws_outgoing_tx = Some(new_tx);
                            ws_client_handle = Some(new_handle);
                            ws_shutdown_signal_tx = Some(new_shutdown_tx);
                        } else if !should_be_running && is_running {
                            tracing::info!("[AMLL Actor] 配置已禁用，正在停止客户端...");
                            if let Some(tx) = ws_shutdown_signal_tx.take() { let _ = tx.send(()); }
                        }
                    },
                    ConnectorCommand::DisconnectWebsocket => {
                        tracing::info!("[AMLL Actor] 收到 Disconnect 命令，正在关闭 WebSocket 客户端...");
                        if let Some(tx) = ws_shutdown_signal_tx.take() { let _ = tx.send(()); }
                        let _ = update_tx.send(ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::断开));
                    },
                    ConnectorCommand::SendLyricTtml(ttml) => {
                        if let Some(tx) = &ws_outgoing_tx
                           && tx.try_send(ProtocolBody::SetLyricFromTTML { data: ttml.into() }).is_err() {
                                tracing::warn!("[AMLL Actor] 发送 TTML 到客户端任务失败 (通道已满或关闭)。");
                            }
                    },
                    ConnectorCommand::SendProtocolBody(body) => {
                         if let Some(tx) = &ws_outgoing_tx
                            && tx.try_send(body).is_err() {
                                tracing::warn!("[AMLL Actor] 发送 ProtocolBody 到客户端任务失败 (通道已满或关闭)。");
                            }
                    },
                    _ => {}
                }
            },

            Some(status) = ws_status_rx.recv() => {
                tracing::debug!("[AMLL Actor] 收到 WebSocket 状态更新: {:?}", status);
                let enable_high_freq = matches!(status, WebsocketStatus::已连接);
                let command = MediaCommand::SetHighFrequencyProgressUpdates(enable_high_freq);
                if smtc_command_tx.send(command).is_err() {
                    tracing::error!("[AMLL Actor] 向 smtc-suite 发送高频更新开关命令失败。");
                }
                let _ = update_tx.send(ConnectorUpdate::WebsocketStatusChanged(status));
            },

            Some(media_cmd) = media_cmd_rx.recv() => {
                tracing::debug!("[AMLL Actor] 从客户端收到媒体命令: {:?}", media_cmd);
                let _ = update_tx.send(ConnectorUpdate::MediaCommand(media_cmd));
            },

            Some(update) = smtc_update_rx_async.recv() => {
                 if update_tx.send(ConnectorUpdate::SmtcUpdate(update.clone())).is_err() {
                     tracing::warn!("[AMLL Actor] 转发 SMTC 更新到 UI 线程失败，UI 可能已关闭。");
                 }

                if let MediaUpdate::TrackChanged(track_info) = update {
                    tracing::trace!("[AMLL Actor] 收到 SmtcTrackChanged 更新，直接处理...");

                    let is_new_song = track_info.title.as_deref() != last_sent_title.as_deref();
                    if is_new_song {
                        last_sent_title = track_info.title.clone();
                    }

                    if let Some(tx) = &ws_outgoing_tx {
                        if is_new_song {
                            let artists_vec = track_info.artist.as_ref().map_or_else(Vec::new, |name| {
                                vec![ws_protocol::Artist { id: Default::default(), name: name.as_str().into() }]
                            });
                            let set_music_info_body = ProtocolBody::SetMusicInfo {
                                music_id: Default::default(),
                                music_name: track_info.title.clone().map_or(Default::default(), |s| s.as_str().into()),
                                album_id: Default::default(),
                                album_name: track_info.album_title.clone().map_or(Default::default(), |s| s.as_str().into()),
                                artists: artists_vec,
                                duration: track_info.duration_ms.unwrap_or(0),
                            };
                            let _ = tx.try_send(set_music_info_body);
                            if let Some(ref cover_data) = track_info.cover_data
                                && !cover_data.is_empty() {
                                    let _ = tx.try_send(ProtocolBody::SetMusicAlbumCoverImageData { data: cover_data.to_vec() });
                                }
                        }
                        if let Some(is_playing) = track_info.is_playing {
                            let _ = tx.try_send(if is_playing { ProtocolBody::OnResumed } else { ProtocolBody::OnPaused });
                        }
                        if let Some(progress) = track_info.position_ms {
                            let _ = tx.try_send(ProtocolBody::OnPlayProgress { progress });
                        }
                    }
                }
            },
        }
    }
    tracing::info!("[AMLL Actor] 主事件循环已结束，Actor 任务即将完成。");
}

fn start_websocket_client_task(
    config: &AMLLConnectorConfig,
    status_tx: TokioSender<WebsocketStatus>,
    media_cmd_tx: TokioSender<SmtcControlCommand>,
) -> (
    TokioSender<ProtocolBody>,
    tokio::task::JoinHandle<()>,
    oneshot::Sender<()>,
) {
    let (ws_outgoing_tx, ws_outgoing_rx) = tokio_channel(32);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let url = config.websocket_url.clone();

    let handle = tokio::spawn(async move {
        let (status_tx_sync, status_rx_sync) = std::sync::mpsc::channel();

        tokio::task::spawn_blocking(move || {
            while let Ok(status) = status_rx_sync.recv() {
                if status_tx.blocking_send(status).is_err() {
                    break;
                }
            }
        });

        websocket_client::run_websocket_client(
            url,
            ws_outgoing_rx,
            status_tx_sync,
            media_cmd_tx,
            shutdown_rx,
        )
        .await;
    });

    (ws_outgoing_tx, handle, shutdown_tx)
}
