use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_NO_MORE_FILES, HANDLE},
        Media::Audio::{
            AudioSessionStateActive, IAudioSessionControl, IAudioSessionControl2,
            IAudioSessionEnumerator, IAudioSessionManager2, IMMDeviceEnumerator,
            ISimpleAudioVolume, MMDeviceEnumerator, eConsole, eRender,
        },
        System::{
            Com::{
                CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoCreateInstance,
                CoInitializeEx, CoUninitialize,
            },
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
        },
    },
    core::{GUID, HRESULT, Interface, Result as WinResult},
};

// 定义特定的 HRESULT 错误码，用于音频客户端没有关联进程的情况
const AUDCLNT_S_NO_CURRENT_PROCESS: HRESULT = HRESULT(0x08890008_u32 as i32);

/// COM 初始化/反初始化 RAII Guard。
/// 确保在当前线程初始化 COM，并在 Guard 离开作用域时自动反初始化。
struct ComInitializer;

impl ComInitializer {
    /// 初始化 COM。
    /// `apartment_threaded`: true 表示使用 STA，false 表示使用 MTA。
    /// 对于音频会话管理，通常推荐使用 STA。
    fn initialize_com(apartment_threaded: bool) -> WinResult<()> {
        unsafe {
            CoInitializeEx(
                None,
                if apartment_threaded {
                    COINIT_APARTMENTTHREADED
                } else {
                    COINIT_MULTITHREADED
                },
            )
            .ok()
        }
    }
}

impl Drop for ComInitializer {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
        log::trace!("[音量控制] COM 已通过 RAII Guard 自动反初始化。");
    }
}

/// 从标识符（AUMID 或可执行文件名）获取进程 ID (PID)。
///
/// # Arguments
/// * `identifier`: AUMID 字符串或可执行文件名 (例如 "Spotify.exe")。
///
/// # Returns
/// `Some(pid)` 如果找到，否则 `None`。
pub fn get_pid_from_identifier(identifier: &str) -> Option<u32> {
    log::debug!("[音量控制] 尝试从标识符 '{}' 获取 PID。", identifier);

    // 优先尝试作为可执行文件名进行精确匹配
    if let Some(pid) = get_pid_from_executable_name(identifier) {
        log::trace!(
            "[音量控制] 通过可执行文件名 '{}' 直接找到 PID: {}",
            identifier,
            pid
        );
        return Some(pid);
    }

    // 如果标识符包含 '!'，则尝试作为 AUMID 处理
    if identifier.contains('!') {
        log::debug!(
            "[音量控制] 标识符 '{}' 包含 '!'，尝试作为 AUMID 通过音频会话查找。",
            identifier
        );
        // AUMID 格式通常是 PackageFamilyName!ApplicationId
        let parts: Vec<&str> = identifier.split('!').collect();
        if let Some(package_family_name_from_aumid) = parts.first() {
            // 尝试通过音频会话的 IconPath 匹配 PFN
            match find_pid_for_aumid_via_audio_sessions(package_family_name_from_aumid, identifier)
            {
                Ok(Some(pid)) => {
                    log::debug!(
                        "[音量控制] 通过音频会话为 AUMID '{}' (PFN: {}) 找到 PID: {}",
                        identifier,
                        package_family_name_from_aumid,
                        pid
                    );
                    return Some(pid);
                }
                Ok(None) => {
                    log::warn!(
                        "[音量控制] 无法通过音频会话为 AUMID '{}' 找到匹配的 PID。",
                        identifier
                    );
                }
                Err(e) => {
                    log::error!(
                        "[音量控制] 通过音频会话查找 AUMID '{}' 的 PID 时出错: {}",
                        identifier,
                        e
                    );
                }
            }
        } else {
            log::warn!(
                "[音量控制] AUMID '{}' 格式无效，无法提取 PackageFamilyName。",
                identifier
            );
        }

        // 作为后备：如果 AUMID 的第一部分是 .exe 文件名 (例如一些桌面应用的 AUMID)
        // 例如 "SomeCompany.SomeApp.exe!AppId"
        let exe_candidate_from_aumid = parts.first().unwrap_or(&"");
        if exe_candidate_from_aumid.to_lowercase().ends_with(".exe") {
            log::debug!(
                "[音量控制] AUMID '{}' 的第一部分是可执行文件名 '{}'，再次尝试。",
                identifier,
                exe_candidate_from_aumid
            );
            if let Some(pid) = get_pid_from_executable_name(exe_candidate_from_aumid) {
                log::debug!(
                    "[音量控制] 通过从 AUMID 提取的可执行文件名 '{}' 找到 PID: {}",
                    exe_candidate_from_aumid,
                    pid
                );
                return Some(pid);
            }
        }
    }

    log::warn!("[音量控制] 无法从标识符 '{}' 解析 PID。", identifier);
    None
}

/// 通过可执行文件名获取进程 ID (PID)。
/// 遍历系统中的所有进程，匹配其可执行文件名。
fn get_pid_from_executable_name(executable_name: &str) -> Option<u32> {
    let snapshot_handle = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(handle) if !handle.is_invalid() => handle,
        Ok(_) => {
            // 无效句柄
            log::error!(
                "[音量控制] CreateToolhelp32Snapshot 返回了无效的句柄: {:?}",
                std::io::Error::last_os_error()
            );
            return None;
        }
        Err(e) => {
            log::error!("[音量控制] CreateToolhelp32Snapshot 调用失败: {:?}", e);
            return None;
        }
    };

    // RAII Guard 确保 CloseHandle 在函数退出时被调用
    struct HandleGuard(HANDLE);
    impl Drop for HandleGuard {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                unsafe { CloseHandle(self.0).ok() };
            }
        }
    }
    let _guard = HandleGuard(snapshot_handle);

    let mut process_entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    if unsafe { Process32FirstW(snapshot_handle, &mut process_entry) }.is_err() {
        let err = windows::core::Error::from_win32();
        if err.code() != ERROR_NO_MORE_FILES.to_hresult() {
            log::warn!("[音量控制] Process32FirstW 失败或快照为空: {:?}", err);
        } else {
            log::trace!("[音量控制] Process32FirstW: 快照为空或无法获取第一个进程。");
        }
        return None;
    }

    loop {
        let current_exe_name_wide = &process_entry.szExeFile;
        let len = current_exe_name_wide
            .iter()
            .take_while(|&&c| c != 0)
            .count();
        let current_exe_name_os = OsString::from_wide(&current_exe_name_wide[..len]);

        if let Some(current_exe_name_str) = current_exe_name_os.to_str() {
            if current_exe_name_str.eq_ignore_ascii_case(executable_name) {
                return Some(process_entry.th32ProcessID);
            }
        }

        if unsafe { Process32NextW(snapshot_handle, &mut process_entry) }.is_err() {
            let err = windows::core::Error::from_win32();
            if err.code() != ERROR_NO_MORE_FILES.to_hresult() {
                log::error!("[音量控制] Process32NextW 失败: {:?}", err);
            } else {
                log::trace!("[音量控制] Process32NextW: 没有更多进程了。");
            }
            break;
        }
    }
    None
}

/// 尝试通过分析活动音频会话的 IconPath 来为给定的 PackageFamilyName (源自 AUMID) 找到 PID。
/// 这是针对 UWP 应用等使用 AUMID 的场景。
fn find_pid_for_aumid_via_audio_sessions(
    target_pfn_from_aumid: &str,
    full_aumid_for_log: &str,
) -> Result<Option<u32>, String> {
    log::trace!(
        "[音量控制] find_pid_for_aumid: 目标 PFN='{}', 完整 AUMID='{}'",
        target_pfn_from_aumid,
        full_aumid_for_log
    );
    // 音频会话操作通常需要在单线程单元 (STA) 中执行
    ComInitializer::initialize_com(true)
        .map_err(|e| format!("COM 初始化失败 (STA), AUMID PID : {:?}", e))?;

    unsafe {
        let device_enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| format!("创建 IMMDeviceEnumerator 失败: {:?}", e))?;

        let default_device = device_enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| format!("获取默认音频端点失败: {:?}", e))?;

        let session_manager: IAudioSessionManager2 = default_device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| format!("激活 IAudioSessionManager2 失败: {:?}", e))?;

        let session_enumerator: IAudioSessionEnumerator = session_manager
            .GetSessionEnumerator()
            .map_err(|e| format!("获取 IAudioSessionEnumerator 失败: {:?}", e))?;

        let count = session_enumerator
            .GetCount()
            .map_err(|e| format!("获取会话数量失败: {:?}", e))?;
        log::trace!(
            "[音量控制] find_pid_for_aumid: 系统中发现 {} 个音频会话。",
            count
        );

        for i in 0..count {
            let session_control: IAudioSessionControl = match session_enumerator.GetSession(i) {
                Ok(sc) => sc,
                Err(e) => {
                    log::warn!(
                        "[音量控制] find_pid_for_aumid: 获取会话 {} 失败: {:?}. 跳过。",
                        i,
                        e
                    );
                    continue;
                }
            };

            let session_control2: IAudioSessionControl2 = match session_control.cast() {
                Ok(sc2) => sc2,
                Err(e) => {
                    log::warn!(
                        "[音量控制] find_pid_for_aumid: 转换到 IAudioSessionControl2 失败 (会话 {}): {:?}. 跳过。",
                        i,
                        e
                    );
                    continue;
                }
            };

            // 仅处理活动的音频会话
            if let Ok(current_state) = session_control2.GetState() {
                if current_state == AudioSessionStateActive {
                    // 直接与常量比较
                    if let Ok(pid) = session_control2.GetProcessId() {
                        if pid == 0 {
                            continue;
                        } // 跳过 PID 为 0 的会话 (通常是系统声音)

                        // 尝试从 IconPath 提取 PFN 进行匹配
                        if let Ok(icon_path_pwstr) = session_control2.GetIconPath() {
                            if !icon_path_pwstr.is_null() {
                                match icon_path_pwstr.to_string() {
                                    Ok(icon_path_str_val) if !icon_path_str_val.is_empty() => {
                                        log::trace!(
                                            "[音量控制] find_pid_for_aumid: 会话 PID {}, IconPath: '{}'",
                                            pid,
                                            icon_path_str_val
                                        );
                                        if let Some(pfn_from_icon) =
                                            extract_pfn_from_uwp_icon_path(&icon_path_str_val)
                                        {
                                            log::trace!(
                                                "[音量控制] find_pid_for_aumid: 从 IconPath '{}' 提取的 PFN: '{}'",
                                                icon_path_str_val,
                                                pfn_from_icon
                                            );
                                            if pfn_from_icon
                                                .eq_ignore_ascii_case(target_pfn_from_aumid)
                                            {
                                                log::debug!(
                                                    "[音量控制] find_pid_for_aumid: 找到匹配! AUMID PFN '{}' == IconPath PFN '{}'. PID: {}",
                                                    target_pfn_from_aumid,
                                                    pfn_from_icon,
                                                    pid
                                                );
                                                return Ok(Some(pid));
                                            }
                                        }
                                    }
                                    Ok(_) => log::trace!(
                                        "[音量控制] find_pid_for_aumid: 会话 PID {} 的 IconPath 为空字符串。",
                                        pid
                                    ),
                                    Err(e) => log::warn!(
                                        "[音量控制] find_pid_for_aumid: 转换 IconPath (PID {}) 到 String 失败: {:?}",
                                        pid,
                                        e
                                    ),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(None) // 未找到匹配的 PID
}

/// 从 UWP 应用的 IconPath 字符串中启发式地提取 PackageFamilyName (PFN)。
/// PFN 格式: Name_PublisherId
/// IconPath 示例: "C:\Program Files\WindowsApps\Microsoft.ZuneMusic_11.2202.46.0_x64__8wekyb3d8bbwe\Spotify.exe,-100"
/// 或 "@{C:\Program Files\WindowsApps\Microsoft.ZuneMusic_11.2202.46.0_x64__8wekyb3d8bbwe\resources.pri?..."
/// 目标是提取 "Microsoft.ZuneMusic_8wekyb3d8bbwe"。
fn extract_pfn_from_uwp_icon_path(icon_path: &str) -> Option<String> {
    const WINDOWS_APPS_MARKER: &str = "WindowsApps\\";
    if let Some(start_index) = icon_path.find(WINDOWS_APPS_MARKER) {
        let path_after_marker = &icon_path[start_index + WINDOWS_APPS_MARKER.len()..];
        // PFN 通常是下一级目录名，直到版本号或下一个 '\'
        if let Some(first_slash_after_pfn_part) = path_after_marker.find('\\') {
            let pfn_candidate_full = &path_after_marker[..first_slash_after_pfn_part];
            // pfn_candidate_full 示例: "Microsoft.ZuneMusic_11.2202.46.0_x64__8wekyb3d8bbwe"
            // 我们需要分离 Name 部分和 PublisherId 部分
            // PublisherId 通常是最后一个下划线之后的部分，并且长度较固定 (例如13个字符)
            if let Some(last_underscore_pos) = pfn_candidate_full.rfind('_') {
                let publisher_id_candidate = &pfn_candidate_full[last_underscore_pos + 1..];
                // 粗略检查 PublisherId 的有效性 (例如长度和字符)
                if publisher_id_candidate.len() >= 10
                    && publisher_id_candidate.chars().all(|c| c.is_alphanumeric())
                {
                    // Name_Version_Arch 部分
                    let name_version_arch_part = &pfn_candidate_full[..last_underscore_pos];
                    // 尝试找到 Name 部分 (在版本号之前)
                    // 版本号通常是 X.Y.Z.W 格式，Name 和版本号之间也是下划线
                    // 我们假设 Name 部分不包含连续的数字点数字组合 (版本号特征)
                    // 这是一个启发式方法，可能不完美
                    let mut name_end_pos = last_underscore_pos; // 从最后一个下划线开始往前找
                    if let Some(arch_underscore_pos) = name_version_arch_part.rfind('_') {
                        // 去掉 _x64 或 _arm 等
                        name_end_pos = arch_underscore_pos;
                        if let Some(version_underscore_pos) =
                            name_version_arch_part[..arch_underscore_pos].rfind('_')
                        {
                            // 检查 version_underscore_pos 之后的是否像版本号
                            let version_candidate = &name_version_arch_part
                                [version_underscore_pos + 1..arch_underscore_pos];
                            if version_candidate.chars().any(|c| c == '.')
                                && version_candidate
                                    .chars()
                                    .all(|c| c.is_ascii_digit() || c == '.')
                            {
                                name_end_pos = version_underscore_pos;
                            }
                        }
                    }
                    let name_part = &pfn_candidate_full[..name_end_pos];
                    return Some(format!("{}_{}", name_part, publisher_id_candidate));
                }
            }
        }
    }
    None
}

/// 设置指定 PID 进程的音频会话音量和静音状态。
///
/// # Arguments
/// * `target_pid`: 目标进程的 PID。
/// * `volume_level`: 可选的目标音量级别 (0.0 到 1.0)。
/// * `mute`: 可选的静音状态。
///
/// # Returns
/// `Ok(())` 如果成功设置，否则返回错误信息字符串。
pub fn set_process_volume_by_pid(
    target_pid: u32,
    volume_level: Option<f32>,
    mute: Option<bool>,
) -> Result<(), String> {
    if volume_level.is_none() && mute.is_none() {
        log::trace!(
            "[音量控制] set_process_volume_by_pid: volume_level 和 mute 都为 None，无需操作。"
        );
        return Ok(());
    }
    if let Some(vol_f32) = volume_level {
        if !(0.0..=1.0).contains(&vol_f32) {
            return Err(format!("音量级别 {} (f32) 超出范围 (0.0-1.0)。", vol_f32));
        }
    }

    log::debug!(
        "[音量控制] 尝试为 PID {} 设置音量: {:?}, 静音: {:?}",
        target_pid,
        volume_level,
        mute
    );
    ComInitializer::initialize_com(true)
        .map_err(|e| format!("COM 初始化失败 (STA), set_volume: {}", e))?;

    unsafe {
        let device_enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let default_device = device_enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| e.to_string())?;
        let session_manager: IAudioSessionManager2 = default_device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| e.to_string())?;
        let session_enumerator: IAudioSessionEnumerator = session_manager
            .GetSessionEnumerator()
            .map_err(|e| e.to_string())?;
        let count = session_enumerator.GetCount().map_err(|e| e.to_string())?;

        for i in 0..count {
            let session_control: IAudioSessionControl = match session_enumerator.GetSession(i) {
                Ok(sc) => sc,
                Err(_) => continue,
            };
            let session_control2: IAudioSessionControl2 = match session_control.cast() {
                Ok(sc2) => sc2,
                Err(_) => continue,
            };

            match session_control2.GetProcessId() {
                Ok(pid) if pid == target_pid => {
                    if let Ok(current_state) = session_control2.GetState() {
                        if current_state == windows::Win32::Media::Audio::AudioSessionStateActive {
                            log_session_details_if_relevant(&session_control2, pid, i, target_pid);
                            let simple_audio_volume: ISimpleAudioVolume = match session_control2
                                .cast()
                            {
                                Ok(sav) => sav,
                                Err(e) => {
                                    let err_msg = format!(
                                        "set_volume: 转换到 ISimpleAudioVolume 失败 (PID {}): {}",
                                        pid, e
                                    );
                                    log::error!("[音量控制] {}", err_msg);
                                    return Err(err_msg);
                                }
                            };

                            if let Some(vol_f32) = volume_level {
                                simple_audio_volume
                                    .SetMasterVolume(vol_f32, &GUID::default())
                                    .map_err(|e| e.to_string())?;
                                log::trace!(
                                    "[音量控制] PID {} 的音量已设置为: {} (f32)",
                                    target_pid,
                                    vol_f32
                                );
                            }

                            if let Some(m) = mute {
                                simple_audio_volume
                                    .SetMute(m, &GUID::default())
                                    .map_err(|e| e.to_string())?;
                                log::trace!(
                                    "[音量控制] PID {} 的静音状态已设置为: {}",
                                    target_pid,
                                    m
                                );
                            }
                            return Ok(());
                        } else {
                            log::debug!(
                                "[音量控制] set_volume: PID {} 匹配，但会话状态 {:?} 非活动，跳过。",
                                pid,
                                current_state
                            );
                        }
                    } else {
                        log::warn!("[音量控制] set_volume: 无法获取 PID {} 的会话状态。", pid);
                    }
                }
                Ok(other_pid) => {
                    if i < 3 || i == count - 1 {
                        log_session_details_if_relevant(
                            &session_control2,
                            other_pid,
                            i,
                            target_pid,
                        );
                    }
                }
                Err(e) if e.code() == AUDCLNT_S_NO_CURRENT_PROCESS => {
                    log::trace!("[音量控制] 会话 {} 没有关联的进程。", i);
                }
                Err(e) => {
                    log::warn!("[音量控制] 获取会话 {} 的 PID 失败: {}", i, e);
                }
            }
        }
    }
    Err(format!("未找到 PID {} 对应的会话来设置音量。", target_pid))
}

/// 获取指定 PID 进程的音频会话音量和静音状态。
///
/// # Returns
/// `Ok((volume, is_muted))` 其中 volume 是 f64 (0.0-1.0)，is_muted 是布尔值。
/// 否则返回错误信息字符串。
pub fn get_process_volume_by_pid(target_pid: u32) -> Result<(f32, bool), String> {
    log::debug!("[音量控制] 尝试获取 PID {} 的音量和静音状态。", target_pid);
    ComInitializer::initialize_com(true)
        .map_err(|e| format!("COM 初始化失败 (STA), get_volume: {}", e))?;

    unsafe {
        let device_enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let default_device = device_enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| e.to_string())?;
        let session_manager: IAudioSessionManager2 = default_device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| e.to_string())?;
        let session_enumerator: IAudioSessionEnumerator = session_manager
            .GetSessionEnumerator()
            .map_err(|e| e.to_string())?;
        let count = session_enumerator.GetCount().map_err(|e| e.to_string())?;

        for i in 0..count {
            let session_control: IAudioSessionControl = match session_enumerator.GetSession(i) {
                Ok(sc) => sc,
                Err(_) => continue,
            };
            let session_control2: IAudioSessionControl2 = match session_control.cast() {
                Ok(sc2) => sc2,
                Err(_) => continue,
            };

            if let Ok(pid) = session_control2.GetProcessId() {
                if pid == target_pid {
                    if let Ok(current_state) = session_control2.GetState() {
                        if current_state == windows::Win32::Media::Audio::AudioSessionStateActive {
                            // 使用导入的常量
                            let simple_audio_volume: ISimpleAudioVolume =
                                session_control2.cast().map_err(|e| {
                                    format!("转换到 ISimpleAudioVolume 失败 (PID {}): {}", pid, e)
                                })?;

                            let volume_f32 = simple_audio_volume
                                .GetMasterVolume()
                                .map_err(|e| e.to_string())?; // 直接是 f32
                            let muted = simple_audio_volume
                                .GetMute()
                                .map_err(|e| e.to_string())?
                                .as_bool();

                            log::trace!(
                                "[音量控制] PID {} 的当前音量: {} (f32), 静音状态: {}",
                                target_pid,
                                volume_f32,
                                muted
                            );
                            return Ok((volume_f32, muted)); // 直接返回 f32
                        }
                    }
                }
            }
        }
    }
    Err(format!(
        "未找到 PID {} 对应的活动音频会话来获取音量。",
        target_pid
    ))
}

/// 辅助函数，用于记录音频会话的详细信息（如果相关）。
/// 仅当日志级别允许时，或者会话是目标会话时才记录，以避免日志泛滥。
fn log_session_details_if_relevant(
    session_control2: &IAudioSessionControl2,
    pid: u32,
    index: i32,
    target_pid_context: u32,
) {
    // 仅当是目标 PID 或日志级别足够详细时才记录
    if pid == target_pid_context || log::max_level() >= log::LevelFilter::Trace {
        let display_name_str = unsafe { session_control2.GetDisplayName() }
            .ok()
            .and_then(|pwstr| {
                if pwstr.is_null() {
                    None
                } else {
                    unsafe { pwstr.to_string().ok() }
                }
            })
            .unwrap_or_else(|| "N/A".to_string());

        let icon_path_str = unsafe { session_control2.GetIconPath() }
            .ok()
            .and_then(|pwstr| {
                if pwstr.is_null() {
                    None
                } else {
                    unsafe { pwstr.to_string().ok() }
                }
            })
            .unwrap_or_else(|| "N/A".to_string());

        let state_str = unsafe { session_control2.GetState() }
            .map(|s| format!("{:?}", s))
            .unwrap_or_else(|_| "N/A".to_string());

        let is_target_str = if pid == target_pid_context {
            "[TARGET]"
        } else {
            ""
        };

        log::trace!(
            "[音量控制] 会话索引: {}, PID: {} {}, 显示名称: '{}', 图标路径: '{}', 状态: {}",
            index,
            pid,
            is_target_str,
            display_name_str,
            icon_path_str,
            state_str,
        );
    }
}
