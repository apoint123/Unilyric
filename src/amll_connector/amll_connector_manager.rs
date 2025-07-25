use super::{ConnectorUpdate, WebsocketStatus};
use crate::amll_connector::NowPlayingInfo;
use crate::amll_connector::{self, AMLLConnectorConfig, ConnectorCommand};
use crate::app_definition::UniLyricApp;
use log::{self, trace, warn};
use std::sync::Arc;
use std::sync::mpsc::Sender as StdSender;
use tokio::sync::{Mutex as TokioMutex, oneshot};
use tokio::time::Duration;
use ws_protocol::Body as ProtocolBody;

/// 确保AMLL Connector worker 正在运行（如果已启用）
pub fn ensure_running(app: &mut UniLyricApp) {
    let is_enabled_in_config;
    let current_ws_status;
    {
        let config_guard = app.player.config.lock().unwrap();
        is_enabled_in_config = config_guard.enabled;
        current_ws_status = app.player.status.lock().unwrap().clone();
    }

    if !is_enabled_in_config {
        stop_worker(app);
        return;
    }

    let worker_is_running = app
        .player
        .worker_handle
        .as_ref()
        .is_some_and(|h| !h.is_finished());

    let is_connection_healthy = matches!(
        current_ws_status,
        WebsocketStatus::已连接 | WebsocketStatus::连接中
    );

    if worker_is_running && is_connection_healthy {
        log::debug!(
            "[AMLLManager] AMLL Connector worker 正常运行中 (Status: {current_ws_status:?})。无需重启。"
        );
        return;
    }

    log::info!(
        "[AMLLManager] 正在启动/重启 AMLL Connector worker。原因: [Worker运行中: {worker_is_running}, WS状态: {current_ws_status:?}]"
    );
    stop_worker(app);

    let (command_tx_for_worker, command_rx_for_worker) =
        std::sync::mpsc::channel::<ConnectorCommand>();
    app.player.command_tx = Some(command_tx_for_worker);

    let update_tx_clone_for_worker = app.player.update_tx_for_worker.clone();

    let initial_config_clone;
    {
        let config_guard = app.player.config.lock().unwrap();
        initial_config_clone = config_guard.clone();
    }

    let handle = amll_connector::worker::start_amll_connector_worker_thread(
        initial_config_clone,
        command_rx_for_worker,
        update_tx_clone_for_worker,
    );
    app.player.worker_handle = Some(handle);

    if app.player.audio_visualization_is_active
        && let Some(ref tx) = app.player.command_tx
            && tx.send(ConnectorCommand::StartAudioVisualization).is_err() {
                log::error!(
                    "[AMLLManager] Failed to send StartAudioVisualization command to the new worker upon restart."
                );
            }

    if let Some(ref initial_id) = app.player.initial_selected_smtc_session_id_from_settings {
        if let Some(ref tx) = app.player.command_tx {
            log::debug!("[AMLLManager] 应用启动时，尝试恢复上次选择的 SMTC 会话 ID: {initial_id}");
            if tx
                .send(ConnectorCommand::SelectSmtcSession(initial_id.clone()))
                .is_err()
            {
                log::error!("[AMLLManager] 启动时发送 SelectSmtcSession 命令失败。");
            }
            *app.player.selected_smtc_session_id.lock().unwrap() = Some(initial_id.clone());
        } else {
            log::warn!("[AMLLManager] 启动时无法应用上次选择的 SMTC 会话：command_tx 不可用。");
        }
    }
}

/// 停止AMLL Connector worker
pub fn stop_worker(app: &mut UniLyricApp) {
    if let Some(command_tx) = app.player.command_tx.take() {
        log::debug!("[AMLLManager] 向 AMLL Connector worker 发送关闭命令...");
        if command_tx.send(ConnectorCommand::Shutdown).is_err() {
            log::warn!("[AMLLManager] 发送关闭命令给 AMLL Connector worker 失败 (可能已关闭)。");
        }
    }

    *app.player.status.lock().unwrap() = WebsocketStatus::断开;

    let media_info_arc_clone = Arc::clone(&app.player.current_media_info);
    app.tokio_runtime.block_on(async move {
        let mut guard = media_info_arc_clone.lock().await;
        *guard = None;
    });

    app.stop_progress_timer();
}

pub(crate) async fn run_progress_timer_task(
    interval: Duration,
    media_info_arc: Arc<TokioMutex<Option<NowPlayingInfo>>>,
    command_tx_to_worker: StdSender<ConnectorCommand>,
    _connector_config_arc: Arc<std::sync::Mutex<AMLLConnectorConfig>>,
    mut shutdown_rx: oneshot::Receiver<()>,
    update_tx_to_app: StdSender<ConnectorUpdate>,
) {
    trace!("[ProgressTimer] 定时器已启动。间隔: {interval:?}");
    let mut ticker = tokio::time::interval(interval);

    loop {
        tokio::select! {
            biased; // 优先处理关闭信号

            _ = &mut shutdown_rx => {
                trace!("[ProgressTimer] 收到关闭信号，定时器正在退出。");
                break;
            }
            _ = ticker.tick() => {
                // 只读访问 media_info_arc 来计算，不修改它内部的 position_ms
                let base_info_for_simulation: Option<NowPlayingInfo>;
                { // 限制锁的范围
                    let media_info_guard = media_info_arc.lock().await;
                    base_info_for_simulation = media_info_guard.clone(); // 克隆一份用于只读计算
                }

                if let Some(ref info) = base_info_for_simulation { // info 现在是克隆出来的值
                    if info.is_playing.unwrap_or(false) {
                        let elapsed_since_report = info.position_report_time
                            .map_or(Duration::ZERO, |rt| rt.elapsed());

                        let mut current_simulated_pos_ms = info.position_ms.unwrap_or(0)
                            + elapsed_since_report.as_millis() as u64;

                        if let Some(duration_ms) = info.duration_ms
                            && duration_ms > 0 && current_simulated_pos_ms >= duration_ms {
                                current_simulated_pos_ms = duration_ms;
                                // 发送 OnPaused 命令给 AMLL Player
                                if command_tx_to_worker.send(ConnectorCommand::SendProtocolBody(ProtocolBody::OnPaused)).is_err() {
                                    warn!("[ProgressTimer] 发送 OnPaused (到达末尾) 到 worker 失败。");
                                }
                                // 更新 app.rs 中的播放状态 (通过 SimulatedProgressUpdate 携带一个特殊标记或发送一个新类型的Update)
                                // 或者，app.rs 在接收到 OnPaused 后自行处理 is_playing 状态。
                                // 为简单起见，这里只发送进度。app.rs 的 SMTC 事件会最终确认播放状态。
                            }

                        // 只发送模拟的播放进度
                        if command_tx_to_worker.send(ConnectorCommand::SendProtocolBody(ProtocolBody::OnPlayProgress { progress: current_simulated_pos_ms })).is_ok() {
                            if update_tx_to_app.send(ConnectorUpdate::SimulatedProgressUpdate(current_simulated_pos_ms)).is_err() {
                                warn!("[ProgressTimer] 发送 SimulatedProgressUpdate 到主应用失败。");
                            }
                        } else {
                            warn!("[ProgressTimer] 发送 OnPlayProgress 到 worker 失败。");
                        }
                    }
                }
            }
        }
    }
    trace!("[ProgressTimer] 定时器已停止。");
}
