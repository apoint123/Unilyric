use std::future::IntoFuture;
use std::sync::{
    Arc,
    mpsc::{Receiver as StdReceiver, Sender as StdSender},
};
use std::time::Instant;

use easer::functions::Easing;
use tokio::sync::mpsc::error::TrySendError;
use tokio::{
    sync::{
        Mutex as TokioMutex,
        mpsc::{Sender as TokioSender, channel as tokio_channel},
    },
    task::{JoinHandle, LocalSet},
    time::{Duration as TokioDuration, timeout as tokio_timeout},
};
use windows::{
    Foundation::TypedEventHandler,
    Media::Control::{
        GlobalSystemMediaTransportControlsSession as MediaSession,
        GlobalSystemMediaTransportControlsSessionManager as MediaSessionManager,
        GlobalSystemMediaTransportControlsSessionMediaProperties,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus,
    },
    Storage::Streams::{
        Buffer, DataReader, IRandomAccessStreamReference, IRandomAccessStreamWithContentType,
        InputStreamOptions,
    },
    Win32::{
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
            Threading::GetCurrentThreadId,
        },
        UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG, TranslateMessage, WM_QUIT},
    },
    core::{Error as WinError, HSTRING, Result as WinResult},
};
use windows_future::IAsyncOperation;

use super::types::{
    ConnectorCommand, ConnectorUpdate, NowPlayingInfo, SharedPlayerState, SmtcControlCommand,
    SmtcSessionInfo,
};
use crate::{amll_connector::volume_control, utils::convert_traditional_to_simplified};

/// SMTC 异步操作的通用超时时长。
const SMTC_ASYNC_OPERATION_TIMEOUT: TokioDuration = TokioDuration::from_secs(5);
/// Windows API 操作被中止时返回的 HRESULT 错误码 (E_ABORT)。
const E_ABORT_HRESULT: windows::core::HRESULT = windows::core::HRESULT(0x80004004_u32 as i32);
/// 音量缓动动画的总时长（毫秒）。
const VOLUME_EASING_DURATION_MS: f32 = 250.0;
/// 音量缓动动画的总步数。
const VOLUME_EASING_STEPS: u32 = 15;
/// 音量变化小于此阈值时，不执行缓动动画。
const VOLUME_EASING_THRESHOLD: f32 = 0.01;
/// 允许获取的封面图片的最大字节数。
const MAX_COVER_SIZE_BYTES: usize = 20_971_520; // 20 MB

/// 将 Windows HSTRING 转换为 Rust String。
/// 如果 HSTRING 为空或无效，则返回空 String。
fn hstring_to_string(hstr: &HSTRING) -> String {
    if hstr.is_empty() {
        String::new()
    } else {
        hstr.to_string_lossy()
    }
}

/// 计算封面图片数据的哈希值 (u64)，用于高效地检测封面图片是否发生变化。
pub fn calculate_cover_hash(data: &[u8]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

/// 定义一个状态更新函数的类型别名。
/// 这是一个闭包，它接收一个 `SharedPlayerState` 的可变引用，并对其进行修改。
type StateUpdateFn = Box<dyn FnOnce(&mut SharedPlayerState) + Send>;

/// SMTC 内部事件信号，用于在事件处理器和主循环之间传递具体事件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmtcEventSignal {
    MediaProperties,
    PlaybackInfo,
    TimelineProperties,
    Sessions,
}

/// 封装了所有可能从后台异步任务返回的结果。
#[derive(Debug)]
enum AsyncTaskResult {
    ManagerReady(WinResult<MediaSessionManager>),
    MediaPropertiesReady(WinResult<GlobalSystemMediaTransportControlsSessionMediaProperties>),
    MediaControlCompleted(SmtcControlCommand, WinResult<bool>),
}

/// 封装了主事件循环中所有可变的状态。
struct SmtcState {
    manager: Option<MediaSessionManager>,
    manager_sessions_changed_token: Option<i64>,
    current_monitored_session: Option<MediaSession>,
    current_listener_tokens: Option<(i64, i64, i64)>,
    target_session_id: Option<String>,
    active_volume_easing_task: Option<JoinHandle<()>>,
    active_cover_fetch_task: Option<JoinHandle<()>>,
    next_easing_task_id: Arc<std::sync::atomic::AtomicU64>,
}

impl SmtcState {
    /// 创建一个新的 SmtcState 实例，并初始化所有字段。
    fn new() -> Self {
        Self {
            manager: None,
            manager_sessions_changed_token: None,
            current_monitored_session: None,
            current_listener_tokens: None,
            target_session_id: None,
            active_volume_easing_task: None,
            active_cover_fetch_task: None,
            next_easing_task_id: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }
}

// --- 异步任务处理器 ---

/// 一个通用的辅助函数，用于将返回 `IAsyncOperation` 的 WinRT API 调用派发到 Tokio 的本地任务中执行。
///
/// 它处理超时逻辑，并将最终结果（成功或失败）通过通道发送回主循环进行统一处理。
///
/// # 参数
/// * `async_op_result`: 一个 `WinResult`，其中应包含了要执行的 `IAsyncOperation`。
/// * `result_tx`: 用于发送异步任务结果的 Tokio 通道发送端。
/// * `result_mapper`: 一个闭包，用于将 `IAsyncOperation` 的最终结果 (`WinResult<T>`) 包装成 `AsyncTaskResult` 枚举。
fn spawn_async_op<T, F>(
    async_op_result: WinResult<IAsyncOperation<T>>,
    result_tx: &tokio::sync::mpsc::Sender<AsyncTaskResult>,
    result_mapper: F,
) where
    T: windows::core::RuntimeType + 'static,
    T::Default: 'static,
    IAsyncOperation<T>: IntoFuture<Output = WinResult<T>>,
    F: FnOnce(WinResult<T>) -> AsyncTaskResult + Send + 'static,
{
    if let Ok(async_op) = async_op_result {
        let tx = result_tx.clone();
        // 使用 spawn_local 至关重要，因为它不要求 Future 是 `Send`。
        // WinRT 的代理对象不是 `Send` 的，因此不能在多线程运行时中被 `.await`。
        tokio::task::spawn_local(async move {
            let result = tokio_timeout(SMTC_ASYNC_OPERATION_TIMEOUT, async_op.into_future()).await;
            let mapped_result = match result {
                Ok(res) => result_mapper(res), // 异步操作成功完成或返回了其内部错误
                Err(_) => {
                    log::warn!(
                        "[异步操作] WinRT 异步操作超时 (>{SMTC_ASYNC_OPERATION_TIMEOUT:?})。"
                    );
                    result_mapper(Err(WinError::from(E_ABORT_HRESULT))) // 封装成超时错误
                }
            };

            match tx.try_send(mapped_result) {
                Ok(_) => {} // 发送成功
                Err(TrySendError::Full(_)) => {
                    log::warn!("[异步操作] 无法将结果发送回主循环，业务繁忙，通道已满。");
                }
                Err(TrySendError::Closed(_)) => {
                    log::warn!("[异步操作] 无法将结果发送回主循环，通道已关闭，任务将提前中止。");
                }
            }
        });
    } else if let Err(e) = async_op_result {
        // 如果创建 IAsyncOperation 本身就失败了，直接将错误打包发送回去。
        log::warn!("[异步操作] 启动失败: {e:?}");
        if result_tx.try_send(result_mapper(Err(e))).is_err() {
            log::warn!("[异步操作] 启动失败，且无法将错误发送回主循环。");
        }
    }
}

/// 从 SMTC 会话中获取封面图片数据
async fn fetch_cover_data_async(
    thumb_ref: IRandomAccessStreamReference,
    state_update_tx: TokioSender<StateUpdateFn>,
) {
    log::trace!("[Cover Fetcher] 开始异步获取封面数据...");
    let cover_result: WinResult<Option<Vec<u8>>> = async {
        let stream_op: IAsyncOperation<IRandomAccessStreamWithContentType> =
            thumb_ref.OpenReadAsync()?;
        let stream = tokio_timeout(SMTC_ASYNC_OPERATION_TIMEOUT, stream_op.into_future())
            .await
            .map_err(|_| WinError::from(E_ABORT_HRESULT))??;

        if stream.Size()? == 0 {
            log::warn!("[Cover Fetcher] 媒体会话提供了缩略图引用，但流大小为0。");
            return Ok(None);
        }

        let buffer = Buffer::Create(stream.Size()? as u32)?;
        let read_op = stream.ReadAsync(&buffer, buffer.Capacity()?, InputStreamOptions::None)?;

        // 等待数据读取完成
        let bytes_buffer = tokio_timeout(SMTC_ASYNC_OPERATION_TIMEOUT, read_op.into_future())
            .await
            .map_err(|_| WinError::from(E_ABORT_HRESULT))??;

        let reader = DataReader::FromBuffer(&bytes_buffer)?;
        let mut bytes = vec![0u8; bytes_buffer.Length()? as usize];
        reader.ReadBytes(&mut bytes)?;
        Ok(Some(bytes))
    }
    .await;

    match cover_result {
        Ok(Some(bytes)) => {
            if bytes.len() > MAX_COVER_SIZE_BYTES {
                log::warn!(
                    "[Cover Fetcher] 获取到的封面数据 ({} 字节) 超出最大限制 ({} 字节)，已丢弃。",
                    bytes.len(),
                    MAX_COVER_SIZE_BYTES
                );
                return;
            }

            log::debug!(
                "[Cover Fetcher] 成功获取封面数据 ({} 字节)，将发送更新请求。",
                bytes.len()
            );
            let update_closure: StateUpdateFn = Box::new(move |state| {
                log::trace!("[State Update] 正在更新封面数据...");
                state.cover_data_hash = Some(calculate_cover_hash(&bytes));
                state.cover_data = Some(bytes);
            });
            match state_update_tx.try_send(update_closure) {
                Ok(_) => {} // 发送成功
                Err(TrySendError::Full(_)) => {
                    log::warn!("[Cover Fetcher] 状态更新通道已满，本次封面更新可能被丢弃。");
                }
                Err(TrySendError::Closed(_)) => {
                    log::warn!("[Cover Fetcher] 状态更新通道已关闭，无法发送封面数据。");
                }
            }
        }
        Ok(None) => {
            // 没有封面也算正常情况
        }
        Err(e) => {
            log::warn!("[Cover Fetcher] 异步获取封面数据失败: {e:?}");
        }
    }
}

/// 一个独立的任务，用于平滑地调整指定进程的音量
async fn volume_easing_task(
    task_id: u64,
    target_vol: f32,
    session_id: String,
    connector_tx: StdSender<ConnectorUpdate>,
) {
    log::debug!(
        "[音量缓动任务][ID:{task_id}] 启动。目标音量: {target_vol:.2}，会话: '{session_id}'"
    );

    if let Some(pid) = volume_control::get_pid_from_identifier(&session_id) {
        if let Ok(Ok((initial_vol, _))) =
            tokio::task::spawn_blocking(move || volume_control::get_process_volume_by_pid(pid))
                .await
        {
            if (target_vol - initial_vol).abs() < VOLUME_EASING_THRESHOLD {
                let _ = tokio::task::spawn_blocking(move || {
                    volume_control::set_process_volume_by_pid(pid, Some(target_vol), None)
                })
                .await;
                return;
            }

            log::trace!("[音量缓动任务][ID:{task_id}] 初始音量: {initial_vol:.2}");
            let animation_duration_ms = VOLUME_EASING_DURATION_MS;
            let steps = VOLUME_EASING_STEPS;
            let step_duration =
                TokioDuration::from_millis((animation_duration_ms / steps as f32) as u64);

            for s in 0..=steps {
                let current_time = (s as f32 / steps as f32) * animation_duration_ms;
                let change_in_vol = target_vol - initial_vol;
                let current_vol = easer::functions::Quad::ease_out(
                    current_time,
                    initial_vol,
                    change_in_vol,
                    animation_duration_ms,
                );

                let set_res = tokio::task::spawn_blocking(move || {
                    volume_control::set_process_volume_by_pid(pid, Some(current_vol), None)
                })
                .await;

                if set_res.is_err() || set_res.as_ref().unwrap().is_err() {
                    log::warn!("[音量缓动任务][ID:{task_id}] 设置音量失败，任务中止。");
                    break;
                }
                tokio::time::sleep(step_duration).await;
            }

            // 任务结束后，获取最终的音量状态并报告
            if let Ok(Ok((final_vol, final_mute))) =
                tokio::task::spawn_blocking(move || volume_control::get_process_volume_by_pid(pid))
                    .await
            {
                log::debug!("[音量缓动任务][ID:{task_id}] 完成。最终音量: {final_vol:.2}");
                let _ = connector_tx.send(ConnectorUpdate::AudioSessionVolumeChanged {
                    session_id,
                    volume: final_vol,
                    is_muted: final_mute,
                });
            }
        } else {
            log::warn!("[音量缓动任务][ID:{task_id}] 无法获取初始音量，任务中止。");
        }
    } else {
        log::warn!("[音量缓动任务][ID:{task_id}] 无法从会话ID '{session_id}' 获取PID，任务中止。");
    }
}

/// 处理 SMTC 会话列表发生变化的事件。
///
/// 这是 SMTC 监听器的核心逻辑之一。它负责：
/// 1. 获取所有可用的媒体会话列表并通知外部。
/// 2. 根据 `target_session_id`（用户选择）或默认规则（系统当前会话）来决定要监听哪个会话。
/// 3. 如果需要切换会话，它会注销旧会话的事件监听器，并为新会话注册新的监听器。
/// 4. 如果没有可用的会话，它会重置播放状态。
fn handle_sessions_changed(
    state: &mut SmtcState,
    connector_update_tx: &StdSender<ConnectorUpdate>,
    smtc_event_tx: &TokioSender<SmtcEventSignal>,
    state_update_tx: &TokioSender<StateUpdateFn>,
) -> WinResult<()> {
    log::debug!("[会话处理器] 开始处理会话变更...");

    // 从状态上下文中获取 SMTC 管理器，如果尚未初始化则直接返回。
    let manager = state
        .manager
        .as_ref()
        .ok_or_else(|| WinError::from(E_ABORT_HRESULT))?;

    // 1. 获取所有当前会话的信息
    let sessions_ivector = manager.GetSessions()?;
    let mut sessions_info_list = Vec::new();
    let mut session_candidates = Vec::new(); // 存储 (ID, MediaSession) 元组

    for s in sessions_ivector {
        if let Ok(id_hstr) = s.SourceAppUserModelId() {
            let id_str = hstring_to_string(&id_hstr);
            if id_str.is_empty() {
                log::trace!("[会话处理器] 发现一个没有有效ID的会话，已忽略。");
                continue;
            }
            log::trace!("[会话处理器] 发现会话: '{id_str}'");
            sessions_info_list.push(SmtcSessionInfo {
                source_app_user_model_id: id_str.clone(),
                session_id: id_str.clone(),
                display_name: id_str.split('!').next_back().unwrap_or(&id_str).to_string(),
            });
            session_candidates.push((id_str, s.clone()));
        }
    }
    // 将最新的会话列表发送出去
    if let Err(e) =
        connector_update_tx.send(ConnectorUpdate::SmtcSessionListChanged(sessions_info_list))
    {
        log::warn!("[会话处理器] 无法发送会话列表更新，主连接器可能已关闭: {e:?}");
    }

    // 2. 决定要监控哪个会话
    let new_session_to_monitor = if let Some(target_id) = state.target_session_id.as_ref() {
        log::debug!("[会话处理器] 正在寻找目标会话: '{target_id}'");
        if let Some((_, session)) = session_candidates
            .into_iter()
            .find(|(id, _)| id == target_id)
        {
            log::info!("[会话处理器] 已成功找到并选择目标会话: '{target_id}'");
            Some(session)
        } else {
            // 用户指定的目标会话消失了
            log::warn!("[会话处理器] 目标会话 '{target_id}' 已消失。");
            let _ = connector_update_tx.send(ConnectorUpdate::SelectedSmtcSessionVanished(
                target_id.clone(),
            ));
            state.target_session_id = None; // 清除目标，回到自动模式
            log::info!("[会话处理器] 已清除目标会话，回退到默认会话。");
            manager.GetCurrentSession().ok() // 尝试获取系统当前的会话作为备用
        }
    } else {
        log::debug!("[会话处理器] 处于自动模式，将使用默认会话。");
        manager.GetCurrentSession().ok() // 自动模式，获取系统当前会话
    };

    let new_session_id = new_session_to_monitor
        .as_ref()
        .and_then(|s| s.SourceAppUserModelId().ok())
        .map(|h| h.to_string_lossy());

    let current_session_id = state
        .current_monitored_session
        .as_ref()
        .and_then(|s| s.SourceAppUserModelId().ok())
        .map(|h| h.to_string_lossy());

    // 3. 检查是否需要切换会话
    if new_session_id != current_session_id {
        log::info!(
            "[会话处理器] 检测到会话切换: 从 {:?} -> 到 {:?}",
            current_session_id.as_deref().unwrap_or("无"),
            new_session_id.as_deref().unwrap_or("无")
        );

        // 3a. 注销旧会话的监听器
        if let Some(old_s) = state.current_monitored_session.take()
            && let Some(tokens) = state.current_listener_tokens.take()
        {
            log::debug!("[会话处理器] 正在注销旧会话的事件监听器...");
            let _ = old_s
                .RemoveMediaPropertiesChanged(tokens.0)
                .map_err(|e| log::warn!("注销 MediaPropertiesChanged 失败: {e}"));
            let _ = old_s
                .RemovePlaybackInfoChanged(tokens.1)
                .map_err(|e| log::warn!("注销 PlaybackInfoChanged 失败: {e}"));
            let _ = old_s
                .RemoveTimelinePropertiesChanged(tokens.2)
                .map_err(|e| log::warn!("注销 TimelinePropertiesChanged 失败: {e}"));
        }

        // 3b. 为新会话设置监听器
        if let Some(new_s) = new_session_to_monitor {
            log::debug!("[会话处理器] 正在为新会话注册事件监听器...");
            let tx_media = smtc_event_tx.clone();
            let tx_playback = smtc_event_tx.clone();
            let tx_timeline = smtc_event_tx.clone();

            let tokens = (
                new_s.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    log::trace!("[Event] MediaPropertiesChanged 信号触发");
                    let _ = tx_media.try_send(SmtcEventSignal::MediaProperties);
                    Ok(())
                }))?,
                new_s.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                    log::trace!("[Event] PlaybackInfoChanged 信号触发");
                    let _ = tx_playback.try_send(SmtcEventSignal::PlaybackInfo);
                    Ok(())
                }))?,
                new_s.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    log::trace!("[Event] TimelinePropertiesChanged 信号触发");
                    let _ = tx_timeline.try_send(SmtcEventSignal::TimelineProperties);
                    Ok(())
                }))?,
            );

            state.current_listener_tokens = Some(tokens);
            state.current_monitored_session = Some(new_s);

            // 切换后立即获取一次全量信息，确保UI快速更新
            log::info!("[会话处理器] 会话切换完成，立即获取初始状态。");
            let _ = smtc_event_tx.try_send(SmtcEventSignal::MediaProperties);
            let _ = smtc_event_tx.try_send(SmtcEventSignal::PlaybackInfo);
            let _ = smtc_event_tx.try_send(SmtcEventSignal::TimelineProperties);
        } else {
            // 3c. 没有活动会话了，重置状态
            log::info!("[会话处理器] 没有可用的媒体会话，将重置播放器状态。");
            let _ = state_update_tx.try_send(Box::new(|state| state.reset_to_empty()));
        }
    } else {
        log::debug!(
            "[会话处理器] 无需切换会话，目标与当前一致 ({:?})。",
            current_session_id.as_deref().unwrap_or("无")
        );
    }

    Ok(())
}

// --- 主监听器函数 ---

/// 运行 SMTC 监听器的主函数。
///
/// # 参数
/// * `connector_update_tx` - 用于向外部发送状态更新的通道。
/// * `control_rx` - 用于从外部接收控制命令的通道。
/// * `player_state_arc` - 指向共享播放器状态的原子引用计数指针。
/// * `shutdown_rx` - 用于接收关闭信号的通道。
pub fn run_smtc_listener(
    connector_update_tx: StdSender<ConnectorUpdate>,
    control_rx: StdReceiver<ConnectorCommand>,
    player_state_arc: Arc<TokioMutex<SharedPlayerState>>,
    shutdown_rx: StdReceiver<()>,
) -> Result<(), String> {
    log::info!("[SMTC Handler] 正在启动 SMTC 监听器");

    // 1. 初始化 Tokio 单线程运行时
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("创建 Tokio 运行时失败: {e}"))?;
    let local_set = LocalSet::new();

    let (thread_id_tx, thread_id_rx) = std::sync::mpsc::channel::<u32>();

    // 2. 将所有逻辑包装在 local_set.block_on 中，确保它们运行在同一个 STA 线程上
    local_set.block_on(&rt, async move {
        // 使用 RAII Guard 确保 COM 在线程退出时被正确反初始化
        struct ComGuard;
        impl ComGuard {
            fn new() -> WinResult<Self> {
                // SAFETY: CoInitializeEx 必须在每个使用 COM/WinRT 的线程上调用一次。
                // 我们在此处初始化 STA，这是使用大多数 UI 和事件相关的 WinRT API 所必需的。
                // 这个调用是安全的，因为它在此线程的开始处被调用，且只调用一次。
                unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
                Ok(Self)
            }
        }
        impl Drop for ComGuard {
            fn drop(&mut self) {
                // SAFETY: CoUninitialize 必须与 CoInitializeEx 成对出现。
                // 当 ComGuard 离开作用域时（即 async 块结束时），它会自动被调用，确保资源被正确释放。
                unsafe { CoUninitialize() };
                log::trace!("[COM Guard] CoUninitialize 已调用。");
            }
        }

        // 初始化 COM，如果失败则无法继续
        if let Err(e) = ComGuard::new() {
            log::error!("[SMTC Handler] COM 初始化失败 (STA): {e}，监听器线程无法启动。");
            return;
        }
        log::trace!("[SMTC Handler] COM (STA) 初始化成功。");


        // 3. 在后台线程中启动一个标准的 Win32 消息泵。
        // 这是接收 WinRT 事件所必需的。
        let message_pump_handle = tokio::task::spawn_blocking(move || {
            // SAFETY: GetCurrentThreadId 是安全的
            let thread_id = unsafe { GetCurrentThreadId() };
            // 将ID发送回主任务，如果失败则直接 panic，因为这是启动的关键步骤
            if thread_id_tx.send(thread_id).is_err() {
                log::error!("[消息泵] 无法发送线程ID，启动失败！");
                return;
            }

            log::trace!("[消息泵线程] 启动...");
            // SAFETY: 这是标准的 Win32 消息循环。
            // GetMessageW 会在没有消息时阻塞线程，将 CPU 使用率降至 0。
            // 当它收到 PostThreadMessageW 发送的 WM_QUIT 消息时，会返回 0，循环结束。
            // 它是线程安全的，因为它只处理属于这个线程的消息队列。
            unsafe {
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            log::trace!("[消息泵线程] 收到 WM_QUIT，已退出。");
        });

        let pump_thread_id = match thread_id_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(id) => id,
            Err(_) => {
                log::error!("[SMTC 主循环] 等待消息泵线程ID超时，启动失败！");
                return;
            }
        };
        log::debug!("[SMTC 主循环] 已成功获取消息泵线程ID: {pump_thread_id}");

        // 4. 设置所有异步通信通道
        let (smtc_event_tx, mut smtc_event_rx) = tokio_channel::<SmtcEventSignal>(32);
        let (async_result_tx, mut async_result_rx) = tokio_channel::<AsyncTaskResult>(32);
        let (state_update_tx, mut state_update_rx) = tokio_channel::<StateUpdateFn>(32);

        // 5. 启动状态管理器 Actor
        let state_manager_handle = {
            let player_state_clone = player_state_arc;
            let connector_update_tx_clone = connector_update_tx.clone();
            tokio::task::spawn_local(async move {
                log::info!("[状态管理器 Actor] 任务已启动。");
                while let Some(update_fn) = state_update_rx.recv().await {
                    log::trace!("[状态管理器 Actor] 收到状态更新请求。");
                    let state_for_update;
                    {
                        let mut state_guard = player_state_clone.lock().await;
                        update_fn(&mut state_guard);
                        state_for_update = (*state_guard).clone();
                    }
                    if connector_update_tx_clone.send(ConnectorUpdate::NowPlayingTrackChanged(NowPlayingInfo::from(&state_for_update))).is_err() {
                        log::warn!("[状态管理器 Actor] 无法发送状态更新通知，主连接器可能已关闭。");
                        break; // 通道关闭，退出 Actor
                    }
                }
                log::warn!("[状态管理器 Actor] 状态更新通道已关闭，任务退出。");
            })
        };

        // 6. 桥接同步通道到异步世界
        // 外部传入的是 std::sync::mpsc，我们需要在阻塞任务中将其消息转发到 tokio channel，
        // 以便在主循环的 `select!` 中使用。
        let (control_tx, mut control_tokio_rx) = tokio_channel::<ConnectorCommand>(32);
        tokio::task::spawn_blocking(move || {
            while let Ok(cmd) = control_rx.recv() {
                if control_tx.blocking_send(cmd).is_err() {
                    log::info!("[控制桥接] 目标通道已关闭，停止转发。");
                    break;
                }
            }
        });
        let (shutdown_tx, mut shutdown_tokio_rx) = tokio_channel::<()>(1);
        tokio::task::spawn_blocking(move || {
            let _ = shutdown_rx.recv();
            let _ = shutdown_tx.blocking_send(());
        });

        // 7. 初始化主循环状态和 SMTC 管理器
        let mut state = SmtcState::new();

        // 异步请求 SMTC 管理器，这是所有操作的起点
        log::debug!("[SMTC 主循环] 正在异步请求 SMTC 管理器...");
        spawn_async_op(
            MediaSessionManager::RequestAsync(),
            &async_result_tx,
            AsyncTaskResult::ManagerReady,
        );

        log::info!("[SMTC 主循环] 进入 select! 事件循环...");

        // 8. 核心事件循环
        loop {
            tokio::select! {
                biased; // 优先处理关闭信号

                // 分支 1: 处理关闭信号
                _ = shutdown_tokio_rx.recv() => {
                    log::info!("[SMTC 主循环] 收到关闭信号，准备退出...");
                    break;
                }

                // 分支 2: 处理来自外部的控制命令
                Some(command) = control_tokio_rx.recv() => {
                    log::debug!("[SMTC 主循环] 收到外部命令: {command:?}");
                    match command {
                        ConnectorCommand::SelectSmtcSession(id) => {
                            let new_target = if id.is_empty() { None } else { Some(id) };
                            log::info!("[SMTC 主循环] 切换目标会话 -> {new_target:?}");
                            state.target_session_id = new_target;
                            // 立即触发一次会话变更检查，以应用新的选择
                            match smtc_event_tx.try_send(SmtcEventSignal::Sessions) {
                                Ok(_) => {} // 发送成功
                                Err(TrySendError::Full(_)) => {
                                    log::warn!("[SMTC 主循环] 尝试触发会话变更检查失败，事件通道已满。");
                                }
                                Err(TrySendError::Closed(_)) => {
                                    log::warn!("[SMTC 主循环] 尝试触发会话变更检查失败，事件通道已关闭。");
                                }
                            }
                        }
                        ConnectorCommand::MediaControl(media_cmd) => {
                            if let Some(session) = &state.current_monitored_session {
                                log::debug!("[SMTC 主循环] 正在执行媒体控制指令: {media_cmd:?}");
                                match media_cmd {
                                    // 音量设置是特殊情况，因为它涉及一个本地的缓动任务
                                    SmtcControlCommand::SetVolume(level) => {
                                        if let Ok(id_hstr) = session.SourceAppUserModelId() {
                                            let session_id_str = hstring_to_string(&id_hstr);
                                            if session_id_str.is_empty() {
                                                log::warn!("[SMTC 主循环] 无法为当前会话设置音量，因为它没有有效的 SourceAppUserModelId。");
                                            } else {
                                                // 如果有正在运行的音量缓动任务，先取消它
                                                if let Some(old_task) = state.active_volume_easing_task.take() {
                                                    log::trace!("[SMTC 主循环] 取消了正在进行的音量缓动任务。");
                                                    old_task.abort();
                                                }
                                                // 创建并启动一个新的音量缓动任务
                                                let task_id = state.next_easing_task_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                                state.active_volume_easing_task = Some(tokio::task::spawn_local(
                                                    volume_easing_task(
                                                        task_id,
                                                        level,
                                                        session_id_str,
                                                        connector_update_tx.clone()
                                                    )
                                                ));
                                            }
                                        } else {
                                            log::warn!("[SMTC 主循环] 无法为当前会话设置音量，获取 SourceAppUserModelId 失败。");
                                        }
                                    }
                                    // 其他媒体控制命令直接调用 SMTC API
                                    other_cmd => {
                                        let async_op = match other_cmd {
                                            SmtcControlCommand::Play => session.TryPlayAsync(),
                                            SmtcControlCommand::Pause => session.TryPauseAsync(),
                                            SmtcControlCommand::SkipNext => session.TrySkipNextAsync(),
                                            SmtcControlCommand::SkipPrevious => session.TrySkipPreviousAsync(),
                                            SmtcControlCommand::SeekTo(pos) => session.TryChangePlaybackPositionAsync(pos as i64 * 10000), // SMTC 使用 100ns 单位
                                            _ => unreachable!(), // SetVolume 已在上面处理
                                        };
                                        spawn_async_op(async_op, &async_result_tx, move |res| {
                                            AsyncTaskResult::MediaControlCompleted(other_cmd, res)
                                        });
                                    }
                                }
                            } else {
                                log::warn!("[SMTC 主循环] 收到媒体控制指令 {media_cmd:?}，但当前没有活动的 SMTC 会话，已忽略。");
                            }
                        }
                        // 对于所有其他不相关的命令，直接忽略
                        _ => {
                            log::trace!("[SMTC 主循环] 收到与 SMTC无关的指令 {command:?}，已忽略。");
                        }
                    }
                }

                // 分支 3: 处理内部的 WinRT 事件信号
                Some(signal) = smtc_event_rx.recv() => {
                    log::trace!("[SMTC 主循环] 收到内部事件信号: {signal:?}");
                    match signal {
                        SmtcEventSignal::Sessions => {
                            if state.manager.is_some()
                                && let Err(e) = handle_sessions_changed(
                                    &mut state,
                                    &connector_update_tx,
                                    &smtc_event_tx,
                                    &state_update_tx,
                                ) {
                                    log::error!("[SMTC 主循环] 处理会话变更时出错: {e}");
                                }
                        }
                        SmtcEventSignal::MediaProperties => {
                            if let Some(s) = &state.current_monitored_session {
                                spawn_async_op(
                                    s.TryGetMediaPropertiesAsync(),
                                    &async_result_tx,
                                    AsyncTaskResult::MediaPropertiesReady,
                                );
                            }
                        }
                        SmtcEventSignal::PlaybackInfo => {
                            if let Some(s) = &state.current_monitored_session {
                                match s.GetPlaybackInfo() {
                                    Ok(info) => {
                                        let is_playing_now = info.PlaybackStatus().map_or_else(
                                            |e| {
                                                log::warn!("[State Update] 获取 PlaybackStatus 失败: {e:?}, 默认为 Paused");
                                                false
                                            },
                                            |status| status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing,
                                        );

                                        let update_fn = Box::new(move |state: &mut SharedPlayerState| {
                                            if state.is_playing != is_playing_now {
                                                log::trace!("[State Update] 播放状态改变: {} -> {}", state.is_playing, is_playing_now);
                                                let estimated_pos = state.get_estimated_current_position_ms();
                                                state.last_known_position_ms = estimated_pos;
                                                state.last_known_position_report_time = Some(Instant::now());
                                            }
                                            state.is_playing = is_playing_now;

                                            if let Ok(c) = info.Controls() {
                                                state.can_pause = c.IsPauseEnabled().unwrap_or(false);
                                                state.can_play = c.IsPlayEnabled().unwrap_or(false);
                                                state.can_skip_next = c.IsNextEnabled().unwrap_or(false);
                                                state.can_skip_previous = c.IsPreviousEnabled().unwrap_or(false);
                                                state.can_seek = c.IsPlaybackPositionEnabled().unwrap_or(false);
                                            } else {
                                                log::warn!("[State Update] 获取媒体控件 (Controls) 失败。");
                                            }
                                        });

                                        match state_update_tx.try_send(update_fn) {
                                            Ok(_) => {} // 发送成功
                                            Err(TrySendError::Full(_)) => {
                                                log::warn!("[SMTC 主循环] 状态更新通道已满，丢弃了一次 PlaybackInfo 更新。");
                                            }
                                            Err(TrySendError::Closed(_)) => {
                                                log::error!("[SMTC 主循环] 状态更新通道已关闭，无法发送 PlaybackInfo 更新。");
                                            }
                                        }
                                    },
                                    Err(e) => log::warn!("[SMTC 主循环] 获取 PlaybackInfo 失败: {e:?}")
                                }
                            }
                        }
                        SmtcEventSignal::TimelineProperties => {
                            if let Some(s) = &state.current_monitored_session {
                                match s.GetTimelineProperties() {
                                    Ok(props) => {
                                        let pos_ms = props.Position().map_or(0, |d| (d.Duration / 10000) as u64);
                                        let dur_ms = props.EndTime().map_or(0, |d| (d.Duration / 10000) as u64);

                                        let update_fn = Box::new(move |state: &mut SharedPlayerState| {
                                            state.last_known_position_ms = pos_ms;
                                            if dur_ms > 0 { state.song_duration_ms = dur_ms; }
                                            state.last_known_position_report_time = Some(Instant::now());
                                        });

                                        match state_update_tx.try_send(update_fn) {
                                            Ok(_) => {} // Success
                                            Err(TrySendError::Full(_)) => {
                                                log::warn!("[SMTC 主循环] 状态更新通道已满，丢弃了一次 TimelineProperties 更新。");
                                            }
                                            Err(TrySendError::Closed(_)) => {
                                                log::error!("[SMTC 主循环] 状态更新通道已关闭，无法发送 TimelineProperties 更新。");
                                            }
                                        }
                                    },
                                    Err(e) => log::warn!("[SMTC 主循环] 获取 TimelineProperties 失败: {e:?}")
                                }
                            }
                        }
                    }
                }

                // 分支 4: 处理后台异步任务的结果
                Some(result) = async_result_rx.recv() => {
                    log::trace!("[SMTC 主循环] 收到异步任务结果。");
                    match result {
                        AsyncTaskResult::ManagerReady(Ok(mgr)) => {
                            log::info!("[SMTC 主循环] SMTC 管理器已就绪。");
                            let tx = smtc_event_tx.clone();
                            state.manager_sessions_changed_token = mgr.SessionsChanged(&TypedEventHandler::new(move |_, _| {
                                log::trace!("[Event] SessionsChanged 信号触发");
                                let _ = tx.try_send(SmtcEventSignal::Sessions);
                                Ok(())
                             })).ok();
                            state.manager = Some(mgr);
                            // 管理器就绪后，立即触发一次会话检查
                            match smtc_event_tx.try_send(SmtcEventSignal::Sessions) {
                                Ok(_) => {} // 发送成功
                                Err(TrySendError::Full(_)) => {
                                    log::warn!("[SMTC 主循环] 准备触发初始会话检查时发现事件通道已满。");
                                }
                                Err(TrySendError::Closed(_)) => {
                                    log::error!("[SMTC 主循环] 准备触发初始会话检查时发现事件通道已关闭。");
                                }
                            }
                        }
                        AsyncTaskResult::ManagerReady(Err(e)) => {
                            log::error!("[SMTC 主循环] 初始化 SMTC 管理器失败: {e:?}，监听器将关闭。");
                            break; // 致命错误，退出循环
                        }
                        AsyncTaskResult::MediaPropertiesReady(Ok(props)) => {
                            let get_prop_string = |prop_res: WinResult<HSTRING>, name: &str| {
                                prop_res.map_or_else(
                                    |e| {
                                        log::warn!("[SMTC 主循环] 获取媒体属性 '{name}' 失败: {e:?}");
                                        String::new()
                                    },
                                    |hstr| convert_traditional_to_simplified(&hstring_to_string(&hstr))
                                )
                            };

                            let title = get_prop_string(props.Title(), "Title");
                            let artist = get_prop_string(props.Artist(), "Artist");
                            let album = get_prop_string(props.AlbumTitle(), "AlbumTitle");

                            // 更新文本信息
                            let update_fn = Box::new(move |state: &mut SharedPlayerState| { state.title = title; state.artist = artist; state.album = album; });
                            match state_update_tx.try_send(update_fn) {
                                Ok(_) => {} // 发送成功
                                Err(TrySendError::Full(_)) => {
                                    log::warn!("[SMTC 主循环] 状态更新通道已满，丢弃了一次文本信息更新。");
                                }
                                Err(TrySendError::Closed(_)) => {
                                    log::error!("[SMTC 主循环] 状态更新通道已关闭，无法发送文本信息更新。");
                                }
                            }

                            // 1. 如果有旧的封面获取任务正在运行，先取消它
                            if let Some(old_handle) = state.active_cover_fetch_task.take() {
                                log::trace!("[SMTC 主循环] 检测到新歌曲，取消旧的封面获取任务...");
                                old_handle.abort();
                            }

                            // 2. 启动新的封面获取任务
                            if let Ok(thumb_ref) = props.Thumbnail() {
                                let new_handle = tokio::task::spawn_local(fetch_cover_data_async(thumb_ref, state_update_tx.clone()));
                                state.active_cover_fetch_task = Some(new_handle);
                            } else {
                                // 如果没有缩略图引用，清空封面
                                let update_fn = Box::new(|state: &mut SharedPlayerState| { state.cover_data = None; state.cover_data_hash = None; });
                                match state_update_tx.try_send(update_fn) {
                                    Ok(_) => {} // 发送成功
                                    Err(TrySendError::Full(_)) => {
                                        log::warn!("[SMTC 主循环] 状态更新通道已满，丢弃了一次清空封面更新。");
                                    }
                                    Err(TrySendError::Closed(_)) => {
                                        log::error!("[SMTC 主循环] 状态更新通道已关闭，无法发送清空封面更新。");
                                    }
                                }

                            }
                        }
                        AsyncTaskResult::MediaPropertiesReady(Err(e)) => {
                            log::warn!("[SMTC 主循环] 获取媒体属性失败: {e:?}");
                        }
                        AsyncTaskResult::MediaControlCompleted(cmd, res) => {
                            match res {
                                Ok(true) => log::info!("[SMTC 主循环] 媒体控制指令 {cmd:?} 成功执行。"),
                                Ok(false) => log::warn!("[SMTC 主循环] 媒体控制指令 {cmd:?} 执行失败 (返回 false)。"),
                                Err(e) => log::warn!("[SMTC 主循环] 媒体控制指令 {cmd:?} 异步调用失败: {e:?}"),
                            }
                        }
                    }
                }
            }
        }

        // --- 优雅关闭 ---
        log::info!("[SMTC 主循环] 正在清理资源...");

        // 注销所有事件监听器
        if let Some(mgr) = state.manager
            && let Some(token) = state.manager_sessions_changed_token {
                let _ = mgr.RemoveSessionsChanged(token);
            }
        if let Some(session) = state.current_monitored_session
            && let Some(tokens) = state.current_listener_tokens {
                let _ = session.RemoveMediaPropertiesChanged(tokens.0);
                let _ = session.RemovePlaybackInfoChanged(tokens.1);
                let _ = session.RemoveTimelinePropertiesChanged(tokens.2);
            }
        // 中断正在运行的任务
        if let Some(task) = state.active_volume_easing_task {
            task.abort();
        }
        if let Some(task) = state.active_cover_fetch_task.take() {
             task.abort();
        }
        state_manager_handle.abort();

        // 发送 WM_QUIT 消息来停止消息泵线程
        // SAFETY: PostThreadMessageW 是向特定线程发送消息的标准 Win32 API。
        // 我们在启动时获取了正确的线程ID `main_thread_id`，所以这是安全的。
        // 这是从外部线程安全地与消息循环交互的推荐方式。
        unsafe {
            use windows::Win32::Foundation::{LPARAM, WPARAM};
            use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

            if PostThreadMessageW(pump_thread_id, WM_QUIT, WPARAM(0), LPARAM(0)).is_err() {
                log::error!("[SMTC 清理] 发送 WM_QUIT 消息到消息泵线程 (ID:{pump_thread_id}) 失败。");
            }
        }
        if let Err(e) = message_pump_handle.await {
            log::warn!("[SMTC 清理] 等待消息泵线程退出时出错: {e:?}");
        }
    });

    log::info!("[SMTC Handler] 监听器线程已完全退出。");
    Ok(())
}
