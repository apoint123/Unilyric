use std::sync::mpsc::{Receiver as StdReceiver, Sender as StdSender, channel as std_channel};
use std::sync::{Arc, Mutex};
use std::thread;

use tokio::runtime::Runtime;
use tokio::sync::mpsc::{
    Receiver as TokioReceiver, Sender as TokioSender, channel as tokio_channel,
};
use tokio::sync::oneshot;
use tokio::task::LocalSet;

use super::audio_capture::AudioCapturer;
use super::smtc_handler;
use super::types::{
    AMLLConnectorConfig, ConnectorCommand, ConnectorUpdate, SharedPlayerState, SmtcControlCommand,
    WebsocketStatus,
};
use super::volume_control;
use super::websocket_client;
use ws_protocol::Body as ProtocolBody;

/// `AMLLConnectorWorker` 是连接器的核心，它协调所有子系统并处理事件。
///
/// 该结构体在一个独立的线程中运行，其内部实现了一个纯异步的事件循环，
/// 用于高效地处理来自主应用、SMTC、WebSocket 和其他模块的通信。
pub struct AMLLConnectorWorker {
    /// 应用配置的共享引用。
    /// 使用 `Arc<Mutex<T>>` 以便在多个线程之间安全地共享和修改配置。
    config: Arc<Mutex<AMLLConnectorConfig>>,

    /// 从主应用接收命令的异步通道。
    /// 所有对 worker 的控制请求（如更新配置、发送数据、关闭）都通过此通道进入。
    command_rx: TokioReceiver<ConnectorCommand>,

    /// 向主应用发送更新的同步通道。
    /// 用于将 worker 及其子系统的状态（如连接状态、播放信息）报告给 UI 线程。
    update_tx_to_app: StdSender<ConnectorUpdate>,

    /// 从 SMTC 处理程序接收更新的异步通道。
    /// 例如，当前播放曲目改变、SMTC 会话列表更新等。
    smtc_update_rx: TokioReceiver<ConnectorUpdate>,

    /// 向 WebSocket 客户端发送出站消息 (`ProtocolBody`) 的异步通道。
    /// 当需要向远程服务器发送歌词、播放状态等信息时使用。
    ws_outgoing_tx: Option<TokioSender<ProtocolBody>>,

    /// 从 WebSocket 客户端接收状态更新的异步通道。
    /// 例如，连接中、已连接、已断开、错误等状态。
    ws_status_rx: TokioReceiver<WebsocketStatus>,

    /// 向 SMTC 处理程序发送控制命令的同步通道。
    /// 用于执行播放、暂停、切歌等媒体控制操作。
    smtc_control_tx: Option<StdSender<ConnectorCommand>>,

    /// 从 WebSocket 客户端接收媒体控制命令的异步通道。
    /// 允许远程服务器通过 WebSocket 控制本机的 SMTC。
    ws_media_cmd_rx: TokioReceiver<SmtcControlCommand>,

    /// Tokio 运行时的共享句柄。
    /// 用于在 worker 内部派生新的异步任务（例如，发送消息的 `spawn`）。
    tokio_runtime: Arc<Runtime>,

    /// 共享的播放器状态。
    /// 虽然 worker 本身不直接修改它，但它持有这个 Arc 并将其传递给 SMTC Handler。
    /// `_` 前缀表示它是一个传递用途的字段。
    _shared_player_state: Arc<tokio::sync::Mutex<SharedPlayerState>>,

    /// SMTC 处理程序线程的 `JoinHandle`。
    /// 用于在关闭时等待 SMTC 线程正确退出。
    smtc_handler_thread_handle: Option<thread::JoinHandle<()>>,

    /// 向 SMTC 处理程序线程发送关闭信号的通道。
    smtc_shutdown_signal_tx: Option<StdSender<()>>,

    /// WebSocket 客户端任务的 `JoinHandle`。
    /// 用于在关闭时等待 WebSocket 任务正确结束。
    websocket_client_task_handle: Option<tokio::task::JoinHandle<()>>,

    /// 向 WebSocket 客户端任务发送关闭信号的 `oneshot` 通道。
    ws_shutdown_signal_tx: Option<oneshot::Sender<()>>,

    /// 标志位：用于防止重复报告 SMTC 通道断开的错误。
    smtc_channel_error_reported: bool,

    /// 标志位：用于防止重复报告 WebSocket 通道断开的错误。
    ws_channel_error_reported: bool,

    /// 音频捕获器的实例。
    /// `Some` 表示音频可视化功能已启动。
    audio_capturer: Option<AudioCapturer>,

    /// 从音频捕获器接收音频数据的异步通道。
    audio_capture_update_rx: Option<TokioReceiver<ConnectorUpdate>>,

    ws_status: WebsocketStatus,

    /// 标志位：用于区分 WebSocket 的媒体命令通道是意外断开还是有意关闭。
    ws_media_cmd_rx_unexpectedly_disconnected: bool,
}

impl AMLLConnectorWorker {
    /// 启动并运行 `AMLLConnectorWorker`。
    ///
    /// 这是 Worker 的主入口点，在一个新的专用线程中被调用。
    /// 它负责设置完整的异步环境，包括 Tokio 运行时和 `LocalSet`，
    /// 创建所有必需的通信通道和桥接任务，并最终启动核心的异步事件循环 `run_async`。
    ///
    /// # Arguments
    ///
    /// * `initial_config` - 启动时使用的初始配置。
    /// * `command_rx_from_app` - 从主应用接收命令的同步通道接收端。
    /// * `update_tx_to_app` - 向主应用发送更新的同步通道发送端。
    pub fn run(
        initial_config: AMLLConnectorConfig,
        command_rx_from_app: StdReceiver<ConnectorCommand>,
        update_tx_to_app: StdSender<ConnectorUpdate>,
    ) {
        log::debug!("[[AMLL Connector Worker] 工作线程正在启动...");

        let rt = match Runtime::new() {
            Ok(rt) => Arc::new(rt),
            Err(e) => {
                log::error!("[[AMLL Connector Worker] 创建Tokio运行时失败: {e}");
                let _ = update_tx_to_app.send(ConnectorUpdate::WebsocketStatusChanged(
                    WebsocketStatus::错误(format!("工作线程运行时初始化失败: {e}")),
                ));
                return;
            }
        };

        // LocalSet 允许我们在同一个线程上运行 !Send 的 Future，尽管在此 worker 中可能不是严格必需的，
        // 但保持这种模式有助于架构的一致性。
        let local_set = LocalSet::new();

        // --- 通道设置 ---
        // 1. 用于与子模块（运行在它们自己的线程中）通信的【同步】通道。
        let (smtc_update_tx_for_handler, smtc_update_rx_for_worker_sync) =
            std_channel::<ConnectorUpdate>();
        let (smtc_control_tx_for_worker, smtc_control_rx_for_handler) =
            std_channel::<ConnectorCommand>();
        let (ws_status_tx_for_client, ws_status_rx_for_worker_sync) =
            std_channel::<WebsocketStatus>();
        let (ws_media_cmd_tx_for_client, ws_media_cmd_rx_for_worker_sync) =
            std_channel::<SmtcControlCommand>();

        // 2. 用于核心 `select!` 循环的【异步】通道。
        let (command_tx_async, command_rx_async) = tokio_channel::<ConnectorCommand>(32);
        let (smtc_update_tx_async, smtc_update_rx_async) = tokio_channel::<ConnectorUpdate>(32);
        let (ws_status_tx_async, ws_status_rx_async) = tokio_channel::<WebsocketStatus>(32);
        let (ws_media_cmd_tx_async, ws_media_cmd_rx_async) =
            tokio_channel::<SmtcControlCommand>(32);

        // --- 桥接任务 ---
        // 3. 派生异步任务，将同步消息从 std::sync::mpsc "桥接" 到 tokio::sync::mpsc，
        //    以便 `select!` 循环可以 `await` 它们。
        log::debug!("[[AMLL Connector Worker] 正在启动所有同步->异步通道桥接任务...");
        let rt_clone = rt.clone();
        rt_clone.spawn(async move {
            while let Ok(cmd) = command_rx_from_app.recv() {
                if command_tx_async.send(cmd).await.is_err() {
                    log::error!("[桥接任务-主命令] 无法将命令发送至异步通道，接收端可能已关闭。");
                    break;
                }
            }
            log::debug!("[桥接任务-主命令] 通道已关闭，任务结束。");
        });

        rt_clone.spawn(async move {
            while let Ok(update) = smtc_update_rx_for_worker_sync.recv() {
                if smtc_update_tx_async.send(update).await.is_err() {
                    log::error!("[桥接任务-SMTC更新] 无法将更新发送至异步通道。");
                    break;
                }
            }
            log::debug!("[桥接任务-SMTC更新] 通道已关闭，任务结束。");
        });

        rt_clone.spawn(async move {
            while let Ok(status) = ws_status_rx_for_worker_sync.recv() {
                log::trace!("[桥接任务-WS状态] 从 sync 通道收到状态: {:?}", status);
                if ws_status_tx_async.send(status).await.is_err() {
                    log::error!("[桥接任务-WS状态] 无法将状态发送至异步通道。");
                    break;
                }
                log::trace!("[桥接任务-WS状态] 已将状态发送至 async 通道。");
            }
            log::trace!("[桥接任务-WS状态] 通道已关闭，任务结束。");
        });

        let mut worker_instance = Self {
            config: Arc::new(Mutex::new(initial_config.clone())),
            command_rx: command_rx_async,
            update_tx_to_app,
            tokio_runtime: rt.clone(),
            smtc_update_rx: smtc_update_rx_async,
            ws_outgoing_tx: None,
            ws_status_rx: ws_status_rx_async,
            smtc_control_tx: Some(smtc_control_tx_for_worker),
            ws_media_cmd_rx: ws_media_cmd_rx_async,
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
            ws_status: WebsocketStatus::断开,
        };

        rt_clone.spawn(async move {
            while let Ok(cmd) = ws_media_cmd_rx_for_worker_sync.recv() {
                if ws_media_cmd_tx_async.send(cmd).await.is_err() {
                    log::error!("[桥接任务-WS媒体命令] 无法将命令发送至异步通道。");
                    break;
                }
            }
            log::debug!("[桥接任务-WS媒体命令] 通道已关闭，任务结束。");
        });

        if initial_config.enabled {
            worker_instance.start_all_subsystems(
                smtc_control_rx_for_handler,
                smtc_update_tx_for_handler,
                ws_status_tx_for_client,
                ws_media_cmd_tx_for_client,
            );
        }

        log::trace!("[[AMLL Connector Worker] 初始化完成，即将进入核心事件循环。");

        // 使用 `block_on` 在当前线程上驱动异步主循环。
        // 这将运行 `local_set` 中的所有任务，直到 `run_async` 完成。
        local_set.block_on(&rt, async {
            worker_instance.run_async().await;
        });

        log::trace!("[[AMLL Connector Worker] 核心事件循环已退出，工作线程即将终止。");
    }

    /// 启动所有已配置的子系统。
    fn start_all_subsystems(
        &mut self,
        smtc_ctrl_rx: StdReceiver<ConnectorCommand>,
        smtc_update_tx: StdSender<ConnectorUpdate>,
        ws_status_tx: StdSender<WebsocketStatus>,
        ws_media_cmd_tx: StdSender<SmtcControlCommand>,
    ) {
        log::debug!("[[AMLL Connector Worker] 正在启动所有子系统 (SMTC, WebSocket)...");
        self.start_smtc_handler_thread(smtc_ctrl_rx, smtc_update_tx);
        self.start_websocket_client_task(ws_status_tx, ws_media_cmd_tx);
        self.smtc_channel_error_reported = false;
        self.ws_channel_error_reported = false;
    }

    /// 派生一个新的系统线程来运行 SMTC 处理程序。
    fn start_smtc_handler_thread(
        &mut self,
        control_receiver_for_smtc: StdReceiver<ConnectorCommand>,
        update_sender_for_smtc: StdSender<ConnectorUpdate>,
    ) {
        if !self.config.lock().unwrap().enabled {
            log::warn!("[[AMLL Connector Worker] SMTC处理器无法启动，因为连接器未启用。");
            return;
        }
        self.stop_smtc_handler_thread();

        log::debug!("[[AMLL Connector Worker] 正在启动SMTC处理器线程...");
        let player_state_clone = Arc::clone(&self._shared_player_state);
        let (shutdown_tx, shutdown_rx_for_smtc) = std_channel::<()>();
        self.smtc_shutdown_signal_tx = Some(shutdown_tx);

        let handle = thread::Builder::new()
            .name("smtc_handler_thread".to_string())
            .spawn(move || {
                log::debug!("[SMTC处理器线程] 线程已启动。");
                if let Err(e) = smtc_handler::run_smtc_listener(
                    update_sender_for_smtc,
                    control_receiver_for_smtc,
                    player_state_clone,
                    shutdown_rx_for_smtc,
                ) {
                    log::error!("[SMTC处理器线程] 运行出错: {e}");
                }
                log::debug!("[SMTC处理器线程] 线程已结束。");
            })
            .expect("无法启动SMTC处理器线程");
        self.smtc_handler_thread_handle = Some(handle);
    }

    /// 在 Tokio 运行时上派生一个异步任务来运行 WebSocket 客户端。
    fn start_websocket_client_task(
        &mut self,
        status_sender_for_client: StdSender<WebsocketStatus>,
        media_command_sender_for_client: StdSender<SmtcControlCommand>,
    ) {
        let config_guard = self.config.lock().unwrap();
        if !config_guard.enabled {
            log::warn!("[[AMLL Connector Worker] WebSocket客户端无法启动，因为连接器未启用。");
            let _ = self
                .update_tx_to_app
                .send(ConnectorUpdate::WebsocketStatusChanged(
                    WebsocketStatus::断开,
                ));
            return;
        }
        if config_guard.websocket_url.is_empty() {
            log::error!("[[AMLL Connector Worker] WebSocket URL为空，无法启动客户端。");
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
            "[[AMLL Connector Worker] 正在启动WebSocket客户端任务 (URL: {current_websocket_url})..."
        );

        // 为新的客户端任务创建专用的出站消息通道
        let (new_ws_outgoing_tx, new_ws_outgoing_rx) = tokio_channel(32);
        self.ws_outgoing_tx = Some(new_ws_outgoing_tx);
        self.ws_media_cmd_rx_unexpectedly_disconnected = false;

        let (shutdown_tx, shutdown_rx_for_ws) = oneshot::channel::<()>();
        self.ws_shutdown_signal_tx = Some(shutdown_tx);

        let rt_clone = Arc::clone(&self.tokio_runtime);
        let handle = rt_clone.spawn(async move {
            log::debug!("[WebSocket任务] 异步任务已启动。");
            websocket_client::run_websocket_client(
                current_websocket_url,
                new_ws_outgoing_rx,
                status_sender_for_client,
                media_command_sender_for_client,
                shutdown_rx_for_ws,
            )
            .await;
            log::debug!("[WebSocket任务] 异步任务已结束。");
        });
        self.websocket_client_task_handle = Some(handle);
    }

    /// 停止 SMTC 处理程序线程。
    fn stop_smtc_handler_thread(&mut self) {
        if let Some(tx) = self.smtc_shutdown_signal_tx.take() {
            log::debug!("[[AMLL Connector Worker] 正在向SMTC处理器发送关闭信号...");
            if tx.send(()).is_err() {
                log::warn!(
                    "[[AMLL Connector Worker] 发送关闭信号至SMTC处理器失败 (可能已自行关闭)。"
                );
            }
        }
        if let Some(handle) = self.smtc_handler_thread_handle.take() {
            log::debug!("[[AMLL Connector Worker] 正在等待SMTC处理器线程退出...");
            match handle.join() {
                Ok(_) => log::debug!("[[AMLL Connector Worker] SMTC处理器线程已成功退出。"),
                Err(e) => log::warn!("[[AMLL Connector Worker] 等待SMTC处理器线程退出失败: {e:?}"),
            }
        }
    }

    /// 停止 WebSocket 客户端任务。
    fn stop_websocket_client_task(&mut self) {
        if let Some(tx) = self.ws_shutdown_signal_tx.take() {
            log::debug!("[[AMLL Connector Worker] 正在向WebSocket客户端发送关闭信号...");
            if tx.send(()).is_err() {
                log::warn!(
                    "[[AMLL Connector Worker] 发送关闭信号至WebSocket客户端失败 (任务可能已结束)。"
                );
            }
        }
        if self.websocket_client_task_handle.take().is_some() {
            log::debug!(
                "[[AMLL Connector Worker] WebSocket客户端任务的JoinHandle已移除，关闭将由其自身处理。"
            );
        }
        // 清理相关的发送通道，防止后续尝试使用已关闭的通道
        if self.ws_outgoing_tx.is_some() {
            self.ws_outgoing_tx = None;
            log::debug!("[[AMLL Connector Worker] WebSocket出站消息通道已清理。");
        }
    }

    /// 统一关闭所有子系统。
    fn shutdown_all_subsystems(&mut self) {
        log::debug!("[[AMLL Connector Worker] 正在关闭所有子系统...");
        self.stop_smtc_handler_thread();
        self.stop_websocket_client_task();
        self.stop_audio_visualization_internal();
    }

    /// 启动音频捕获和可视化。
    fn start_audio_visualization_internal(&mut self) {
        if !self.config.lock().unwrap().enabled {
            return;
        }
        if self.audio_capturer.is_some() {
            log::debug!("[[AMLL Connector Worker] 音频可视化已在运行，无需重复启动。");
            return;
        }

        log::debug!("[[AMLL Connector Worker] 正在启动音频可视化...");

        let (audio_update_tx_sync, audio_update_rx_sync) = std_channel::<ConnectorUpdate>();
        let (audio_update_tx_async, audio_update_rx_async) = tokio_channel::<ConnectorUpdate>(256);

        let mut capturer = AudioCapturer::new();

        match capturer.start_capture(audio_update_tx_sync) {
            Ok(_) => {
                self.audio_capturer = Some(capturer);
                self.audio_capture_update_rx = Some(audio_update_rx_async);

                // 启动音频桥接任务
                let rt_clone = self.tokio_runtime.clone();
                rt_clone.spawn(async move {
                    log::debug!("[桥接任务-音频] 音频数据桥接任务已启动。");
                    while let Ok(update) = audio_update_rx_sync.recv() {
                        if audio_update_tx_async.send(update).await.is_err() {
                            log::warn!("[桥接任务-音频] 无法将音频数据包发送至异步通道。");
                            break;
                        }
                    }
                    log::debug!("[桥接任务-音频] 通道已关闭，任务结束。");
                });
            }
            Err(e) => {
                log::error!("[[AMLL Connector Worker] 启动音频捕获失败: {e}");
                self.audio_capturer = None;
                self.audio_capture_update_rx = None;
            }
        }
    }

    /// 停止音频捕获和可视化。
    fn stop_audio_visualization_internal(&mut self) {
        if let Some(mut capturer) = self.audio_capturer.take() {
            log::debug!("[[AMLL Connector Worker] 正在停止音频可视化...");
            capturer.stop_capture();
        }
        self.audio_capture_update_rx = None; // 清理异步接收通道
    }

    /// Worker 的核心异步事件循环。
    ///
    /// 使用 `tokio::select!` 以非阻塞方式并发等待来自所有来源的事件。
    /// 当没有事件发生时，此循环会使线程进入休眠状态，CPU 占用率接近零，从而实现高效的事件驱动。
    async fn run_async(&mut self) {
        log::debug!("[[AMLL Connector Worker] 已进入纯异步 select! 事件循环。");

        loop {
            tokio::select! {
                // `biased` 关键字确保优先处理更高优先级的事件，
                // 特别是来自主应用的 Shutdown 命令，以实现快速响应。
                biased;

                // --- 1. 处理来自主应用的命令 ---
                maybe_command = self.command_rx.recv() => {
                    match maybe_command {
                        Some(command) => {
                            // 立即处理 Shutdown 命令以跳出循环
                            if let ConnectorCommand::Shutdown = command {
                                log::trace!("[[AMLL Connector Worker] 收到主应用关闭命令，准备退出循环...");
                                break;
                            }
                            self.handle_command_from_app(command).await;
                        },
                        None => {
                             log::error!("[[AMLL Connector Worker] 与主应用的命令通道已断开，Worker必须退出。");
                             break;
                        }
                    }
                },

                // --- 2. 处理来自 SMTC 处理器的更新 ---
                maybe_update = self.smtc_update_rx.recv() => {
                    match maybe_update {
                        Some(connector_update) => {
                            self.smtc_channel_error_reported = false;
                            if self.update_tx_to_app.send(connector_update).is_err() {
                                log::error!("[[AMLL Connector Worker] 发送SMTC更新到主应用失败，Worker即将退出。");
                                break;
                            }
                        },
                        None => {
                             if self.config.lock().unwrap().enabled && !self.smtc_channel_error_reported {
                                log::warn!("[[AMLL Connector Worker] SMTC处理器更新通道已断开 (线程可能已退出)。");
                                if self.update_tx_to_app.send(ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::错误("SMTC处理器异常".to_string()))).is_ok() {
                                    self.smtc_channel_error_reported = true;
                                } else {
                                    break;
                                }
                            }
                            self.smtc_handler_thread_handle = None;
                            self.smtc_shutdown_signal_tx = None;
                        }
                    }
                },

                // --- 3. 处理来自 WebSocket 客户端的状态更新 ---
                maybe_status = self.ws_status_rx.recv() => {
                    match maybe_status {
                        Some(ws_status) => {
                            log::trace!("[AMLL Connector Worker] 从 ws_status_rx 收到状态: {:?}", ws_status);

                            self.ws_status = ws_status.clone();

                            if ws_status == WebsocketStatus::已连接 {
                                self.ws_channel_error_reported = false;
                                self.ws_media_cmd_rx_unexpectedly_disconnected = false;
                            }

                            if self.update_tx_to_app.send(ConnectorUpdate::WebsocketStatusChanged(ws_status)).is_err() {
                                log::error!("[[AMLL Connector Worker] 发送WebSocket状态更新到主应用失败，Worker即将退出。");
                                break;
                            }
                            log::debug!("[[AMLL Connector Worker]] 已成功发送 WebsocketStatusChanged 更新到主应用。");

                        },
                        None => {
                            let was_intentional_stop = self.ws_shutdown_signal_tx.is_none() || self.ws_outgoing_tx.is_none();
                            if !was_intentional_stop && !self.ws_channel_error_reported {
                                log::error!("[[AMLL Connector Worker] WebSocket客户端状态通道意外断开。");
                                if self.update_tx_to_app.send(ConnectorUpdate::WebsocketStatusChanged(WebsocketStatus::错误("WebSocket客户端异常".to_string()))).is_ok() {
                                    self.ws_channel_error_reported = true;
                                } else {
                                    break;
                                }
                                self.websocket_client_task_handle = None;
                                self.ws_shutdown_signal_tx = None;
                                self.ws_outgoing_tx = None;
                                self.ws_media_cmd_rx_unexpectedly_disconnected = true;
                            }
                        }
                    }
                },

                // --- 4. 处理来自 WebSocket 客户端的媒体控制命令 ---
                 maybe_media_cmd = self.ws_media_cmd_rx.recv() => {
                    match maybe_media_cmd {
                        Some(smtc_cmd_from_ws) => {
                             if self.config.lock().unwrap().enabled {
                                 if let Some(sender) = &self.smtc_control_tx {
                                     if sender.send(ConnectorCommand::MediaControl(smtc_cmd_from_ws.clone())).is_err() {
                                         log::error!("[[AMLL Connector Worker] 发送来自网络的媒体控制命令 ({:?}) 到SMTC处理器失败。", smtc_cmd_from_ws);
                                     }
                                 }
                             }
                        },
                        None => {
                            let was_intentional_stop = self.ws_shutdown_signal_tx.is_none() || self.ws_outgoing_tx.is_none();
                             if !was_intentional_stop && !self.ws_media_cmd_rx_unexpectedly_disconnected {
                                log::warn!("[[AMLL Connector Worker] 与WebSocket客户端的媒体命令通道意外断开。");
                                self.ws_media_cmd_rx_unexpectedly_disconnected = true;
                            }
                        }
                    }
                },

                // --- 5. 处理来自音频捕获器的更新 ---
                // `if self.audio_capture_update_rx.is_some()` 确保只在音频捕获启动时才监听此分支。
                maybe_update = async {
                    if let Some(rx) = self.audio_capture_update_rx.as_mut() {
                        rx.recv().await
                    } else {
                        // 返回一个永远不会完成的Future，因为此代码路径不应被执行。
                        std::future::pending().await
                    }
                }, if self.audio_capture_update_rx.is_some() => {
                    match maybe_update {
                        Some(ConnectorUpdate::AudioDataPacket(audio_bytes)) => {
                            let config_guard = self.config.lock().unwrap();
                            if config_guard.enabled && self.ws_outgoing_tx.is_some() && !audio_bytes.is_empty() {
                                let body = ProtocolBody::OnAudioData { data: audio_bytes };
                                self.send_protocol_body_to_ws(body);
                            }
                        }
                        Some(unexpected_update) => {
                             log::warn!("[AMLL连接器-Worker] 从音频捕获器收到意外的更新类型: {:?}", unexpected_update);
                        }
                        None => {
                            log::error!("[AMLL连接器-Worker] 与音频捕获器的数据通道已断开 (线程可能已退出)。");
                            self.stop_audio_visualization_internal();
                        }
                    }
                }

            }
        }

        self.shutdown_all_subsystems();
    }

    /// 异步处理从主应用接收到的单个命令。
    ///
    /// 这个辅助函数被 `run_async` 调用，以保持 `select!` 循环的整洁。
    async fn handle_command_from_app(&mut self, command: ConnectorCommand) {
        log::trace!("[[AMLL Connector Worker] 正在处理命令: {:?}", command);
        match command {
            ConnectorCommand::UpdateConfig(new_config) => {
                let old_config;
                {
                    let current_config_mg = self.config.lock().unwrap();
                    old_config = current_config_mg.clone();
                }

                if old_config.enabled && !new_config.enabled {
                    log::debug!(
                        "[[AMLL Connector Worker] 配置从“启用”变为“禁用”，正在停止所有子系统。"
                    );
                    self.shutdown_all_subsystems();
                    if self
                        .update_tx_to_app
                        .send(ConnectorUpdate::WebsocketStatusChanged(
                            WebsocketStatus::断开,
                        ))
                        .is_err()
                    {
                        log::error!("[[AMLL Connector Worker] 发送禁用后的“断开”状态失败。");
                    }
                } else if !old_config.enabled && new_config.enabled {
                    log::debug!(
                        "[[AMLL Connector Worker] 配置从“禁用”变为“启用”。此状态变更应由外部管理器通过重启Worker来处理。"
                    );
                } else if old_config.enabled
                    && new_config.enabled
                    && old_config.websocket_url != new_config.websocket_url
                {
                    log::debug!(
                        "[[AMLL Connector Worker] WebSocket URL已更改。建议重启Worker以应用新URL。"
                    );
                }

                *self.config.lock().unwrap() = new_config;
            }
            ConnectorCommand::SendLyricTtml(ttml_string) => {
                if self.config.lock().unwrap().enabled {
                    let body = ProtocolBody::SetLyricFromTTML {
                        data: ttml_string.into(),
                    };
                    // ▼▼▼【修改】移除 .await ▼▼▼
                    self.send_protocol_body_to_ws(body);
                }
            }
            ConnectorCommand::SendProtocolBody(protocol_body) => {
                if self.config.lock().unwrap().enabled {
                    // ▼▼▼【修改】移除 .await ▼▼▼
                    self.send_protocol_body_to_ws(protocol_body);
                }
            }

            ConnectorCommand::SelectSmtcSession(session_id) => {
                if self.config.lock().unwrap().enabled {
                    if let Some(sender) = &self.smtc_control_tx {
                        if sender
                            .send(ConnectorCommand::SelectSmtcSession(session_id.clone()))
                            .is_err()
                        {
                            log::error!(
                                "[[AMLL Connector Worker] 发送“选择SMTC会话 ({})”命令失败。",
                                session_id
                            );
                        }
                    } else {
                        log::error!("[[AMLL Connector Worker] SMTC控制通道无效，无法选择会话。");
                    }
                }
            }
            ConnectorCommand::MediaControl(smtc_cmd) => {
                if self.config.lock().unwrap().enabled {
                    if let Some(sender) = &self.smtc_control_tx {
                        if sender
                            .send(ConnectorCommand::MediaControl(smtc_cmd.clone()))
                            .is_err()
                        {
                            log::error!(
                                "[[AMLL Connector Worker] 发送媒体控制命令 ({:?}) 失败。",
                                smtc_cmd
                            );
                        }
                    } else {
                        log::error!(
                            "[[AMLL Connector Worker] SMTC控制通道无效，无法发送媒体控制命令。"
                        );
                    }
                }
            }
            ConnectorCommand::AdjustAudioSessionVolume {
                target_identifier,
                volume,
                mute,
            } => {
                self.handle_volume_adjustment(target_identifier, volume, mute);
            }
            ConnectorCommand::RequestAudioSessionVolume(target_identifier) => {
                self.handle_volume_adjustment(target_identifier, None, None);
            }
            ConnectorCommand::StartAudioVisualization => {
                self.start_audio_visualization_internal();
            }
            ConnectorCommand::StopAudioVisualization => {
                self.stop_audio_visualization_internal();
            }
            ConnectorCommand::DisconnectWebsocket => {
                if self.config.lock().unwrap().enabled {
                    log::debug!(
                        "[[AMLL Connector Worker] 收到断开WebSocket命令，正在停止客户端..."
                    );
                    self.stop_websocket_client_task();
                    if self
                        .update_tx_to_app
                        .send(ConnectorUpdate::WebsocketStatusChanged(
                            WebsocketStatus::断开,
                        ))
                        .is_err()
                    {
                        log::error!("[[AMLL Connector Worker] 发送断开WebSocket后的状态更新失败。");
                    }
                }
            }
            ConnectorCommand::Shutdown => {
                // 此命令已在 `select!` 循环中被优先处理，这里仅作穷举匹配。
            }
        }
    }

    /// 在一个独立的阻塞任务中处理音量获取或设置。
    ///
    /// # Arguments
    /// * `target_identifier` - 目标会话的标识符。
    /// * `volume` - 如果是 `Some`, 则设置为此音量。
    /// * `mute` - 如果是 `Some`, 则设置为此静音状态。
    ///
    /// 如果 `volume` 和 `mute` 都是 `None`，则只获取当前状态。
    fn handle_volume_adjustment(
        &self,
        target_identifier: String,
        volume: Option<f32>,
        mute: Option<bool>,
    ) {
        if !self.config.lock().unwrap().enabled {
            log::warn!("[[AMLL Connector Worker] 音量控制命令被忽略，因为连接器未启用。");
            return;
        }

        let rt_clone = Arc::clone(&self.tokio_runtime);
        let update_tx_clone = self.update_tx_to_app.clone();
        rt_clone.spawn_blocking(move || {
            match volume_control::get_pid_from_identifier(&target_identifier) {
                Some(pid) => {
                    if volume.is_some() || mute.is_some() {
                        if let Err(e) = volume_control::set_process_volume_by_pid(pid, volume, mute)
                        {
                            log::error!(
                                "[音量控制任务] 设置PID {} ({}) 的音量/静音失败: {}",
                                pid,
                                target_identifier,
                                e
                            );
                            return;
                        }
                    }

                    match volume_control::get_process_volume_by_pid(pid) {
                        Ok((current_vol, current_mute)) => {
                            let update_msg = ConnectorUpdate::AudioSessionVolumeChanged {
                                session_id: target_identifier,
                                volume: current_vol,
                                is_muted: current_mute,
                            };
                            if let Err(e_send) = update_tx_clone.send(update_msg) {
                                log::error!("[音量控制任务] 发送音量变更更新失败: {}", e_send);
                            }
                        }
                        Err(e_get) => {
                            log::error!(
                                "[音量控制任务] 获取PID {} ({}) 的音量状态失败: {}",
                                pid,
                                target_identifier,
                                e_get
                            );
                        }
                    }
                }
                None => {
                    log::warn!(
                        "[音量控制任务] 无法为目标 '{}' 找到PID。",
                        target_identifier
                    );
                }
            }
        });
    }

    /// 异步地将一个 `ProtocolBody` 发送到 WebSocket 客户端。
    fn send_protocol_body_to_ws(&self, body: ProtocolBody) {
        if !matches!(self.ws_status, WebsocketStatus::已连接) {
            // 如果WebSocket未连接，则直接丢弃消息，不尝试发送，也不记录日志（除非是调试需要）。
            if !matches!(
                body,
                ProtocolBody::OnAudioData { .. } | ProtocolBody::OnPlayProgress { .. }
            ) {
                log::trace!(
                    "[[AMLL Connector Worker] WebSocket 未连接 (状态: {:?})，丢弃协议消息。",
                    self.ws_status
                );
            }
            return;
        }

        if let Some(sender) = &self.ws_outgoing_tx {
            // 对高频的音频数据不记录日志以避免刷屏
            let is_high_freq_data = matches!(body, ProtocolBody::OnAudioData { .. });

            match sender.try_send(body) {
                Ok(_) => {
                    // 成功发送，可以选择性地记录日志
                    if !is_high_freq_data {
                        log::trace!("[[AMLL Connector Worker] 已将协议消息放入WebSocket出站通道。");
                    }
                }
                Err(e) => match e {
                    tokio::sync::mpsc::error::TrySendError::Full(_) => {
                        // 通道已满，这是一个预料之中的情况，当客户端正在重连时会发生。
                        // 记录一个警告即可，不要阻塞。
                        if !is_high_freq_data {
                            log::info!(
                                "[[AMLL Connector Worker] WebSocket命令通道已满，丢弃一条消息。"
                            );
                        }
                    }
                    tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                        // 通道已关闭，说明客户端任务已经终止。
                        if !is_high_freq_data {
                            log::warn!(
                                "[[AMLL Connector Worker] WebSocket命令通道已关闭，无法发送消息。"
                            );
                        }
                    }
                },
            }
        } else if !matches!(body, ProtocolBody::OnAudioData { .. }) {
            let body_type_for_log = format!("{body:?}")
                .split_whitespace()
                .next()
                .unwrap_or("UnknownProtocol")
                .to_string();
            log::warn!(
                "[[AMLL Connector Worker] WebSocket发送通道无效，无法发送协议消息 ({})。",
                body_type_for_log
            );
        }
    }
}

/// `AMLLConnectorWorker` 的 `Drop` 实现。
/// 确保在 Worker 实例被意外丢弃时，所有子系统都能被正确关闭，防止资源泄漏。
impl Drop for AMLLConnectorWorker {
    fn drop(&mut self) {
        log::trace!("[[AMLL Connector Worker] Worker实例正在被Drop，执行最后的清理...");
        self.shutdown_all_subsystems();
    }
}

/// 启动 `AMLLConnectorWorker` 线程的公共入口函数。
///
/// 这是从外部模块（通常是 `amll_connector_manager`）创建和运行 Worker 的标准方式。
/// 它会创建一个新的系统线程，并将所有权和控制权移交给 `AMLLConnectorWorker::run`。
pub fn start_amll_connector_worker_thread(
    initial_config: AMLLConnectorConfig,
    command_rx: StdReceiver<ConnectorCommand>,
    update_tx: StdSender<ConnectorUpdate>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("amll_connector_worker_thread".to_string())
        .spawn(move || {
            AMLLConnectorWorker::run(initial_config, command_rx, update_tx);
        })
        .expect("无法启动AMLL连接器核心工作线程")
}
