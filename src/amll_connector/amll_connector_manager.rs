use super::{ConnectorUpdate, WebsocketStatus};
use crate::amll_connector::NowPlayingInfo;
use crate::amll_connector::{self, AMLLConnectorConfig, ConnectorCommand};
use crate::amll_lyrics_fetcher::{self, AmllIndexEntry, AmllSearchField};
use crate::app_definition::UniLyricApp;
use crate::types::{AmllIndexDownloadState, AmllTtmlDownloadState};
use log::{self, debug, error, info, trace, warn};
use reqwest::Client;
use std::path::PathBuf;
use std::sync::mpsc::Sender as StdSender;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::sync::{Mutex as TokioMutex, oneshot};
use tokio::time::Duration;
use ws_protocol::Body as ProtocolBody;

/// 确保AMLL Connector worker 正在运行（如果已启用）
pub fn ensure_running(app: &mut UniLyricApp) {
    let is_enabled_in_config;
    let current_ws_status;
    {
        let config_guard = app.media_connector_config.lock().unwrap();
        is_enabled_in_config = config_guard.enabled;
        current_ws_status = app.media_connector_status.lock().unwrap().clone();
    }

    if !is_enabled_in_config {
        stop_worker(app);
        return;
    }

    let worker_is_running = app
        .media_connector_worker_handle
        .as_ref()
        .is_some_and(|h| !h.is_finished());

    let is_connection_healthy = matches!(
        current_ws_status,
        WebsocketStatus::已连接 | WebsocketStatus::连接中
    );

    if worker_is_running && is_connection_healthy {
        log::debug!(
            "[AMLLManager] AMLL Connector worker 正常运行中 (Status: {:?})。无需重启。",
            current_ws_status
        );
        check_index_download(app);
        return;
    }

    log::info!(
        "[AMLLManager] 正在启动/重启 AMLL Connector worker。原因: [Worker运行中: {}, WS状态: {:?}]",
        worker_is_running,
        current_ws_status
    );
    stop_worker(app);

    let (command_tx_for_worker, command_rx_for_worker) =
        std::sync::mpsc::channel::<ConnectorCommand>();
    app.media_connector_command_tx = Some(command_tx_for_worker);

    let update_tx_clone_for_worker = app.media_connector_update_tx_for_worker.clone();

    let initial_config_clone;
    {
        let config_guard = app.media_connector_config.lock().unwrap();
        initial_config_clone = config_guard.clone();
    }

    let handle = amll_connector::worker::start_amll_connector_worker_thread(
        initial_config_clone,
        command_rx_for_worker,
        update_tx_clone_for_worker,
    );
    app.media_connector_worker_handle = Some(handle);

    if app.audio_visualization_is_active {
        if let Some(ref tx) = app.media_connector_command_tx {
            if tx.send(ConnectorCommand::StartAudioVisualization).is_err() {
                log::error!(
                    "[AMLLManager] Failed to send StartAudioVisualization command to the new worker upon restart."
                );
            }
        }
    }

    check_index_download(app);

    if let Some(ref initial_id) = app.initial_selected_smtc_session_id_from_settings {
        if let Some(ref tx) = app.media_connector_command_tx {
            log::debug!("[AMLLManager] 应用启动时，尝试恢复上次选择的 SMTC 会话 ID: {initial_id}");
            if tx
                .send(ConnectorCommand::SelectSmtcSession(initial_id.clone()))
                .is_err()
            {
                log::error!("[AMLLManager] 启动时发送 SelectSmtcSession 命令失败。");
            }
            *app.selected_smtc_session_id.lock().unwrap() = Some(initial_id.clone());
        } else {
            log::warn!("[AMLLManager] 启动时无法应用上次选择的 SMTC 会话：command_tx 不可用。");
        }
    }
}

/// 停止AMLL Connector worker
pub fn stop_worker(app: &mut UniLyricApp) {
    if let Some(command_tx) = app.media_connector_command_tx.take() {
        log::debug!("[AMLLManager] 向 AMLL Connector worker 发送关闭命令...");
        if command_tx.send(ConnectorCommand::Shutdown).is_err() {
            log::warn!("[AMLLManager] 发送关闭命令给 AMLL Connector worker 失败 (可能已关闭)。");
        }
    }

    *app.media_connector_status.lock().unwrap() = WebsocketStatus::断开;

    let media_info_arc_clone = Arc::clone(&app.current_media_info);
    app.tokio_runtime.block_on(async move {
        let mut guard = media_info_arc_clone.lock().await;
        *guard = None;
    });

    app.stop_progress_timer();
}

/// 检查AMLL索引状态，如果需要则触发下载。
pub fn check_index_download(app: &mut UniLyricApp) {
    let connector_enabled = app.media_connector_config.lock().unwrap().enabled;
    if !connector_enabled {
        return;
    }

    let current_index_state_clone;
    {
        let index_state_lock = app.amll_index_download_state.lock().unwrap();
        current_index_state_clone = index_state_lock.clone();
    }

    match current_index_state_clone {
        AmllIndexDownloadState::Idle | AmllIndexDownloadState::Error(_) => {
            log::info!(
                "[AMLLManager] AMLL Connector 已启用，索引状态为 {current_index_state_clone:?}。正在检查 AMLL DB 索引更新。"
            );
            app.check_for_amll_index_update();
        }
        AmllIndexDownloadState::Success(ref loaded_head) => {
            log::info!(
                "[AMLLManager] AMLL Connector 已启用，AMLL DB 索引已加载 (HEAD: {}，{} 条)。正在检查是否有新版本。",
                loaded_head.chars().take(7).collect::<String>(),
                app.amll_index.lock().unwrap().len()
            );
            app.check_for_amll_index_update();
        }
        AmllIndexDownloadState::CheckingForUpdate
        | AmllIndexDownloadState::Downloading(_)
        | AmllIndexDownloadState::UpdateAvailable(_) => {
            // 如果正在检查、下载或已知有更新，则不重复触发检查。
            log::info!(
                "[AMLLManager] AMLL Connector 已启用，AMLL DB 索引操作正在进行中 (状态: {current_index_state_clone:?})。"
            );
        }
    }
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

/// 触发 AMLL 索引的异步更新检查。
///
/// 此函数会生成一个 Tokio 任务来执行实际的网络请求和状态更新。
pub fn trigger_amll_index_update_check(
    http_client: Client,
    amll_index_download_state: Arc<Mutex<AmllIndexDownloadState>>,
    amll_index_cache_path: Option<PathBuf>,
    tokio_runtime: Arc<Runtime>,
) {
    // 生成一个异步任务来执行检查
    tokio_runtime.spawn(async move {
        info!("[AMLL_Manager check_update_task] 开始检查 AMLL 索引更新...");
        match amll_lyrics_fetcher::amll_fetcher::fetch_remote_index_head(&http_client).await {
            Ok(remote_head) => {
                let mut state_lock = amll_index_download_state.lock().unwrap();
                let mut cached_head_on_disk: Option<String> = None;

                if let Some(ref cache_p) = amll_index_cache_path {
                     match amll_lyrics_fetcher::amll_fetcher::load_cached_index_head(cache_p) {
                        Ok(Some(head)) => cached_head_on_disk = Some(head),
                        Ok(None) => info!("[AMLL_Manager check_update_task] 未找到缓存的 HEAD。"),
                        Err(e) => warn!("[AMLL_Manager check_update_task] 加载缓存 HEAD 失败: {e}"),
                    }
                }

                if cached_head_on_disk.as_ref() == Some(&remote_head) {
                    info!("[AMLL_Manager check_update_task] 远程 HEAD ({}) 与缓存 HEAD 相同。索引已是最新。", remote_head.chars().take(7).collect::<String>());
                    // 即使 HEAD 相同，如果之前的状态不是 Success(remote_head)，也更新它
                    if !matches!(*state_lock, AmllIndexDownloadState::Success(ref s) if *s == remote_head) {
                         *state_lock = AmllIndexDownloadState::Success(remote_head);
                    }
                } else {
                    info!("[AMLL_Manager check_update_task] 远程 HEAD ({}) 与缓存 HEAD ({:?}) 不同。有可用更新。",
                               remote_head.chars().take(7).collect::<String>(),
                               cached_head_on_disk.as_ref().map(|s| s.chars().take(7).collect::<String>()));
                    *state_lock = AmllIndexDownloadState::UpdateAvailable(remote_head);
                }
            }
            Err(e) => {
                error!("[AMLL_Manager check_update_task] 检查 AMLL 索引更新失败: {e}");
                let mut state_lock = amll_index_download_state.lock().unwrap();
                *state_lock = AmllIndexDownloadState::Error(format!("检查更新失败: {e}"));
            }
        }
    });
}

pub struct AmllIndexDownloadParams {
    pub http_client: Client,
    pub amll_db_repo_url_base: String,
    pub amll_index_data: Arc<Mutex<Vec<AmllIndexEntry>>>,
    pub amll_index_download_state: Arc<Mutex<AmllIndexDownloadState>>,
    pub amll_index_cache_path: Option<PathBuf>,
    pub tokio_runtime: Arc<Runtime>,
}

/// 触发 AMLL 索引的异步下载和解析。
///
/// UniLyricApp 会进行前置检查并设置初始的 Downloading 状态。
/// 此函数负责获取最终的 HEAD SHA (如果需要) 并执行下载。
pub fn trigger_amll_index_download_async(
    params: AmllIndexDownloadParams,
    force_network_refresh: bool,
    initial_head_candidate: Option<String>,
) {
    params.tokio_runtime.spawn(async move {
        let final_head_sha_to_use: String;

        // 1. 确定最终要使用的 HEAD SHA
        if force_network_refresh {
            trace!("[AMLL_Manager dl_task] 强制刷新模式：获取最新的远程 HEAD SHA...");
            match amll_lyrics_fetcher::amll_fetcher::fetch_remote_index_head(&params.http_client).await {
                Ok(head) => {
                    info!("[AMLL_Manager dl_task] (强制刷新) 成功获取最新 HEAD: {}", head.chars().take(7).collect::<String>());
                    final_head_sha_to_use = head;
                }
                Err(e) => {
                    error!("[AMLL_Manager dl_task] (强制刷新) 获取远程 HEAD 失败: {e}. 将尝试使用 'unknown' 作为 HEAD 进行下载。");
                    final_head_sha_to_use = "unknown".to_string();
                    // 更新下载状态以反映正在尝试下载的版本（即使是 "unknown"）
                    let mut state_lock = params.amll_index_download_state.lock().unwrap();
                    *state_lock = AmllIndexDownloadState::Downloading(Some(final_head_sha_to_use.clone()));
                    // 不在此处返回，继续尝试下载
                }
            }
        } else if let Some(head_from_caller) = initial_head_candidate {
            // 如果调用者提供了一个 HEAD (通常来自 UpdateAvailable 状态)
            trace!("[AMLL_Manager dl_task] 使用调用者提供的 HEAD: {}", head_from_caller.chars().take(7).collect::<String>());
            final_head_sha_to_use = head_from_caller;
        } else {
            // 非强制刷新，且调用者未提供 HEAD (例如从 Idle 或 Error 状态触发)
            trace!("[AMLL_Manager dl_task] (非强制，无预设 HEAD) 获取最新的远程 HEAD SHA...");
            match amll_lyrics_fetcher::amll_fetcher::fetch_remote_index_head(&params.http_client).await {
                Ok(head) => {
                    info!("[AMLL_Manager dl_task] (非强制) 成功获取最新 HEAD: {}", head.chars().take(7).collect::<String>());
                    final_head_sha_to_use = head;
                }
                Err(e) => {
                    error!("[AMLL_Manager dl_task] (非强制) 获取远程 HEAD 失败: {e}");
                    let mut state_lock = params.amll_index_download_state.lock().unwrap();
                    *state_lock = AmllIndexDownloadState::Error(format!("获取远程 HEAD 失败: {e}"));
                    return; // 获取 HEAD 失败，则不继续下载
                }
            }
        }

        // 2. 确保下载状态反映了正在下载的 HEAD 版本
        // (UniLyricApp 可能已经设置了 Downloading(initial_head_candidate)，这里确保它与 final_head_sha_to_use 一致)
        {
            let mut state_lock = params.amll_index_download_state.lock().unwrap();
            if !matches!(*state_lock, AmllIndexDownloadState::Downloading(Some(ref s)) if *s == final_head_sha_to_use) {
                *state_lock = AmllIndexDownloadState::Downloading(Some(final_head_sha_to_use.clone()));
            }
        }

        info!(
            "[AMLL_Manager dl_task] 开始下载 AMLL 索引 (HEAD: '{}')...",
            final_head_sha_to_use.chars().take(7).collect::<String>()
        );

        let Some(cache_p_for_network_save) = params.amll_index_cache_path else {
            error!("[AMLL_Manager dl_task] 缓存路径未设置，无法保存下载的索引。");
            let mut state_lock = params.amll_index_download_state.lock().unwrap();
            *state_lock = AmllIndexDownloadState::Error("缓存路径未设置".to_string());
            return;
        };

        // 3. 执行下载和解析
        match amll_lyrics_fetcher::amll_fetcher::download_and_parse_index(
            &params.http_client,
            &params.amll_db_repo_url_base,
            &cache_p_for_network_save,
            final_head_sha_to_use.clone(), // 用于写入 .head 文件的 SHA
        )
        .await
        {
            Ok(parsed_entries) => {
                let mut index_data_lock = params.amll_index_data.lock().unwrap();
                *index_data_lock = parsed_entries;
                drop(index_data_lock);

                let mut state_lock = params.amll_index_download_state.lock().unwrap();
                *state_lock = AmllIndexDownloadState::Success(final_head_sha_to_use);
                info!("[AMLL_Manager dl_task] AMLL 索引文件下载并解析成功。");
            }
            Err(e) => {
                error!("[AMLL_Manager dl_task] AMLL 索引文件下载或解析失败: {e}");
                let mut state_lock = params.amll_index_download_state.lock().unwrap();
                *state_lock = AmllIndexDownloadState::Error(format!("索引下载/解析失败: {e}"));
            }
        }
    });
}

/// 代表 `handle_amll_lyrics_search_or_download_async` 函数可以执行的操作。
pub enum AmllLyricsAction {
    /// 下载指定的单个歌词条目。
    Download(AmllIndexEntry),
    /// 在索引中搜索歌词。
    Search {
        /// 用户的搜索查询字符串。
        query: String,
        /// 搜索的目标字段（如标题、艺术家等）。
        field: AmllSearchField,
        /// 持有完整索引数据的 Arc<Mutex>，用于执行搜索。
        index_data: Arc<Mutex<Vec<AmllIndexEntry>>>,
        /// 用于存放和更新搜索结果的 Arc<Mutex>。
        search_results: Arc<Mutex<Vec<AmllIndexEntry>>>,
    },
}

/// 异步处理 AMLL TTML 歌词的搜索或下载。
///
/// # 参数
///
/// * `http_client` - 用于发起网络请求的 `reqwest::Client`。
/// * `amll_db_repo_url_base` - AMLL 数据库仓库的基础 URL。
/// * `amll_ttml_download_state` - 用于向 UI 反映当前 TTML 操作状态的共享状态。
/// * `tokio_runtime` - 用于生成异步任务的 Tokio 运行时。
/// * `action` - 定义了具体要执行的操作，是 `AmllLyricsAction::Download` 或 `AmllLyricsAction::Search`。
pub fn handle_amll_lyrics_search_or_download_async(
    http_client: Client,
    amll_db_repo_url_base: String,
    amll_ttml_download_state: Arc<Mutex<AmllTtmlDownloadState>>,
    tokio_runtime: Arc<Runtime>,
    action: AmllLyricsAction,
) {
    // 根据 action 的类型分派任务
    match action {
        // --- 情况1: 执行下载操作 ---
        AmllLyricsAction::Download(entry) => {
            debug!(
                "[AMLL_Manager] 准备下载选定的 AMLL TTML Database 文件 '{}'",
                entry.raw_lyric_file
            );

            // 在异步任务开始前，立即更新状态为 DownloadingTtml
            {
                let mut ttml_dl_state_lock = amll_ttml_download_state.lock().unwrap();
                *ttml_dl_state_lock = AmllTtmlDownloadState::DownloadingTtml;
            }

            // 生成一个异步任务来执行网络下载
            tokio_runtime.spawn(async move {
                match amll_lyrics_fetcher::amll_fetcher::download_ttml_from_entry(
                    &http_client,
                    &amll_db_repo_url_base,
                    &entry,
                )
                .await
                {
                    Ok(fetched_lyrics) => {
                        // 下载成功，更新状态为 Success 并附带歌词内容
                        let mut state_lock = amll_ttml_download_state.lock().unwrap();
                        *state_lock = AmllTtmlDownloadState::Success(fetched_lyrics);
                        info!(
                            "[AMLL_Manager] AMLL TTML 文件下载成功: {}",
                            entry.raw_lyric_file
                        );
                    }
                    Err(e) => {
                        // 下载失败，更新状态为 Error 并附带错误信息
                        error!("[AMLL_Manager] AMLL TTML 文件下载失败: {e}");
                        let mut state_lock = amll_ttml_download_state.lock().unwrap();
                        *state_lock = AmllTtmlDownloadState::Error(e.to_string());
                    }
                }
            });
        }

        // --- 情况2: 执行搜索操作 ---
        AmllLyricsAction::Search {
            query,
            field,
            index_data,
            search_results,
        } => {
            let query_str = query.trim();
            if query_str.is_empty() {
                info!("[AMLL_Manager] AMLL TTML Database 搜索查询为空，清空搜索结果。");
                // 清空UI上显示的搜索结果
                if let Ok(mut results_lock) = search_results.lock() {
                    results_lock.clear();
                }
                // 将状态重置为空闲
                if let Ok(mut ttml_dl_state_lock) = amll_ttml_download_state.lock() {
                    *ttml_dl_state_lock = AmllTtmlDownloadState::Idle;
                }
                return;
            }

            // 设置状态为正在搜索
            {
                let mut ttml_dl_state_lock = amll_ttml_download_state.lock().unwrap();
                *ttml_dl_state_lock = AmllTtmlDownloadState::SearchingIndex;
            }

            debug!("[AMLL_Manager] 开始在 AMLL 索引中搜索: '{query_str}' (字段: {field:?})");

            // 搜索是在内存中进行的，通常很快，所以直接在这里执行。
            let search_results_vec = {
                // 限制 index_data_lock 的作用域，使其在搜索后立即被释放
                let index_data_lock = index_data.lock().unwrap();
                amll_lyrics_fetcher::amll_fetcher::search_lyrics_in_index(
                    query_str,
                    &field,
                    &index_data_lock,
                )
            };

            // 更新用于UI展示的搜索结果列表
            {
                let mut results_display_lock = search_results.lock().unwrap();
                *results_display_lock = search_results_vec;
                info!(
                    "[AMLL_Manager] AMLL 索引搜索完成，找到 {} 个结果。",
                    results_display_lock.len()
                );
            }

            // 搜索完成后，将状态设置回 Idle，等待用户选择条目进行下载或进行下一次操作
            {
                let mut ttml_dl_state_lock = amll_ttml_download_state.lock().unwrap();
                *ttml_dl_state_lock = AmllTtmlDownloadState::Idle;
            }
        }
    }
}
