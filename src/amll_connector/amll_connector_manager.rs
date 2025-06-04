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
    {
        let config_guard = app.media_connector_config.lock().unwrap();
        is_enabled_in_config = config_guard.enabled;
    }

    if !is_enabled_in_config {
        stop_worker(app);
        return;
    }

    if app
        .media_connector_worker_handle
        .as_ref()
        .is_some_and(|h| !h.is_finished())
    {
        log::debug!("[AMLLManager] AMLL Connector worker 已在运行 (根据配置应启用).");
        check_index_download(app);
        return;
    }

    log::debug!("[AMLLManager] 正在启动 AMLL Connector worker 线程 (配置已启用)...");
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

    check_index_download(app);

    if let Some(ref initial_id) = app.initial_selected_smtc_session_id_from_settings {
        if let Some(ref tx) = app.media_connector_command_tx {
            log::debug!(
                "[AMLLManager] 应用启动时，尝试恢复上次选择的 SMTC 会话 ID: {}",
                initial_id
            );
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
                "[AMLLManager] AMLL Connector 已启用，索引状态为 {:?}。正在检查 AMLL DB 索引更新。",
                current_index_state_clone
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
                "[AMLLManager] AMLL Connector 已启用，AMLL DB 索引操作正在进行中 (状态: {:?})。",
                current_index_state_clone
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
    trace!("[ProgressTimer] 定时器已启动。间隔: {:?}", interval);
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

                        if let Some(duration_ms) = info.duration_ms {
                            if duration_ms > 0 && current_simulated_pos_ms >= duration_ms {
                                current_simulated_pos_ms = duration_ms;
                                // 发送 OnPaused 命令给 AMLL Player
                                if command_tx_to_worker.send(ConnectorCommand::SendProtocolBody(ProtocolBody::OnPaused)).is_err() {
                                    warn!("[ProgressTimer] 发送 OnPaused (到达末尾) 到 worker 失败。");
                                }
                                // 更新 app.rs 中的播放状态 (通过 SimulatedProgressUpdate 携带一个特殊标记或发送一个新类型的Update)
                                // 或者，app.rs 在接收到 OnPaused 后自行处理 is_playing 状态。
                                // 为简单起见，这里只发送进度。app.rs 的 SMTC 事件会最终确认播放状态。
                            }
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
                        Err(e) => warn!("[AMLL_Manager check_update_task] 加载缓存 HEAD 失败: {}", e),
                    }
                }

                if Some(remote_head.clone()) == cached_head_on_disk {
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
                error!("[AMLL_Manager check_update_task] 检查 AMLL 索引更新失败: {}", e);
                let mut state_lock = amll_index_download_state.lock().unwrap();
                *state_lock = AmllIndexDownloadState::Error(format!("检查更新失败: {}", e));
            }
        }
    });
}

/// 触发 AMLL 索引的异步下载和解析。
///
/// UniLyricApp 会进行前置检查并设置初始的 Downloading 状态。
/// 此函数负责获取最终的 HEAD SHA (如果需要) 并执行下载。
pub fn trigger_amll_index_download_async(
    http_client: Client,
    amll_db_repo_url_base: String,
    amll_index_data: Arc<Mutex<Vec<AmllIndexEntry>>>, // 用于更新索引数据
    amll_index_download_state: Arc<Mutex<AmllIndexDownloadState>>, // 用于更新下载状态
    amll_index_cache_path: Option<PathBuf>,
    tokio_runtime: Arc<Runtime>,
    force_network_refresh: bool,
    // 从 UniLyricApp 传递过来的，如果之前检查到 UpdateAvailable，则这是那个 head。
    // 如果是 Idle/Error 状态触发下载，或者 force_network_refresh 为 true，这个可能是 None。
    initial_head_candidate: Option<String>,
) {
    tokio_runtime.spawn(async move {
        let final_head_sha_to_use: String;

        // 1. 确定最终要使用的 HEAD SHA
        if force_network_refresh {
            trace!("[AMLL_Manager dl_task] 强制刷新模式：获取最新的远程 HEAD SHA...");
            match amll_lyrics_fetcher::amll_fetcher::fetch_remote_index_head(&http_client).await {
                Ok(head) => {
                    info!("[AMLL_Manager dl_task] (强制刷新) 成功获取最新 HEAD: {}", head.chars().take(7).collect::<String>());
                    final_head_sha_to_use = head;
                }
                Err(e) => {
                    error!("[AMLL_Manager dl_task] (强制刷新) 获取远程 HEAD 失败: {}. 将尝试使用 'unknown' 作为 HEAD 进行下载。", e);
                    final_head_sha_to_use = "unknown".to_string();
                    // 更新下载状态以反映正在尝试下载的版本（即使是 "unknown"）
                    let mut state_lock = amll_index_download_state.lock().unwrap();
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
            match amll_lyrics_fetcher::amll_fetcher::fetch_remote_index_head(&http_client).await {
                Ok(head) => {
                    info!("[AMLL_Manager dl_task] (非强制) 成功获取最新 HEAD: {}", head.chars().take(7).collect::<String>());
                    final_head_sha_to_use = head;
                }
                Err(e) => {
                    error!("[AMLL_Manager dl_task] (非强制) 获取远程 HEAD 失败: {}", e);
                    let mut state_lock = amll_index_download_state.lock().unwrap();
                    *state_lock = AmllIndexDownloadState::Error(format!("获取远程 HEAD 失败: {}", e));
                    return; // 获取 HEAD 失败，则不继续下载
                }
            }
        }

        // 2. 确保下载状态反映了正在下载的 HEAD 版本
        // (UniLyricApp 可能已经设置了 Downloading(initial_head_candidate)，这里确保它与 final_head_sha_to_use 一致)
        {
            let mut state_lock = amll_index_download_state.lock().unwrap();
            if !matches!(*state_lock, AmllIndexDownloadState::Downloading(Some(ref s)) if *s == final_head_sha_to_use) {
                *state_lock = AmllIndexDownloadState::Downloading(Some(final_head_sha_to_use.clone()));
            }
        }

        info!(
            "[AMLL_Manager dl_task] 开始下载 AMLL 索引 (HEAD: '{}')...",
            final_head_sha_to_use.chars().take(7).collect::<String>()
        );

        let cache_p_for_network_save = match amll_index_cache_path {
            Some(p) => p,
            None => {
                error!("[AMLL_Manager dl_task] 缓存路径未设置，无法保存下载的索引。");
                let mut state_lock = amll_index_download_state.lock().unwrap();
                *state_lock = AmllIndexDownloadState::Error("缓存路径未设置".to_string());
                return;
            }
        };

        // 3. 执行下载和解析
        match amll_lyrics_fetcher::amll_fetcher::download_and_parse_index(
            &http_client,
            &amll_db_repo_url_base,
            &cache_p_for_network_save,
            final_head_sha_to_use.clone(), // 用于写入 .head 文件的 SHA
        )
        .await
        {
            Ok(parsed_entries) => {
                let mut index_data_lock = amll_index_data.lock().unwrap();
                *index_data_lock = parsed_entries;
                drop(index_data_lock);

                let mut state_lock = amll_index_download_state.lock().unwrap();
                *state_lock = AmllIndexDownloadState::Success(final_head_sha_to_use);
                info!("[AMLL_Manager dl_task] AMLL 索引文件下载并解析成功。");
            }
            Err(e) => {
                error!("[AMLL_Manager dl_task] AMLL 索引文件下载或解析失败: {}", e);
                let mut state_lock = amll_index_download_state.lock().unwrap();
                *state_lock = AmllIndexDownloadState::Error(format!("索引下载/解析失败: {}", e));
            }
        }
    });
}

/// 异步处理 AMLL TTML 歌词的搜索或下载。
///
/// - 如果 `entry_to_download` 是 `Some`, 则直接下载该条目。
/// - 如果 `entry_to_download` 是 `None`, 则使用 `search_query` 和 `search_field` 在 `amll_index_data` 中搜索，
///   并将结果存入 `amll_search_results_output`。
pub fn handle_amll_lyrics_search_or_download_async(
    http_client: Client,
    amll_db_repo_url_base: String,
    amll_ttml_download_state: Arc<Mutex<AmllTtmlDownloadState>>,
    tokio_runtime: Arc<Runtime>,
    entry_to_download: Option<AmllIndexEntry>,
    // 仅当 entry_to_download 为 None (即执行搜索时) 才使用以下参数
    search_query: Option<String>,
    search_field: Option<AmllSearchField>,
    amll_index_data: Option<Arc<Mutex<Vec<AmllIndexEntry>>>>, // 用于搜索
    amll_search_results_output: Option<Arc<Mutex<Vec<AmllIndexEntry>>>>, // 存储搜索结果
) {
    if let Some(entry) = entry_to_download {
        // --- 情况1: 直接下载选定的条目 ---
        debug!(
            "[AMLL_Manager] 准备下载选定的 AMLL TTML Database 文件 '{}'",
            entry.raw_lyric_file
        );
        // 在异步任务开始前，立即更新状态为 DownloadingTtml
        {
            let mut ttml_dl_state_lock = amll_ttml_download_state.lock().unwrap();
            *ttml_dl_state_lock = AmllTtmlDownloadState::DownloadingTtml;
        }

        tokio_runtime.spawn(async move {
            match amll_lyrics_fetcher::amll_fetcher::download_ttml_from_entry(
                &http_client,
                &amll_db_repo_url_base,
                &entry,
            )
            .await
            {
                Ok(fetched_lyrics) => {
                    let mut state_lock = amll_ttml_download_state.lock().unwrap();
                    *state_lock = AmllTtmlDownloadState::Success(fetched_lyrics);
                    info!(
                        "[AMLL_Manager] AMLL TTML 文件下载成功: {}",
                        entry.raw_lyric_file
                    );
                }
                Err(e) => {
                    error!("[AMLL_Manager] AMLL TTML 文件下载失败: {}", e);
                    let mut state_lock = amll_ttml_download_state.lock().unwrap();
                    *state_lock = AmllTtmlDownloadState::Error(e.to_string());
                }
            }
        });
    } else if let (Some(query_str), Some(field), Some(index_arc), Some(results_arc)) = (
        search_query,
        search_field,
        amll_index_data,
        amll_search_results_output,
    ) {
        // --- 情况2: 执行搜索 ---
        let query = query_str.trim();
        if query.is_empty() {
            info!("[AMLL_Manager] AMLL TTML Database 搜索查询为空，清空搜索结果。");
            if let Ok(mut results_lock) = results_arc.lock() {
                results_lock.clear();
            }
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

        // 搜索是同步的（因为它操作内存中的索引），但我们仍然可以在这里处理状态更新
        // 或者，如果搜索本身可能耗时，也可以考虑将其放入 tokio::task::spawn_blocking
        // 但通常对于内存中的Vec搜索，这是快速的。
        debug!(
            "[AMLL_Manager] 开始在 AMLL 索引中搜索: '{}' (字段: {:?})",
            query, field
        );
        let search_results_vec = {
            // 限制 index_data_lock 的作用域
            let index_data_lock = index_arc.lock().unwrap();
            amll_lyrics_fetcher::amll_fetcher::search_lyrics_in_index(
                query,
                &field,
                &index_data_lock,
            )
        };

        {
            // 限制 results_display_lock 的作用域
            let mut results_display_lock = results_arc.lock().unwrap();
            *results_display_lock = search_results_vec;
            info!(
                "[AMLL_Manager] AMLL 索引搜索完成，找到 {} 个结果。",
                results_display_lock.len()
            );
        }

        // 搜索完成后，将状态设置回 Idle，等待用户选择或进行下一次操作
        {
            let mut ttml_dl_state_lock = amll_ttml_download_state.lock().unwrap();
            *ttml_dl_state_lock = AmllTtmlDownloadState::Idle;
        }
    } else {
        warn!(
            "[AMLL_Manager] 调用 handle_amll_lyrics_search_or_download_async 参数不足，无法执行操作。"
        );
        // 确保状态不会卡住
        if let Ok(mut ttml_dl_state_lock) = amll_ttml_download_state.lock() {
            if matches!(
                *ttml_dl_state_lock,
                AmllTtmlDownloadState::DownloadingTtml | AmllTtmlDownloadState::SearchingIndex
            ) {
                *ttml_dl_state_lock = AmllTtmlDownloadState::Idle;
            }
        }
    }
}
