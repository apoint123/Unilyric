use std::sync::mpsc::{
    Receiver as StdReceiver, Sender as StdSender, TryRecvError as StdTryRecvError,
    channel as std_channel,
};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tokio::runtime::Runtime;
use tokio::sync::mpsc::{Sender as TokioSender, channel as tokio_channel};
use tokio::sync::oneshot;

use super::audio_capture::AudioCapturer;
use super::smtc_handler;
use super::types::{
    AMLLConnectorConfig, ConnectorCommand, ConnectorUpdate, SharedPlayerState, SmtcControlCommand,
    WebsocketStatus,
};
use super::volume_control;
use super::websocket_client;
use ws_protocol::Body as ProtocolBody;

/// `AMLLConnectorWorker` 结构体。
/// 管理 SMTC 监听器和 WebSocket 客户端的生命周期，
/// 处理来自主应用的命令，并将子系统的更新转发回主应用。
pub struct AMLLConnectorWorker {
    // 应用配置，使用 Arc<Mutex> 实现跨线程共享和可变性。
    config: Arc<Mutex<AMLLConnectorConfig>>,
    // 从主应用接收命令的通道接收端。
    command_rx_from_app: StdReceiver<ConnectorCommand>,
    // 向主应用发送更新的通道发送端。
    update_tx_to_app: StdSender<ConnectorUpdate>,

    // 从 SMTC 处理程序接收所有更新 (NowPlayingTrackChanged, SmtcSessionListChanged 等) 的通道接收端。
    smtc_update_rx: StdReceiver<ConnectorUpdate>,
    // 向 WebSocket 客户端发送出站协议体 (如歌词数据) 的通道发送端。
    ws_outgoing_tx: Option<TokioSender<ProtocolBody>>,
    // 从 WebSocket 客户端接收状态更新的通道接收端。
    ws_status_rx: StdReceiver<WebsocketStatus>,
    // 向 SMTC 处理程序发送控制命令 (包括会话选择和媒体控制) 的通道发送端。
    smtc_control_tx: Option<StdSender<ConnectorCommand>>,
    // 从 WebSocket 客户端接收原始 SMTC 媒体控制命令的通道接收端。
    ws_media_cmd_rx: StdReceiver<SmtcControlCommand>,

    // Tokio 运行时实例，用于管理异步任务。
    tokio_runtime: Arc<Runtime>,
    // 共享的播放器状态，由 SMTC Handler 更新，可能被其他部分读取。
    _shared_player_state: Arc<tokio::sync::Mutex<SharedPlayerState>>, // 加下划线表示 worker 本身不直接操作

    // SMTC 处理程序线程的 JoinHandle，用于等待线程结束。
    smtc_handler_thread_handle: Option<thread::JoinHandle<()>>,
    // 向 SMTC 处理程序发送关闭信号的通道发送端。
    smtc_shutdown_signal_tx: Option<StdSender<()>>,

    // WebSocket 客户端任务的 JoinHandle。
    websocket_client_task_handle: Option<tokio::task::JoinHandle<()>>,
    // 向 WebSocket 客户端发送关闭信号的 Oneshot 通道发送端。
    ws_shutdown_signal_tx: Option<oneshot::Sender<()>>,

    // 标志：当前会话是否已报告 SMTC 通道错误。
    smtc_channel_error_reported: bool,
    // 标志：当前会话是否已报告 WebSocket 通道错误。
    ws_channel_error_reported: bool,

    audio_capturer: Option<AudioCapturer>,
    audio_capture_update_rx: Option<StdReceiver<ConnectorUpdate>>,

    ws_media_cmd_rx_unexpectedly_disconnected: bool,
}

impl AMLLConnectorWorker {
    /// 启动并运行 AMLLConnectorWorker。
    /// 这是 Worker 的主入口点，通常在一个新线程中被调用。
    pub fn run(
        initial_config: AMLLConnectorConfig,
        command_rx_from_app: StdReceiver<ConnectorCommand>,
        update_tx_to_app: StdSender<ConnectorUpdate>,
    ) {
        log::info!("[AMLL Connector Worker] AMLL Connector Worker 正在启动...");

        let tokio_runtime = match Runtime::new() {
            Ok(rt) => Arc::new(rt),
            Err(e) => {
                log::error!("[AMLL Connector Worker] 创建 Tokio 运行时失败: {}", e);
                let _ = update_tx_to_app.send(ConnectorUpdate::WebsocketStatusChanged(
                    WebsocketStatus::错误(format!("Worker 运行时初始化失败: {}", e)),
                ));
                return;
            }
        };

        // 这些通道在 worker 内部创建，用于与子模块通信
        let (smtc_update_tx_for_handler, smtc_update_rx_for_worker) =
            std_channel::<ConnectorUpdate>();
        let (smtc_control_tx_for_worker, smtc_control_rx_for_handler) =
            std_channel::<ConnectorCommand>();
        // ws_outgoing_tx 和其配对的 rx 将在 start_websocket_client_task 内部创建
        let (ws_status_tx_for_client, ws_status_rx_for_worker) = std_channel::<WebsocketStatus>();
        let (ws_media_cmd_tx_for_client, ws_media_cmd_rx_for_worker) =
            std_channel::<SmtcControlCommand>();

        let mut worker_instance = Self {
            config: Arc::new(Mutex::new(initial_config.clone())),
            command_rx_from_app,
            update_tx_to_app,
            tokio_runtime,
            smtc_update_rx: smtc_update_rx_for_worker,
            ws_outgoing_tx: None, // 初始化为 None，将在 start_websocket_client_task 中设置
            ws_status_rx: ws_status_rx_for_worker,
            smtc_control_tx: Some(smtc_control_tx_for_worker),
            ws_media_cmd_rx: ws_media_cmd_rx_for_worker,
            _shared_player_state: Arc::new(tokio::sync::Mutex::new(SharedPlayerState::default())),
            smtc_handler_thread_handle: None,
            smtc_shutdown_signal_tx: None,
            websocket_client_task_handle: None,
            ws_shutdown_signal_tx: None,
            smtc_channel_error_reported: false,
            ws_channel_error_reported: false,
            audio_capturer: None,
            audio_capture_update_rx: None,
            ws_media_cmd_rx_unexpectedly_disconnected: false,
        };

        if initial_config.enabled {
            // start_all_subsystems 现在不需要 ws_outgoing_rx 参数
            worker_instance.start_all_subsystems(
                smtc_control_rx_for_handler,
                smtc_update_tx_for_handler,
                ws_status_tx_for_client,
                ws_media_cmd_tx_for_client,
            );
        }

        log::trace!("[AMLL Connector Worker] AMLL Connector Worker 初始化完成，进入主事件循环。");
        worker_instance.main_event_loop();

        log::trace!("[AMLL Connector Worker] AMLL Connector Worker 主循环结束。");
    }

    /// 启动所有子系统（SMTC 处理器线程和 WebSocket 客户端任务）。
    fn start_all_subsystems(
        &mut self,
        smtc_ctrl_rx: StdReceiver<ConnectorCommand>,
        smtc_update_tx: StdSender<ConnectorUpdate>,
        ws_status_tx: StdSender<WebsocketStatus>,
        ws_media_cmd_tx: StdSender<SmtcControlCommand>,
    ) {
        log::debug!("[AMLL Connector Worker] 正在启动所有子系统...");
        self.start_smtc_handler_thread(smtc_ctrl_rx, smtc_update_tx);
        self.start_websocket_client_task(ws_status_tx, ws_media_cmd_tx);
        self.smtc_channel_error_reported = false;
        self.ws_channel_error_reported = false;
    }

    /// 启动 SMTC 处理程序线程。
    fn start_smtc_handler_thread(
        &mut self,
        control_receiver_for_smtc: StdReceiver<ConnectorCommand>,
        update_sender_for_smtc: StdSender<ConnectorUpdate>,
    ) {
        if !self.config.lock().unwrap().enabled {
            log::trace!(
                "[AMLL Connector Worker] SMTC Handler 无法启动，因为 AMLL Connector 未启用。"
            );
            return;
        }
        self.stop_smtc_handler_thread();

        log::debug!("[AMLL Connector Worker] 正在启动 SMTC Handler 线程...");
        let player_state_clone = Arc::clone(&self._shared_player_state);
        let (shutdown_tx, shutdown_rx_for_smtc) = std_channel::<()>();
        self.smtc_shutdown_signal_tx = Some(shutdown_tx);

        let handle = thread::Builder::new()
            .name("smtc_handler_thread".to_string())
            .spawn(move || {
                log::debug!("[SMTC Handler Thread] SMTC Handler 线程已启动。");
                if let Err(e) = smtc_handler::run_smtc_listener(
                    update_sender_for_smtc,
                    control_receiver_for_smtc,
                    player_state_clone,
                    shutdown_rx_for_smtc,
                ) {
                    log::error!("[SMTC Handler Thread] SMTC Handler 运行出错: {}", e);
                }
                log::debug!("[SMTC Handler Thread] SMTC Handler 线程已结束。");
            })
            .expect("无法启动 SMTC Handler 线程");
        self.smtc_handler_thread_handle = Some(handle);
    }

    /// 启动 WebSocket 客户端异步任务。
    fn start_websocket_client_task(
        &mut self,
        status_sender_for_client: StdSender<WebsocketStatus>,
        media_command_sender_for_client: StdSender<SmtcControlCommand>,
    ) {
        let config_guard = self.config.lock().unwrap();
        if !config_guard.enabled {
            log::trace!(
                "[AMLL Connector Worker] WebSocket 客户端无法启动，因为 AMLL Connector 未启用。"
            );
            let _ = self
                .update_tx_to_app
                .send(ConnectorUpdate::WebsocketStatusChanged(
                    WebsocketStatus::断开,
                ));
            return;
        }
        if config_guard.websocket_url.is_empty() {
            log::error!("[AMLL Connector Worker] WebSocket URL 为空，无法启动 WebSocket 客户端。");
            let _ = self
                .update_tx_to_app
                .send(ConnectorUpdate::WebsocketStatusChanged(
                    WebsocketStatus::错误("WebSocket URL未配置".to_string()),
                ));
            return;
        }
        let current_websocket_url = config_guard.websocket_url.clone();
        drop(config_guard);

        self.stop_websocket_client_task();

        log::info!(
            "[AMLL Connector Worker] 正在启动 WebSocket 客户端任务 (URL: {})...",
            current_websocket_url
        );

        // 为新的 WebSocket 客户端任务创建新的 outgoing channel
        let (new_ws_outgoing_tx, new_ws_outgoing_rx) = tokio_channel(32);
        self.ws_outgoing_tx = Some(new_ws_outgoing_tx); // 存储新的发送端
        self.ws_media_cmd_rx_unexpectedly_disconnected = false;

        let (shutdown_tx, shutdown_rx_for_ws) = oneshot::channel::<()>();
        self.ws_shutdown_signal_tx = Some(shutdown_tx);

        let rt_clone = Arc::clone(&self.tokio_runtime);
        let handle = rt_clone.spawn(async move {
            log::debug!("[WebSocket Task] WebSocket 客户端任务已启动。");
            websocket_client::run_websocket_client(
                current_websocket_url,
                new_ws_outgoing_rx, // 将新的接收端传递给客户端任务
                status_sender_for_client,
                media_command_sender_for_client,
                shutdown_rx_for_ws,
            )
            .await;
            log::debug!("[WebSocket Task] WebSocket 客户端任务已结束。");
        });
        self.websocket_client_task_handle = Some(handle);
    }

    /// 停止 SMTC 处理程序线程。
    fn stop_smtc_handler_thread(&mut self) {
        if let Some(tx) = self.smtc_shutdown_signal_tx.take() {
            log::debug!("[AMLL Connector Worker] 正在发送关闭信号给 SMTC Handler...");
            if tx.send(()).is_err() {
                log::warn!(
                    "[AMLL Connector Worker] 发送关闭信号给 SMTC Handler 失败 (可能已自行关闭)。"
                );
            }
        }
        if let Some(handle) = self.smtc_handler_thread_handle.take() {
            log::debug!("[AMLL Connector Worker] 正在等待 SMTC Handler 线程结束...");
            match handle.join() {
                Ok(_) => log::debug!("[AMLL Connector Worker] SMTC Handler 线程已成功结束。"),
                Err(e) => log::warn!(
                    "[AMLL Connector Worker] SMTC Handler 线程 join 失败: {:?}",
                    e
                ),
            }
        }
    }

    /// 停止 WebSocket 客户端任务。
    fn stop_websocket_client_task(&mut self) {
        if let Some(tx) = self.ws_shutdown_signal_tx.take() {
            log::debug!("[AMLL Connector Worker] 正在发送关闭信号给 WebSocket 客户端...");
            if tx.send(()).is_err() {
                log::warn!(
                    "[AMLL Connector Worker] 发送关闭信号给 WebSocket 客户端失败 (任务可能已结束)。"
                );
            }
        }
        if self.websocket_client_task_handle.take().is_some() {
            log::debug!(
                "[AMLL Connector Worker] WebSocket 客户端任务的 JoinHandle 已移除，关闭将由其自身处理。"
            );
        }
        // 清理相关的发送通道，防止后续尝试使用已关闭的通道
        if self.ws_outgoing_tx.is_some() {
            self.ws_outgoing_tx = None;
            log::debug!("[AMLL Connector Worker] WebSocket outgoing_tx 通道已清理。");
        }
    }

    /// 内部函数，用于关闭所有子系统。
    fn shutdown_all_subsystems(&mut self) {
        log::trace!("[AMLL Connector Worker] 正在关闭所有子系统...");
        self.stop_smtc_handler_thread();
        self.stop_websocket_client_task();
        self.stop_audio_visualization_internal();
    }

    fn start_audio_visualization_internal(&mut self) {
        if !self.config.lock().unwrap().enabled {
            return;
        }
        if self.audio_capturer.is_some() {
            return;
        }

        let (audio_update_tx, audio_update_rx) = std_channel::<ConnectorUpdate>();
        let mut capturer = AudioCapturer::new(); // AudioCapturer::new() 不接受参数

        match capturer.start_capture(audio_update_tx) {
            // start_capture 接受 sender
            Ok(_) => {
                self.audio_capturer = Some(capturer);
                self.audio_capture_update_rx = Some(audio_update_rx);
            }
            Err(_) => {
                self.audio_capturer = None;
                self.audio_capture_update_rx = None;
            }
        }
    }

    fn stop_audio_visualization_internal(&mut self) {
        if let Some(mut capturer) = self.audio_capturer.take() {
            // take() 会移出 Option 中的值
            capturer.stop_capture();
            // AudioCapturer 的 Drop impl 也会调用 stop_capture，但显式调用更清晰
        }
        self.audio_capture_update_rx = None; // 清空接收通道
    }

    /// Worker 的主事件循环。
    /// 监听来自主应用的命令、SMTC 更新和 WebSocket 状态更新。
    fn main_event_loop(&mut self) {
        let mut should_shutdown_worker = false;

        loop {
            // --- 1. 处理来自主应用 (app.rs) 的命令 ---
            match self.command_rx_from_app.try_recv() {
                Ok(command) => {
                    // log::debug!("[AMLL Connector Worker] 收到主应用命令: {:?}", command);
                    match command {
                        ConnectorCommand::UpdateConfig(new_config) => {
                            let mut current_config_mg = self.config.lock().unwrap();
                            let old_config = current_config_mg.clone();
                            *current_config_mg = new_config.clone();
                            drop(current_config_mg); // 尽早释放锁
                            log::debug!(
                                "[AMLL Connector Worker] 配置已更新: old_enabled={}, new_enabled={}, old_url='{}', new_url='{}'",
                                old_config.enabled,
                                new_config.enabled,
                                old_config.websocket_url,
                                new_config.websocket_url
                            );

                            if new_config.enabled {
                                // 场景1: 从禁用 -> 启用
                                if !old_config.enabled {
                                    log::info!(
                                        "[AMLL Connector Worker] 配置从禁用变为启用。重新启动所有子系统。"
                                    );
                                    // (重新)创建所有必要的通道
                                    let (smtc_update_tx, smtc_update_rx) =
                                        std_channel::<ConnectorUpdate>();
                                    self.smtc_update_rx = smtc_update_rx;
                                    let (smtc_ctrl_tx, smtc_ctrl_rx) =
                                        std_channel::<ConnectorCommand>();
                                    self.smtc_control_tx = Some(smtc_ctrl_tx);
                                    let (ws_status_tx, ws_status_rx) =
                                        std_channel::<WebsocketStatus>();
                                    self.ws_status_rx = ws_status_rx;
                                    let (ws_cmd_tx, ws_cmd_rx) =
                                        std_channel::<SmtcControlCommand>();
                                    self.ws_media_cmd_rx = ws_cmd_rx;
                                    // start_all_subsystems 会调用 start_websocket_client_task，后者会创建自己的 ws_outgoing_tx
                                    self.start_all_subsystems(
                                        smtc_ctrl_rx,
                                        smtc_update_tx,
                                        ws_status_tx,
                                        ws_cmd_tx,
                                    );
                                } else {
                                    // 场景2: 保持启用状态，但配置可能更改（例如URL），或者需要强制重新连接（例如在DisconnectWebsocket后点击Connect）
                                    log::info!(
                                        "[AMLL Connector Worker] 收到启用状态下的配置更新。确保 WebSocket 客户端正在运行/尝试连接。"
                                    );
                                    // 总是尝试(重新)启动 WebSocket 客户端，因为它内部会先停止旧的。
                                    // 需要新的状态和命令通道，因为旧的可能与已停止的客户端关联。
                                    let (ws_status_tx, ws_status_rx) =
                                        std_channel::<WebsocketStatus>();
                                    self.ws_status_rx = ws_status_rx;
                                    let (ws_cmd_tx, ws_cmd_rx) =
                                        std_channel::<SmtcControlCommand>();
                                    self.ws_media_cmd_rx = ws_cmd_rx;
                                    self.start_websocket_client_task(ws_status_tx, ws_cmd_tx);
                                }
                            } else {
                                // new_config.enabled is false
                                // 场景3: 从启用 -> 禁用
                                if old_config.enabled {
                                    log::info!(
                                        "[AMLL Connector Worker] 配置从启用变为禁用。停止所有子系统。"
                                    );
                                    self.shutdown_all_subsystems();
                                    // 通知UI已断开
                                    if self
                                        .update_tx_to_app
                                        .send(ConnectorUpdate::WebsocketStatusChanged(
                                            WebsocketStatus::断开,
                                        ))
                                        .is_err()
                                    {
                                        log::error!(
                                            "[AMLL Connector Worker] 发送禁用后的断开状态失败。"
                                        );
                                        should_shutdown_worker = true; // 如果连这个都失败，worker可能无法正常工作
                                    }
                                } else {
                                    // 场景4: 保持禁用状态
                                    log::debug!(
                                        "[AMLL Connector Worker] 配置更新，但仍处于禁用状态。无操作。"
                                    );
                                }
                            }
                        }
                        ConnectorCommand::SendLyricTtml(ttml_string) => {
                            if self.config.lock().unwrap().enabled {
                                if let Some(sender) = &self.ws_outgoing_tx {
                                    let body = ProtocolBody::SetLyricFromTTML {
                                        data: ttml_string.into(),
                                    };
                                    let sender_clone = sender.clone();
                                    self.tokio_runtime.spawn(async move {
                                        if let Err(e) = sender_clone.send(body).await {
                                            log::error!("[AMLL Connector Worker] 发送歌词到 WebSocket 客户端内部通道失败: {}", e);
                                        }
                                    });
                                } else {
                                    log::warn!(
                                        "[AMLL Connector Worker] SendLyricTtml: WebSocket 发送通道无效 (可能已断开)，消息未发送。"
                                    );
                                }
                            }
                        }
                        ConnectorCommand::SendProtocolBody(protocol_body) => {
                            if self.config.lock().unwrap().enabled {
                                if let Some(sender) = &self.ws_outgoing_tx {
                                    let sender_clone = sender.clone();
                                    let cloned_body_for_async = protocol_body.clone();
                                    self.tokio_runtime.spawn(async move {
                                        if let Err(e) = sender_clone.send(cloned_body_for_async).await {
                                            log::error!("[AMLL Connector Worker] 发送 ProtocolBody 到 WebSocket 客户端内部通道失败: {}", e);
                                        }
                                    });
                                } else {
                                    // log::warn!("[AMLL Connector Worker] SendProtocolBody: WebSocket 发送通道无效 (可能已断开)，消息 {:?} 未发送。", protocol_body);
                                }
                            }
                        }
                        ConnectorCommand::SelectSmtcSession(session_id) => {
                            if self.config.lock().unwrap().enabled {
                                if let Some(sender) = &self.smtc_control_tx {
                                    if sender
                                        .send(ConnectorCommand::SelectSmtcSession(session_id))
                                        .is_err()
                                    {
                                        log::error!(
                                            "[AMLL Connector Worker] 发送 SelectSmtcSession 命令到 SMTC Handler 内部通道失败。"
                                        );
                                    }
                                } else {
                                    log::error!(
                                        "[AMLL Connector Worker] SMTC 控制通道无效，无法发送 SelectSmtcSession 命令。"
                                    );
                                }
                            }
                        }
                        ConnectorCommand::MediaControl(smtc_cmd) => {
                            if self.config.lock().unwrap().enabled {
                                if let Some(sender) = &self.smtc_control_tx {
                                    if sender
                                        .send(ConnectorCommand::MediaControl(smtc_cmd))
                                        .is_err()
                                    {
                                        log::error!(
                                            "[AMLL Connector Worker] 发送 MediaControl 命令到 SMTC Handler 内部通道失败。"
                                        );
                                    }
                                } else {
                                    log::error!(
                                        "[AMLL Connector Worker] SMTC 控制通道无效，无法发送 MediaControl 命令。"
                                    );
                                }
                            }
                        }
                        ConnectorCommand::AdjustAudioSessionVolume {
                            target_identifier,
                            volume,
                            mute,
                        } => {
                            if self.config.lock().unwrap().enabled {
                                log::trace!(
                                    "[AMLL Connector Worker] 收到 AdjustAudioSessionVolume 命令: target='{}', vol={:?}, mute={:?}",
                                    target_identifier,
                                    volume,
                                    mute
                                );
                                let rt_clone = Arc::clone(&self.tokio_runtime);
                                let update_tx_clone = self.update_tx_to_app.clone();
                                rt_clone.spawn_blocking(move || {
                                    match volume_control::get_pid_from_identifier(&target_identifier) {
                                        Some(pid) => {
                                            log::debug!("[Worker spawn_blocking] AdjustVolume: PID {} 找到，目标 '{}'.", pid, target_identifier);
                                            if let Err(e) = volume_control::set_process_volume_by_pid(pid, volume, mute) {
                                                log::error!(
                                                    "[Worker spawn_blocking] AdjustVolume: 设置 PID {} ({}) 的音量/静音失败: {}",
                                                    pid, target_identifier, e
                                                );
                                                // 可选: 发送错误通知回主应用
                                                // let _ = update_tx_clone.send(ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::错误(format!("音量控制失败: {}", e))));
                                            } else {
                                                log::trace!(
                                                    "[Worker spawn_blocking] AdjustVolume: PID {} ({}) 的音量/静音已尝试设置。",
                                                    pid, target_identifier
                                                );
                                                // 设置成功后，立即获取当前状态并发送更新
                                                match volume_control::get_process_volume_by_pid(pid) {
                                                    Ok((current_vol, current_mute)) => {
                                                        log::debug!(
                                                            "[Worker spawn_blocking] AdjustVolume: 获取 PID {} ({}) 的当前音量状态成功: vol={}, mute={}",
                                                            pid, target_identifier, current_vol, current_mute
                                                        );
                                                        let update_msg = ConnectorUpdate::AudioSessionVolumeChanged {
                                                            session_id: target_identifier.clone(), // 使用原始标识符
                                                            volume: current_vol,
                                                            is_muted: current_mute,
                                                        };
                                                        if let Err(e_send) = update_tx_clone.send(update_msg) {
                                                            log::error!(
                                                                "[Worker spawn_blocking] AdjustVolume: 发送 AudioSessionVolumeChanged 更新失败: {}",
                                                                e_send
                                                            );
                                                        }
                                                    }
                                                    Err(e_get) => {
                                                        log::error!(
                                                            "[Worker spawn_blocking] AdjustVolume: 设置音量后获取 PID {} ({}) 的状态失败: {}",
                                                            pid, target_identifier, e_get
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                        None => {
                                            log::warn!(
                                                "[Worker spawn_blocking] AdjustVolume: 无法为目标 '{}' 找到 PID。",
                                                target_identifier
                                            );
                                             // 可选: 发送错误通知
                                            // let _ = update_tx_clone.send(ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::错误(format!("找不到目标 '{}' 进行音量控制", target_identifier))));
                                        }
                                    }
                                });
                            } else {
                                log::warn!(
                                    "[AMLL Connector Worker] AdjustAudioSessionVolume 命令被忽略，因为连接器未启用。"
                                );
                            }
                        }
                        ConnectorCommand::RequestAudioSessionVolume(target_identifier) => {
                            if self.config.lock().unwrap().enabled {
                                log::trace!(
                                    "[AMLL Connector Worker] 收到 RequestAudioSessionVolume 命令: target='{}'",
                                    target_identifier
                                );
                                let rt_clone = Arc::clone(&self.tokio_runtime);
                                let update_tx_clone = self.update_tx_to_app.clone();
                                rt_clone.spawn_blocking(move || {
                                    match volume_control::get_pid_from_identifier(&target_identifier) {
                                        Some(pid) => {
                                            log::debug!("[Worker spawn_blocking] RequestVolume: PID {} 找到，目标 '{}'.", pid, target_identifier);
                                            match volume_control::get_process_volume_by_pid(pid) {
                                                Ok((current_vol, current_mute)) => {
                                                    log::trace!(
                                                        "[Worker spawn_blocking] RequestVolume: 获取 PID {} ({}) 的当前音量状态成功: vol={}, mute={}",
                                                        pid, target_identifier, current_vol, current_mute
                                                    );
                                                    let update_msg = ConnectorUpdate::AudioSessionVolumeChanged {
                                                        session_id: target_identifier.clone(),
                                                        volume: current_vol,
                                                        is_muted: current_mute,
                                                    };
                                                    if let Err(e_send) = update_tx_clone.send(update_msg) {
                                                        log::error!(
                                                            "[Worker spawn_blocking] RequestVolume: 发送 AudioSessionVolumeChanged 更新失败: {}",
                                                            e_send
                                                        );
                                                    }
                                                }
                                                Err(e_get) => {
                                                    log::error!(
                                                        "[Worker spawn_blocking] RequestVolume: 获取 PID {} ({}) 的状态失败: {}",
                                                        pid, target_identifier, e_get
                                                    );
                                                    // 可选: 发送错误通知
                                                    // let _ = update_tx_clone.send(ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::错误(format!("获取音量状态失败: {}", e_get))));
                                                }
                                            }
                                        }
                                        None => {
                                            log::warn!(
                                                "[Worker spawn_blocking] RequestVolume: 无法为目标 '{}' 找到 PID。",
                                                target_identifier
                                            );
                                            // 可选: 发送错误通知
                                            // let _ = update_tx_clone.send(ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::错误(format!("找不到目标 '{}' 获取音量状态", target_identifier))));
                                        }
                                    }
                                });
                            } else {
                                log::warn!(
                                    "[AMLL Connector Worker] RequestAudioSessionVolume 命令被忽略，因为连接器未启用。"
                                );
                            }
                        }
                        ConnectorCommand::StartAudioVisualization => {
                            log::debug!(
                                "[AMLL Connector Worker] 收到 StartAudioVisualization 命令。"
                            );
                            self.start_audio_visualization_internal();
                        }
                        ConnectorCommand::StopAudioVisualization => {
                            log::debug!(
                                "[AMLL Connector Worker] 收到 StopAudioVisualization 命令。"
                            );
                            self.stop_audio_visualization_internal();
                        }
                        ConnectorCommand::DisconnectWebsocket => {
                            log::debug!("[AMLL Connector Worker] 收到 DisconnectWebsocket 命令。");
                            if self.config.lock().unwrap().enabled {
                                self.stop_websocket_client_task();
                                if self
                                    .update_tx_to_app
                                    .send(ConnectorUpdate::WebsocketStatusChanged(
                                        WebsocketStatus::断开,
                                    ))
                                    .is_err()
                                {
                                    log::error!(
                                        "[AMLL Connector Worker] 发送 DisconnectWebsocket 后的状态更新到主应用失败。"
                                    );
                                    should_shutdown_worker = true;
                                }
                            } else {
                                log::warn!(
                                    "[AMLL Connector Worker] DisconnectWebsocket 命令被忽略，因为 AMLL Connector 功能当前已禁用。"
                                );
                            }
                        }
                        ConnectorCommand::Shutdown => {
                            log::debug!("[AMLL Connector Worker] 收到主应用关闭命令，准备退出...");
                            should_shutdown_worker = true;
                        }
                    }
                }
                Err(StdTryRecvError::Empty) => { /* 无命令，正常 */ }
                Err(StdTryRecvError::Disconnected) => {
                    log::error!(
                        "[AMLL Connector Worker] 与主应用的命令通道已断开，Worker 即将退出。"
                    );
                    should_shutdown_worker = true;
                }
            }

            if should_shutdown_worker {
                break;
            }

            // --- 2. 处理来自 SMTC 处理程序的更新 ---
            match self.smtc_update_rx.try_recv() {
                Ok(connector_update) => {
                    self.smtc_channel_error_reported = false;

                    // 发送该数据会导致 AMLL Player 不发送音量调节信息
                    // if let ConnectorUpdate::AudioSessionVolumeChanged { session_id: _, volume, is_muted: _ } = &connector_update {
                    // let volume_f64 = *volume as f64;
                    // log::debug!("[AMLL Connector Worker] 准备将音量更新 {:.2} (f32) -> {:.2} (f64) 发送给 AMLL Player。", *volume, volume_f64);

                    // if self.config.lock().unwrap().enabled {
                    //     if let Some(sender) = &self.ws_outgoing_tx {
                    //         let body = ProtocolBody::OnVolumeChanged { volume: volume_f64 };
                    //         let sender_clone = sender.clone();
                    //         self.tokio_runtime.spawn(async move {
                    //             if let Err(e) = sender_clone.send(body).await {
                    //                 log::error!("[AMLL Connector Worker] 发送 OnVolumeChanged 到 WebSocket 客户端内部通道失败: {}", e);
                    //             } else {
                    //                 log::trace!("[AMLL Connector Worker] 已将 OnVolumeChanged({:.2}) 发送到 WebSocket 客户端通道。", volume_f64);
                    //             }
                    //         });
                    //     } else {
                    //         log::warn!("[AMLL Connector Worker] WebSocket 发送通道无效，无法发送 OnVolumeChanged。");
                    //     }
                    // }
                    // }

                    if self
                        .update_tx_to_app
                        .send(connector_update.clone())
                        .is_err()
                    {
                        log::error!(
                            "[AMLL Connector Worker] 发送 ConnectorUpdate (来自SMTC) 到主应用失败。"
                        );
                        should_shutdown_worker = true;
                    }
                }
                Err(StdTryRecvError::Empty) => {}
                Err(StdTryRecvError::Disconnected) => {
                    if self.config.lock().unwrap().enabled && !self.smtc_channel_error_reported {
                        log::warn!(
                            "[AMLL Connector Worker] SMTC Handler 更新通道已断开 (SMTC 线程可能已退出)。"
                        );
                        if self
                            .update_tx_to_app
                            .send(ConnectorUpdate::WebsocketStatusChanged(
                                WebsocketStatus::错误("SMTC Handler 异常".to_string()),
                            ))
                            .is_ok()
                        {
                            self.smtc_channel_error_reported = true;
                        } else {
                            should_shutdown_worker = true;
                        }
                    }
                    self.smtc_handler_thread_handle = None;
                    self.smtc_shutdown_signal_tx = None;
                }
            }
            if should_shutdown_worker {
                break;
            }

            // --- 音频捕获更新处理 ---
            if let Some(rx) = &self.audio_capture_update_rx {
                match rx.try_recv() {
                    Ok(ConnectorUpdate::AudioDataPacket(audio_bytes)) => {
                        let config_guard = self.config.lock().unwrap();
                        let connector_enabled = config_guard.enabled;
                        let ws_tx_is_some = self.ws_outgoing_tx.is_some();
                        drop(config_guard);

                        if connector_enabled && ws_tx_is_some && !audio_bytes.is_empty() {
                            let body = ProtocolBody::OnAudioData { data: audio_bytes };
                            self.send_protocol_body_to_ws(body);
                        } else if connector_enabled && !ws_tx_is_some {
                        }
                    }
                    Ok(unexpected_update) => {
                        log::warn!(
                            "[AMLL Connector Worker] 从 AudioCapturer 收到意外的更新类型: {:?}",
                            unexpected_update
                        );
                    }
                    Err(StdTryRecvError::Empty) => {}
                    Err(StdTryRecvError::Disconnected) => {
                        log::error!(
                            "[AMLL Connector Worker] 与 AudioCapturer 的数据通道已断开。可能捕获线程已退出。"
                        );
                        self.stop_audio_visualization_internal(); // 确保音频捕获停止
                    }
                }
            }

            // --- 3. 处理来自 WebSocket 客户端的 SMTC 媒体控制命令 ---
            match self.ws_media_cmd_rx.try_recv() {
                Ok(smtc_cmd_from_ws) => {
                    if self.config.lock().unwrap().enabled {
                        if let Some(sender) = &self.smtc_control_tx {
                            if sender
                                .send(ConnectorCommand::MediaControl(smtc_cmd_from_ws))
                                .is_err()
                            {
                                log::error!(
                                    "[AMLL Connector Worker] 发送包装后的 MediaControl (来自WS: {:?}) 到 SMTC Handler 失败。",
                                    smtc_cmd_from_ws
                                );
                            }
                        }
                    }
                }
                Err(StdTryRecvError::Empty) => { /* 无命令，正常 */ }
                Err(StdTryRecvError::Disconnected) => {
                    let was_intentional_stop =
                        self.ws_shutdown_signal_tx.is_none() || self.ws_outgoing_tx.is_none();
                    if was_intentional_stop {
                        if self.ws_media_cmd_rx_unexpectedly_disconnected {
                            log::trace!(
                                "[AMLL Connector Worker] ws_media_cmd_rx 断开，现确认为符合预期（WebSocket客户端已停止），重置意外断开标志。"
                            );
                            self.ws_media_cmd_rx_unexpectedly_disconnected = false;
                        }
                    } else if !self.ws_media_cmd_rx_unexpectedly_disconnected {
                        log::warn!(
                            "[AMLL Connector Worker] 与WebSocket客户端的SMTC媒体命令通道意外断开。"
                        );
                        self.ws_media_cmd_rx_unexpectedly_disconnected = true;
                    }
                }
            }

            // --- 4. 处理来自 WebSocket 客户端的状态更新 ---
            match self.ws_status_rx.try_recv() {
                Ok(ws_status) => {
                    if ws_status == WebsocketStatus::已连接 {
                        self.ws_channel_error_reported = false;
                        self.ws_media_cmd_rx_unexpectedly_disconnected = false;
                    } else if ws_status != WebsocketStatus::连接中 {
                        self.ws_channel_error_reported = false;
                    }

                    log::trace!(
                        "[AMLL Connector Worker] 从 WebSocket 客户端收到状态更新: {:?}",
                        ws_status
                    );
                    if self
                        .update_tx_to_app
                        .send(ConnectorUpdate::WebsocketStatusChanged(ws_status.clone()))
                        .is_err()
                    {
                        log::error!(
                            "[AMLL Connector Worker] 发送 WebsocketStatusChanged (来自WS) 到主应用失败。"
                        );
                        should_shutdown_worker = true;
                    }
                }
                Err(StdTryRecvError::Empty) => { /* 无更新，正常 */ }
                Err(StdTryRecvError::Disconnected) => {
                    let was_intentional_stop =
                        self.ws_shutdown_signal_tx.is_none() || self.ws_outgoing_tx.is_none();
                    if was_intentional_stop {
                        if !self.ws_channel_error_reported {
                            log::trace!(
                                "[AMLL Connector Worker] ws_status_rx 断开，符合预期（WebSocket客户端已停止）。"
                            );
                        }
                        if self.websocket_client_task_handle.is_some() {
                            self.websocket_client_task_handle = None;
                        }
                        if self.ws_outgoing_tx.is_some() && self.ws_shutdown_signal_tx.is_none() {
                            self.ws_outgoing_tx = None;
                        }
                        self.ws_channel_error_reported = false;
                        self.ws_media_cmd_rx_unexpectedly_disconnected = false;
                    } else if !self.ws_channel_error_reported {
                        log::error!(
                            "[AMLL Connector Worker] WebSocket 客户端状态更新通道意外断开。"
                        );
                        if self
                            .update_tx_to_app
                            .send(ConnectorUpdate::WebsocketStatusChanged(
                                WebsocketStatus::错误("WebSocket 客户端异常".to_string()),
                            ))
                            .is_ok()
                        {
                            self.ws_channel_error_reported = true;
                        } else {
                            should_shutdown_worker = true;
                        }
                        self.websocket_client_task_handle = None;
                        if self.ws_shutdown_signal_tx.is_some() {
                            self.ws_shutdown_signal_tx = None;
                        }
                        if self.ws_outgoing_tx.is_some() {
                            self.ws_outgoing_tx = None;
                        }
                        self.ws_media_cmd_rx_unexpectedly_disconnected = true;
                    }
                }
            }
            if should_shutdown_worker {
                break;
            }

            thread::sleep(Duration::from_millis(20));
        }

        self.shutdown_all_subsystems();
        log::trace!("[AMLL Connector Worker] 事件循环已退出。");
    }

    fn send_protocol_body_to_ws(&self, body: ProtocolBody) {
        if let Some(sender) = &self.ws_outgoing_tx {
            let sender_clone = sender.clone();
            let body_type_for_log = match &body {
                ProtocolBody::OnAudioData { data, .. } => {
                    format!("OnAudioData(len:{})", data.len())
                }
                _ => format!("{:?}", body)
                    .split_whitespace()
                    .next()
                    .unwrap_or("UnknownProtocol")
                    .to_string(),
            };

            self.tokio_runtime.spawn(async move {
                if let Err(e) = sender_clone.send(body).await {
                    // body 被移动到异步块
                    log::error!(
                        "[AMLL Connector Worker] 发送 {} 到 WebSocket 客户端内部通道失败: {}",
                        body_type_for_log, // 使用之前获取的类型
                        e
                    );
                } else if !matches!(body_type_for_log.as_str(), "OnAudioData(_)") {
                    log::trace!(
                        "[AMLL Connector Worker] 已将 {} 发送到 WebSocket 客户端通道。",
                        body_type_for_log
                    );
                }
            });
        } else {
            // 当 ws_outgoing_tx 为 None 时，我们知道 WebSocket 客户端没有在运行
            // 或者至少 Worker 认为它没有在运行。
            // 对于 OnAudioData 这种高频消息，不应每次都打印警告。
            // 我们可以选择完全不打印，或者只在非 OnAudioData 消息时打印。
            if !matches!(body, ProtocolBody::OnAudioData { .. }) {
                let body_type_for_log = format!("{:?}", body)
                    .split_whitespace()
                    .next()
                    .unwrap_or("UnknownProtocol")
                    .to_string();
                log::warn!(
                    "[AMLL Connector Worker] WebSocket 发送通道无效，无法发送 {}。(ws_outgoing_tx is None)",
                    body_type_for_log
                );
            } else {
                log::trace!("[AMLL Connector Worker] WebSocket 发送通道无效，OnAudioData 未发送。");
            }
        }
    }
}

/// `AMLLConnectorWorker` 的 `Drop` 实现。
/// 确保在 Worker 实例被丢弃时，所有子系统都能被正确关闭。
impl Drop for AMLLConnectorWorker {
    fn drop(&mut self) {
        log::trace!("[AMLL Connector Worker] 正在丢弃 AMLLConnectorWorker，确保关闭子系统...");
        self.shutdown_all_subsystems();
    }
}

/// 启动 `AMLLConnectorWorker` 线程的公共函数。
/// 这是从外部（通常是 `app.rs`）创建和运行 Worker 的主要方式。
pub fn start_amll_connector_worker_thread(
    initial_config: AMLLConnectorConfig,
    command_rx: StdReceiver<ConnectorCommand>,
    update_tx: StdSender<ConnectorUpdate>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("amll_connector_worker_thread".to_string()) // 为线程设置名称，便于调试
        .spawn(move || {
            AMLLConnectorWorker::run(initial_config, command_rx, update_tx);
        })
        .expect("无法启动 AMLLConnectorWorker 线程")
}
