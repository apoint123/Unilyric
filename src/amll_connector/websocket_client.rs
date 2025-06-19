use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use log::{error, warn};
use std::sync::mpsc::Sender as StdSender;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::mpsc::Receiver as TokioReceiver;
use tokio::sync::oneshot::Receiver as OneshotReceiver;
use tokio::task;
use tokio::time::sleep;

use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async, tungstenite::protocol::Message as WsMessage,
};

use super::types::{SmtcControlCommand, WebsocketStatus};
use ws_protocol::{
    Body as ProtocolBody, parse_body as deserialize_protocol_body,
    to_body as serialize_protocol_body,
};

/// 连接生命周期结束的原因枚举
#[derive(Debug, Clone)]
enum LifecycleEndReason {
    PongTimeout,                    // Pong 响应超时
    InitialConnectFailed(String),   // 初始连接失败，附带错误描述
    StreamFailure(String),          // WebSocket 流错误，附带错误描述
    ServerClosed,                   // 服务器关闭了连接
    ShutdownSignalReceived,         // 收到了外部关闭信号
    CriticalChannelFailure(String), // 关键的内部通道发生故障，附带错误描述
}

/// 定义实际的 WebSocket 流类型别名，简化代码
type ActualWebSocketStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

// --- 常量定义 ---
/// 定义重连延迟时间（毫秒）
const RECONNECT_DELAY_MS: u64 = 3000;
/// 定义连接超时时长
const CONNECT_TIMEOUT_DURATION: Duration = Duration::from_secs(10);
/// 定义最大连续失败连接尝试次数
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// 跳转请求的防抖持续时间
const SEEK_DEBOUNCE_DURATION: Duration = Duration::from_millis(500);
/// 设置音量的最小间隔，用于节流
const MIN_VOLUME_SET_INTERVAL: Duration = Duration::from_millis(100); // 每100ms最多处理一次音量设置

/// 应用层 Ping 消息的发送间隔
const APP_PING_INTERVAL: Duration = Duration::from_secs(5);
/// 应用层 Pong 消息的等待超时时长
const APP_PONG_TIMEOUT: Duration = Duration::from_secs(5);

/// 辅助函数：异步发送 WebSocket 消息
/// 将 `ProtocolBody` 序列化为二进制数据并通过 WebSocket 发送出去。
async fn send_ws_message(
    writer: &mut SplitSink<ActualWebSocketStream, WsMessage>, // WebSocket 写入器 (Sink 部分)
    body: ProtocolBody,                                       // 要发送的协议消息体
) -> Result<(), String> {
    // 尝试序列化协议体
    match serialize_protocol_body(&body) {
        Ok(binary_data) => {
            // 根据协议体类型生成日志描述，方便追踪
            let body_type_for_log = match &body {
                ProtocolBody::SetLyricFromTTML { data } => {
                    format!("SetLyricFromTTML(长度:{})", data.0.len())
                }
                ProtocolBody::SetMusicInfo { music_name, .. } => {
                    format!("SetMusicInfo({})", String::from_utf8_lossy(&music_name.0))
                }
                ProtocolBody::OnPlayProgress { progress } => {
                    format!("OnPlayProgress(进度:{progress})")
                }
                ProtocolBody::Ping => "Ping (应用层 - 发往服务器)".to_string(),
                ProtocolBody::Pong => "Pong (应用层 - 回复服务器)".to_string(),
                _ => {
                    let debug_str = format!("{body:?}");
                    debug_str
                        .split_whitespace()
                        .next()
                        .unwrap_or("未知协议体")
                        .to_string()
                }
            };

            log::debug!(
                "[WebSocket 客户端] 准备发送消息 (类型: {}, 大小: {} 字节)",
                body_type_for_log,
                binary_data.len()
            );

            // 发送二进制消息
            if let Err(e) = writer.send(WsMessage::Binary(binary_data.into())).await {
                let err_msg =
                    format!("发送 WebSocket 二进制消息 (类型: {body_type_for_log}) 失败: {e:?}");
                log::error!("[WebSocket 客户端] 发送失败: {err_msg}");
                return Err(err_msg);
            } else if matches!(body, ProtocolBody::Pong) {
                log::info!("[WebSocket 客户端] 已成功发送 Pong 到服务器。");
            } else {
                log::trace!("[WebSocket 客户端] 已成功发送 {body_type_for_log} 消息。");
            }
        }
        Err(e) => {
            // 协议体序列化失败
            let err_msg = format!("序列化 ProtocolBody {body:?} 失败: {e:?}");
            log::error!("[WebSocket 客户端] 序列化失败: {err_msg}");
            return Err(err_msg);
        }
    }
    Ok(())
}

async fn send_std_message<T: Send + 'static>(
    sender: &StdSender<T>,
    message: T,
    log_context: &str, // 用于日志，例如 "状态更新" 或 "SMTC命令"
) -> Result<(), String> {
    let sender_clone = sender.clone(); // StdSender is Clone
    match task::spawn_blocking(move || sender_clone.send(message)).await {
        Ok(Ok(())) => {
            // log::trace!("[WebSocket 客户端] 成功发送 {} 到 std::mpsc 通道。", log_context);
            Ok(())
        }
        Ok(Err(e)) => {
            // SendError from std::sync::mpsc::Sender
            let err_msg = format!("发送 {log_context} 到 std::mpsc 通道失败: {e:?}");
            log::error!("[WebSocket 客户端] {err_msg}");
            Err(err_msg)
        }
        Err(e) => {
            // JoinError from spawn_blocking itself (e.g., task panicked or was cancelled)
            let err_msg = format!("spawn_blocking 执行 {log_context} 发送失败: {e:?}");
            log::error!("[WebSocket 客户端] {err_msg}");
            Err(err_msg)
        }
    }
}

/// 运行 WebSocket 客户端的主异步函数。
/// 负责建立连接、处理消息的接收与发送、报告状态以及实现重连逻辑。
pub async fn run_websocket_client(
    websocket_url: String,
    mut outgoing_rx: TokioReceiver<ProtocolBody>, // 从外部接收待发送消息的通道 (Tokio MPSC)
    status_tx: StdSender<WebsocketStatus>, // 用于向外部报告 WebSocket 连接状态的通道 (Std MPSC)
    smtc_control_tx: StdSender<SmtcControlCommand>, // 用于向 SMTC 控制器发送命令的通道 (Std MPSC)
    mut shutdown_rx: OneshotReceiver<()>,  // 用于接收外部关闭信号的通道 (Tokio Oneshot)
) {
    log::info!("[WebSocket 客户端] 启动，目标 URL: {websocket_url}");

    // 用于跟踪连续初始连接失败的次数
    let mut consecutive_failures: u32 = 0;

    // 主循环，负责管理连接的建立和重连
    'main_loop: loop {
        let outcome = 'connection_attempt: loop {
            // --- 阶段 1: 尝试建立连接 ---
            log::info!(
                "[WebSocket 客户端] 正在尝试连接... (已连续失败: {} 次)",
                consecutive_failures
            );
            if send_std_message(&status_tx, WebsocketStatus::连接中, "连接中状态")
                .await
                .is_err()
            {
                break 'connection_attempt LifecycleEndReason::CriticalChannelFailure(
                    "发送 '连接中' 状态失败".to_string(),
                );
            }

            let connect_result =
                tokio::time::timeout(CONNECT_TIMEOUT_DURATION, connect_async(&websocket_url)).await;

            let ws_stream = match connect_result {
                // 连接成功
                Ok(Ok((stream, response))) => {
                    log::info!(
                        "[WebSocket 客户端] 成功连接到服务器。HTTP 状态码: {}",
                        response.status()
                    );
                    consecutive_failures = 0; // 连接成功，重置失败计数
                    if send_std_message(&status_tx, WebsocketStatus::已连接, "已连接状态")
                        .await
                        .is_err()
                    {
                        break 'connection_attempt LifecycleEndReason::CriticalChannelFailure(
                            "发送 '已连接' 状态失败".to_string(),
                        );
                    }
                    stream
                }
                // 握手失败
                Ok(Err(e)) => {
                    break 'connection_attempt LifecycleEndReason::InitialConnectFailed(format!(
                        "连接握手失败: {e}"
                    ));
                }
                // 连接超时
                Err(_elapsed) => {
                    break 'connection_attempt LifecycleEndReason::InitialConnectFailed(format!(
                        "连接超时 (超过 {} 秒)",
                        CONNECT_TIMEOUT_DURATION.as_secs()
                    ));
                }
            };

            // --- 阶段 2: 处理已建立的连接 ---
            let (mut ws_writer, mut ws_reader) = ws_stream.split();
            let (internal_pong_tx, mut internal_pong_rx) = tokio::sync::mpsc::channel(5);

            let mut last_seek_request_info: Option<(u64, Instant)> = None;
            let mut last_volume_set_processed_time: Option<Instant> = None;
            let mut app_ping_interval_timer = tokio::time::interval(APP_PING_INTERVAL);
            app_ping_interval_timer.tick().await;
            let mut last_app_ping_sent_at: Option<Instant> = None;
            let mut waiting_for_app_pong = false;

            // 内部消息处理循环
            let connection_outcome = loop {
                tokio::select! {
                    biased; // 优先处理关闭信号

                    // 1. 处理外部关闭信号
                    _ = &mut shutdown_rx => {
                        log::trace!("[WebSocket 客户端] (活跃连接期间) 收到外部关闭信号。");
                        let _ = ws_writer.close().await; // 尝试优雅关闭 WebSocket
                        break LifecycleEndReason::ShutdownSignalReceived;
                    }

                    // 2. 处理从外部 (Worker) 发来的待发送消息 (来自 Tokio MPSC)
                    maybe_body_to_send = outgoing_rx.recv() => {
                        if let Some(body_to_send) = maybe_body_to_send {
                            if send_ws_message(&mut ws_writer, body_to_send).await.is_err() {
                                break LifecycleEndReason::StreamFailure("发送主通道消息失败".to_string());
                            }
                        } else {
                            log::error!("[WebSocket 客户端] 主发送通道 (outgoing_rx) 已关闭。");
                            let _ = ws_writer.close().await;
                            break LifecycleEndReason::StreamFailure("主发送通道已关闭".to_string());
                        }
                    }

                    // 3. 处理内部需要发送的消息 (例如，回复服务器的 Pong, 来自内部 Tokio MPSC)
                    maybe_internal_msg_to_send = internal_pong_rx.recv() => {
                        if let Some(internal_msg_to_send) = maybe_internal_msg_to_send {
                            if send_ws_message(&mut ws_writer, internal_msg_to_send).await.is_err() {
                                break LifecycleEndReason::StreamFailure("发送内部 Pong 消息失败".to_string());
                            }
                        } else {
                             log::error!("[WebSocket 客户端] 内部 Pong 通道 (internal_pong_rx) 已关闭。");
                            let _ = ws_writer.close().await;
                            break LifecycleEndReason::StreamFailure("内部 Pong 通道已关闭".to_string());
                        }
                    }

                    // 4. 处理从 WebSocket 服务器接收到的消息
                    ws_msg_option = ws_reader.next() => {
                        match ws_msg_option {
                            Some(Ok(message_type)) => {
                                match message_type {
                                    WsMessage::Binary(bin_data) => {
                                        match deserialize_protocol_body(&bin_data) {
                                            Ok(parsed_body) => {
                                                match parsed_body {
                                                    ProtocolBody::Ping => {
                                                        log::info!("[WebSocket 客户端] 收到服务器的 Ping 请求。准备回复 Pong。");
                                                        if internal_pong_tx.send(ProtocolBody::Pong).await.is_err() {
                                                            log::error!("[WebSocket 客户端] 无法将应用层 Pong 排队以回复服务器。");
                                                            break LifecycleEndReason::StreamFailure("排队回复服务器 Ping 失败".to_string());
                                                        }
                                                    }
                                                    ProtocolBody::Pong => {
                                                        log::info!("[WebSocket 客户端] 收到服务器的 Pong 回复。");
                                                        if waiting_for_app_pong {
                                                            waiting_for_app_pong = false;
                                                            last_app_ping_sent_at = None;
                                                        } else {
                                                            log::warn!("[WebSocket 客户端] 收到意外的 Pong (当前未在等待 Pong，或已超时并重置状态)。");
                                                        }
                                                    }
                                                    ProtocolBody::Pause => {
                                                        log::info!("[WebSocket 客户端] 收到服务器命令: 暂停。");
                                                        last_seek_request_info = None;
                                                        if send_std_message(&smtc_control_tx, SmtcControlCommand::Pause, "SMTC暂停命令").await.is_err() {
                                                            log::error!("[WebSocket 客户端] 发送“暂停”命令到 SMTC 处理器失败。");
                                                            // 根据业务逻辑决定是否 return StreamFailure
                                                        }
                                                        if send_ws_message(&mut ws_writer, ProtocolBody::OnPaused).await.is_err() {
                                                            log::error!("[WebSocket 客户端] 发送“已暂停”响应到服务器失败。");
                                                        }
                                                    }
                                                    ProtocolBody::Resume => {
                                                        log::info!("[WebSocket 客户端] 收到服务器命令: 播放。");
                                                        last_seek_request_info = None;
                                                        if send_std_message(&smtc_control_tx, SmtcControlCommand::Play, "SMTC播放命令").await.is_err() {
                                                            log::error!("[WebSocket 客户端] 发送“播放”命令到 SMTC 处理器失败。");
                                                        }
                                                        if send_ws_message(&mut ws_writer, ProtocolBody::OnResumed).await.is_err() {
                                                            log::error!("[WebSocket 客户端] 发送“已播放”响应到服务器失败。");
                                                        }
                                                    }
                                                    ProtocolBody::ForwardSong => {
                                                        log::info!("[WebSocket 客户端] 收到服务器命令: 下一首。");
                                                        if send_std_message(&smtc_control_tx, SmtcControlCommand::SkipNext, "SMTC下一首命令").await.is_err() {
                                                            log::error!("[WebSocket 客户端] 发送“下一首”命令到 SMTC 处理器失败。");
                                                        }
                                                    }
                                                    ProtocolBody::BackwardSong => {
                                                        log::info!("[WebSocket 客户端] 收到服务器命令: 上一首。");
                                                        if send_std_message(&smtc_control_tx, SmtcControlCommand::SkipPrevious, "SMTC上一首命令").await.is_err() {
                                                            log::error!("[WebSocket 客户端] 发送“上一首”命令到 SMTC 处理器失败。");
                                                        }
                                                    }
                                                    ProtocolBody::SeekPlayProgress { progress } => {
                                                        let now = Instant::now();
                                                        let mut process_this_seek = true;
                                                        if let Some((last_progress, last_time)) = last_seek_request_info
                                                            && progress == last_progress && now.duration_since(last_time) < SEEK_DEBOUNCE_DURATION {
                                                                process_this_seek = false;
                                                            }
                                                        if process_this_seek {
                                                            log::info!("[WebSocket 客户端] 收到服务器命令: 跳转到 {progress}.");
                                                            last_seek_request_info = Some((progress, now));
                                                            if send_std_message(&smtc_control_tx, SmtcControlCommand::SeekTo(progress), "SMTC跳转命令").await.is_err() {
                                                                log::error!("[WebSocket 客户端] 发送“跳转”命令到 SMTC 处理器失败。");
                                                            }
                                                        }
                                                    }
                                                    ProtocolBody::SetVolume { volume } => {
                                                        let now = Instant::now();
                                                        let mut process_this_volume_set = true;
                                                        if let Some(last_time) = last_volume_set_processed_time
                                                            && now.duration_since(last_time) < MIN_VOLUME_SET_INTERVAL {
                                                                process_this_volume_set = false;
                                                            }
                                                        if process_this_volume_set {
                                                            log::info!("[WebSocket 客户端] 收到服务器命令: 设置音量为 {volume:.2}");
                                                            last_volume_set_processed_time = Some(now);
                                                            if (0.0..=1.0).contains(&volume) {
                                                                let volume_f32 = volume as f32;
                                                                let command_to_send = SmtcControlCommand::SetVolume(volume_f32);
                                                                if send_std_message(&smtc_control_tx, command_to_send, "SMTC设置音量命令").await.is_err() {
                                                                    error!("[WebSocket 客户端] 发送 SetVolume({volume_f32:.2}) 命令到 Worker 失败。");
                                                                }
                                                            } else {
                                                                warn!("[WebSocket 客户端] 收到的音量值 {volume} 超出有效范围 (0.0-1.0)，已忽略。");
                                                            }
                                                        }
                                                    }
                                                    ProtocolBody::OnPaused => { log::info!("[WebSocket 客户端] 收到服务器事件: 播放已暂停。"); }
                                                    ProtocolBody::OnResumed => { log::info!("[WebSocket 客户端] 收到服务器事件: 播放已恢复。"); }
                                                    ProtocolBody::OnVolumeChanged { volume } => { log::info!("[WebSocket 客户端] 收到服务器事件: 音量更改为 {volume}."); }
                                                    ProtocolBody::OnAudioData { .. } => { log::warn!("[WebSocket 客户端] 收到服务器 OnAudioData 消息，已忽略。"); }
                                                    ProtocolBody::SetMusicAlbumCoverImageURI { img_url } => { log::warn!("[WebSocket 客户端] 服务器请求设置专辑封面 URI ({img_url:?})，已忽略。"); }
                                                    p @ ProtocolBody::SetLyricFromTTML { .. } |
                                                    p @ ProtocolBody::SetMusicInfo { .. } |
                                                    p @ ProtocolBody::OnPlayProgress { .. } |
                                                    p @ ProtocolBody::SetMusicAlbumCoverImageData { .. }
                                                    => { log::warn!("[WebSocket 客户端] 收到了应该是我们发送的信息类型: {p:?}"); }
                                                    _ => { log::error!("[WebSocket 客户端] 收到未处理或未知的协议消息体: {parsed_body:?}"); }
                                                }
                                            }
                                            Err(e) => {
                                                log::error!("[WebSocket 客户端] 反序列化服务器二进制消息失败: {:?}. 数据 (前16字节十六进制): {:02X?}", e, &bin_data[..std::cmp::min(bin_data.len(), 16)]);
                                                break LifecycleEndReason::StreamFailure("收到无法解析的二进制消息".to_string());
                                            }
                                        }
                                    }
                                    WsMessage::Text(text_msg) => { log::warn!("[WebSocket 客户端] 收到意外的文本消息: {text_msg}"); }
                                    WsMessage::Ping(ping_payload) => { log::trace!("[WebSocket 客户端] 收到 WebSocket 底层 PING: {ping_payload:?}"); }
                                    WsMessage::Pong(pong_payload) => { log::trace!("[WebSocket 客户端] 收到 WebSocket 底层 PONG: {pong_payload:?}"); }
                                    WsMessage::Close(close_frame) => {
                                        log::error!("[WebSocket 客户端] 服务器发送了 WebSocket 关闭帧: {close_frame:?}. 中断当前连接。");
                                        break LifecycleEndReason::ServerClosed;
                                    }
                                    WsMessage::Frame(frame) => { log::trace!("[WebSocket 客户端] 从服务器收到原始 WebSocket 帧: {:?}", frame.header()); }
                                }
                            }
                            Some(Err(e)) => {
                                log::error!("[WebSocket 客户端] 从 WebSocket 流读取消息时发生错误: {e:?}. 中断当前连接。");
                                break LifecycleEndReason::StreamFailure("WebSocket读取错误".to_string());
                            }
                            None => {
                                log::error!("[WebSocket 客户端] WebSocket 流已关闭 (读取到 None)，服务器可能已断开连接。中断当前连接。");
                                break LifecycleEndReason::ServerClosed;
                            }
                        }
                    }

                    // 5. 处理应用层 Ping 定时器
                    _ = app_ping_interval_timer.tick() => {
                        if waiting_for_app_pong {
                            if let Some(sent_at) = last_app_ping_sent_at {
                                if Instant::now().duration_since(sent_at) > APP_PONG_TIMEOUT {
                                    log::warn!("[WebSocket 客户端] 服务器应用层 Pong 超时! 正在断开连接。");
                                    let _ = ws_writer.close().await;
                                    break LifecycleEndReason::PongTimeout;
                                }
                            } else {
                                log::error!("[WebSocket 客户端] 心跳逻辑状态不一致：正在等待 Pong，但上次未发送 Ping。重置状态。");
                                waiting_for_app_pong = false;
                            }
                        } else {
                            log::info!("[WebSocket 客户端] 定时发送 Ping 到服务器。");
                            if send_ws_message(&mut ws_writer, ProtocolBody::Ping).await.is_err() {
                                break LifecycleEndReason::StreamFailure("发送应用层 Ping 失败".to_string());
                            }
                            last_app_ping_sent_at = Some(Instant::now());
                            waiting_for_app_pong = true;
                        }
                    }
                } // 内部消息处理 select! 循环结束
            };
            break 'connection_attempt connection_outcome;
        }; // `outcome` is determined here

        // --- 阶段 3: 统一处理所有连接结束事件 ---
        match outcome {
            LifecycleEndReason::ShutdownSignalReceived => {
                log::info!("[WebSocket 客户端] 因收到关闭信号而退出。");
                send_std_message(&status_tx, WebsocketStatus::断开, "关闭时断开状态")
                    .await
                    .ok();
                break 'main_loop;
            }
            LifecycleEndReason::CriticalChannelFailure(reason) => {
                log::error!("[WebSocket 客户端] 发生关键通道错误: {reason}。任务将退出。");
                send_std_message(&status_tx, WebsocketStatus::错误(reason), "关键错误状态")
                    .await
                    .ok();
                break 'main_loop;
            }
            reason @ (LifecycleEndReason::InitialConnectFailed(_)
            | LifecycleEndReason::StreamFailure(_)
            | LifecycleEndReason::PongTimeout
            | LifecycleEndReason::ServerClosed) => {
                let error_message = match reason.clone() {
                    LifecycleEndReason::InitialConnectFailed(msg) => msg,
                    LifecycleEndReason::StreamFailure(msg) => format!("连接流错误: {msg}"),
                    LifecycleEndReason::PongTimeout => "心跳响应超时".to_string(),
                    LifecycleEndReason::ServerClosed => "服务器关闭了连接".to_string(),
                    _ => "未知错误".to_string(), // 不会发生
                };

                log::warn!(
                    "[WebSocket 客户端] 连接生命周期因 '{error_message}' 而结束，准备重连..."
                );
                send_std_message(&status_tx, WebsocketStatus::错误(error_message), "错误状态")
                    .await
                    .ok();

                // 任何导致重连的失败都应该增加计数器
                consecutive_failures += 1;
            }
        }

        // --- 阶段 4: 重连延迟与退出检查 ---
        if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            log::error!(
                "[WebSocket 客户端] 已达到最大连续失败连接次数 ({})。",
                MAX_CONSECUTIVE_FAILURES
            );
            let msg = format!("已达最大重连次数 ({})", MAX_CONSECUTIVE_FAILURES);
            send_std_message(&status_tx, WebsocketStatus::错误(msg), "最大重连次数状态")
                .await
                .ok();

            log::info!("[WebSocket 客户端] 暂停自动重连，等待外部指令。");
            tokio::select! { biased; _ = &mut shutdown_rx => {} }
            break 'main_loop;
        }

        log::debug!(
            "[WebSocket 客户端] 将等待 {}ms 后尝试下一次连接...",
            RECONNECT_DELAY_MS
        );
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                log::info!("[WebSocket 客户端] (重连延迟期间) 收到关闭信号，任务退出。");
                break 'main_loop;
            }
            _ = sleep(Duration::from_millis(RECONNECT_DELAY_MS)) => {}
        }
    } // 'main_loop' 结束

    log::info!("[WebSocket 客户端] 任务已完全停止。");
}
