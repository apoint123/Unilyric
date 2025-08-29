use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use smtc_suite::SmtcControlCommand;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver as TokioReceiver, Sender as TokioSender};
use tokio::sync::oneshot::Receiver as OneshotReceiver;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async, tungstenite::protocol::Message as WsMessage,
};
use tracing::warn;

use super::protocol::{ClientMessage, ServerMessage};
use crate::amll_connector::WebsocketStatus;

/// 连接结束的原因枚举
#[derive(Debug, Clone)]
enum LifecycleEndReason {
    PongTimeout,
    InitialConnectFailed(String),
    StreamFailure(String),
    ServerClosed,
}

type ActualWebSocketStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 定义连接超时时长
const CONNECT_TIMEOUT_DURATION: Duration = Duration::from_secs(10);

/// 跳转请求的防抖持续时间
const SEEK_DEBOUNCE_DURATION: Duration = Duration::from_millis(500);
/// 设置音量的最小间隔，用于节流
const MIN_VOLUME_SET_INTERVAL: Duration = Duration::from_millis(100); // 每100ms最多处理一次音量设置

/// 应用层 Ping 消息的发送间隔
const APP_PING_INTERVAL: Duration = Duration::from_secs(5);
/// 应用层 Pong 消息的等待超时时长
const APP_PONG_TIMEOUT: Duration = Duration::from_secs(5);

type WsWriter = SplitSink<ActualWebSocketStream, WsMessage>;

/// 用于封装单个活跃连接期间所有状态的结构体
struct ConnectionState {
    last_seek_request_info: Option<(u64, Instant)>,
    last_volume_set_processed_time: Option<Instant>,
    last_app_ping_sent_at: Option<Instant>,
    waiting_for_app_pong: bool,
}

impl ConnectionState {
    fn new() -> Self {
        Self {
            last_seek_request_info: None,
            last_volume_set_processed_time: None,
            last_app_ping_sent_at: None,
            waiting_for_app_pong: false,
        }
    }
}

/// 辅助函数：异步发送 WebSocket 消息
/// 将 `ClientMessage` 序列化为二进制数据并通过 WebSocket 发送出去。
async fn send_ws_message(writer: &mut WsWriter, body: ClientMessage) -> Result<(), String> {
    // 尝试序列化协议体
    match body.encode() {
        Ok(binary_data) => {
            // 根据协议体类型生成日志描述，方便追踪
            let body_type_for_log = match &body {
                ClientMessage::SetLyricFromTTML { data } => {
                    format!("SetLyricFromTTML(长度:{})", data.len())
                }
                ClientMessage::SetMusicInfo { music_name, .. } => {
                    format!(
                        "SetMusicInfo({})",
                        String::from_utf8_lossy(music_name.as_bytes())
                    )
                }
                ClientMessage::OnPlayProgress { progress } => {
                    format!("OnPlayProgress(进度:{progress})")
                }
                ClientMessage::Ping => "Ping (应用层 - 发往服务器)".to_string(),
                ClientMessage::Pong => "Pong (应用层 - 回复服务器)".to_string(),
                _ => {
                    let debug_str = format!("{body:?}");
                    debug_str
                        .split_whitespace()
                        .next()
                        .unwrap_or("未知协议体")
                        .to_string()
                }
            };

            tracing::trace!(
                "[WebSocket 客户端] 准备发送消息 (类型: {}, 大小: {} 字节)",
                body_type_for_log,
                binary_data.len()
            );

            // 发送二进制消息
            if let Err(e) = writer.send(WsMessage::Binary(binary_data.into())).await {
                let err_msg =
                    format!("发送 WebSocket 二进制消息 (类型: {body_type_for_log}) 失败: {e:?}");
                tracing::error!("[WebSocket 客户端] 发送失败: {err_msg}");
                return Err(err_msg);
            } else if matches!(body, ClientMessage::Pong) {
                tracing::info!("[WebSocket 客户端] 已成功发送 Pong 到服务器。");
            } else {
                tracing::trace!("[WebSocket 客户端] 已成功发送 {body_type_for_log} 消息。");
            }
        }
        Err(e) => {
            let err_msg = format!("序列化 ClientMessage {body:?} 失败: {e:?}");
            tracing::error!("[WebSocket 客户端] 序列化失败: {err_msg}");
            return Err(err_msg);
        }
    }
    Ok(())
}

/// 处理已解析的业务协议消息体
async fn handle_protocol_body(
    parsed_body: ServerMessage,
    internal_pong_tx: &TokioSender<ClientMessage>,
    media_cmd_tx: &TokioSender<SmtcControlCommand>,
    state: &mut ConnectionState,
) -> Result<(), LifecycleEndReason> {
    match parsed_body {
        ServerMessage::Ping => {
            tracing::trace!("[WebSocket 客户端] 收到服务器的 Ping 请求。准备回复 Pong。");
            if internal_pong_tx.send(ClientMessage::Pong).await.is_err() {
                let reason = "排队回复服务器 Ping 失败".to_string();
                tracing::error!("[WebSocket 客户端] {reason}");
                return Err(LifecycleEndReason::StreamFailure(reason));
            }
        }
        ServerMessage::Pong => {
            tracing::trace!("[WebSocket 客户端] 收到服务器的 Pong 回复。");
            if state.waiting_for_app_pong {
                state.waiting_for_app_pong = false;
                state.last_app_ping_sent_at = None;
            }
        }

        ServerMessage::Pause => {
            tracing::info!("[WebSocket 客户端] 收到服务器命令: 暂停。");
            state.last_seek_request_info = None;
            if media_cmd_tx.try_send(SmtcControlCommand::Pause).is_err() {
                tracing::warn!("[WebSocket 客户端] 发送暂停命令到 Actor 失败 (通道已满或关闭)。");
            }
        }
        ServerMessage::Resume => {
            tracing::info!("[WebSocket 客户端] 收到服务器命令: 播放。");
            state.last_seek_request_info = None;
            if media_cmd_tx.try_send(SmtcControlCommand::Play).is_err() {
                tracing::warn!("[WebSocket 客户端] 发送播放命令到 Actor 失败 (通道已满或关闭)。");
            }
        }
        ServerMessage::ForwardSong => {
            tracing::info!("[WebSocket 客户端] 收到服务器命令: 下一首。");
            if media_cmd_tx.try_send(SmtcControlCommand::SkipNext).is_err() {
                tracing::warn!("[WebSocket 客户端] 发送下一首命令到 Actor 失败 (通道已满或关闭)。");
            }
        }
        ServerMessage::BackwardSong => {
            tracing::info!("[WebSocket 客户端] 收到服务器命令: 上一首。");
            if media_cmd_tx
                .try_send(SmtcControlCommand::SkipPrevious)
                .is_err()
            {
                tracing::warn!("[WebSocket 客户端] 发送上一首命令到 Actor 失败 (通道已满或关闭)。");
            }
        }
        ServerMessage::SeekPlayProgress { progress } => {
            let now = Instant::now();
            let process_this_seek =
                if let Some((last_progress, last_time)) = state.last_seek_request_info {
                    !(progress == last_progress
                        && now.duration_since(last_time) < SEEK_DEBOUNCE_DURATION)
                } else {
                    true
                };
            if process_this_seek {
                tracing::info!("[WebSocket 客户端] 收到服务器命令: 跳转到 {progress}.");
                state.last_seek_request_info = Some((progress, now));
                if media_cmd_tx
                    .try_send(SmtcControlCommand::SeekTo(progress))
                    .is_err()
                {
                    tracing::warn!(
                        "[WebSocket 客户端] 发送跳转命令到 Actor 失败 (通道已满或关闭)。"
                    );
                }
            }
        }
        ServerMessage::SetVolume { volume } => {
            let now = Instant::now();
            let should_process = if let Some(last_time) = state.last_volume_set_processed_time {
                now.duration_since(last_time) >= MIN_VOLUME_SET_INTERVAL
            } else {
                true
            };

            if should_process {
                tracing::info!("[WebSocket 客户端] 收到服务器命令: 设置音量为 {volume:.2}");
                state.last_volume_set_processed_time = Some(now);
                if (0.0..=1.0).contains(&volume) {
                    if media_cmd_tx
                        .try_send(SmtcControlCommand::SetVolume(volume as f32))
                        .is_err()
                    {
                        tracing::warn!(
                            "[WebSocket 客户端] 发送设置音量命令到 Actor 失败 (通道已满或关闭)。"
                        );
                    }
                } else {
                    tracing::warn!("[WebSocket 客户端] 收到无效的音量值: {volume}。");
                }
            }
        }
    }
    Ok(())
}

/// 处理从 WebSocket 流接收到的单个消息
async fn handle_ws_message(
    ws_msg_option: Option<Result<WsMessage, tokio_tungstenite::tungstenite::Error>>,
    internal_pong_tx: &TokioSender<ClientMessage>,
    media_cmd_tx: &TokioSender<SmtcControlCommand>,
    state: &mut ConnectionState,
) -> Result<(), LifecycleEndReason> {
    match ws_msg_option {
        Some(Ok(message_type)) => match message_type {
            WsMessage::Binary(bin_data) => match ServerMessage::decode(&bin_data) {
                Ok(parsed_body) => {
                    handle_protocol_body(parsed_body, internal_pong_tx, media_cmd_tx, state)
                        .await?;
                }
                Err(e) => {
                    tracing::error!("[WebSocket 客户端] 反序列化服务器二进制消息失败: {e:?}.");
                    return Err(LifecycleEndReason::StreamFailure(
                        "收到无法解析的二进制消息".to_string(),
                    ));
                }
            },
            WsMessage::Text(text_msg) => warn!("[WebSocket 客户端] 收到意外的文本消息: {text_msg}"),
            WsMessage::Ping(_) => tracing::trace!("[WebSocket 客户端] 收到 WebSocket 底层 PING"),
            WsMessage::Pong(_) => tracing::trace!("[WebSocket 客户端] 收到 WebSocket 底层 PONG"),
            WsMessage::Close(close_frame) => {
                tracing::error!(
                    "[WebSocket 客户端] 服务器发送了 WebSocket 关闭帧: {close_frame:?}."
                );
                return Err(LifecycleEndReason::ServerClosed);
            }
            WsMessage::Frame(_) => {} // 忽略原始帧
        },
        Some(Err(e)) => {
            tracing::error!("[WebSocket 客户端] 从 WebSocket 流读取消息时发生错误: {e:?}.");
            return Err(LifecycleEndReason::StreamFailure(
                "WebSocket读取错误".to_string(),
            ));
        }
        None => {
            tracing::error!("[WebSocket 客户端] WebSocket 流已关闭 (读取到 None).");
            return Err(LifecycleEndReason::ServerClosed);
        }
    }
    Ok(())
}

/// 管理一个已建立的 WebSocket 连接
async fn handle_connection(
    ws_stream: ActualWebSocketStream,
    outgoing_rx: &mut TokioReceiver<ClientMessage>,
    media_cmd_tx: &TokioSender<SmtcControlCommand>,
    mut shutdown_rx: OneshotReceiver<()>,
) -> Result<(), LifecycleEndReason> {
    let (mut ws_writer, mut ws_reader) = ws_stream.split();
    let (internal_pong_tx, mut internal_pong_rx) = tokio::sync::mpsc::channel(5);

    let mut state = ConnectionState::new();
    let mut app_ping_interval_timer = tokio::time::interval(APP_PING_INTERVAL);
    app_ping_interval_timer.tick().await;

    loop {
        tokio::select! {
            biased;

            // 1. 处理外部关闭信号
            _ = &mut shutdown_rx => {
                tracing::trace!("[WebSocket 客户端] 收到外部关闭信号。");
                ws_writer.close().await.ok();
                return Ok(());
            }

            // 2. 处理待发送消息 (来自外部)
            maybe_body_to_send = outgoing_rx.recv() => {
                if let Some(body_to_send) = maybe_body_to_send {
                    if send_ws_message(&mut ws_writer, body_to_send).await.is_err() {
                        return Err(LifecycleEndReason::StreamFailure("发送主通道消息失败".to_string()));
                    }
                } else {
                    return Err(LifecycleEndReason::StreamFailure("主发送通道已关闭".to_string()));
                }
            },

            // 3. 处理待发送消息 (来自内部，如 Pong)
            maybe_internal_msg_to_send = internal_pong_rx.recv() => {
                if let Some(internal_msg_to_send) = maybe_internal_msg_to_send {
                    if send_ws_message(&mut ws_writer, internal_msg_to_send).await.is_err() {
                        return Err(LifecycleEndReason::StreamFailure("发送内部 Pong 消息失败".to_string()));
                    }
                } else {
                    tracing::error!("[WebSocket 客户端] 内部 Pong 通道 (internal_pong_rx) 已关闭。");
                    ws_writer.close().await.ok();
                    return Err(LifecycleEndReason::StreamFailure("内部 Pong 通道已关闭".to_string()));
                }
            }

            // 4. 处理从 WebSocket 服务器接收到的消息
            ws_msg_option = ws_reader.next() => {
                handle_ws_message(
                    ws_msg_option,
                    &internal_pong_tx,
                    media_cmd_tx,
                    &mut state,
                ).await?
            }

            // 5. 处理应用层 Ping 定时器
            _ = app_ping_interval_timer.tick() => {
                if state.waiting_for_app_pong {
                    if let Some(sent_at) = state.last_app_ping_sent_at
                        && Instant::now().duration_since(sent_at) > APP_PONG_TIMEOUT {
                            tracing::warn!("[WebSocket 客户端] 服务器应用层 Pong 超时! 断开连接。");
                            ws_writer.close().await.ok();
                            return Err(LifecycleEndReason::PongTimeout);
                        }
                } else {
                    tracing::trace!("[WebSocket 客户端] 定时发送 Ping 到服务器。");
                    if send_ws_message(&mut ws_writer, ClientMessage::Ping).await.is_err() {
                        return Err(LifecycleEndReason::StreamFailure("发送应用层 Ping 失败".to_string()));
                    }
                    state.last_app_ping_sent_at = Some(Instant::now());
                    state.waiting_for_app_pong = true;
                }
            }
        }
    }
}

/// 运行 WebSocket 客户端的主函数
pub async fn run_websocket_client(
    websocket_url: String,
    mut outgoing_rx: TokioReceiver<ClientMessage>,
    status_tx: TokioSender<WebsocketStatus>,
    media_cmd_tx: TokioSender<SmtcControlCommand>,
    mut shutdown_rx: OneshotReceiver<()>,
) -> anyhow::Result<()> {
    tracing::info!("[WebSocket 客户端] 启动，目标 URL: {websocket_url}");

    let ws_stream;

    tokio::select! {
        biased;

        _ = &mut shutdown_rx => {
            return Ok(());
        }

        connect_result = tokio::time::timeout(CONNECT_TIMEOUT_DURATION, connect_async(&websocket_url)) => {
            match connect_result {
                Ok(Ok((stream, response))) => {
                    tracing::info!(
                        "[WebSocket 客户端] 成功连接到服务器。HTTP 状态码: {}",
                        response.status()
                    );
                    ws_stream = stream;
                }
                Ok(Err(e)) => {
                    return Err(anyhow::anyhow!("连接握手失败: {e}"));
                }
                Err(_) => {
                    return Err(anyhow::anyhow!(
                        "连接超时 (超过 {} 秒)",
                        CONNECT_TIMEOUT_DURATION.as_secs()
                    ));
                }
            }
        }
    }

    if status_tx
        .send(super::types::WebsocketStatus::Connected)
        .await
        .is_err()
    {
        return Err(anyhow::anyhow!("Actor 可能已关闭"));
    }

    let reason = handle_connection(ws_stream, &mut outgoing_rx, &media_cmd_tx, shutdown_rx).await;

    match reason {
        Ok(_) => {
            tracing::info!("[WebSocket 客户端] 因收到关闭信号而正常退出。");
            Ok(())
        }
        Err(reason) => {
            let error_message = match reason {
                LifecycleEndReason::StreamFailure(msg) => format!("连接流错误: {msg}"),
                LifecycleEndReason::PongTimeout => "心跳响应超时".to_string(),
                LifecycleEndReason::ServerClosed => "服务器关闭了连接".to_string(),
                LifecycleEndReason::InitialConnectFailed(msg) => {
                    tracing::error!(
                        "[WebSocket 客户端] 逻辑错误：在 handle_connection 中收到了 InitialConnectFailed"
                    );
                    msg
                }
            };
            tracing::warn!("[WebSocket 客户端] 连接因 '{error_message}' 而异常终止。");
            Err(anyhow::anyhow!(error_message))
        }
    }
}
