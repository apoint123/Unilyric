use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use smtc_suite::{RepeatMode as SmtcRepeatMode, SmtcControlCommand};
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver as TokioReceiver, Sender as TokioSender};
use tokio::sync::oneshot::Receiver as OneshotReceiver;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async, tungstenite::protocol::Message as WsMessage,
};
use tracing::{debug, error, info, trace, warn};

use super::protocol_v2::*;
use crate::amll_connector::WebsocketStatus;

/// 连接结束的原因枚举
#[derive(Debug, Clone)]
enum LifecycleEndReason {
    StreamFailure(String),
    ServerClosed,
}

type ActualWebSocketStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsWriter = SplitSink<ActualWebSocketStream, WsMessage>;

/// 定义连接超时时长
const CONNECT_TIMEOUT_DURATION: Duration = Duration::from_secs(10);

/// 跳转请求的防抖持续时间
const SEEK_DEBOUNCE_DURATION: Duration = Duration::from_millis(500);
/// 设置音量的最小间隔，用于节流
const MIN_VOLUME_SET_INTERVAL: Duration = Duration::from_millis(100);

/// 应用层 Ping 消息的发送间隔
const APP_PING_INTERVAL: Duration = Duration::from_secs(5);

/// 用于封装单个活跃连接期间所有状态的结构体
struct ConnectionState {
    last_seek_request_info: Option<(u64, Instant)>,
    last_volume_set_processed_time: Option<Instant>,
    waiting_for_app_pong: bool,
}

impl ConnectionState {
    fn new() -> Self {
        Self {
            last_seek_request_info: None,
            last_volume_set_processed_time: None,
            waiting_for_app_pong: false,
        }
    }
}

async fn handle_v2_message(
    payload: Payload,
    writer: &mut WsWriter,
    media_cmd_tx: &TokioSender<SmtcControlCommand>,
    state: &mut ConnectionState,
) -> Result<(), LifecycleEndReason> {
    match payload {
        Payload::Ping => {
            trace!("[WebSocket 客户端] 收到服务器的 Ping。回复 Pong。");
            let pong_payload = Payload::Pong;
            let pong_msg = MessageV2 {
                payload: pong_payload,
            };
            if let Ok(text) = serde_json::to_string(&pong_msg) {
                if writer.send(WsMessage::Text(text.into())).await.is_err() {
                    return Err(LifecycleEndReason::StreamFailure("回复 Pong 失败".into()));
                }
            } else {
                error!("[WebSocket 客户端] 序列化 Pong 失败。");
            }
        }
        Payload::Pong => {
            trace!("[WebSocket 客户端] 收到服务器的 Pong。");
            state.waiting_for_app_pong = false;
        }
        Payload::Command(command) => match command {
            Command::Pause => {
                info!("[WebSocket 客户端] 收到服务器命令: 暂停。");
                state.last_seek_request_info = None;
                if media_cmd_tx.try_send(SmtcControlCommand::Pause).is_err() {
                    warn!("[WebSocket 客户端] 发送暂停命令到 Actor 失败 (通道已满或关闭)。");
                }
            }
            Command::Resume => {
                info!("[WebSocket 客户端] 收到服务器命令: 播放。");
                state.last_seek_request_info = None;
                if media_cmd_tx.try_send(SmtcControlCommand::Play).is_err() {
                    warn!("[WebSocket 客户端] 发送播放命令到 Actor 失败 (通道已满或关闭)。");
                }
            }
            Command::ForwardSong => {
                info!("[WebSocket 客户端] 收到服务器命令: 下一首。");
                if media_cmd_tx.try_send(SmtcControlCommand::SkipNext).is_err() {
                    warn!("[WebSocket 客户端] 发送下一首命令到 Actor 失败 (通道已满或关闭)。");
                }
            }
            Command::BackwardSong => {
                info!("[WebSocket 客户端] 收到服务器命令: 上一首。");
                if media_cmd_tx
                    .try_send(SmtcControlCommand::SkipPrevious)
                    .is_err()
                {
                    warn!("[WebSocket 客户端] 发送上一首命令到 Actor 失败 (通道已满或关闭)。");
                }
            }
            Command::SeekPlayProgress { progress } => {
                let now = Instant::now();
                if state.last_seek_request_info.is_none_or(|(_, last_time)| {
                    now.duration_since(last_time) >= SEEK_DEBOUNCE_DURATION
                }) {
                    info!("[WebSocket 客户端] 收到服务器命令: 跳转到 {progress}.");
                    state.last_seek_request_info = Some((progress, now));
                    if media_cmd_tx
                        .try_send(SmtcControlCommand::SeekTo(progress))
                        .is_err()
                    {
                        warn!("[WebSocket 客户端] 发送跳转命令到 Actor 失败 (通道已满或关闭)。");
                    }
                }
            }
            Command::SetVolume { volume } => {
                let now = Instant::now();
                if state
                    .last_volume_set_processed_time
                    .is_none_or(|last_time| {
                        now.duration_since(last_time) >= MIN_VOLUME_SET_INTERVAL
                    })
                {
                    info!("[WebSocket 客户端] 收到服务器命令: 设置音量为 {volume:.2}");
                    state.last_volume_set_processed_time = Some(now);
                    if (0.0..=1.0).contains(&volume) {
                        if media_cmd_tx
                            .try_send(SmtcControlCommand::SetVolume(volume as f32))
                            .is_err()
                        {
                            warn!(
                                "[WebSocket 客户端] 发送设置音量命令到 Actor 失败 (通道已满或关闭)。"
                            );
                        }
                    } else {
                        warn!("[WebSocket 客户端] 收到无效的音量值: {volume}。");
                    }
                }
            }
            Command::SetRepeatMode { mode } => {
                info!(
                    "[WebSocket 客户端] 收到服务器命令: 设置重复播放模式为 {:?}。",
                    mode
                );
                let smtc_mode = match mode {
                    super::protocol_v2::RepeatMode::Off => SmtcRepeatMode::Off,
                    super::protocol_v2::RepeatMode::One => SmtcRepeatMode::One,
                    super::protocol_v2::RepeatMode::All => SmtcRepeatMode::All,
                };
                if media_cmd_tx
                    .try_send(SmtcControlCommand::SetRepeatMode(smtc_mode))
                    .is_err()
                {
                    warn!(
                        "[WebSocket 客户端] 发送设置重复播放模式命令到 Actor 失败 (通道已满或关闭)。"
                    );
                }
            }
            Command::SetShuffleMode { enabled } => {
                info!("[WebSocket 客户端] 收到服务器命令: 设置随机播放模式为 {enabled}。");
                if media_cmd_tx
                    .try_send(SmtcControlCommand::SetShuffle(enabled))
                    .is_err()
                {
                    warn!(
                        "[WebSocket 客户端] 发送设置随机播放模式命令到 Actor 失败 (通道已满或关闭)。"
                    );
                }
            }
        },
        Payload::Initialize | Payload::State(_) => {
            warn!("[WebSocket 客户端] 收到意外的 Initialize/State 消息 (应该是我们发送的)");
        }
    }
    Ok(())
}

/// 处理从 WebSocket 流接收到的单个消息
async fn handle_ws_message(
    ws_msg_option: Option<Result<WsMessage, tokio_tungstenite::tungstenite::Error>>,
    writer: &mut WsWriter,
    media_cmd_tx: &TokioSender<SmtcControlCommand>,
    state: &mut ConnectionState,
) -> Result<(), LifecycleEndReason> {
    match ws_msg_option {
        Some(Ok(message)) => match message {
            WsMessage::Text(text_data) => match serde_json::from_str::<MessageV2>(&text_data) {
                Ok(parsed_message) => {
                    handle_v2_message(parsed_message.payload, writer, media_cmd_tx, state).await?;
                }
                Err(e) => {
                    error!("[WebSocket 客户端] 反序列化服务器消息失败: {e:?}. 内容: {text_data}");
                }
            },
            WsMessage::Binary(_) => {
                warn!("[WebSocket 客户端] 收到一个二进制消息，但不再支持，请尝试更新 AMLL Player");
            }
            WsMessage::Ping(_) => trace!("[WebSocket 客户端] 收到 WebSocket 底层 PING"),
            WsMessage::Pong(_) => trace!("[WebSocket 客户端] 收到 WebSocket 底层 PONG"),
            WsMessage::Close(close_frame) => {
                error!("[WebSocket 客户端] 服务器发送了 WebSocket 关闭帧: {close_frame:?}.");
                return Err(LifecycleEndReason::ServerClosed);
            }
            WsMessage::Frame(_) => {} // 忽略原始帧
        },
        Some(Err(e)) => {
            error!("[WebSocket 客户端] 从 WebSocket 流读取消息时发生错误: {e:?}.");
            return Err(LifecycleEndReason::StreamFailure(
                "WebSocket读取错误".to_string(),
            ));
        }
        None => {
            error!("[WebSocket 客户端] WebSocket 流已关闭 (读取到 None).");
            return Err(LifecycleEndReason::ServerClosed);
        }
    }
    Ok(())
}

/// 管理一个已建立的 WebSocket 连接
async fn handle_connection(
    ws_stream: ActualWebSocketStream,
    outgoing_rx: &mut TokioReceiver<OutgoingMessage>,
    media_cmd_tx: &TokioSender<SmtcControlCommand>,
    mut shutdown_rx: OneshotReceiver<()>,
) -> Result<(), LifecycleEndReason> {
    let (mut ws_writer, mut ws_reader) = ws_stream.split();
    let mut state = ConnectionState::new();
    let mut app_ping_interval_timer = tokio::time::interval(APP_PING_INTERVAL);
    app_ping_interval_timer.tick().await;

    loop {
        tokio::select! {
            biased;

            // 1. 处理外部关闭信号
            _ = &mut shutdown_rx => {
                trace!("[WebSocket 客户端] 收到外部关闭信号。");
                ws_writer.close().await.ok();
                return Ok(());
            }

            // 2. 处理待发送消息 (来自Actor)
            maybe_body_to_send = outgoing_rx.recv() => {
                if let Some(body_to_send) = maybe_body_to_send {
                    let ws_message = match body_to_send {
                        OutgoingMessage::Json(v2_msg) => {
                            match serde_json::to_string(&v2_msg) {
                                Ok(text) => WsMessage::Text(text.into()),
                                Err(e) => {
                                    error!("[WebSocket 客户端] 序列化失败: {e:?}");
                                    continue;
                                }
                            }
                        }
                        OutgoingMessage::Binary(bin_body) => {
                            match super::protocol_v2::to_binary_v2(&bin_body) {
                                Ok(bytes) => WsMessage::Binary(bytes.into()),
                                Err(e) => {
                                    error!("[WebSocket 客户端] 二进制编码失败: {e:?}");
                                    continue;
                                }
                            }
                        }
                    };

                    if ws_writer.send(ws_message).await.is_err() {
                        return Err(LifecycleEndReason::StreamFailure("发送主通道消息失败".to_string()));
                    }
                } else {
                    return Err(LifecycleEndReason::StreamFailure("主发送通道已关闭".to_string()));
                }
            },

            // 3. 处理从 WebSocket 服务器接收到的消息
            ws_msg_option = ws_reader.next() => {
                handle_ws_message(
                    ws_msg_option,
                    &mut ws_writer,
                    media_cmd_tx,
                    &mut state,
                ).await?
            }
        }
    }
}

/// 运行 WebSocket 客户端的主函数
pub async fn run_websocket_client(
    websocket_url: String,
    mut outgoing_rx: TokioReceiver<OutgoingMessage>,
    status_tx: TokioSender<WebsocketStatus>,
    media_cmd_tx: TokioSender<SmtcControlCommand>,
    mut shutdown_rx: OneshotReceiver<()>,
) -> anyhow::Result<()> {
    info!("[WebSocket 客户端] 启动，目标 URL: {websocket_url}");

    let ws_stream;

    tokio::select! {
        biased;

        _ = &mut shutdown_rx => {
            debug!("[WebSocket 客户端] 在连接建立前收到关闭信号。");
            return Ok(());
        }

        connect_result = tokio::time::timeout(CONNECT_TIMEOUT_DURATION, connect_async(&websocket_url)) => {
            match connect_result {
                Ok(Ok((stream, response))) => {
                    info!(
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

    if status_tx.send(WebsocketStatus::Connected).await.is_err() {
        return Err(anyhow::anyhow!(
            "向 Actor 发送 'Connected' 状态失败，可能已关闭"
        ));
    }

    let reason = handle_connection(ws_stream, &mut outgoing_rx, &media_cmd_tx, shutdown_rx).await;

    match reason {
        Ok(_) => {
            debug!("[WebSocket 客户端] 因收到关闭信号而正常退出。");
            Ok(())
        }
        Err(lifecycle_reason) => {
            let error_message = match lifecycle_reason {
                LifecycleEndReason::StreamFailure(msg) => format!("连接流错误: {msg}"),
                LifecycleEndReason::ServerClosed => "服务器关闭了连接".to_string(),
            };
            warn!("[WebSocket 客户端] 连接因 '{error_message}' 而异常终止。");
            Err(anyhow::anyhow!(error_message))
        }
    }
}
