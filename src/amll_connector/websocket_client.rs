use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use log::{error, warn};
use std::sync::mpsc::Sender as StdSender;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Receiver as TokioReceiver, Sender as TokioMSender};
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
#[derive(Debug, Clone, Copy)]
enum LifecycleEndReason {
    PongTimeout,                                     // Pong 响应超时
    InitialConnectFailed(&'static str),              // 初始连接失败，附带错误描述
    StreamFailure(#[allow(dead_code)] &'static str), // WebSocket 流错误，附带错误描述
    ServerClosed,                                    // 服务器关闭了连接
    ShutdownSignalReceived,                          // 收到了外部关闭信号
    CriticalChannelFailure(&'static str),            // 关键的内部通道发生故障，附带错误描述
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
                    format!("OnPlayProgress(进度:{})", progress)
                }
                ProtocolBody::Ping => "Ping (应用层 - 发往服务器)".to_string(),
                ProtocolBody::Pong => "Pong (应用层 - 回复服务器)".to_string(),
                _ => {
                    let debug_str = format!("{:?}", body);
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
                let err_msg = format!(
                    "发送 WebSocket 二进制消息 (类型: {}) 失败: {:?}",
                    body_type_for_log, e
                );
                log::error!("[WebSocket 客户端] 发送失败: {}", err_msg);
                return Err(err_msg);
            } else if matches!(body, ProtocolBody::Pong) {
                log::info!("[WebSocket 客户端] 已成功发送 Pong 到服务器。");
            } else {
                log::trace!("[WebSocket 客户端] 已成功发送 {} 消息。", body_type_for_log);
            }
        }
        Err(e) => {
            // 协议体序列化失败
            let err_msg = format!("序列化 ProtocolBody {:?} 失败: {:?}", body, e);
            log::error!("[WebSocket 客户端] 序列化失败: {}", err_msg);
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
            let err_msg = format!("发送 {} 到 std::mpsc 通道失败: {:?}", log_context, e);
            log::error!("[WebSocket 客户端] {}", err_msg);
            Err(err_msg)
        }
        Err(e) => {
            // JoinError from spawn_blocking itself (e.g., task panicked or was cancelled)
            let err_msg = format!("spawn_blocking 执行 {} 发送失败: {:?}", log_context, e);
            log::error!("[WebSocket 客户端] {}", err_msg);
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
    log::info!("[WebSocket 客户端] 启动，目标 URL: {}", websocket_url);

    let mut consecutive_failures: u32 = 0;
    let mut last_seek_request_info: Option<(u64, Instant)> = None;
    let mut last_volume_set_processed_time: Option<Instant> = None;

    'main_loop: loop {
        if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            log::info!(
                "[WebSocket 客户端] 已达到最大连续失败连接次数 ({})，将暂停自动重连。",
                MAX_CONSECUTIVE_FAILURES
            );
            let msg_to_send = WebsocketStatus::错误(format!(
                "已达到最大重连次数 ({})，请稍后重试。",
                MAX_CONSECUTIVE_FAILURES
            ));
            // 使用辅助函数发送状态
            if send_std_message(&status_tx, msg_to_send, "最大重连次数状态")
                .await
                .is_err()
            {
                log::error!(
                    "[WebSocket 客户端] 发送 '已达最大重连次数' 状态失败，Worker 可能已关闭。"
                );
                // 即使发送失败，也应该中断主循环，因为无法通知外部
                break 'main_loop;
            }
            log::trace!("[WebSocket 客户端] 自动重连已暂停，等待外部关闭信号或程序重启...");
            tokio::select! { biased; _ = &mut shutdown_rx => {
                log::trace!("[WebSocket 客户端] (重连已暂停时) 收到关闭信号，任务即将退出。");
            }}
            break 'main_loop;
        }

        let outcome: LifecycleEndReason = async {
            log::info!("[WebSocket 客户端] 正在尝试连接到: {} (当前失败次数: {})", websocket_url, consecutive_failures);
            // 使用辅助函数发送状态
            if send_std_message(&status_tx, WebsocketStatus::连接中, "连接中状态").await.is_err() {
                log::error!("[WebSocket 客户端] 发送 '连接中' 状态失败，Worker 可能已关闭。");
                return LifecycleEndReason::CriticalChannelFailure("发送“连接中”状态失败");
            }

            match tokio::time::timeout(CONNECT_TIMEOUT_DURATION, connect_async(&websocket_url)).await {
                Ok(Ok((ws_stream, response))) => {
                    log::info!("[WebSocket 客户端] 成功连接到 WebSocket 服务器: {}. HTTP 状态码: {}", websocket_url, response.status());

                    // 使用辅助函数发送状态
                    if send_std_message(&status_tx, WebsocketStatus::已连接, "已连接状态").await.is_err() {
                        log::error!("[WebSocket 客户端] 发送 '已连接' 状态失败，Worker 可能已关闭。");
                        // 不立即认为是关键错误，因为连接本身是成功的
                    }

                    let (mut ws_writer, mut ws_reader) = ws_stream.split();
                    // 内部 Tokio MPSC 通道，用于从读取逻辑发送 Pong 到发送逻辑
                    let (internal_pong_tx, mut internal_pong_rx): (TokioMSender<ProtocolBody>, TokioReceiver<ProtocolBody>) = tokio::sync::mpsc::channel(5);
                    last_seek_request_info = None;
                    last_volume_set_processed_time = None;
                    let mut app_ping_interval_timer = tokio::time::interval(APP_PING_INTERVAL);
                    app_ping_interval_timer.tick().await; // 消耗第一次立即的 tick
                    let mut last_app_ping_sent_at: Option<Instant> = None;
                    let mut waiting_for_app_pong = false;

                    loop { // 活跃连接的消息处理循环
                        tokio::select! {
                            biased; // 优先处理关闭信号

                            // 1. 处理外部关闭信号
                            _ = &mut shutdown_rx => {
                                log::trace!("[WebSocket 客户端] (活跃连接期间) 收到外部关闭信号。");
                                let _ = ws_writer.close().await; // 尝试优雅关闭 WebSocket
                                return LifecycleEndReason::ShutdownSignalReceived;
                            }

                            // 2. 处理从外部 (Worker) 发来的待发送消息 (来自 Tokio MPSC)
                            maybe_body_to_send = outgoing_rx.recv() => {
                                if let Some(body_to_send) = maybe_body_to_send {
                                    if send_ws_message(&mut ws_writer, body_to_send).await.is_err() {
                                        return LifecycleEndReason::StreamFailure("发送主通道消息失败");
                                    }
                                } else {
                                    log::error!("[WebSocket 客户端] 主发送通道 (outgoing_rx) 已关闭。");
                                    let _ = ws_writer.close().await;
                                    return LifecycleEndReason::StreamFailure("主发送通道已关闭");
                                }
                            }

                            // 3. 处理内部需要发送的消息 (例如，回复服务器的 Pong, 来自内部 Tokio MPSC)
                            maybe_internal_msg_to_send = internal_pong_rx.recv() => {
                                if let Some(internal_msg_to_send) = maybe_internal_msg_to_send {
                                    if send_ws_message(&mut ws_writer, internal_msg_to_send).await.is_err() {
                                        return LifecycleEndReason::StreamFailure("发送内部 Pong 消息失败");
                                    }
                                } else {
                                     log::error!("[WebSocket 客户端] 内部 Pong 通道 (internal_pong_rx) 已关闭。");
                                    let _ = ws_writer.close().await;
                                    return LifecycleEndReason::StreamFailure("内部 Pong 通道已关闭");
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
                                                                    return LifecycleEndReason::StreamFailure("排队回复服务器 Ping 失败");
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
                                                                if let Some((last_progress, last_time)) = last_seek_request_info {
                                                                    if progress == last_progress && now.duration_since(last_time) < SEEK_DEBOUNCE_DURATION {
                                                                        process_this_seek = false;
                                                                    }
                                                                }
                                                                if process_this_seek {
                                                                    log::info!("[WebSocket 客户端] 收到服务器命令: 跳转到 {}.", progress);
                                                                    last_seek_request_info = Some((progress, now));
                                                                    if send_std_message(&smtc_control_tx, SmtcControlCommand::SeekTo(progress), "SMTC跳转命令").await.is_err() {
                                                                        log::error!("[WebSocket 客户端] 发送“跳转”命令到 SMTC 处理器失败。");
                                                                    }
                                                                }
                                                            }
                                                            ProtocolBody::SetVolume { volume } => {
                                                                let now = Instant::now();
                                                                let mut process_this_volume_set = true;
                                                                if let Some(last_time) = last_volume_set_processed_time {
                                                                    if now.duration_since(last_time) < MIN_VOLUME_SET_INTERVAL {
                                                                        process_this_volume_set = false;
                                                                    }
                                                                }
                                                                if process_this_volume_set {
                                                                    log::info!("[WebSocket 客户端] 收到服务器命令: 设置音量为 {:.2}", volume);
                                                                    last_volume_set_processed_time = Some(now);
                                                                    if (0.0..=1.0).contains(&volume) {
                                                                        let volume_f32 = volume as f32;
                                                                        let command_to_send = SmtcControlCommand::SetVolume(volume_f32);
                                                                        if send_std_message(&smtc_control_tx, command_to_send, "SMTC设置音量命令").await.is_err() {
                                                                            error!("[WebSocket 客户端] 发送 SetVolume({:.2}) 命令到 Worker 失败。", volume_f32);
                                                                        }
                                                                    } else {
                                                                        warn!("[WebSocket 客户端] 收到的音量值 {} 超出有效范围 (0.0-1.0)，已忽略。", volume);
                                                                    }
                                                                }
                                                            }
                                                            ProtocolBody::OnPaused => { log::info!("[WebSocket 客户端] 收到服务器事件: 播放已暂停。"); }
                                                            ProtocolBody::OnResumed => { log::info!("[WebSocket 客户端] 收到服务器事件: 播放已恢复。"); }
                                                            ProtocolBody::OnVolumeChanged { volume } => { log::info!("[WebSocket 客户端] 收到服务器事件: 音量更改为 {}.", volume); }
                                                            ProtocolBody::OnAudioData { .. } => { log::warn!("[WebSocket 客户端] 收到服务器 OnAudioData 消息，已忽略。"); }
                                                            ProtocolBody::SetMusicAlbumCoverImageURI { img_url } => { log::warn!("[WebSocket 客户端] 服务器请求设置专辑封面 URI ({:?})，已忽略。", img_url); }
                                                            p @ ProtocolBody::SetLyricFromTTML { .. } |
                                                            p @ ProtocolBody::SetMusicInfo { .. } |
                                                            p @ ProtocolBody::OnPlayProgress { .. } |
                                                            p @ ProtocolBody::SetMusicAlbumCoverImageData { .. }
                                                            => { log::warn!("[WebSocket 客户端] 收到了应该是我们发送的信息类型: {:?}", p); }
                                                            _ => { log::error!("[WebSocket 客户端] 收到未处理或未知的协议消息体: {:?}", parsed_body); }
                                                        }
                                                    }
                                                    Err(e) => {
                                                        log::error!("[WebSocket 客户端] 反序列化服务器二进制消息失败: {:?}. 数据 (前16字节十六进制): {:02X?}", e, &bin_data[..std::cmp::min(bin_data.len(), 16)]);
                                                    }
                                                }
                                            }
                                            WsMessage::Text(text_msg) => { log::warn!("[WebSocket 客户端] 收到意外的文本消息: {}", text_msg); }
                                            WsMessage::Ping(ping_payload) => { log::trace!("[WebSocket 客户端] 收到 WebSocket 底层 PING: {:?}", ping_payload); }
                                            WsMessage::Pong(pong_payload) => { log::trace!("[WebSocket 客户端] 收到 WebSocket 底层 PONG: {:?}", pong_payload); }
                                            WsMessage::Close(close_frame) => {
                                                log::error!("[WebSocket 客户端] 服务器发送了 WebSocket 关闭帧: {:?}. 中断当前连接。", close_frame);
                                                return LifecycleEndReason::ServerClosed;
                                            }
                                            WsMessage::Frame(frame) => { log::trace!("[WebSocket 客户端] 从服务器收到原始 WebSocket 帧: {:?}", frame.header()); }
                                        }
                                    }
                                    Some(Err(e)) => {
                                        log::error!("[WebSocket 客户端] 从 WebSocket 流读取消息时发生错误: {:?}. 中断当前连接。", e);
                                        return LifecycleEndReason::StreamFailure("WebSocket读取错误");
                                    }
                                    None => {
                                        log::error!("[WebSocket 客户端] WebSocket 流已关闭 (读取到 None)，服务器可能已断开连接。中断当前连接。");
                                        return LifecycleEndReason::ServerClosed;
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
                                            return LifecycleEndReason::PongTimeout;
                                        }
                                    } else {
                                        log::error!("[WebSocket 客户端] 心跳逻辑状态不一致：正在等待 Pong，但上次未发送 Ping。重置状态。");
                                        waiting_for_app_pong = false;
                                    }
                                } else {
                                    log::info!("[WebSocket 客户端] 定时发送 Ping 到服务器。");
                                    if send_ws_message(&mut ws_writer, ProtocolBody::Ping).await.is_err() {
                                        return LifecycleEndReason::StreamFailure("发送应用层 Ping 失败");
                                    }
                                    last_app_ping_sent_at = Some(Instant::now());
                                    waiting_for_app_pong = true;
                                }
                            }
                        } // 内部消息处理 select! 循环结束
                    } // 活跃连接的消息处理 loop 结束
                } // end Ok(Ok((ws_stream, response)))
                Ok(Err(e)) => { // connect_async 返回错误
                    log::error!("[WebSocket 客户端] 连接到 WebSocket 服务器失败: {:?}", e);
                    let msg_to_send = WebsocketStatus::错误(format!("连接握手失败: {}", e));
                    if send_std_message(&status_tx, msg_to_send, "连接握手失败状态").await.is_err() {
                        return LifecycleEndReason::CriticalChannelFailure("发送“连接握手失败”状态失败");
                    }
                    LifecycleEndReason::InitialConnectFailed("连接握手错误")
                }
                Err(_elapsed) => { // connect_async 超时
                    log::error!("[WebSocket 客户端] 连接到 WebSocket 服务器尝试超时 (超过 {} 秒)。", CONNECT_TIMEOUT_DURATION.as_secs());
                    let msg_to_send = WebsocketStatus::错误("连接超时".to_string());
                    if send_std_message(&status_tx, msg_to_send, "连接超时状态").await.is_err() {
                        return LifecycleEndReason::CriticalChannelFailure("发送“连接超时”状态失败");
                    }
                    LifecycleEndReason::InitialConnectFailed("连接超时错误")
                }
            } // end match connect_async
        }.await; // 单个连接生命周期 `async` 块结束

        // 处理连接生命周期的结果
        if matches!(
            outcome,
            LifecycleEndReason::ShutdownSignalReceived
                | LifecycleEndReason::CriticalChannelFailure(_)
        ) {
            // 如果是这些原因，直接准备退出，不应该影响 consecutive_failures 或触发重连逻辑
        } else if !matches!(outcome, LifecycleEndReason::InitialConnectFailed(_)) {
            // 对于非初始连接失败的情况（例如 PongTimeout, StreamFailure, ServerClosed，或者正常关闭后准备重连）
            // 并且如果上一个状态是“已连接”（通过检查 outcome 是否是这些非初始失败类型来间接判断）
            // 则重置 consecutive_failures。
            // 一个更直接的方法是在成功连接时（即 connect_async 返回 Ok(Ok(...))）就重置。
            // 我们将采纳在成功连接时重置的策略，所以这里不需要特别处理。
        }
        if let Ok(Ok((_ws_stream, _response))) =
            tokio::time::timeout(CONNECT_TIMEOUT_DURATION, connect_async(&websocket_url)).await
        {
            // 如果是在上面的 async 块内部判断成功连接并返回一个特殊的 LifecycleEndReason::ConnectedSuccessfully
            // 然后在这里匹配并重置会更好。但由于 async 块的结构，我们在这里重置。
            // 实际上，更稳妥的做法是在 outcome 确定后，如果 outcome 不是 InitialConnectFailed，
            // 并且也不是 Shutdown/CriticalFailure，那么就意味着之前的连接尝试（如果失败了）不应计入“连续初始失败”。
            // 或者，更简单：只在 InitialConnectFailed 时增加，在成功连接时清零。
        }

        // 处理连接生命周期的结果
        let mut apply_reconnect_delay = false;
        match outcome {
            LifecycleEndReason::PongTimeout
            | LifecycleEndReason::StreamFailure(_)
            | LifecycleEndReason::ServerClosed => {
                log::info!(
                    "[WebSocket 客户端] 因 {:?} 而断开连接，准备尝试重连...",
                    outcome
                );
                // 对于这些在已连接后发生的断开，我们不应该增加 consecutive_failures。
                // 如果之前 consecutive_failures 因 InitialConnectFailed 而累积，这里应该考虑是否重置。
                // 最好的做法是在成功连接时重置。
            }
            LifecycleEndReason::InitialConnectFailed(err_msg) => {
                log::error!("[WebSocket 客户端] 初始连接失败: '{}'。", err_msg);
                consecutive_failures += 1; // 只在这里增加
                if consecutive_failures < MAX_CONSECUTIVE_FAILURES {
                    apply_reconnect_delay = true;
                }
            }
            LifecycleEndReason::ShutdownSignalReceived => {
                log::debug!("[WebSocket 客户端] 收到外部关闭信号，正在退出主循环。");
                break 'main_loop;
            }
            LifecycleEndReason::CriticalChannelFailure(err_msg) => {
                log::error!(
                    "[WebSocket 客户端] 发生关键内部通道错误: '{}'。任务将退出。",
                    err_msg
                );
                break 'main_loop;
            }
        }

        match outcome {
            LifecycleEndReason::PongTimeout
            | LifecycleEndReason::StreamFailure(_)
            | LifecycleEndReason::ServerClosed => {
                if consecutive_failures > 0 {
                    consecutive_failures = 0;
                }
            }
            LifecycleEndReason::InitialConnectFailed(_) => {
                // consecutive_failures 已经在上面增加了
            }
            LifecycleEndReason::ShutdownSignalReceived
            | LifecycleEndReason::CriticalChannelFailure(_) => {
                // 这些情况不需要重置，因为要退出了
            }
        }

        if apply_reconnect_delay {
            log::debug!(
                "[WebSocket 客户端] 将等待 {}ms 后尝试重连 (已连续失败: {} 次)...",
                RECONNECT_DELAY_MS,
                consecutive_failures
            );
            tokio::select! {
                biased;
                _ = &mut shutdown_rx => {
                    log::trace!("[WebSocket 客户端] (重连延迟期间) 收到关闭信号，任务退出。");
                    break 'main_loop;
                }
                _ = sleep(Duration::from_millis(RECONNECT_DELAY_MS)) => {
                    log::trace!("[WebSocket 客户端] 重连延迟结束，准备下一次连接尝试。");
                }
            }
        } else if !matches!(
            outcome,
            LifecycleEndReason::ShutdownSignalReceived
                | LifecycleEndReason::CriticalChannelFailure(_)
        ) {
            // 对于非延迟重连的情况，也快速检查一下关闭信号
            if tokio::time::timeout(Duration::from_millis(1), &mut shutdown_rx)
                .await
                .is_ok()
            {
                log::trace!("[WebSocket 客户端] (准备立即重连前) 快速检查到关闭信号，任务退出。");
                break 'main_loop;
            }
        }
    } // 'main_loop' 结束
    log::trace!("[WebSocket 客户端] WebSocket 客户端已停止。");
}
