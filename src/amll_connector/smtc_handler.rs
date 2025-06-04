use std::future::IntoFuture;
use std::sync::Arc;
use std::sync::mpsc::{
    Receiver as StdReceiver, Sender as StdSender, TryRecvError as StdTryRecvError,
};
use std::time::Instant;

use tokio::sync::Mutex as TokioMutex;
use tokio::task::JoinHandle;
use tokio::time::{Duration as TokioDuration, sleep as tokio_sleep, timeout as tokio_timeout};

use windows::{
    Foundation::TypedEventHandler,
    Media::Control::{
        GlobalSystemMediaTransportControlsSession as MediaSession,
        GlobalSystemMediaTransportControlsSessionManager as MediaSessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackControls as PlaybackControls,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus,
    },
    Storage::Streams::{Buffer, DataReader, IRandomAccessStreamReference, InputStreamOptions},
    Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
    core::{Error as WinError, HSTRING, Result as WinResult},
};

use easer::functions::{Easing, Quad};
use windows_future::IAsyncOperation;

use super::types::{
    ConnectorCommand, ConnectorUpdate, NowPlayingInfo, SharedPlayerState, SmtcControlCommand,
    SmtcSessionInfo, WebsocketStatus,
};
use crate::amll_connector::volume_control;

/// SMTC 异步操作的超时时长
const SMTC_ASYNC_OPERATION_TIMEOUT: TokioDuration = TokioDuration::from_secs(5);
/// Windows API 操作被中止时返回的 HRESULT 错误码 (E_ABORT)
const E_ABORT_HRESULT: windows::core::HRESULT = windows::core::HRESULT(0x80004004_u32 as i32);

/// 将 Windows HSTRING 转换为 Rust String。
/// 如果 HSTRING 为空，则返回空 String。
fn hstring_to_string(hstr: &HSTRING) -> String {
    if hstr.is_empty() {
        String::new()
    } else {
        hstr.to_string_lossy() // to_string_lossy 会在转换无效 UTF-16 时替换为 U+FFFD
    }
}

/// 计算封面图片数据的哈希值 (u64)。
/// 用于检测封面图片是否发生变化。
pub fn calculate_cover_hash(data: &[u8]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

/// 更新共享播放器状态中的播放控制能力 (如是否可以播放、暂停等)。
fn update_playback_controls_state(
    player_state: &mut SharedPlayerState, // 可变的共享播放器状态引用
    controls_opt: Option<&PlaybackControls>, // 可选的 SMTC 播放控制接口引用
) {
    if let Some(controls) = controls_opt {
        // 从 SMTC 获取各项控制的可用状态，并更新到共享状态中
        // unwrap_or(false) 用于在获取失败时提供一个默认值
        player_state.can_pause = controls.IsPauseEnabled().unwrap_or(false);
        player_state.can_play = controls.IsPlayEnabled().unwrap_or(false);
        player_state.can_skip_next = controls.IsNextEnabled().unwrap_or(false);
        player_state.can_skip_previous = controls.IsPreviousEnabled().unwrap_or(false);
        player_state.can_seek = controls.IsPlaybackPositionEnabled().unwrap_or(false);
    } else {
        // 如果没有提供 PlaybackControls，则将所有控制能力设置为 false
        player_state.can_pause = false;
        player_state.can_play = false;
        player_state.can_skip_next = false;
        player_state.can_skip_previous = false;
        player_state.can_seek = false;
    }
}

/// 异步获取封面图片数据。
///
/// # 参数
/// * `thumbnail_ref`: 指向封面缩略图的 `IRandomAccessStreamReference`。
///
/// # 返回
/// * `WinResult<Option<Vec<u8>>>`: 成功时返回包含可选封面数据字节的 `Ok`，失败时返回 `Err`。
///   如果流为空或读取失败，`Option` 为 `None`。
async fn get_cover_data(
    thumbnail_ref: &IRandomAccessStreamReference,
) -> WinResult<Option<Vec<u8>>> {
    use windows::Storage::Streams::IRandomAccessStreamWithContentType;

    // 异步打开缩略图流
    let open_stream_operation: IAsyncOperation<IRandomAccessStreamWithContentType> =
        thumbnail_ref.OpenReadAsync()?;

    // 为打开流的操作设置超时
    match tokio_timeout(
        SMTC_ASYNC_OPERATION_TIMEOUT, // 使用定义的超时常量
        open_stream_operation, // Windows IAsyncOperation 实现了 IntoFuture
    )
    .await // 等待异步操作或超时
    {
        Ok(Ok(stream)) => { // 异步操作成功完成，且内部结果也为 Ok
            if stream.Size()? > 0 { // 检查流的大小是否大于0
                let size = stream.Size()? as u32;
                let buffer = Buffer::Create(size)?; // 创建一个足够大的缓冲区
                // 异步读取流数据到缓冲区
                let read_operation = stream.ReadAsync(&buffer, size, InputStreamOptions::None)?;
                // 为读取操作设置超时
                match tokio_timeout(SMTC_ASYNC_OPERATION_TIMEOUT, read_operation.into_future())
                    .await
                {
                    Ok(Ok(bytes_buffer_read)) => { // 读取成功
                        let data_reader = DataReader::FromBuffer(&bytes_buffer_read)?; // 从缓冲区创建 DataReader
                        let mut bytes = vec![0u8; bytes_buffer_read.Length()? as usize]; // 创建字节数组
                        data_reader.ReadBytes(&mut bytes)?; // 将数据读取到字节数组
                        Ok(Some(bytes)) // 返回包含数据的 Option
                    }
                    Ok(Err(e)) => { // 读取操作的异步部分失败
                        log::error!("[SMTC Handler] 读取封面数据时出错 (ReadAsync): {:?}", e);
                        Err(e)
                    }
                    Err(_) => { // 读取操作超时
                        log::warn!("[SMTC Handler] 读取封面数据超时 (ReadAsync)。");
                        Err(WinError::from(E_ABORT_HRESULT)) // 返回中止错误码
                    }
                }
            } else { // 流大小为0，表示没有封面数据
                log::trace!("[SMTC Handler] 封面图片流为空。");
                Ok(None)
            }
        }
        Ok(Err(e)) => { // 打开流的异步操作本身失败
            log::error!("[SMTC Handler] 打开封面时出错 (OpenReadAsync): {:?}", e);
            Err(e)
        }
        Err(_) => { // 打开流的操作超时
            log::warn!("[SMTC Handler] 打开封面超时 (OpenReadAsync)。");
            Err(WinError::from(E_ABORT_HRESULT))
        }
    }
}

/// 定义 SMTC 事件信号枚举，用于在 Tokio 异步任务之间传递内部事件类型。
/// 这些信号表明 SMTC 的某些属性已发生变化，需要重新查询和处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmtcEventSignal {
    MediaProperties,    // 媒体属性（如歌曲信息、封面）已更改
    PlaybackInfo,       // 播放信息（如播放状态、可用的控制操作）已更改
    TimelineProperties, // 进度条属性（如播放进度、总时长）已更改
    Sessions,           // SMTC 会话列表（如活动的媒体播放器应用）已更改
}

/// 处理媒体属性更改事件 (`MediaPropertiesChanged`)。
/// 获取当前会话的歌曲标题、艺术家、专辑和封面图片，并更新共享播放器状态和 `NowPlayingInfo`。
/// 然后通过 `connector_update_tx` 发送 `NowPlayingTrackChanged` 更新。
async fn process_media_properties(
    session: &MediaSession,
    connector_update_tx: &StdSender<ConnectorUpdate>,
    player_state_arc: &Arc<TokioMutex<SharedPlayerState>>,
) -> WinResult<()> {
    let report_time_for_this_update = Instant::now();
    let media_props_op = session.TryGetMediaPropertiesAsync()?;
    let media_props =
        match tokio_timeout(SMTC_ASYNC_OPERATION_TIMEOUT, media_props_op.into_future()).await {
            Ok(Ok(props)) => props,
            Ok(Err(e)) => {
                log::error!("[SMTC Handler] 获取媒体属性时出错: {:?}", e);
                return Err(e);
            }
            Err(_) => {
                log::warn!("[SMTC Handler] 获取媒体属性超时。");
                return Err(WinError::from(E_ABORT_HRESULT));
            }
        };

    let original_title = hstring_to_string(&media_props.Title()?);
    let original_artist = hstring_to_string(&media_props.Artist()?);
    let original_album_title = hstring_to_string(&media_props.AlbumTitle()?);

    // 进行简繁转换
    let title = crate::utils::convert_traditional_to_simplified(&original_title);
    let artist = crate::utils::convert_traditional_to_simplified(&original_artist);
    let album_title = crate::utils::convert_traditional_to_simplified(&original_album_title);

    if original_title != title {
        log::info!(
            "[SMTC Handler] 标题繁转简: '{}' -> '{}'",
            original_title,
            title
        );
    }
    if original_artist != artist {
        log::info!(
            "[SMTC Handler] 艺术家繁转简: '{}' -> '{}'",
            original_artist,
            artist
        );
    }
    if original_album_title != album_title {
        log::info!(
            "[SMTC Handler] 专辑繁转简: '{}' -> '{}'",
            original_album_title,
            album_title
        );
    }

    let mut fetched_cover_data_bytes: Option<Vec<u8>> = None;
    let mut new_cover_data_hash: Option<u64> = None;

    if let Ok(thumbnail_ref) = media_props.Thumbnail() {
        if let Ok(Some(bytes)) = get_cover_data(&thumbnail_ref).await {
            if !bytes.is_empty() {
                new_cover_data_hash = Some(calculate_cover_hash(&bytes));
                fetched_cover_data_bytes = Some(bytes);
            }
        }
    }

    let info_to_send: NowPlayingInfo;
    {
        let mut state = player_state_arc.lock().await;
        // 存储简体版本
        state.title = title.clone(); // 使用转换后的 title
        state.artist = artist.clone(); // 使用转换后的 artist
        state.album = album_title.clone(); // 使用转换后的 album_title
        state.cover_data = fetched_cover_data_bytes.clone();
        state.cover_data_hash = new_cover_data_hash;

        if state.last_known_position_report_time.is_some() {
            state.last_known_position_report_time = Some(report_time_for_this_update);
        }

        info_to_send = NowPlayingInfo {
            title: Some(state.title.clone()),       // 发送简体
            artist: Some(state.artist.clone()),     // 发送简体
            album_title: Some(state.album.clone()), // 发送简体
            is_playing: Some(state.is_playing),
            duration_ms: Some(state.song_duration_ms),
            position_ms: Some(state.last_known_position_ms),
            position_report_time: state.last_known_position_report_time,
            cover_data: state.cover_data.clone(),
            cover_data_hash: state.cover_data_hash,
        };
    }

    if connector_update_tx
        .send(ConnectorUpdate::NowPlayingTrackChanged(info_to_send))
        .is_err()
    {
        log::error!("[SMTC Handler] 发送“歌曲已更改”更新失败 (原因: 媒体属性变更)。");
    }
    Ok(())
}

/// 处理播放信息更改事件 (`PlaybackInfoChanged`)。
/// 获取当前会话的播放状态 (如播放、暂停、停止) 和可用的控制操作 (如是否可以暂停)，
/// 并更新共享播放器状态和 `NowPlayingInfo`。
/// 然后通过 `connector_update_tx` 发送 `NowPlayingTrackChanged` 更新。
async fn process_playback_info(
    session: &MediaSession,
    connector_update_tx: &StdSender<ConnectorUpdate>,
    player_state_arc: &Arc<TokioMutex<SharedPlayerState>>,
) -> WinResult<()> {
    let report_time_for_this_update = Instant::now(); // 记录当前时间，用于时间戳
    let playback_info = session.GetPlaybackInfo()?; // 获取播放信息对象
    let smtc_status = playback_info.PlaybackStatus()?; // 获取SMTC报告的播放状态
    let controls_opt = playback_info.Controls().ok(); // 获取可用的控制按钮
    let is_playing_now =
        smtc_status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing; // 判断当前是否在播放

    let mut new_position_ms_if_changed: Option<u64> = None; // 用于存储如果状态改变时获取到的新位置
    let mut new_duration_ms_if_changed: Option<u64> = None; // 用于存储如果状态改变时获取到的新总时长

    // 先锁定一次共享状态，检查实际的播放状态是否真的改变了
    let mut status_actually_changed = false;
    {
        // 限制 state_check 锁的作用域
        let state_check = player_state_arc.lock().await;
        if state_check.is_playing != is_playing_now {
            status_actually_changed = true; // 标记状态已改变
        }
    } // 锁在此释放

    // 如果检测到播放状态（如从播放到暂停，或从暂停到播放）确实发生了变化，
    // 那么就尝试立即获取最新的时间轴属性。
    // 这有助于确保伴随状态更新发送的播放位置尽可能准确。
    if status_actually_changed {
        match session.GetTimelineProperties() {
            Ok(timeline_props) => {
                // SMTC 的时间单位是100纳秒，转换为毫秒
                new_position_ms_if_changed =
                    Some((timeline_props.Position()?.Duration / 10000) as u64);
                let duration_from_timeline = (timeline_props.EndTime()?.Duration / 10000) as u64;
                if duration_from_timeline > 0 {
                    // 只有当获取到的总时长有效时才更新
                    new_duration_ms_if_changed = Some(duration_from_timeline);
                }
                log::trace!(
                    "[SMTC Handler process_playback_info] 播放状态已改变。获取到新的时间轴信息: 位置={:?}, 总时长={:?}",
                    new_position_ms_if_changed,
                    new_duration_ms_if_changed
                );
            }
            Err(e) => {
                // 如果获取失败，记录警告，后续逻辑会使用之前缓存的时间
                log::warn!(
                    "[SMTC Handler process_playback_info] 播放状态改变后，获取时间轴属性失败: {:?}",
                    e
                );
            }
        }
    }

    let info_to_send: NowPlayingInfo; // 准备发送给 app.rs 的信息结构体
    {
        // 再次锁定共享状态以更新并构造 NowPlayingInfo
        let mut state = player_state_arc.lock().await;
        let old_is_playing = state.is_playing; // 记录旧的播放状态，用于比较
        state.is_playing = is_playing_now; // 更新为SMTC报告的当前播放状态

        // 如果因为播放状态改变而获取到了新的播放位置，则用这个新位置更新共享状态。
        // 否则，共享状态中的 last_known_position_ms 保持不变（它会被 TimelinePropertiesChanged 事件更新）。
        if let Some(new_pos) = new_position_ms_if_changed {
            state.last_known_position_ms = new_pos;
            // 关键：因为这个播放位置是刚刚查询到的，所以它的报告时间戳也应该是当前时间。
            state.last_known_position_report_time = Some(report_time_for_this_update);
        } else if old_is_playing != is_playing_now {
            // 如果播放状态改变了，但未能获取到新的播放位置（例如API调用失败），
            // 至少更新一下当前已知位置的报告时间戳，因为这个事件（状态改变）是现在发生的。
            state.last_known_position_report_time = Some(report_time_for_this_update);
        }

        // 如果获取到了新的总时长，则更新共享状态。
        if let Some(new_dur) = new_duration_ms_if_changed {
            state.song_duration_ms = new_dur;
        }

        update_playback_controls_state(&mut state, controls_opt.as_ref()); // 更新可用的控制按钮状态

        // 构造要发送的 NowPlayingInfo
        info_to_send = NowPlayingInfo {
            title: Some(state.title.clone()),
            artist: Some(state.artist.clone()),
            album_title: Some(state.album.clone()),
            is_playing: Some(state.is_playing),
            duration_ms: Some(state.song_duration_ms),
            position_ms: Some(state.last_known_position_ms), // 使用（可能已更新的）播放位置
            position_report_time: state.last_known_position_report_time, // 使用（可能已更新的）报告时间戳
            cover_data: state.cover_data.clone(),
            cover_data_hash: state.cover_data_hash,
        };
    } // 共享状态锁在此释放

    // 将构造好的 NowPlayingInfo 发送给 worker (最终到 app.rs)
    if connector_update_tx
        .send(ConnectorUpdate::NowPlayingTrackChanged(info_to_send))
        .is_err()
    {
        log::error!("[SMTC Handler] 发送“歌曲已更改”更新失败 (原因: 播放信息变更)。");
    }
    Ok(())
}

/// 处理进度条属性更改事件 (`TimelinePropertiesChanged`)。
/// 获取当前会话的播放进度和总时长，并更新共享播放器状态和 `NowPlayingInfo`。
/// 然后通过 `connector_update_tx` 发送 `NowPlayingTrackChanged` 更新。
async fn process_timeline_properties(
    session: &MediaSession,
    connector_update_tx: &StdSender<ConnectorUpdate>,
    player_state_arc: &Arc<TokioMutex<SharedPlayerState>>,
) -> WinResult<()> {
    let report_time_for_this_update = Instant::now();
    let timeline_props = session.GetTimelineProperties()?;
    let smtc_progress_ms = (timeline_props.Position()?.Duration / 10000) as u64;
    let smtc_duration_ms = (timeline_props.EndTime()?.Duration / 10000) as u64;

    let info_to_send: NowPlayingInfo;
    {
        let mut state = player_state_arc.lock().await;
        state.last_known_position_ms = smtc_progress_ms;
        state.last_known_position_report_time = Some(report_time_for_this_update);
        if smtc_duration_ms > 0 {
            state.song_duration_ms = smtc_duration_ms;
        }
        if let Ok(playback_info) = session.GetPlaybackInfo() {
            if let Ok(controls) = playback_info.Controls() {
                update_playback_controls_state(&mut state, Some(&controls));
            }
        }

        // Corrected NowPlayingInfo construction
        info_to_send = NowPlayingInfo {
            title: Some(state.title.clone()),
            artist: Some(state.artist.clone()),
            album_title: Some(state.album.clone()),
            is_playing: Some(state.is_playing),
            duration_ms: Some(state.song_duration_ms),
            position_ms: Some(state.last_known_position_ms), // Use new field name
            position_report_time: state.last_known_position_report_time, // Use new field name
            cover_data: state.cover_data.clone(),
            cover_data_hash: state.cover_data_hash,
        };
    }

    if connector_update_tx
        .send(ConnectorUpdate::NowPlayingTrackChanged(info_to_send))
        .is_err()
    {
        log::error!("[SMTC Handler] 发送“歌曲已更改”更新失败 (原因: 进度条属性变更)。");
    }
    Ok(())
}

/// 为当前的 SMTC 会话注册事件监听器。
/// 这些监听器会将 SMTC 的各种属性更改事件转换为内部的 `SmtcEventSignal`，
/// 并通过 `signal_tx` (一个 Tokio MPSC 发送端) 发送到事件处理循环。
/// 返回一个包含所有已注册事件监听器 token 的元组，用于后续取消注册。
async fn register_session_event_listeners_tokio(
    session: &MediaSession,                                // 要监听的 SMTC 会话
    signal_tx: tokio::sync::mpsc::Sender<SmtcEventSignal>, // 用于发送内部事件信号的 Tokio MPSC 发送端
) -> WinResult<(
    i64, // MediaPropertiesChanged 事件的 token
    i64, // PlaybackInfoChanged 事件的 token
    i64, // TimelinePropertiesChanged 事件的 token
)> {
    // 为 MediaPropertiesChanged 事件注册监听器
    let tx_media_props = signal_tx.clone(); // 克隆发送端用于此事件
    let media_props_token: i64 =
        session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
            // 当事件触发时，尝试发送 MediaProperties 信号
            // try_send 是非阻塞的，如果通道已满或关闭会立即返回错误
            tx_media_props
                .try_send(SmtcEventSignal::MediaProperties)
                .map_err(|e| {
                    WinError::new(
                        E_ABORT_HRESULT,
                        format!("发送 SMTC 媒体属性变更信号失败: {}", e),
                    )
                })?;
            Ok(())
        }))?;

    // 为 PlaybackInfoChanged 事件注册监听器
    let tx_playback_info = signal_tx.clone();
    let playback_info_token: i64 =
        session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
            tx_playback_info
                .try_send(SmtcEventSignal::PlaybackInfo)
                .map_err(|e| {
                    WinError::new(
                        E_ABORT_HRESULT,
                        format!("发送 SMTC 播放信息变更信号失败: {}", e),
                    )
                })?;
            Ok(())
        }))?;

    // 为 TimelinePropertiesChanged 事件注册监听器
    // 注意：这里直接使用原始的 signal_tx，因为它是最后一个需要克隆发送端的监听器
    let timeline_props_token: i64 =
        session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
            signal_tx
                .try_send(SmtcEventSignal::TimelineProperties)
                .map_err(|e| {
                    WinError::new(
                        E_ABORT_HRESULT,
                        format!("发送 SMTC 进度条属性变更信号失败: {}", e),
                    )
                })?;
            Ok(())
        }))?;

    log::trace!("[SMTC Handler] 已成功为当前媒体会话注册所有必要的事件监听器。");
    Ok((media_props_token, playback_info_token, timeline_props_token))
}

/// 取消注册指定 SMTC 会话的所有事件监听器。
///
/// # 参数
/// * `session`: 之前注册了监听器的 SMTC 会话。
/// * `tokens`: 一个包含所有事件监听器 token 的元组。
fn unregister_session_event_listeners(
    session: &MediaSession,
    tokens: (i64, i64, i64), // (media_props_token, playback_info_token, timeline_props_token)
) -> WinResult<()> {
    log::trace!("[SMTC Handler] 正在取消注册当前媒体会话的事件监听器...");
    session.RemoveMediaPropertiesChanged(tokens.0)?;
    session.RemovePlaybackInfoChanged(tokens.1)?;
    session.RemoveTimelinePropertiesChanged(tokens.2)?;
    log::trace!("[SMTC Handler] 所有事件监听器已成功取消注册。");
    Ok(())
}

/// 辅助函数：从 SMTC 会话的 `SourceAppUserModelId` 生成一个更易读的显示名称。
/// 例如，将 "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify" 转换为 "SpotifyMusic" 或 "Spotify"。
fn generate_display_name(app_user_model_id: &str) -> String {
    if app_user_model_id.is_empty() {
        return "未知应用".to_string();
    }
    let mut name_intermediate = app_user_model_id.to_string();
    // 尝试移除 ".exe" 后缀 (不区分大小写)
    if let Some(idx) = name_intermediate.to_lowercase().rfind(".exe") {
        if idx == name_intermediate.len() - 4 {
            // 确保 ".exe" 在末尾
            name_intermediate.truncate(idx);
        }
    }
    // 尝试按 '!' 分割 (通常用于 UWP 应用 ID)，取 '!' 后面的部分
    let name_after_bang = name_intermediate
        .split('!')
        .next_back() // 取最后一部分
        .unwrap_or(&name_intermediate); // 如果没有 '!'，则使用原字符串
    // 尝试取最后一个 '.' 之后的部分作为最终名称候选
    let final_name_candidate = name_after_bang
        .split('.')
        .next_back()
        .unwrap_or(name_after_bang);

    // 简单地将首字母大写处理
    if let Some(first_char) = final_name_candidate.chars().next() {
        if final_name_candidate.len() == 1 {
            first_char.to_uppercase().to_string()
        } else {
            format!(
                "{}{}",
                first_char.to_uppercase(),
                &final_name_candidate[1..]
            )
        }
    } else if final_name_candidate.is_empty() {
        // 如果处理后变为空字符串
        "未知应用".to_string()
    } else {
        final_name_candidate.to_string() // 返回处理后的候选名称
    }
}

/// 获取所有当前活动的 SMTC 会话信息列表。
/// 每个 `SmtcSessionInfo` 包含会话 ID (通常是 SourceAppUserModelId) 和一个易读的显示名称。
fn get_all_session_infos(manager: &MediaSessionManager) -> WinResult<Vec<SmtcSessionInfo>> {
    let mut sessions_info_list = Vec::new();
    let sessions_ivector = manager.GetSessions()?; // 获取所有活动会话的 IVectorView
    for session_media_obj in sessions_ivector {
        // 使用 SourceAppUserModelId 作为唯一且稳定的会话 ID
        let id_hstr = session_media_obj.SourceAppUserModelId()?;
        let id_str = hstring_to_string(&id_hstr);
        if !id_str.is_empty() {
            // 确保会话 ID 不是空的
            sessions_info_list.push(SmtcSessionInfo {
                session_id: id_str.clone(),                   // 用于唯一标识
                source_app_user_model_id: id_str.clone(),     // 原始 ID
                display_name: generate_display_name(&id_str), // 生成的显示名称
            });
        } else {
            log::warn!(
                "[SMTC Handler] (get_all_session_infos) 发现一个 SourceAppUserModelId 为空的会话，已忽略。"
            );
        }
    }
    Ok(sessions_info_list)
}

/// 为指定的 SMTC 会话设置事件监听器并获取其初始状态。
/// 此函数会：
/// 1. 获取并处理会话的初始媒体属性。
/// 2. 获取并处理会话的初始播放信息。
/// 3. 为会话注册所有必要的事件监听器。
async fn setup_event_listeners_for_session(
    session_to_monitor: MediaSession,
    connector_update_tx: &StdSender<ConnectorUpdate>,
    smtc_event_signal_tx: &tokio::sync::mpsc::Sender<SmtcEventSignal>,
    player_state_arc: &Arc<TokioMutex<SharedPlayerState>>,
    listener_tokens_storage: &mut Option<(i64, i64, i64)>,
) -> WinResult<()> {
    let session_id_for_log = session_to_monitor
        .SourceAppUserModelId()
        .map(|h_id| hstring_to_string(&h_id))
        .unwrap_or_else(|_| "未知ID (获取失败)".to_string());
    log::trace!(
        "[SMTC Handler] 正在为媒体会话 '{}' 设置事件监听器和获取初始状态 (v2)...",
        session_id_for_log
    );

    // 重置共享状态，因为我们要开始监听一个新会话（或重新监听）
    // 这确保了发送的初始 NowPlayingInfo 不会携带旧会话的残留信息
    {
        let mut state_guard = player_state_arc.lock().await;
        *state_guard = SharedPlayerState::default();
        update_playback_controls_state(&mut state_guard, None);
    }

    // 1. 获取并处理初始媒体属性 (这将发送一个 NowPlayingInfo)
    if let Err(e) =
        process_media_properties(&session_to_monitor, connector_update_tx, player_state_arc).await
    {
        log::error!(
            "[SMTC Handler] (Setup) 处理会话 '{}' 的初始媒体属性时出错: {:?}",
            session_id_for_log,
            e
        );
    }
    // 2. 获取并处理初始播放信息 (这将发送另一个 NowPlayingInfo，包含最新的播放状态和时间)
    //    注意：这可能会覆盖上面媒体属性发送的 NowPlayingInfo 中的时间信息，但这是期望的，
    //    因为播放信息事件通常更关注即时状态。
    if let Err(e) =
        process_playback_info(&session_to_monitor, connector_update_tx, player_state_arc).await
    {
        log::error!(
            "[SMTC Handler] (Setup) 处理会话 '{}' 的初始播放信息时出错: {:?}",
            session_id_for_log,
            e
        );
    }
    // 3. （可选）可以再调用一次 process_timeline_properties 来确保时间轴信息是最新的，
    //    但这可能与 process_playback_info 中的时间获取重复。通常 playback_info 事件后，时间轴也应是最新的。
    //    如果发现时间不准，可以考虑在这里也调用它。

    // 4. 注册事件监听器
    match register_session_event_listeners_tokio(&session_to_monitor, smtc_event_signal_tx.clone())
        .await
    {
        Ok(tokens) => *listener_tokens_storage = Some(tokens),
        Err(e) => {
            log::error!(
                "[SMTC Handler] (Setup) 为会话 '{}' 注册事件监听器失败: {:?}",
                session_id_for_log,
                e
            );
            return Err(e);
        }
    }
    log::trace!(
        "[SMTC Handler] (Setup) 已成功为会话 '{}' 设置事件监听器并获取了初始状态。",
        session_id_for_log
    );
    Ok(())
}

/// SMTC 监听器的主运行函数。
/// 此函数通常在一个单独的线程中运行。
///
/// # 参数
/// * `connector_update_tx`: 用于将 SMTC 更新 (如歌曲信息、会话列表) 发送回 Worker 的通道。
/// * `control_rx`: 用于从 Worker 接收控制命令 (如选择特定 SMTC 会话、媒体控制) 的通道。
/// * `player_state_arc`: 共享的播放器状态。
/// * `shutdown_rx`: 用于接收关闭信号的通道，当收到信号时，此函数应清理并退出。
///
/// # 返回
/// * `Ok(())` 如果监听器正常结束。
/// * `Err(String)` 如果发生无法恢复的错误导致监听器提前终止。
pub fn run_smtc_listener(
    connector_update_tx: StdSender<ConnectorUpdate>,
    control_rx: StdReceiver<ConnectorCommand>,
    player_state_arc: Arc<TokioMutex<SharedPlayerState>>,
    shutdown_rx: StdReceiver<()>, // 用于接收关闭信号
) -> Result<(), String> {
    log::trace!("[SMTC Handler] SMTC 监听器线程正在启动...");

    // 为此线程创建一个 Tokio 当前线程运行时 (current_thread runtime)
    // SMTC 的一些异步 API (如获取封面) 需要在 Tokio 上下文中运行
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all() // 启用所有 Tokio 功能 (如 IO, time)
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            let err_msg = format!("SMTC处理器创建 Tokio 运行时失败: {}", e);
            log::error!("{}", err_msg);
            // 尝试通知 Worker 发生了错误
            let _ = connector_update_tx.send(ConnectorUpdate::WebsocketStatusChanged(
                WebsocketStatus::错误(err_msg.clone()), // 使用 WebsocketStatus::错误 来传递一般性错误
            ));
            return Err(err_msg);
        }
    };

    // 使用 Tokio 运行时来执行异步的 SMTC 监听逻辑
    rt.block_on(async move {
        // COM 初始化/反初始化 RAII Guard
        // 确保在此异步块 (即 Tokio worker 线程) 的生命周期内 COM 被正确管理
        struct ComGuard;

        impl ComGuard {
            fn new() -> WinResult<Self> {
                // SAFETY: 调用 Windows COM API。
                // COINIT_MULTITHREADED 表示为此线程初始化一个多线程套间 (MTA)。
                // SMTC API 通常在 MTA 中工作得更好或需要 MTA。
                unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
                log::trace!("[SMTC Handler 异步块] COM 已为 Tokio worker 线程初始化 (MTA)。");
                Ok(Self)
            }
        }

        impl Drop for ComGuard {
            fn drop(&mut self) {
                // SAFETY: 调用 Windows COM API。
                unsafe { CoUninitialize(); }
                log::trace!("[SMTC Handler 异步块] COM 已通过 RAII Guard 自动反初始化。");
            }
        }

        // 创建并持有 ComGuard 实例，直到异步块结束
        let _com_guard = match ComGuard::new() {
            Ok(guard) => guard,
            Err(e) => {
                let err_msg = format!(
                    "SMTC处理器 Tokio worker 线程 COM 初始化失败: HRESULT 0x{:08X}",
                    e.code().0 // 获取 HRESULT 错误码
                );
                log::error!("{}", err_msg);
                let _ = connector_update_tx.send(ConnectorUpdate::WebsocketStatusChanged(
                    WebsocketStatus::错误(err_msg),
                ));
                return; // 从异步块返回，这将导致 block_on 结束
            }
        };

        log::trace!("[SMTC Handler 异步块] 进入 Tokio 异步执行块。");
        // 创建用于内部 SMTC 事件信号的 Tokio MPSC 通道
        // 当 SMTC 事件监听器触发时，它们会向这个通道发送信号
        let (smtc_event_signal_tx, mut smtc_event_signal_rx) =
            tokio::sync::mpsc::channel::<SmtcEventSignal>(32); // 通道容量为 32

        // 获取 SMTC 会话管理器 (MediaSessionManager)
        let manager = match MediaSessionManager::RequestAsync() { // 异步请求管理器
            Ok(operation) => {
                log::trace!("[SMTC Handler 异步块] MediaSessionManager::RequestAsync() 调用成功，等待异步操作完成...");
                match tokio_timeout(SMTC_ASYNC_OPERATION_TIMEOUT, operation.into_future()).await {
                    Ok(Ok(m)) => { // 异步操作成功，管理器获取成功
                        log::debug!("[SMTC Handler 异步块] MediaSessionManager 获取成功。");
                        m
                    }
                    Ok(Err(e)) => { // 异步操作失败
                        log::error!("[SMTC Handler 异步块] 获取 MediaSessionManager 的异步操作失败: {:?}", e);
                        let _ = connector_update_tx.send(ConnectorUpdate::WebsocketStatusChanged(
                            WebsocketStatus::错误(format!("SMTC管理器初始化(异步)失败: {:?}", e)),
                        ));
                        return;
                    }
                    Err(_) => { // 获取管理器超时
                        log::error!("[SMTC Handler 异步块] 获取 MediaSessionManager 超时。");
                        let _ = connector_update_tx.send(ConnectorUpdate::WebsocketStatusChanged(
                            WebsocketStatus::错误("SMTC管理器初始化超时".to_string()),
                        ));
                        return;
                    }
                }
            }
            Err(e) => { // RequestAsync() 本身同步返回错误
                log::error!("[SMTC Handler 异步块] 请求 MediaSessionManager 失败 (同步错误): {:?}", e);
                let _ = connector_update_tx.send(ConnectorUpdate::WebsocketStatusChanged(
                    WebsocketStatus::错误(format!("SMTC管理器请求(同步)失败: {:?}", e)),
                ));
                return;
            }
        };

        // 当前正在监听的 SMTC 会话
        let mut current_monitored_session: Option<MediaSession> = None;
        // 当前监听会话的事件监听器 token
        let mut current_listener_tokens: Option<(i64, i64, i64)> = None;
        // 会话管理器 `SessionsChanged` 事件的监听器 token
        let mut manager_sessions_changed_token: Option<i64> = None;
        // 目标会话 ID (由 Worker 通过命令指定)，使用 Arc<Mutex> 以便在异步回调中安全访问
        let target_session_id_arc: Arc<TokioMutex<Option<String>>> = Arc::new(TokioMutex::new(None));
        // 当前活动的音量缓动任务的 JoinHandle
        let active_volume_easing_task: Arc<TokioMutex<Option<JoinHandle<()>>>> = Arc::new(TokioMutex::new(None));


        // 初始获取所有可用 SMTC 会话列表并发送给 Worker
        log::trace!("[SMTC Handler 异步块] 正在获取初始 SMTC 会话列表...");
        match get_all_session_infos(&manager) {
            Ok(session_infos) => {
                log::debug!("[SMTC Handler 异步块] 初始发现 {} 个 SMTC 会话。", session_infos.len());
                if connector_update_tx.send(ConnectorUpdate::SmtcSessionListChanged(session_infos)).is_err() {
                    log::error!("[SMTC Handler 异步块] 发送初始 SMTC 会话列表更新失败。");
                }
            }
            Err(e) => { log::error!("[SMTC Handler 异步块] 获取初始 SMTC 会话列表失败: {:?}", e); }
        }

        // 初始尝试监听系统当前的默认会话
        log::trace!("[SMTC Handler 异步块] 正在尝试进行初始会话设置 (监听系统当前默认会话)...");
        if let Ok(initial_session) = manager.GetCurrentSession() { // 获取当前会话
            { // 作用域块，用于在设置新会话前重置播放器状态
                let mut state_guard = player_state_arc.lock().await;
                *state_guard = SharedPlayerState::default(); // 重置为默认状态
                update_playback_controls_state(&mut state_guard, None); // 清空控制能力
            }
            // 为初始会话设置事件监听器并获取初始状态
            if let Err(e) = setup_event_listeners_for_session(
                initial_session.clone(), &connector_update_tx, &smtc_event_signal_tx,
                &player_state_arc, &mut current_listener_tokens
            ).await {
                let session_id_for_log = initial_session.SourceAppUserModelId().map(|h| hstring_to_string(&h)).unwrap_or_else(|_| "未知ID".to_string());
                log::error!("[SMTC Handler 异步块] 初始设置监听会话 '{}' 失败: {:?}", session_id_for_log, e);
            } else {
                let session_id_for_log = initial_session.SourceAppUserModelId().map(|h| hstring_to_string(&h)).unwrap_or_else(|_| "未知ID".to_string());
                log::debug!("[SMTC Handler 异步块] 成功设置为初始监听会话 '{}'。", session_id_for_log);
                current_monitored_session = Some(initial_session); // 保存当前监听的会话
            }
        } else { // 获取当前会话失败
            log::warn!("[SMTC Handler] 初始获取当前系统媒体会话失败，当前可能没有活动的播放器。");
            // 发送一个“无活动会话”的更新给 Worker
            let no_session_update = NowPlayingInfo { title: Some("无活动会话".to_string()), position_report_time: Some(Instant::now()), ..Default::default() };
            if connector_update_tx.send(ConnectorUpdate::NowPlayingTrackChanged(no_session_update)).is_err() {
                log::error!("[SMTC Handler 异步块] 发送“无活动会话”状态更新失败。");
            }
        }

        // 注册 SMTC 会话管理器 (manager) 的 `SessionsChanged` 事件监听器
        // 当系统中的媒体会话列表发生变化时 (例如，新的播放器启动或旧的播放器关闭)，此事件会触发
        let signal_tx_for_manager = smtc_event_signal_tx.clone();
        match manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
            // 当事件触发时，发送 Sessions 信号到内部事件处理循环
            signal_tx_for_manager.try_send(SmtcEventSignal::Sessions)
                .map_err(|e| WinError::new(E_ABORT_HRESULT, format!("发送 SessionsChanged 信号失败: {}", e)))?;
            Ok(())
        })) {
            Ok(token) => { manager_sessions_changed_token = Some(token); log::debug!("[SMTC Handler 异步块] 已成功为会话管理器注册 SessionsChanged 事件监听器。"); }
            Err(e) => { log::error!("[SMTC Handler 异步块] 为会话管理器注册 SessionsChanged 事件监听器失败: {:?}", e); }
        };

        // 用于生成音量缓动任务的唯一 ID，方便日志追踪
        let next_easing_task_id = Arc::new(std::sync::atomic::AtomicU64::new(0));


        log::trace!("[SMTC Handler 异步块] 进入主事件循环...");
        'main_smtc_loop: loop {
            // 检查关闭信号 (非阻塞)
            // 这是从 Worker 线程通过 std::sync::mpsc 通道发送过来的同步信号
            match shutdown_rx.try_recv() {
                Ok(_) | Err(StdTryRecvError::Disconnected) => { // 收到关闭信号或通道断开
                    log::debug!("[SMTC Handler 异步块] 收到关闭信号或通道已断开，准备退出主事件循环...");
                    break 'main_smtc_loop;
                }
                Err(StdTryRecvError::Empty) => {} // 通道为空，无关闭信号，继续执行
            }

            // 处理来自 Worker 的控制命令 (非阻塞)
            if let Ok(command) = control_rx.try_recv() {
                match command {
                    ConnectorCommand::SelectSmtcSession(target_id) => { // 选择要监听的 SMTC 会话
                        log::debug!("[SMTC Handler 异步块] 收到命令: 选择 SMTC 会话 (目标ID: '{}')", target_id);
                        let mut target_guard = target_session_id_arc.lock().await; // 异步锁定目标 ID
                        let new_target = if target_id.is_empty() { None } else { Some(target_id) }; // 如果 ID 为空，则表示自动选择
                        if *target_guard != new_target { // 如果目标 ID 发生变化
                            log::debug!("[SMTC Handler 异步块] 更新目标会话 ID 从 {:?} 到 {:?}", *target_guard, new_target);
                            *target_guard = new_target; // 更新目标 ID
                            drop(target_guard); // 释放锁后才能安全地发送信号

                            // 触发 SessionsChanged 处理逻辑，以重新评估和切换到新的目标会话
                            if smtc_event_signal_tx.try_send(SmtcEventSignal::Sessions).is_err() {
                                log::warn!("[SMTC Handler 异步块] (SelectSmtcSession) 发送内部 SessionsChanged 信号失败，可能无法立即切换会话。");
                            }
                        } else { // 目标 ID 未变
                            log::debug!("[SMTC Handler 异步块] 目标会话 ID 已是 {:?} (或等效的自动选择)，无需重复处理。", new_target);
                        }
                    }
                    ConnectorCommand::MediaControl(media_cmd) => { // 媒体控制命令 (播放、暂停、下一首等)
                        log::trace!("[SMTC Handler 异步块] 收到媒体控制命令: {:?}", media_cmd);
                        if let Some(session) = &current_monitored_session { // 确保当前有正在监听的会话
                            match media_cmd {
                                SmtcControlCommand::Play | SmtcControlCommand::Pause | SmtcControlCommand::SkipNext |
                                SmtcControlCommand::SkipPrevious | SmtcControlCommand::SeekTo(_) => {
                                    // 对于这些命令，先检查 SMTC 是否允许执行此操作
                                    let player_state_guard = player_state_arc.lock().await;
                                    let can_execute = match media_cmd {
                                        SmtcControlCommand::Play => player_state_guard.can_play,
                                        SmtcControlCommand::Pause => player_state_guard.can_pause,
                                        SmtcControlCommand::SkipNext => player_state_guard.can_skip_next,
                                        SmtcControlCommand::SkipPrevious => player_state_guard.can_skip_previous,
                                        SmtcControlCommand::SeekTo(_) => player_state_guard.can_seek,
                                        _ => false, // SetVolume 不在此处检查 can_execute，它有自己的逻辑
                                    };
                                    drop(player_state_guard); // 释放锁

                                    if can_execute { // 如果允许执行
                                        // 异步尝试执行相应的 SMTC 控制操作
                                        let op_res = match media_cmd {
                                            SmtcControlCommand::Play => session.TryPlayAsync(),
                                            SmtcControlCommand::Pause => session.TryPauseAsync(),
                                            SmtcControlCommand::SkipNext => session.TrySkipNextAsync(),
                                            SmtcControlCommand::SkipPrevious => session.TrySkipPreviousAsync(),
                                            SmtcControlCommand::SeekTo(ms) => session.TryChangePlaybackPositionAsync((ms * 10000) as i64), // 毫秒转换为100纳秒单位
                                            _ => unreachable!("SetVolume 应该在下面单独处理"), // SetVolume 有特殊处理
                                        };
                                        if let Ok(op) = op_res { // 异步操作创建成功
                                            // 为异步操作设置超时
                                            match tokio_timeout(SMTC_ASYNC_OPERATION_TIMEOUT, op.into_future()).await {
                                                Ok(Ok(success)) => { if !success { log::warn!("[SMTC Handler 异步块] SMTC 命令 {:?} 执行后报告失败 (success=false)。", media_cmd); } }
                                                Ok(Err(e)) => log::error!("[SMTC Handler 异步块] 执行 SMTC 命令 {:?} 的异步操作时出错: {:?}", media_cmd, e),
                                                Err(_) => log::warn!("[SMTC Handler 异步块] 执行 SMTC 命令 {:?} 超时。", media_cmd),
                                            }
                                        } else if let Err(e) = op_res { // 异步操作创建失败
                                            log::error!("[SMTC Handler 异步块] 准备 SMTC 命令 {:?} 失败 (API调用返回错误): {:?}", media_cmd, e);
                                        }
                                    } else { // SMTC 不允许执行此操作
                                        log::warn!("[SMTC Handler 异步块] 无法执行命令 {:?}，当前 SMTC 状态未启用此操作。", media_cmd);
                                    }
                                }
                                SmtcControlCommand::SetVolume(target_volume_level_f32) => { // 设置音量命令
                                    let task_id = next_easing_task_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed); // 获取唯一任务 ID
                                    log::debug!(
                                        "[SMTC Handler 异步块] 任务ID[{}]: 收到 SetVolume 命令，目标音量: {:.2}",
                                        task_id, target_volume_level_f32
                                    );

                                    let mut easing_task_guard = active_volume_easing_task.lock().await; // 锁定当前音量缓动任务句柄
                                    if let Some(old_handle) = easing_task_guard.take() { // 如果存在旧的缓动任务
                                        log::trace!("[SMTC Handler 异步块] 任务ID[{}]: 检测到旧的音量缓动任务，正在尝试取消...", task_id);
                                        old_handle.abort(); // 取消旧任务
                                    } else {
                                        log::trace!("[SMTC Handler 异步块] 任务ID[{}]: 没有活动的旧音量缓动任务。", task_id);
                                    }

                                    // 创建一个新的 Tokio 任务来执行音量缓动
                                    let session_clone_for_task = session.clone(); // 克隆会话对象用于新任务
                                    let connector_update_tx_clone = connector_update_tx.clone(); // 克隆更新通道用于新任务
                                    let new_easing_task = tokio::spawn(async move {
                                        log::trace!("[音量缓动任务][ID:{}] 启动。", task_id);
                                        let animation_duration_ms = 250.0f32; // 缓动总时长 (毫秒)
                                        let steps = 15u32; // 缓动步数
                                        let step_duration_ms = animation_duration_ms / steps as f32; // 每一步的持续时间

                                        // 获取目标应用程序的标识符 (SourceAppUserModelId)
                                        let target_identifier = match session_clone_for_task.SourceAppUserModelId() {
                                            Ok(id_hstr) if !id_hstr.is_empty() => hstring_to_string(&id_hstr),
                                            _ => {
                                                log::warn!("[音量缓动任务][ID:{}] SetVolume: 无法获取当前会话的 SourceAppUserModelId。", task_id);
                                                return; // 无法获取标识符，则无法控制音量
                                            }
                                        };
                                        // 根据标识符获取应用程序的进程 ID (PID)
                                        let pid = match volume_control::get_pid_from_identifier(&target_identifier) {
                                            Some(p) => p,
                                            None => {
                                                log::warn!("[音量缓动任务][ID:{}] SetVolume: 无法为目标应用 '{}' 找到 PID。", task_id, target_identifier);
                                                return; // 找不到 PID，无法控制音量
                                            }
                                        };

                                        // 在阻塞线程中获取初始音量
                                        let initial_volume_f32 = match tokio::task::spawn_blocking(move || {
                                            volume_control::get_process_volume_by_pid(pid)
                                        }).await { // 等待阻塞操作完成
                                            Ok(Ok((vol, _muted))) => vol, // 获取成功
                                            Ok(Err(e)) => { // get_process_volume_by_pid 内部返回错误
                                                log::error!("[音量缓动任务][ID:{}] 获取初始音量失败 (内部错误): {}.", task_id, e);
                                                return;
                                            }
                                            Err(e) => { // spawn_blocking 本身失败 (例如任务被取消)
                                                log::error!("[音量缓动任务][ID:{}] 获取初始音量的 spawn_blocking 操作失败 (JoinError): {}.", task_id, e);
                                                if e.is_cancelled() {
                                                    log::debug!("[音量缓动任务][ID:{}] 音量缓动任务在获取初始音量时被取消。", task_id);
                                                }
                                                return;
                                            }
                                        };

                                        log::trace!(
                                            "[音量缓动任务][ID:{}] 开始音量缓动: 从 {:.2} 到 {:.2}, 应用PID: {}, 时长: {:.0}ms",
                                            task_id, initial_volume_f32, target_volume_level_f32, pid, animation_duration_ms
                                        );

                                        // 执行缓动动画的每一帧
                                        for s_u32 in 0..=steps { // 从第0步到第steps步
                                            let current_time_ms = s_u32 as f32 * step_duration_ms; // 当前时间点
                                            let change_in_volume = target_volume_level_f32 - initial_volume_f32; // 音量总变化量
                                            // 使用 Quad::ease_out 缓动函数计算当前时间点的音量值
                                            let current_step_volume = Quad::ease_out(current_time_ms, initial_volume_f32, change_in_volume, animation_duration_ms);

                                            let pid_for_blocking = pid; // 复制 PID 用于阻塞任务

                                            // 在阻塞线程中设置当前步的音量
                                            let set_volume_result = tokio::task::spawn_blocking(move || {
                                                volume_control::set_process_volume_by_pid(pid_for_blocking, Some(current_step_volume), None)
                                            }).await;

                                            match set_volume_result {
                                                Ok(Ok(())) => { // 音量设置成功
                                                    if s_u32 == steps { // 如果是最后一步
                                                        // 再次获取最终音量并发送更新，以确保状态同步
                                                        let pid_for_get_final = pid;
                                                        let target_id_for_final_update = target_identifier.clone();
                                                        let update_tx_for_final_update = connector_update_tx_clone.clone();

                                                        match tokio::task::spawn_blocking(move || volume_control::get_process_volume_by_pid(pid_for_get_final)).await {
                                                            Ok(Ok((final_vol, final_mute))) => {
                                                                let update_msg = ConnectorUpdate::AudioSessionVolumeChanged {
                                                                    session_id: target_id_for_final_update,
                                                                    volume: final_vol,
                                                                    is_muted: final_mute,
                                                                };
                                                                if update_tx_for_final_update.send(update_msg).is_err() {
                                                                    log::error!("[音量缓动任务][ID:{}] 发送最终音量已更改更新失败。", task_id);
                                                                }
                                                            }
                                                            Ok(Err(e_get)) => log::error!("[音量缓动任务][ID:{}] 缓动结束后获取 PID {} 的音量状态失败 (内部错误): {}", task_id, pid_for_get_final, e_get),
                                                            Err(e_join) =>  {
                                                                log::error!("[音量缓动任务][ID:{}] 缓动结束后获取音量状态的 spawn_blocking 操作失败 (JoinError): {}", task_id, e_join);
                                                                if e_join.is_cancelled() {
                                                                    log::debug!("[音量缓动任务][ID:{}] 音量缓动任务在获取最终音量时被取消。", task_id);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                Ok(Err(e)) => { // set_process_volume_by_pid 内部返回错误
                                                    log::error!("[音量缓动任务][ID:{}] 步骤 {}/{}: 设置 PID {} 音量失败 (内部错误): {}. 中断缓动。", task_id, s_u32, steps, pid, e);
                                                    return; // 中断缓动
                                                }
                                                Err(e) => { // spawn_blocking 本身失败
                                                    log::error!("[音量缓动任务][ID:{}] 步骤 {}/{}: 设置音量的 spawn_blocking 操作失败 (JoinError): {}. 中断缓动。", task_id, s_u32, steps, e);
                                                    if e.is_cancelled() {
                                                        log::debug!("[音量缓动任务][ID:{}] 音量缓动任务在设置音量时被取消。", task_id);
                                                    }
                                                    return; // 中断缓动
                                                }
                                            }

                                            if s_u32 < steps { // 如果不是最后一步
                                                // 异步休眠，等待进入下一步
                                                // 如果任务被 abort(), 这个 await 会使得任务终止
                                                tokio_sleep(TokioDuration::from_secs_f32(step_duration_ms / 1000.0)).await;
                                            }
                                        }
                                        log::trace!("[音量缓动任务][ID:{}] 音量缓动完成或被中断。目标音量: {:.2}", task_id, target_volume_level_f32);
                                    });
                                    *easing_task_guard = Some(new_easing_task); // 保存新的缓动任务句柄
                                }
                                // 其他 SmtcControlCommand 类型不在此处处理
                            }
                        } else { // 没有活动的 SMTC 会话
                            log::warn!("[SMTC Handler 异步块] 收到媒体控制命令 {:?}，但当前没有活动的 SMTC 会话可供控制。", media_cmd);
                        }
                    }
                    _ => { log::warn!("[SMTC Handler 异步块] 收到未处理的连接器命令: {:?}", command); }
                }
            }

            // 处理内部 SMTC 事件信号 (带100毫秒超时，实现非阻塞轮询)
            match tokio::time::timeout(TokioDuration::from_millis(100), smtc_event_signal_rx.recv()).await {
                Ok(Some(signal)) => { // 收到内部事件信号
                    log::trace!("[SMTC Handler 异步块] 收到内部 SMTC 事件信号: {:?}", signal);
                    if signal == SmtcEventSignal::Sessions { // SMTC 会话列表发生变化
                        log::debug!("[SMTC Handler 异步块] 开始处理 SessionsChanged 事件...");
                        // 1. 获取并发送最新的所有可用会话列表给 Worker
                        match get_all_session_infos(&manager) {
                            Ok(all_sessions_list) => {
                                if connector_update_tx.send(ConnectorUpdate::SmtcSessionListChanged(all_sessions_list)).is_err() {
                                    log::error!("[SMTC Handler 异步块] (SessionsChanged) 发送 SMTC 会话列表更新失败。");
                                }
                            }
                            Err(e) => { log::error!("[SMTC Handler 异步块] (SessionsChanged) 获取所有 SMTC 会话列表失败: {:?}", e); }
                        }

                        // 2. 根据 target_session_id_arc (用户或程序指定的期望会话ID) 确定要监听哪个会话
                        let desired_session_id_opt: Option<String> = target_session_id_arc.lock().await.clone(); // 获取期望的会话ID
                        log::debug!("[SMTC Handler 异步块] (SessionsChanged) 当前期望的目标会话 ID: {:?}", desired_session_id_opt);

                        let mut new_session_to_monitor: Option<MediaSession> = None; // 将要监听的新会话
                        if let Some(ref desired_id) = desired_session_id_opt { // 如果指定了期望的会话ID
                            log::debug!("[SMTC Handler 异步块] (SessionsChanged) 用户期望监听特定会话 ID: '{}'", desired_id);
                            if let Ok(sessions_from_manager) = manager.GetSessions() { // 获取当前所有活动会话
                                for s_obj in sessions_from_manager { // 遍历查找匹配的会话
                                    if let Ok(id_hstr) = s_obj.SourceAppUserModelId() {
                                        if hstring_to_string(&id_hstr) == *desired_id {
                                            new_session_to_monitor = Some(s_obj); // 找到匹配的会话
                                            break;
                                        }
                                    }
                                }
                                if new_session_to_monitor.is_none() { // 如果没有找到用户指定的会话
                                    log::warn!("[SMTC Handler 异步块] (SessionsChanged) 用户选择的会话 ID '{}' 未在当前活动会话列表中找到。", desired_id);
                                    // 通知 Worker，用户选择的会话消失了
                                    if connector_update_tx.send(ConnectorUpdate::SelectedSmtcSessionVanished(desired_id.clone())).is_err() {
                                        log::error!("[SMTC Handler 异步块] 发送“选定会话消失”更新失败。");
                                    }
                                    // 清除用户选择，并尝试切换到系统默认会话
                                    *target_session_id_arc.lock().await = None;
                                    log::debug!("[SMTC Handler 异步块] (SessionsChanged) 已清除用户选择的目标会话 ID，将尝试监听系统默认会话。");
                                    new_session_to_monitor = manager.GetCurrentSession().ok(); // 尝试获取系统当前默认会话
                                }
                            } else { // 获取活动会话列表失败
                                log::error!("[SMTC Handler 异步块] (SessionsChanged) 查找目标会话时无法从管理器获取会话列表。");
                            }
                        } else { // 用户未指定特定会话，尝试监听系统当前默认会话
                            log::debug!("[SMTC Handler 异步块] (SessionsChanged) 用户未指定特定会话，尝试监听系统当前默认会话。");
                            new_session_to_monitor = manager.GetCurrentSession().ok();
                        }

                        // 获取当前正在监听的会话 ID 和新目标会话的 ID，用于比较是否需要切换
                        let current_monitored_id = current_monitored_session.as_ref().and_then(|s| s.SourceAppUserModelId().ok().map(|h| hstring_to_string(&h)));
                        let new_target_id_to_monitor = new_session_to_monitor.as_ref().and_then(|s| s.SourceAppUserModelId().ok().map(|h| hstring_to_string(&h)));

                        if current_monitored_id != new_target_id_to_monitor { // 如果目标会话发生变化
                            log::trace!("[SMTC Handler 异步块] (SessionsChanged) 检测到需要切换监听的媒体会话 (从 {:?} 切换到 {:?})。", current_monitored_id, new_target_id_to_monitor);

                            // 1. 如果当前正在监听一个会话，则取消注册其事件监听器
                            if let Some(old_session) = current_monitored_session.take() { // .take() 会移出 Option 中的值
                                if let Some(tokens) = current_listener_tokens.take() {
                                    if let Err(e) = unregister_session_event_listeners(&old_session, tokens) {
                                        log::warn!("[SMTC Handler 异步块] (SessionsChanged) 取消注册旧会话的事件监听器失败: {:?}", e);
                                    }
                                }
                            }

                            // 2. 重置共享播放器状态，并发送一个临时的“切换中”或“无会话”更新
                            {
                                let mut state_guard = player_state_arc.lock().await;
                                *state_guard = SharedPlayerState::default(); // 重置状态
                                update_playback_controls_state(&mut state_guard, None); // 清空控制能力
                                let title = if new_session_to_monitor.is_some() { "切换媒体会话中...".to_string() } else { "无活动媒体会话".to_string() };
                                let info = NowPlayingInfo { title: Some(title), position_report_time: Some(Instant::now()), ..Default::default() };
                                if connector_update_tx.send(ConnectorUpdate::NowPlayingTrackChanged(info)).is_err() {
                                    log::error!("[SMTC Handler 异步块] 发送清空/临时状态的 NowPlayingInfo 失败。");
                                }
                            }

                            // 3. 如果有新的目标会话，则为其设置事件监听器并获取初始状态
                            if let Some(session_obj_to_monitor) = new_session_to_monitor.clone() { // 克隆会话对象
                                let sid = session_obj_to_monitor.SourceAppUserModelId().map(|h| hstring_to_string(&h)).unwrap_or_else(|_| "未知ID".to_string());
                                if let Err(e) = setup_event_listeners_for_session(
                                    session_obj_to_monitor.clone(), &connector_update_tx, &smtc_event_signal_tx,
                                    &player_state_arc, &mut current_listener_tokens
                                ).await { // 设置新会话的监听
                                    log::error!("[SMTC Handler 异步块] (SessionsChanged) 设置新会话 '{}' 的监听失败: {:?}", sid, e);
                                    current_monitored_session = None; // 设置失败，则当前无监听会话
                                } else {
                                    log::trace!("[SMTC Handler 异步块] (SessionsChanged) 已成功切换到监听新会话 '{}'。", sid);
                                    current_monitored_session = Some(session_obj_to_monitor); // 保存新监听的会话
                                }
                            } else { // 没有可监听的新会话
                                log::debug!("[SMTC Handler 异步块] (SessionsChanged) 没有可供监听的新媒体会话。");
                                current_monitored_session = None;
                            }
                        } else { // 无需切换会话 (目标与当前一致，或都为 None)
                            log::debug!("[SMTC Handler 异步块] (SessionsChanged) 无需切换监听的媒体会话 (目标与当前一致，或都为 None)。");
                        }
                    } else if let Some(session) = &current_monitored_session { // 处理当前监听会话的其他事件
                        match signal {
                            SmtcEventSignal::MediaProperties => { // 媒体属性变更
                                if let Err(e) = process_media_properties(session, &connector_update_tx, &player_state_arc).await {
                                    log::error!("[SMTC Handler 异步块] 处理 MediaPropertiesChanged 事件时出错: {:?}", e);
                                }
                            }
                            SmtcEventSignal::PlaybackInfo => { // 播放信息变更
                                if let Err(e) = process_playback_info(session, &connector_update_tx, &player_state_arc).await {
                                    log::error!("[SMTC Handler 异步块] 处理 PlaybackInfoChanged 事件时出错: {:?}", e);
                                }
                            }
                            SmtcEventSignal::TimelineProperties => { // 进度条属性变更
                                if let Err(e) = process_timeline_properties(session, &connector_update_tx, &player_state_arc).await {
                                    log::error!("[SMTC Handler 异步块] 处理 TimelinePropertiesChanged 事件时出错: {:?}", e);
                                }
                            }
                            SmtcEventSignal::Sessions => { /* Sessions 信号已在上面单独处理 */ }
                        }
                    } else { // 收到其他事件，但没有活动的会话可处理
                        log::warn!("[SMTC Handler 异步块] 收到事件 {:?} 但当前没有活动的媒体会话可供处理。可能需要重新评估会话状态。", signal);
                        // 尝试触发一次 SessionsChanged 处理，以期恢复或找到一个会话
                        if smtc_event_signal_tx.try_send(SmtcEventSignal::Sessions).is_err() {
                            log::warn!("[SMTC Handler 异步块] (无活动会话时) 发送内部 SessionsChanged 信号以尝试恢复会话状态失败。");
                        }
                    }
                }
                Ok(None) => { // SMTC 事件信号通道意外关闭
                    log::error!("[SMTC Handler 异步块] SMTC 事件信号通道意外关闭。退出主事件循环。");
                    break 'main_smtc_loop;
                }
                Err(_) => { /* 超时是正常情况，表示在100ms内没有新事件 */ }
            }
        } // 'main_smtc_loop 主事件循环结束

        log::trace!("[SMTC Handler 异步块] 主事件循环已完成。");

        // --- 清理工作 ---
        log::trace!("[SMTC Handler 异步块] 正在清理 SMTC 监听器资源...");

        // 取消活动的音量缓动任务 (如果有)
        if let Some(handle) = active_volume_easing_task.lock().await.take() {
            log::trace!("[SMTC Handler 异步块] 清理：正在取消活动的音量缓动任务...");
            handle.abort();
        }

        // 取消注册当前监听会话的事件监听器 (如果有)
        if let Some(session_to_clean) = current_monitored_session.take() {
            if let Some(tokens_to_clean) = current_listener_tokens.take() {
                if let Err(e) = unregister_session_event_listeners(&session_to_clean, tokens_to_clean) {
                    log::error!("[SMTC Handler 异步块] 清理会话事件监听器时出错: {:?}", e);
                }
            }
        }

        // 取消注册会话管理器的 SessionsChanged 事件监听器 (如果有)
        if let Some(token_val) = manager_sessions_changed_token.take() {
            if let Err(e) = manager.RemoveSessionsChanged(token_val) {
                log::error!("[SMTC Handler 异步块] 清理管理器 SessionsChanged 事件监听器时出错: {:?}", e);
            }
        }
        log::trace!("[SMTC Handler 异步块] SMTC 监听器异步块执行完成，资源已清理。");
        // _com_guard 会在此异步块结束时自动 Drop，从而调用 CoUninitialize
    }); // rt.block_on 结束

    log::trace!("[SMTC Handler] SMTC 监听器线程逻辑已完成并即将退出。");
    Ok(())
}
