use futures_util::{SinkExt, StreamExt};
use smtc_suite::SmtcControlCommand;
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::Receiver as BroadcastReceiver;
use tokio::sync::mpsc::Sender as TokioSender;
use tokio::sync::oneshot::Receiver as OneshotReceiver;
use tokio_tungstenite::{accept_async, tungstenite::protocol::Message as WsMessage};
use tracing::{error, info, warn};

use super::protocol_v2::*;

pub async fn run_websocket_server(
    port: u16,
    base_broadcast_rx: BroadcastReceiver<OutgoingMessage>,
    media_cmd_tx: TokioSender<SmtcControlCommand>,
    new_conn_tx: TokioSender<()>,
    mut shutdown_rx: OneshotReceiver<()>,
) -> anyhow::Result<()> {
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await;

    let listener = match listener {
        Ok(l) => l,
        Err(e) => {
            error!("[WebSocket 服务端] 绑定 {port} 失败: {e}");
            return Err(anyhow::anyhow!("绑定端口失败: {e}"));
        }
    };

    info!("[WebSocket 服务端] 启动成功，正在监听: {addr}");

    loop {
        tokio::select! {
            biased;

            _ = &mut shutdown_rx => {
                break;
            }

            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, client_addr)) => {
                        let broadcast_rx = base_broadcast_rx.resubscribe();
                        let cmd_tx = media_cmd_tx.clone();
                        let notify_tx = new_conn_tx.clone();

                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream, client_addr, broadcast_rx, cmd_tx, notify_tx).await {
                                warn!("[WebSocket 服务端] 客户端 {client_addr} 连接意外地结束了: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        warn!("[WebSocket 服务端] 接受连接失败: {e}");
                    }
                }
            }
        }
    }

    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    mut broadcast_rx: BroadcastReceiver<OutgoingMessage>,
    media_cmd_tx: TokioSender<SmtcControlCommand>,
    new_conn_tx: TokioSender<()>,
) -> anyhow::Result<()> {
    let ws_stream = accept_async(stream).await?;
    info!("[WebSocket 服务端] 客户端 {addr} 已连接",);

    let _ = new_conn_tx.try_send(());

    let (mut ws_writer, mut ws_reader) = ws_stream.split();

    loop {
        tokio::select! {
            biased;

            msg_result = broadcast_rx.recv() => {
                match msg_result {
                    Ok(outgoing_msg) => {
                        let ws_message = match outgoing_msg {
                            OutgoingMessage::Json(v2_msg) => {
                                match serde_json::to_string(&v2_msg) {
                                    Ok(text) => WsMessage::Text(text.into()),
                                    Err(e) => {
                                        error!("[WebSocket 服务端] 序列化 JSON 失败: {e}");
                                        continue;
                                    }
                                }
                            }
                            OutgoingMessage::Binary(bin_body) => {
                                match to_binary_v2(&bin_body) {
                                    Ok(bytes) => WsMessage::Binary(bytes.into()),
                                    Err(e) => {
                                        error!("[WebSocket 服务端] 序列化二进制失败: {e}");
                                        continue;
                                    }
                                }
                            }
                        };

                        if let Err(e) = ws_writer.send(ws_message).await {
                            return Err(anyhow::anyhow!("发送消息失败: {e}"));
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        warn!("[WebSocket 服务端] 客户端 {addr} 滞后，丢失了 {count} 条消息");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Ok(());
                    }
                }
            }

            ws_msg_option = ws_reader.next() => {
                match ws_msg_option {
                    Some(Ok(message)) => {
                        match message {
                            WsMessage::Text(text) => {
                                match serde_json::from_str::<MessageV2>(&text) {
                                    Ok(parsed_msg) => {
                                        handle_incoming_payload(parsed_msg.payload, &media_cmd_tx, &mut ws_writer).await?;
                                    }
                                    Err(e) => {
                                        warn!("[WebSocket 服务端] 无法解析来自 {addr} 的消息: {e} | 内容: {text}" );
                                    }
                                }
                            }
                            WsMessage::Ping(data) => {
                                let _ = ws_writer.send(WsMessage::Pong(data)).await;
                            }
                            WsMessage::Close(_) => {
                                info!("[WebSocket 服务端] 客户端 {addr} 主动断开连接");
                                return Ok(());
                            }
                            _ => {}
                        }
                    }
                    Some(Err(e)) => {
                        return Err(anyhow::anyhow!("读取错误: {e}"));
                    }
                    None => {
                        info!("[WebSocket 服务端] 客户端 {addr} 连接已关闭");
                        return Ok(());
                    }
                }
            }
        }
    }
}

async fn handle_incoming_payload(
    payload: Payload,
    media_cmd_tx: &TokioSender<SmtcControlCommand>,
    writer: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        WsMessage,
    >,
) -> anyhow::Result<()> {
    match payload {
        Payload::Ping => {
            let pong = MessageV2 {
                payload: Payload::Pong,
            };
            if let Ok(text) = serde_json::to_string(&pong) {
                writer.send(WsMessage::Text(text.into())).await?;
            }
        }
        Payload::Command(cmd) => {
            if let Some(smtc_cmd) = convert_protocol_command_to_smtc(cmd)
                && let Err(e) = media_cmd_tx.send(smtc_cmd).await
            {
                warn!("[WebSocket 服务端] 转发命令失败: {e}");
            }
        }
        _ => {}
    }
    Ok(())
}

fn convert_protocol_command_to_smtc(cmd: Command) -> Option<SmtcControlCommand> {
    match cmd {
        Command::Pause => Some(SmtcControlCommand::Pause),
        Command::Resume => Some(SmtcControlCommand::Play),
        Command::ForwardSong => Some(SmtcControlCommand::SkipNext),
        Command::BackwardSong => Some(SmtcControlCommand::SkipPrevious),
        Command::SetVolume { volume } => Some(SmtcControlCommand::SetVolume(volume as f32)),
        Command::SeekPlayProgress { progress } => Some(SmtcControlCommand::SeekTo(progress)),
        Command::SetRepeatMode { mode } => {
            let smtc_mode = match mode {
                RepeatMode::Off => smtc_suite::RepeatMode::Off,
                RepeatMode::One => smtc_suite::RepeatMode::One,
                RepeatMode::All => smtc_suite::RepeatMode::All,
            };
            Some(SmtcControlCommand::SetRepeatMode(smtc_mode))
        }
        Command::SetShuffleMode { enabled } => Some(SmtcControlCommand::SetShuffle(enabled)),
    }
}
