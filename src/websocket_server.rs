use futures_util::{SinkExt, StreamExt};
use log::{error, info, warn};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio_tungstenite::{accept_async, tungstenite::protocol::Message}; // Only need Serialize here for messages sent to client

// --- 1. Define structures for JSON payloads to be sent to the client ---
#[derive(Serialize, Debug, Clone)]
pub struct PlaybackInfoPayload {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub ttml_lyrics: Option<String>, // TTML string, null or omitted if none
}

#[derive(Serialize, Debug, Clone)]
pub struct TimeUpdatePayload {
    pub current_time_seconds: f64, // Current playback time in seconds
}

// --- 2. Define the top-level message structure to be serialized to JSON ---
// This enum will be serialized with a "type" tag and a "payload" content.
#[derive(Serialize, Debug, Clone)]
#[serde(tag = "type", content = "payload")]
#[serde(rename_all = "snake_case")] // Ensures "PlaybackInfo" enum variant becomes "playback_info" type string
pub enum ClientViewMessage {
    PlaybackInfo(PlaybackInfoPayload),
    TimeUpdate(TimeUpdatePayload),
    // You can add other message types here if needed in the future
    // e.g., PlaybackState { playing: bool },
}

// --- 3. Update ServerCommand enum ---
// This enum is for internal communication from UniLyricApp to the WebsocketServer task.
#[derive(Debug, Clone)]
pub enum ServerCommand {
    BroadcastPlaybackInfo(PlaybackInfoPayload), // Carries the fully formed payload
    BroadcastTimeUpdate(TimeUpdatePayload),     // Carries the fully formed payload
    Shutdown,
}

type ClientTx = mpsc::UnboundedSender<Message>;
type Clients = Arc<Mutex<HashMap<std::net::SocketAddr, ClientTx>>>;

pub struct WebsocketServer {
    command_receiver: mpsc::Receiver<ServerCommand>,
    clients: Clients,
}

impl WebsocketServer {
    pub fn new(command_receiver: mpsc::Receiver<ServerCommand>) -> Self {
        Self {
            command_receiver,
            clients: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn run(mut self, addr: String) {
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                error!("[WebSocketServer] 无法绑定到地址 {addr}: {e}");
                return;
            }
        };
        info!("[WebSocketServer] 正在监听: {addr}");

        loop {
            tokio::select! {
                Ok((stream, client_addr)) = listener.accept() => {
                    info!("[WebSocketServer] 新的客户端连接: {client_addr}");
                    let clients_arc = Arc::clone(&self.clients);
                    tokio::spawn(handle_connection(stream, client_addr, clients_arc));
                }
                Some(command) = self.command_receiver.recv() => {
                    match command {
                        ServerCommand::BroadcastPlaybackInfo(payload) => {
                            // info!("[WebSocketServer] 收到 PlaybackInfo 更新，准备广播...");
                            let msg_to_client_view = ClientViewMessage::PlaybackInfo(payload);
                            self.broadcast_message_to_clients(msg_to_client_view).await;
                        }
                        ServerCommand::BroadcastTimeUpdate(payload) => {
                            // info!("[WebSocketServer] 收到 TimeUpdate 更新，准备广播...");
                            let msg_to_client_view = ClientViewMessage::TimeUpdate(payload);
                            self.broadcast_message_to_clients(msg_to_client_view).await;
                        }
                        ServerCommand::Shutdown => {
                            info!("[WebSocketServer] 收到关闭命令，正在关闭...");
                            // TODO: Gracefully close client connections before breaking
                            break;
                        }
                    }
                }
                else => {
                    info!("[WebSocketServer] 命令通道已关闭，服务器将停止。");
                    break;
                }
            }
        }
        info!("[WebSocketServer] 已停止。");
    }

    // --- 4. Update broadcast_message_to_clients to use ClientViewMessage ---
    async fn broadcast_message_to_clients(&self, message_to_client_view: ClientViewMessage) {
        let clients_guard = self.clients.lock().await;
        if clients_guard.is_empty() {
            return;
        }

        let json_string = match serde_json::to_string(&message_to_client_view) {
            Ok(json) => json,
            Err(e) => {
                error!("[WebSocketServer] 序列化 ClientViewMessage 到JSON失败: {e}");
                return;
            }
        };
        // info!("[WebSocketServer] Broadcasting: {}", json_string); // For debugging

        let ws_message = Message::Text(json_string.into());

        for (addr, client_tx) in clients_guard.iter() {
            if let Err(e) = client_tx.send(ws_message.clone()) {
                warn!("[WebSocketServer] 发送消息给客户端 {addr} 失败: {e}");
                // Consider removing client if send fails repeatedly
            }
        }
    }
}

// handle_connection function remains largely the same as in the previous example,
// as its primary role is to manage the WebSocket stream for an individual client
// and forward messages received on its `ClientTx` channel.
async fn handle_connection(stream: TcpStream, client_addr: std::net::SocketAddr, clients: Clients) {
    let ws_stream = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            error!("[WebSocketServer] WebSocket握手失败 (客户端 {client_addr}): {e}");
            return;
        }
    };
    info!("[WebSocketServer] WebSocket 连接已建立: {client_addr}");

    let (tx, mut rx) = mpsc::unbounded_channel();
    clients.lock().await.insert(client_addr, tx);

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    let broadcast_loop = async {
        while let Some(msg_to_send) = rx.recv().await {
            // msg_to_send is Message type
            if ws_sender.send(msg_to_send).await.is_err() {
                // warn!("[WebSocketServer] 发送消息到客户端 {} 失败 (broadcast_loop)", client_addr);
                break; // Error sending, close connection for this client
            }
        }
    };

    let receive_loop = async {
        while let Some(msg_result) = ws_receiver.next().await {
            match msg_result {
                Ok(msg) => {
                    if msg.is_text() || msg.is_binary() {
                        // info!("[WebSocketServer] 从客户端 {} 收到消息: {:?}", client_addr, msg.to_text().unwrap_or_default());
                        // UniLyric currently doesn't expect messages from Unilyric View, so we can ignore them.
                    } else if msg.is_close() {
                        // info!("[WebSocketServer] 客户端 {} 发送了关闭帧。", client_addr);
                        break;
                    }
                }
                Err(_e) => {
                    // warn!("[WebSocketServer] 从客户端 {} 接收消息错误: {}", client_addr, e);
                    break;
                }
            }
        }
    };

    tokio::select! {
        _ = broadcast_loop => {},
        _ = receive_loop => {},
    }

    info!("[WebSocketServer] 客户端 {client_addr} 断开连接。");
    clients.lock().await.remove(&client_addr);
}
