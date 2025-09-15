use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use smtc_suite::SmtcControlCommand;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver as TokioReceiver, Sender as TokioSender};
use tokio::sync::oneshot::Receiver as OneshotReceiver;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async, tungstenite::protocol::Message as WsMessage,
};
use tracing::{debug, error, info, trace, warn};

use super::protocol::{
    BinClientMessage, BinServerMessage, ClientMessage, OutgoingMessage, ServerMessage,
};
use crate::amll_connector::WebsocketStatus;

/// 连接结束的原因枚举
#[derive(Debug, Clone)]
enum LifecycleEndReason {
    PongTimeout,
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
/// 应用层 Pong 消息的等待超时时长
const APP_PONG_TIMEOUT: Duration = Duration::from_secs(5);

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

async fn handle_json_body(
    parsed_body: ServerMessage,
    writer: &mut WsWriter,
    media_cmd_tx: &TokioSender<SmtcControlCommand>,
    state: &mut ConnectionState,
) -> Result<(), LifecycleEndReason> {
    match parsed_body {
        ServerMessage::Ping => {
            trace!("[WebSocket 客户端] 收到服务器的 JSON Ping。回复 Pong。");
            let pong_json = ClientMessage::Pong;
            if let Ok(text) = serde_json::to_string(&pong_json) {
                if writer.send(WsMessage::Text(text.into())).await.is_err() {
                    return Err(LifecycleEndReason::StreamFailure(
                        "回复 JSON Pong 失败".into(),
                    ));
                }
            } else {
                error!("[WebSocket 客户端] 序列化 JSON Pong 失败。");
            }
        }
        ServerMessage::Pong => {
            trace!("[WebSocket 客户端] 收到服务器的 JSON Pong。");
            state.waiting_for_app_pong = false;
        }
        ServerMessage::Pause => {
            info!("[WebSocket 客户端] 收到服务器JSON命令: 暂停。");
            state.last_seek_request_info = None;
            if media_cmd_tx.try_send(SmtcControlCommand::Pause).is_err() {
                warn!("[WebSocket 客户端] 发送暂停命令到 Actor 失败 (通道已满或关闭)。");
            }
        }
        ServerMessage::Resume => {
            info!("[WebSocket 客户端] 收到服务器JSON命令: 播放。");
            state.last_seek_request_info = None;
            if media_cmd_tx.try_send(SmtcControlCommand::Play).is_err() {
                warn!("[WebSocket 客户端] 发送播放命令到 Actor 失败 (通道已满或关闭)。");
            }
        }
        ServerMessage::ForwardSong => {
            info!("[WebSocket 客户端] 收到服务器JSON命令: 下一首。");
            if media_cmd_tx.try_send(SmtcControlCommand::SkipNext).is_err() {
                warn!("[WebSocket 客户端] 发送下一首命令到 Actor 失败 (通道已满或关闭)。");
            }
        }
        ServerMessage::BackwardSong => {
            info!("[WebSocket 客户端] 收到服务器JSON命令: 上一首。");
            if media_cmd_tx
                .try_send(SmtcControlCommand::SkipPrevious)
                .is_err()
            {
                warn!("[WebSocket 客户端] 发送上一首命令到 Actor 失败 (通道已满或关闭)。");
            }
        }
        ServerMessage::SeekPlayProgress { progress } => {
            let now = Instant::now();
            if state.last_seek_request_info.is_none_or(|(_, last_time)| {
                now.duration_since(last_time) >= SEEK_DEBOUNCE_DURATION
            }) {
                info!("[WebSocket 客户端] 收到服务器JSON命令: 跳转到 {progress}.");
                state.last_seek_request_info = Some((progress, now));
                if media_cmd_tx
                    .try_send(SmtcControlCommand::SeekTo(progress))
                    .is_err()
                {
                    warn!("[WebSocket 客户端] 发送跳转命令到 Actor 失败 (通道已满或关闭)。");
                }
            }
        }
        ServerMessage::SetVolume { volume } => {
            let now = Instant::now();
            if state
                .last_volume_set_processed_time
                .is_none_or(|last_time| now.duration_since(last_time) >= MIN_VOLUME_SET_INTERVAL)
            {
                info!("[WebSocket 客户端] 收到服务器JSON命令: 设置音量为 {volume:.2}");
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
    }
    Ok(())
}

async fn handle_binary_body(
    parsed_body: BinServerMessage,
    writer: &mut WsWriter,
    media_cmd_tx: &TokioSender<SmtcControlCommand>,
    state: &mut ConnectionState,
) -> Result<(), LifecycleEndReason> {
    match parsed_body {
        BinServerMessage::Ping => {
            trace!("[WebSocket 客户端] 收到服务器的 Binary Ping。回复 Pong。");
            let pong_bin = BinClientMessage::Pong;
            if let Ok(bytes) = pong_bin.encode()
                && writer.send(WsMessage::Binary(bytes.into())).await.is_err()
            {
                return Err(LifecycleEndReason::StreamFailure(
                    "回复 Binary Pong 失败".into(),
                ));
            }
        }
        BinServerMessage::Pong => {
            trace!("[WebSocket 客户端] 收到服务器的 Binary Pong。");
        }
        BinServerMessage::Pause => {
            info!("[WebSocket 客户端] 收到服务器Binary命令: 暂停。");
            state.last_seek_request_info = None;
            if media_cmd_tx.try_send(SmtcControlCommand::Pause).is_err() {
                warn!("[WebSocket 客户端] (Binary)发送暂停命令到 Actor 失败 (通道已满或关闭)。");
            }
        }
        BinServerMessage::Resume => {
            info!("[WebSocket 客户端] 收到服务器Binary命令: 播放。");
            state.last_seek_request_info = None;
            if media_cmd_tx.try_send(SmtcControlCommand::Play).is_err() {
                warn!("[WebSocket 客户端] (Binary)发送播放命令到 Actor 失败 (通道已满或关闭)。");
            }
        }
        // NEW: 填充以下分支
        BinServerMessage::ForwardSong => {
            info!("[WebSocket 客户端] 收到服务器Binary命令: 下一首。");
            if media_cmd_tx.try_send(SmtcControlCommand::SkipNext).is_err() {
                warn!("[WebSocket 客户端] (Binary)发送下一首命令到 Actor 失败 (通道已满或关闭)。");
            }
        }
        BinServerMessage::BackwardSong => {
            info!("[WebSocket 客户端] 收到服务器Binary命令: 上一首。");
            if media_cmd_tx
                .try_send(SmtcControlCommand::SkipPrevious)
                .is_err()
            {
                warn!("[WebSocket 客户端] (Binary)发送上一首命令到 Actor 失败 (通道已满或关闭)。");
            }
        }
        BinServerMessage::SeekPlayProgress { progress } => {
            let now = Instant::now();
            if state.last_seek_request_info.is_none_or(|(_, last_time)| {
                now.duration_since(last_time) >= SEEK_DEBOUNCE_DURATION
            }) {
                info!("[WebSocket 客户端] 收到服务器Binary命令: 跳转到 {progress}.");
                state.last_seek_request_info = Some((progress, now));
                if media_cmd_tx
                    .try_send(SmtcControlCommand::SeekTo(progress))
                    .is_err()
                {
                    warn!(
                        "[WebSocket 客户端] (Binary)发送跳转命令到 Actor 失败 (通道已满或关闭)。"
                    );
                }
            }
        }
        BinServerMessage::SetVolume { volume } => {
            let now = Instant::now();
            if state
                .last_volume_set_processed_time
                .is_none_or(|last_time| now.duration_since(last_time) >= MIN_VOLUME_SET_INTERVAL)
            {
                info!("[WebSocket 客户端] 收到服务器Binary命令: 设置音量为 {volume:.2}");
                state.last_volume_set_processed_time = Some(now);
                if (0.0..=1.0).contains(&volume) {
                    if media_cmd_tx
                        .try_send(SmtcControlCommand::SetVolume(volume as f32))
                        .is_err()
                    {
                        warn!(
                            "[WebSocket 客户端] (Binary)发送设置音量命令到 Actor 失败 (通道已满或关闭)。"
                        );
                    }
                } else {
                    warn!("[WebSocket 客户端] (Binary)收到无效的音量值: {volume}。");
                }
            }
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
            WsMessage::Text(text_data) => match serde_json::from_str::<ServerMessage>(&text_data) {
                Ok(parsed_body) => {
                    handle_json_body(parsed_body, writer, media_cmd_tx, state).await?;
                }
                Err(e) => {
                    error!(
                        "[WebSocket 客户端] 反序列化服务器 JSON 消息失败: {e:?}. 内容: {text_data}"
                    );
                }
            },
            WsMessage::Binary(bin_data) => match BinServerMessage::decode(&bin_data) {
                Ok(parsed_body) => {
                    handle_binary_body(parsed_body, writer, media_cmd_tx, state).await?;
                }
                Err(_) => {
                    trace!(
                        "[WebSocket 客户端] 收到一个无法解码为旧协议的二进制消息 (可能为音频/封面数据)。"
                    );
                }
            },
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
                        OutgoingMessage::Json(json_body) => {
                            match serde_json::to_string(&json_body) {
                                Ok(text) => WsMessage::Text(text.into()),
                                Err(e) => {
                                    error!("[WebSocket 客户端] JSON 序列化失败: {e:?}");
                                    continue;
                                }
                            }
                        }
                        OutgoingMessage::LegacyBinary(bin_body) => {
                            match bin_body.encode() {
                                Ok(bytes) => WsMessage::Binary(bytes.into()),
                                Err(e) => {
                                    error!("[WebSocket 客户端] 旧协议二进制编码失败: {e:?}");
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

            // 4. 处理应用层 Ping 定时器
            _ = app_ping_interval_timer.tick() => {
                if state.waiting_for_app_pong {
                    if let Some(sent_at) = state.last_app_ping_sent_at
                        && Instant::now().duration_since(sent_at) > APP_PONG_TIMEOUT {
                            warn!("[WebSocket 客户端] 服务器应用层 Pong 超时! 断开连接。");
                            ws_writer.close().await.ok();
                            return Err(LifecycleEndReason::PongTimeout);
                        }
                } else {
                    trace!("[WebSocket 客户端] 定时发送 JSON Ping 到服务器。");
                    let ping_msg = ClientMessage::Ping;
                    if let Ok(text) = serde_json::to_string(&ping_msg) {
                         if ws_writer.send(WsMessage::Text(text.into())).await.is_err() {
                            return Err(LifecycleEndReason::StreamFailure("发送应用层 Ping 失败".to_string()));
                        }
                        state.last_app_ping_sent_at = Some(Instant::now());
                        state.waiting_for_app_pong = true;
                    } else {
                        error!("[WebSocket 客户端] 序列化 Ping 消息失败。");
                    }
                }
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
                LifecycleEndReason::PongTimeout => "心跳响应超时".to_string(),
                LifecycleEndReason::ServerClosed => "服务器关闭了连接".to_string(),
            };
            warn!("[WebSocket 客户端] 连接因 '{error_message}' 而异常终止。");
            Err(anyhow::anyhow!(error_message))
        }
    }
}
