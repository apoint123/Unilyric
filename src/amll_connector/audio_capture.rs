use std::sync::mpsc::Sender as StdSender;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
    IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX,
    WAVEFORMATEXTENSIBLE, eConsole, eRender,
};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::Threading::AvSetMmThreadCharacteristicsW;
use windows::core::HSTRING;
use windows::core::Result as WinResult;

use rubato::{
    ResampleError, Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

use crate::amll_connector::types::ConnectorUpdate;

// --- 常量定义 ---
/// 目标采样率 (单位: Hz)。所有捕获的音频最终会重采样到这个频率。
const TARGET_SAMPLE_RATE: u32 = 48000;
/// 目标声道数。2 代表立体声。
const TARGET_CHANNELS: u16 = 2;
/// WASAPI 内部缓冲区建议的参考时长 (单位: 毫秒)。
const WASAPI_BUFFER_DURATION_MS: u64 = 20;
/// 大约每隔多少毫秒尝试发送一次处理好的音频数据包。
const AUDIO_PACKET_SEND_INTERVAL_MS: u64 = 100;

// --- COM 初始化与反初始化 (RAII Guard) ---

/// `ComThreadInitializer` 结构体用于在当前线程为音频捕获初始化 COM (STA - Single-Threaded Apartment)。
/// 它采用 RAII (Resource Acquisition Is Initialization) 模式，
/// 确保在 Guard 对象离开作用域时自动调用 `CoUninitialize` 来反初始化 COM。
struct ComThreadInitializer;

impl ComThreadInitializer {
    /// 初始化当前线程的 COM 环境。
    ///
    /// # 返回
    /// - `Ok(())` 如果 COM 初始化成功。
    /// - `Err(windows::core::Error)` 如果 COM 初始化失败。
    fn initialize_com() -> WinResult<()> {
        // SAFETY: 调用 Windows API CoInitializeEx。
        // COINIT_APARTMENTTHREADED 表示为当前线程初始化一个单线程套间。
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() }
    }
}

impl Drop for ComThreadInitializer {
    /// 当 `ComThreadInitializer` 对象离开作用域时，自动反初始化 COM。
    fn drop(&mut self) {
        // SAFETY: 调用 Windows API CoUninitialize。
        unsafe {
            CoUninitialize();
        }
        log::trace!("[音频捕获线程] COM (STA) 已通过 RAII Guard 自动反初始化。");
    }
}

// --- 音频捕获器结构体 ---

/// `AudioCapturer` 结构体负责管理音频捕获线程的生命周期和停止信号。
pub struct AudioCapturer {
    /// 音频捕获线程的句柄。`Option` 类型允许在线程未启动或已停止时为 `None`。
    capture_thread_handle: Option<JoinHandle<()>>,
    /// 用于通知捕获线程停止的原子布尔值。
    /// `Arc` 允许多个所有者（主线程和捕获线程）共享这个信号。
    stop_signal: Arc<AtomicBool>,
}

// --- 音频数据处理与发送辅助函数 ---

/// 辅助函数：处理音频数据的声道转换、格式转换 (f32 -> u16)，并通过通道发送。
///
/// 此函数负责将捕获并（可能）重采样后的 `f32` 交错音频数据，
/// 转换为 AMLL Player 期望的 `u16` 格式，并进行必要的声道调整，
/// 最后将字节流通过 `update_tx` 通道发送出去。
///
/// # 参数
/// * `audio_f32_interleaved`: 经过可选重采样后的交错 `f32` 音频数据。
/// * `update_tx`: 用于发送 `ConnectorUpdate::AudioDataPacket` 的通道发送端。
/// * `channels_in_audio_data`: `audio_f32_interleaved` 中每个音频帧包含的声道数。
/// * `target_channels_for_output`: 最终输出数据期望的声道数 (例如 `TARGET_CHANNELS`)。
///
/// # 返回
/// * `Ok(())` 如果处理和发送成功。
/// * `Err(String)` 如果发送失败（例如通道关闭）。
fn process_and_send_audio_data(
    audio_f32_interleaved: Vec<f32>,
    update_tx: &StdSender<ConnectorUpdate>,
    channels_in_audio_data: usize,
    target_channels_for_output: u16,
) -> Result<(), String> {
    if audio_f32_interleaved.is_empty() {
        log::trace!("[音频处理] 接收到空音频数据，无需处理。");
        return Ok(()); // 如果没有数据，则无需处理
    }

    let final_samples_f32_for_conversion: Vec<f32>;

    // 步骤 1: 声道数调整 (如果需要)
    if channels_in_audio_data == target_channels_for_output as usize {
        // 声道数已匹配，无需转换
        final_samples_f32_for_conversion = audio_f32_interleaved;
    } else if channels_in_audio_data > target_channels_for_output as usize
        && target_channels_for_output == 2
    {
        log::debug!(
            "[音频处理] 将 {} 声道音频数据降混为 {} 声道。",
            channels_in_audio_data,
            target_channels_for_output
        );
        final_samples_f32_for_conversion = audio_f32_interleaved
            .chunks_exact(channels_in_audio_data) // 按原始帧 (包含所有声道) 分割
            .flat_map(|frame| {
                // 对于每个帧，只取目标声道数的样本
                frame
                    .iter()
                    .take(target_channels_for_output as usize)
                    .copied() // 复制样本值
            })
            .collect();
    } else if channels_in_audio_data == 1 && target_channels_for_output == 2 {
        // 从单声道复制到立体声 (每个单声道样本复制一次以填充左右声道)
        log::debug!(
            "[音频处理] 将 {} 声道音频数据扩展为 {} 声道。",
            channels_in_audio_data,
            target_channels_for_output
        );
        final_samples_f32_for_conversion = audio_f32_interleaved
            .iter()
            .flat_map(|&sample| [sample, sample]) // 每个单声道样本复制一次
            .collect();
    } else {
        log::warn!(
            "[音频处理] 不支持的声道转换: 从 {} 声道到 {} 声道。将直接使用原始声道数据。",
            channels_in_audio_data,
            target_channels_for_output,
        );
        final_samples_f32_for_conversion = audio_f32_interleaved;
    }

    if !final_samples_f32_for_conversion.is_empty() {
        // 步骤 2: 将 f32 样本 (-1.0 到 1.0) 转换为 u16 样本 (0 到 65535)
        // AMLL Player 通常期望的是 PCM u16 格式。
        let audio_data_u16: Vec<u16> = final_samples_f32_for_conversion
            .iter()
            .map(|&sample_f32| {
                // a. 将 f32 样本从 [-1.0, 1.0] 裁剪并映射到 [0.0, 1.0]
                //    `clamp` 用于防止意外超出范围的 f32 值导致转换错误
                let normalized_sample = (sample_f32.clamp(-1.0, 1.0) + 1.0) / 2.0;
                // b. 将 [0.0, 1.0] 范围的值乘以 u16 的最大值 (65535.0)
                let scaled_sample = normalized_sample * (u16::MAX as f32);
                // c. 四舍五入并转换为 u16
                scaled_sample.round() as u16
            })
            .collect();

        // 步骤 3: 将 u16 样本转换为字节流 (Vec<u8>)，使用小端序 (Little-Endian)
        let audio_data_bytes: Vec<u8> = audio_data_u16
            .iter()
            .flat_map(|&sample_u16| sample_u16.to_le_bytes()) // 每个 u16 转为2字节 (小端序)
            .collect();

        // 步骤 4: 通过通道发送数据包
        if update_tx
            .send(ConnectorUpdate::AudioDataPacket(audio_data_bytes))
            .is_err()
        {
            let err_msg = "[音频处理] 发送音频数据包失败。通道可能已关闭，捕获停止。".to_string();
            log::error!("{}", err_msg);
            return Err(err_msg); // 返回错误，指示发送失败，这通常会导致捕获循环终止
        }
    }
    Ok(())
}

// --- AudioCapturer 实现 ---
impl AudioCapturer {
    /// 创建一个新的 `AudioCapturer` 实例。
    /// 此时捕获线程尚未启动。
    pub fn new() -> Self {
        AudioCapturer {
            capture_thread_handle: None,
            stop_signal: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 启动音频捕获线程。
    /// 如果线程已在运行，则此方法直接返回 `Ok(())`。
    ///
    /// # 参数
    /// * `update_tx`: 用于将捕获到的音频数据发送到其他部分的通道发送端。
    ///
    /// # 返回
    /// * `Ok(())` 如果线程成功启动或已在运行。
    /// * `Err(String)` 如果启动线程失败。
    pub fn start_capture(&mut self, update_tx: StdSender<ConnectorUpdate>) -> Result<(), String> {
        if self.capture_thread_handle.is_some() {
            log::debug!("[音频捕获器] 捕获线程已在运行，无需重复启动。");
            return Ok(());
        }

        log::debug!("[音频捕获器] 准备启动音频捕获线程...");
        self.stop_signal.store(false, Ordering::Relaxed); // 重置停止信号，确保新线程不会立即停止
        let stop_signal_clone = Arc::clone(&self.stop_signal);

        // 创建并配置捕获线程
        let thread_builder = thread::Builder::new().name("audio_capture_thread".to_string());
        self.capture_thread_handle = Some(
            thread_builder.spawn(move || {
                log::debug!("[音频捕获线程] 音频捕获线程已启动 (ID: {:?})。", thread::current().id());

                // 初始化当前线程的 COM 环境。这是与 Windows 音频 API 交互所必需的。
                // 使用 RAII Guard 确保 COM 在线程退出时被正确反初始化。
                let _com_guard = match ComThreadInitializer::initialize_com() {
                    Ok(_) => {
                        log::trace!("[音频捕获线程] COM (STA) 初始化成功。");
                        ComThreadInitializer // 返回 Guard 对象
                    }
                    Err(e) => {
                        log::error!("[音频捕获线程] COM (STA) 初始化失败: {:?}。线程即将退出。", e);
                        // 尝试通过发送一个空数据包来通知主工作线程 COM 初始化失败
                        if update_tx.send(ConnectorUpdate::AudioDataPacket(Vec::new())).is_err(){
                             log::error!("[音频捕获线程] 发送空包以通知COM初始化失败时出错。");
                        };
                        return; // COM 初始化失败则线程无法继续执行捕获逻辑
                    }
                };

                // 尝试提升线程优先级，以获得更稳定的音频捕获性能。
                // "Pro Audio" 特性通常用于需要低延迟和高优先级的音频处理任务。
                // SAFETY: 调用 Windows API AvSetMmThreadCharacteristicsW。
                unsafe {
                    let task_name_hstring = HSTRING::from("Pro Audio");
                    let mut task_index = 0u32;
                    if AvSetMmThreadCharacteristicsW(&task_name_hstring, &mut task_index).is_err() {
                        log::warn!("[音频捕获线程] 无法设置线程特性为 'Pro Audio'。可能需要管理员权限或特定服务正在运行。");
                    } else {
                        log::debug!("[音频捕获线程] 线程特性已成功设置为 'Pro Audio' (任务索引: {}).", task_index);
                    }
                }

                // 执行核心的捕获循环逻辑
                if let Err(e) = AudioCapturer::capture_loop_impl(stop_signal_clone, update_tx) {
                    log::error!("[音频捕获线程] 捕获循环遇到错误: {}。线程即将退出。", e);
                }
                log::debug!("[音频捕获线程] 音频捕获线程已结束 (ID: {:?})。", thread::current().id());
            })
            .map_err(|e| format!("无法启动音频捕获线程: {:?}", e))?,
        );
        log::debug!("[音频捕获器] 音频捕获线程已成功请求启动。");
        Ok(())
    }

    /// 请求停止音频捕获线程并等待其结束。
    /// 如果线程未在运行，则此方法不执行任何操作。
    pub fn stop_capture(&mut self) {
        if let Some(handle) = self.capture_thread_handle.take() {
            log::debug!("[音频捕获器] 正在请求停止音频捕获线程...");
            self.stop_signal.store(true, Ordering::Relaxed); // 设置停止信号

            // 等待线程结束
            log::debug!(
                "[音频捕获器] 等待音频捕获线程 (ID: {:?}) 结束...",
                handle.thread().id()
            );
            match handle.join() {
                Ok(_) => log::debug!("[音频捕获器] 音频捕获线程已成功停止。"),
                Err(e) => log::error!("[音频捕获器] 等待音频捕获线程结束时发生错误: {:?}", e),
            }
        } else {
            log::debug!("[音频捕获器] 音频捕获线程未在运行或已被请求停止。");
        }
    }

    /// 音频捕获的核心逻辑实现，此方法在单独的捕获线程中运行。
    ///
    /// ## 安全性 (Safety)
    /// 此函数包含大量对 Windows COM API 的不安全调用，这些调用需要满足特定的前提条件，
    /// 例如正确的 COM 初始化和线程模型。
    #[allow(clippy::too_many_lines)] // 允许函数体较长，因为音频捕获逻辑本身比较复杂
    fn capture_loop_impl(
        stop_signal: Arc<AtomicBool>,
        update_tx: StdSender<ConnectorUpdate>,
    ) -> Result<(), String> {
        // 所有 WASAPI 调用都应在 unsafe 块内，因为它们是 FFI 调用。
        unsafe {
            // --- 步骤 1: 初始化 WASAPI 组件 ---
            log::debug!("[音频捕获线程] 开始初始化 WASAPI 组件...");

            // 创建设备枚举器实例，用于列举音频端点设备。
            let device_enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| format!("创建 MMDeviceEnumerator 实例失败: {}", e.message()))?;
            log::trace!("[音频捕获线程] MMDeviceEnumerator 实例创建成功。");

            // 获取默认的音频渲染端点 (通常是系统扬声器或耳机)。
            let default_device = device_enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| format!("获取默认音频渲染端点失败: {}", e.message()))?;
            log::trace!("[音频捕获线程] 获取默认音频渲染端点成功。");

            // 从默认设备激活 IAudioClient 接口，这是与音频引擎交互的主要接口。
            let audio_client: IAudioClient = default_device
                .Activate(CLSCTX_INPROC_SERVER, None)
                .map_err(|e| format!("激活 IAudioClient 接口失败: {}", e.message()))?;
            log::trace!("[音频捕获线程] IAudioClient 接口激活成功。");

            // 获取系统混音器的输出格式 (即我们将要捕获的音频流的格式)。
            // 这个格式由系统决定，通常是高质量的浮点格式。
            let wave_format_ptr = audio_client
                .GetMixFormat()
                .map_err(|e| format!("获取混音格式失败: {}", e.message()))?;

            // 确保 GetMixFormat 返回的指针有效。
            if wave_format_ptr.is_null() {
                return Err("GetMixFormat 返回了空指针，无法获取音频格式。".to_string());
            }

            // 使用 RAII Guard 管理 wave_format_ptr 指向的内存。
            // CoTaskMemFree 用于释放由 COM 分配的内存。
            struct WaveFormatGuard(*mut WAVEFORMATEX);
            impl Drop for WaveFormatGuard {
                fn drop(&mut self) {
                    if !self.0.is_null() {
                        // SAFETY: self.0 是从 COM API 获取的有效指针。
                        unsafe {
                            windows::Win32::System::Com::CoTaskMemFree(Some(self.0 as *const _));
                        }
                        log::trace!("[音频捕获线程] WaveFormat 内存已通过 RAII Guard 自动释放。");
                    }
                }
            }
            let _format_guard = WaveFormatGuard(wave_format_ptr); // Guard 对象生命周期结束时自动释放内存

            // 解引用指针以获取 WAVEFORMATEX 结构的引用。
            // SAFETY: wave_format_ptr 已检查非空，并且其生命周期由 _format_guard 管理。
            let wave_format: &WAVEFORMATEX = &*wave_format_ptr;

            // 记录原始捕获格式的详细信息。
            let original_sample_rate = wave_format.nSamplesPerSec;
            let original_channels_u16 = wave_format.nChannels;
            let original_channels_usize = original_channels_u16 as usize; // 方便后续使用
            let original_bits_per_sample = wave_format.wBitsPerSample;
            let original_format_tag = wave_format.wFormatTag;
            let original_block_align = wave_format.nBlockAlign; // 每个音频帧的字节数

            log::debug!(
                "[音频捕获线程] 原始捕获格式: 采样率={}Hz, 声道数={}, 位深度={}, 格式标签={}, 块对齐={}",
                original_sample_rate,
                original_channels_usize,
                original_bits_per_sample,
                original_format_tag,
                original_block_align
            );

            // --- 步骤 2: 校验原始音频格式 ---
            // 我们期望捕获到的是 32 位 IEEE 浮点数格式的音频，因为这是现代 Windows 系统混音器常用的高质量格式，
            // 并且便于后续的数字信号处理（如重采样）。
            let is_source_float = (original_format_tag as u32 == WAVE_FORMAT_IEEE_FLOAT) || // 标准浮点格式
                                  (original_format_tag as u32 == WAVE_FORMAT_EXTENSIBLE && // 可扩展格式
                                   wave_format.cbSize >= (std::mem::size_of::<WAVEFORMATEXTENSIBLE>() - std::mem::size_of::<WAVEFORMATEX>()) as u16 && // 确保有足够的扩展数据
                                   { // 检查可扩展格式的子格式是否为 IEEE Float
                                       // SAFETY: 我们已检查 cbSize，可以安全地将指针转换为 WAVEFORMATEXTENSIBLE。
                                       let wf_ext_ptr = wave_format_ptr as *const WAVEFORMATEXTENSIBLE;
                                       // 获取 SubFormat 字段的指针。
                                       // 使用 `std::ptr::addr_of!` 更安全，避免未对齐的引用。
                                       let sub_format_field_ptr = std::ptr::addr_of!((*wf_ext_ptr).SubFormat);
                                       // 非对齐读取 GUID。
                                       let sub_format_guid = std::ptr::read_unaligned(sub_format_field_ptr);
                                       sub_format_guid == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT // 与标准 IEEE Float 子格式 GUID 比较
                                   });

            if !is_source_float || original_bits_per_sample != 32 {
                let err_msg = format!(
                    "不支持的原始音频格式: 标签={}, 位深度={}. 程序期望捕获 32 位 IEEE 浮点数格式的音频。",
                    original_format_tag, original_bits_per_sample
                );
                log::error!("{}", err_msg);
                return Err(err_msg);
            }

            // --- 步骤 3: 初始化 IAudioClient (环回模式) ---
            // `hns_buffer_duration` 是以 100 纳秒为单位的缓冲区持续时间。
            let hns_buffer_duration: i64 =
                Duration::from_millis(WASAPI_BUFFER_DURATION_MS).as_nanos() as i64 / 100;
            audio_client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,     // 共享模式，允许其他应用同时使用音频设备。
                    AUDCLNT_STREAMFLAGS_LOOPBACK, // 环回捕获标志，用于捕获系统播放的音频。
                    hns_buffer_duration,          // 期望的缓冲区持续时间。
                    0,                            // 周期性 (对于共享模式，通常设置为0)。
                    wave_format_ptr,              // 使用之前获取到的混音格式。
                    None,                         // 无特定会话 GUID。
                )
                .map_err(|e| format!("IAudioClient Initialize (环回模式) 失败: {}", e.message()))?;
            log::debug!(
                "[音频捕获线程] IAudioClient 初始化成功 (环回模式，缓冲区时长约 {}ms)。",
                WASAPI_BUFFER_DURATION_MS
            );

            // --- 步骤 4: 获取 IAudioCaptureClient 服务 ---
            // IAudioCaptureClient 接口用于从捕获端点缓冲区读取数据。
            let capture_client: IAudioCaptureClient = audio_client
                .GetService()
                .map_err(|e| format!("获取 IAudioCaptureClient 服务失败: {}", e.message()))?;
            log::trace!("[音频捕获线程] IAudioCaptureClient 服务获取成功。");

            // --- 步骤 5: 启动音频流 ---
            audio_client
                .Start()
                .map_err(|e| format!("启动音频流 (IAudioClient Start) 失败: {}", e.message()))?;
            log::debug!(
                "[音频捕获线程] 音频捕获流已启动。目标输出格式: {}Hz, {}声道 (u16 PCM)。",
                TARGET_SAMPLE_RATE,
                TARGET_CHANNELS
            );

            // --- 步骤 6: 初始化重采样器 (如果需要) ---
            let mut resampler: Option<SincFixedIn<f32>> = None;
            let mut resampler_input_chunk_size_frames: usize = 0; // 重采样器期望的输入块大小 (单位: 帧)

            if original_sample_rate != TARGET_SAMPLE_RATE {
                log::debug!(
                    "[音频捕获线程] 检测到采样率不匹配，需要进行重采样: {}Hz -> {}Hz (声道数: {})",
                    original_sample_rate,
                    TARGET_SAMPLE_RATE,
                    original_channels_usize
                );
                // 配置重采样器参数
                let params = SincInterpolationParameters {
                    sinc_len: 128, // Sinc 函数长度，影响重采样质量和计算量。较大的值通常质量更好但计算更密集。
                    f_cutoff: 0.95, // 截止频率因子，用于控制低通滤波器的截止频率。
                    interpolation: SincInterpolationType::Linear, // 插值类型。
                    oversampling_factor: 128, // 过采样因子，影响精度。较高的值可以减少混叠。
                    window: WindowFunction::Hann, // 窗函数，用于平滑 Sinc 函数的边缘。
                };

                // 估算一个初始的块大小给重采样器。
                // 目标是大约 AUDIO_PACKET_SEND_INTERVAL_MS 毫秒的数据量。
                let estimated_frames_per_interval = (original_sample_rate as f64
                    * (AUDIO_PACKET_SEND_INTERVAL_MS as f64 / 1000.0))
                    as usize;
                // 确保块大小至少是 Sinc 长度的两倍，这是 rubato 的一个常见建议。
                let initial_chunk_size_for_resampler =
                    estimated_frames_per_interval.max(params.sinc_len * 2);

                let rs_instance = SincFixedIn::<f32>::new(
                    TARGET_SAMPLE_RATE as f64 / original_sample_rate as f64, // 重采样比率
                    2.0, // 抗锯齿滤波器的带宽限制因子 (通常为2.0，对于高质量可以略微减小)
                    params,
                    initial_chunk_size_for_resampler, // 提供给重采样器的初始期望块大小 (帧)
                    original_channels_usize,          // 输入音频的声道数
                )
                .map_err(|e| format!("创建 Rubato 重采样器失败: {:?}", e))?;

                // 获取重采样器实际期望的下一个输入块的大小 (帧数)
                resampler_input_chunk_size_frames = rs_instance.input_frames_next();
                log::debug!(
                    "[音频捕获线程] Rubato 重采样器初始化成功。期望输入块大小: {} 帧。",
                    resampler_input_chunk_size_frames
                );
                resampler = Some(rs_instance);
            } else {
                log::debug!(
                    "[音频捕获线程] 原始采样率与目标采样率一致 ({_samplerate}Hz)，无需重采样。",
                    _samplerate = original_sample_rate
                );
            }

            // --- 步骤 7: 准备重采样过程所需的缓冲区 ---
            // `accumulated_audio_planar`: 用于累积从 WASAPI 获取并已去交错的音频数据。
            // 每个内部 Vec 代表一个声道的数据。
            let mut accumulated_audio_planar: Vec<Vec<f32>> =
                vec![Vec::new(); original_channels_usize];

            // `resampler_input_chunk_planar`: 用于存放一个固定大小 (resampler_input_chunk_size_frames) 的输入块给重采样器。
            // 同样是平面格式。
            let mut resampler_input_chunk_planar: Vec<Vec<f32>> =
                vec![vec![0.0f32; resampler_input_chunk_size_frames]; original_channels_usize];

            // `resampler_output_chunk_planar`: 用于存放重采样器的输出块。
            // 大小根据重采样器可能产生的最大输出帧数确定。
            let mut resampler_output_chunk_planar: Vec<Vec<f32>> = if let Some(rs) = &resampler {
                vec![vec![0.0f32; rs.output_frames_max()]; original_channels_usize]
            } else {
                Vec::new() // 如果不进行重采样，则此缓冲区不需要。
            };

            // --- 步骤 8: 主捕获循环 ---
            log::debug!("[音频捕获线程] 进入主捕获循环...");
            while !stop_signal.load(Ordering::Relaxed) {
                // 循环直到收到外部的停止信号。

                // 短暂休眠以控制轮询 WASAPI 的频率，避免过于频繁的空轮询消耗过多 CPU。
                // 休眠时间可以根据实际需求调整，通常是发送间隔的一小部分。
                thread::sleep(Duration::from_millis(AUDIO_PACKET_SEND_INTERVAL_MS / 5)); // 例如，每 10ms 检查一次

                // 检查下一个可用数据包中的帧数。
                let packet_length_frames = capture_client.GetNextPacketSize().map_err(|e| {
                    format!(
                        "获取下一数据包大小 (GetNextPacketSize) 失败: {}",
                        e.message()
                    )
                })?;

                if packet_length_frames == 0 {
                    continue; // 没有新数据，继续下一次轮询。
                }
                log::trace!(
                    "[音频捕获线程] 检测到 {} 帧可用数据。",
                    packet_length_frames
                );

                // 获取音频数据缓冲区。
                let mut p_data: *mut u8 = std::ptr::null_mut(); // 指向捕获数据的指针
                let mut num_frames_actually_captured: u32 = 0; // WASAPI 实际捕获的帧数
                let mut dw_flags: u32 = 0; // 缓冲区的状态标志

                capture_client
                    .GetBuffer(
                        &mut p_data,                       // [out] 数据指针
                        &mut num_frames_actually_captured, // [out] 实际捕获的帧数
                        &mut dw_flags,                     // [out] 缓冲区状态标志
                        None,                              // [in, optional] 设备位置 (通常为 None)
                        None, // [in, optional] QPC 时间戳 (通常为 None)
                    )
                    .map_err(|e| {
                        format!("从捕获端点获取缓冲区 (GetBuffer) 失败: {}", e.message())
                    })?;

                // 检查是否为静音数据包。如果是，则忽略它。
                if dw_flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 {
                    log::trace!(
                        "[音频捕获线程] 收到静音数据包 ({} 帧)，已忽略。",
                        num_frames_actually_captured
                    );
                    // 即使是静音数据，也需要释放缓冲区。
                    capture_client
                        .ReleaseBuffer(num_frames_actually_captured)
                        .map_err(|e| {
                            format!("释放静音数据缓冲区 (ReleaseBuffer) 失败: {}", e.message())
                        })?;
                    continue;
                }

                if num_frames_actually_captured > 0 && !p_data.is_null() {
                    // a. 将捕获的原始字节数据 (p_data) 转换为 Vec<f32> (交错样本)
                    //    原始数据是 32-bit IEEE Float 格式。
                    let bytes_per_sample = (original_bits_per_sample / 8) as usize; // 每个样本的字节数 (应为 4)
                    let bytes_per_frame = original_channels_usize * bytes_per_sample; // 每个完整帧的字节数
                    // SAFETY: p_data 是从 GetBuffer 获取的有效指针，num_frames_actually_captured 是有效帧数。
                    let captured_bytes_slice = std::slice::from_raw_parts(
                        p_data,
                        num_frames_actually_captured as usize * bytes_per_frame,
                    );

                    let mut captured_f32_interleaved: Vec<f32> = Vec::with_capacity(
                        num_frames_actually_captured as usize * original_channels_usize,
                    );
                    // 将字节切片按帧分割，然后按样本解析
                    for frame_bytes in captured_bytes_slice.chunks_exact(bytes_per_frame) {
                        for channel_index in 0..original_channels_usize {
                            let sample_start_offset = channel_index * bytes_per_sample;
                            let sample_end_offset = sample_start_offset + bytes_per_sample;
                            let sample_bytes = &frame_bytes[sample_start_offset..sample_end_offset];
                            // 假设是小端序 (Little-Endian) 的 f32
                            captured_f32_interleaved.push(f32::from_le_bytes(
                                sample_bytes.try_into().map_err(|_| {
                                    "字节切片转换为f32样本失败：长度不匹配".to_string()
                                })?,
                            ));
                        }
                    }
                    log::trace!(
                        "[音频捕获线程] 成功捕获并转换 {} 帧数据到 f32 交错格式。",
                        num_frames_actually_captured
                    );

                    let mut data_to_send_this_cycle_interleaved: Vec<f32> = Vec::new();

                    // b. 如果需要重采样，则处理数据
                    if let Some(ref mut active_resampler) = resampler {
                        // i. 去交错新捕获的数据，并追加到累积缓冲区 `accumulated_audio_planar`
                        for (sample_index, &sample_value) in
                            captured_f32_interleaved.iter().enumerate()
                        {
                            accumulated_audio_planar[sample_index % original_channels_usize]
                                .push(sample_value);
                        }
                        log::trace!(
                            "[音频捕获线程] {} 帧新数据已去交错并追加到累积缓冲区。",
                            num_frames_actually_captured
                        );

                        // ii. 循环处理累积缓冲区中所有完整的块，直到累积数据不足一个完整块
                        while accumulated_audio_planar.iter().all(|channel_buffer| {
                            channel_buffer.len() >= resampler_input_chunk_size_frames
                        }) {
                            // 从累积缓冲区 `accumulated_audio_planar` 中取出 `resampler_input_chunk_size_frames` 帧数据
                            // 填充到 `resampler_input_chunk_planar` 中。
                            for channel_idx in 0..original_channels_usize {
                                // Drain 从开头移除元素，并返回一个迭代器
                                let drained_samples: Vec<f32> = accumulated_audio_planar
                                    [channel_idx]
                                    .drain(0..resampler_input_chunk_size_frames)
                                    .collect();
                                resampler_input_chunk_planar[channel_idx]
                                    .copy_from_slice(&drained_samples);
                            }

                            // 准备输出缓冲区的可变引用切片
                            let mut output_slices_mut: Vec<&mut [f32]> =
                                resampler_output_chunk_planar
                                    .iter_mut()
                                    .map(|vec_channel_data| vec_channel_data.as_mut_slice())
                                    .collect();
                            // 准备输入缓冲区的不可变引用切片
                            let input_slices_ref: Vec<&[f32]> = resampler_input_chunk_planar
                                .iter()
                                .map(|vec_channel_data| vec_channel_data.as_slice())
                                .collect();

                            // 执行重采样
                            match active_resampler.process_into_buffer(
                                &input_slices_ref,      // 输入平面数据
                                &mut output_slices_mut, // 输出平面数据
                                None,                   // 无特定处理选项
                            ) {
                                Ok((_frames_read_by_resampler, frames_written_by_resampler)) => {
                                    // 将重采样后的数据（从平面转为交错）追加到 data_to_send_this_cycle_interleaved
                                    for frame_idx in 0..frames_written_by_resampler {
                                        for channel_data_vec in resampler_output_chunk_planar.iter()
                                        {
                                            data_to_send_this_cycle_interleaved
                                                .push(channel_data_vec[frame_idx]);
                                        }
                                    }
                                    log::trace!(
                                        "[音频捕获线程] 重采样一个块: {} 输入帧 -> {} 输出帧。",
                                        resampler_input_chunk_size_frames, // _frames_read_by_resampler 应该等于此值
                                        frames_written_by_resampler
                                    );
                                }
                                Err(e) => {
                                    log::error!(
                                        "[音频捕获线程] 重采样处理失败 (块处理): {:?}。将清空累积数据并跳过此块。",
                                        e
                                    );
                                    // 发生错误时，清空所有累积数据以避免错误传播或处理不一致的数据。
                                    for channel_buffer in accumulated_audio_planar.iter_mut() {
                                        channel_buffer.clear();
                                    }
                                    break; // 跳出当前块处理循环 (while)，尝试下一次 GetBuffer
                                }
                            }
                        } // 结束处理累积缓冲区的循环
                    } else {
                        // 不需要重采样，直接使用捕获到的交错数据
                        data_to_send_this_cycle_interleaved = captured_f32_interleaved;
                    }

                    // c. 发送当前处理周期内收集到的所有数据
                    //    此时 data_to_send_this_cycle_interleaved 中的声道数是原始捕获的声道数 (original_channels_usize)
                    //    或者，如果重采样了，声道数仍然是 original_channels_usize (因为重采样器是按原始声道数初始化的)
                    //    process_and_send_audio_data 内部会处理到 TARGET_CHANNELS 的转换。
                    if !data_to_send_this_cycle_interleaved.is_empty() {
                        if let Err(e_send) = process_and_send_audio_data(
                            data_to_send_this_cycle_interleaved,
                            &update_tx,
                            original_channels_usize, // 传递给辅助函数的声道数
                            TARGET_CHANNELS,         // 目标输出声道数
                        ) {
                            // 如果发送失败 (例如通道关闭)，则释放缓冲区并退出线程
                            capture_client
                                .ReleaseBuffer(num_frames_actually_captured)
                                .map_err(|e_rel| {
                                    format!(
                                        "释放缓冲区失败 (因发送错误导致退出): {}",
                                        e_rel.message()
                                    )
                                })?;
                            return Err(e_send); // 向上层报告发送错误
                        }
                        log::trace!("[音频捕获线程] 成功发送一批处理后的音频数据。");
                    }
                } // 结束 if num_frames_actually_captured > 0

                // 释放 WASAPI 缓冲区，使其可供下一次捕获使用。
                capture_client
                    .ReleaseBuffer(num_frames_actually_captured)
                    .map_err(|e| {
                        format!("释放 WASAPI 缓冲区 (ReleaseBuffer) 失败: {}", e.message())
                    })?;
            } // 主捕获循环 (while !stop_signal) 结束

            // --- 步骤 9: 处理流末尾 ---
            // 当捕获循环结束后 (收到停止信号)，处理重采样器中可能剩余的最后一部分数据。
            log::debug!("[音频捕获线程] 主捕获循环已结束，准备处理流末尾数据...");
            if let Some(ref mut active_resampler) = resampler {
                // 检查累积缓冲区是否还有未处理的数据
                if !accumulated_audio_planar.is_empty()
                    && accumulated_audio_planar // 确保至少有一个声道的累积缓冲区非空
                        .iter()
                        .any(|channel_buffer| !channel_buffer.is_empty())
                {
                    let first_channel_len = accumulated_audio_planar[0].len();
                    log::debug!(
                        "[音频捕获线程] 处理流末尾，累积缓冲区中剩余音频数据 (例如，声道0有 {} 样本)...",
                        first_channel_len
                    );

                    // 准备输入和输出切片给重采样器
                    let remaining_input_slices: Vec<&[f32]> = accumulated_audio_planar
                        .iter()
                        .map(|vec_channel_data| vec_channel_data.as_slice())
                        .collect();
                    let mut output_slices_mut: Vec<&mut [f32]> = resampler_output_chunk_planar
                        .iter_mut()
                        .map(|vec_channel_data| vec_channel_data.as_mut_slice())
                        .collect();

                    // 使用 process_into_buffer 处理最后的数据块。
                    // 注意：如果剩余数据少于重采样器期望的 input_frames_next()，
                    // SincFixedIn 可能无法处理或返回错误。更稳健的做法是填充静音数据到期望长度，
                    // 或者使用 process_last_into_buffer (如果 rubato 版本支持且适用)。
                    // 当前的 rubato::SincFixedIn::process_into_buffer 需要足够的输入。
                    // 如果不足，它可能会返回 ResampleError::InsufficientInputBufferSize。
                    match active_resampler.process_into_buffer(
                        &remaining_input_slices,
                        &mut output_slices_mut,
                        None, // 或者 Some(true) 来指示这是最后一块数据，如果API支持
                    ) {
                        Ok((frames_read_by_resampler, frames_written_by_resampler)) => {
                            log::debug!(
                                "[音频捕获线程] 流末尾数据重采样完成: {} 帧读取, {} 帧写入。",
                                frames_read_by_resampler,
                                frames_written_by_resampler
                            );

                            if frames_read_by_resampler != first_channel_len
                                && first_channel_len > 0
                            {
                                log::warn!(
                                    "[音频捕获线程] 流末尾重采样读取的帧数 ({}) 与提供的帧数 ({}) 不匹配。这可能发生在输入不足一个完整块时。",
                                    frames_read_by_resampler,
                                    first_channel_len
                                );
                            }

                            if frames_written_by_resampler > 0 {
                                let mut last_resampled_data_interleaved: Vec<f32> =
                                    Vec::with_capacity(
                                        frames_written_by_resampler * original_channels_usize,
                                    );
                                for frame_idx in 0..frames_written_by_resampler {
                                    for channel_data_vec in resampler_output_chunk_planar.iter() {
                                        last_resampled_data_interleaved
                                            .push(channel_data_vec[frame_idx]);
                                    }
                                }
                                // 发送最后处理的数据
                                log::debug!(
                                    "[音频捕获线程] 发送流末尾最后 {} 帧重采样数据...",
                                    frames_written_by_resampler
                                );
                                process_and_send_audio_data(
                                    last_resampled_data_interleaved,
                                    &update_tx,
                                    original_channels_usize,
                                    TARGET_CHANNELS,
                                )? // 如果发送失败，则传播错误
                            } else {
                                log::debug!("[音频捕获线程] 流末尾重采样未产生输出帧。");
                            }
                        }
                        Err(ResampleError::InsufficientInputBufferSize {
                            channel,
                            expected,
                            actual,
                        }) => {
                            // 这种情况在处理流末尾时很可能发生，因为剩余数据可能不足一个块。
                            log::warn!(
                                "[音频捕获线程] 处理流末尾剩余数据重采样时，输入缓冲区大小不足 (声道 {}, 期望 {}, 实际 {})。部分末尾数据可能未被处理。",
                                channel,
                                expected,
                                actual
                            );
                        }
                        Err(e) => {
                            log::error!(
                                "[音频捕获线程] 处理流末尾剩余数据时发生重采样错误 (其他类型): {:?}。部分末尾数据可能未被处理。",
                                e
                            );
                        }
                    }
                } else {
                    log::debug!("[音频捕获线程] 流末尾，累积缓冲区为空，无需额外处理。");
                }
            }

            // --- 步骤 10: 停止音频客户端并清理 ---
            log::debug!("[音频捕获线程] 停止信号已收到或发生发送错误，正在停止音频客户端...");
            audio_client
                .Stop()
                .map_err(|e| format!("停止音频客户端 (IAudioClient Stop) 失败: {}", e.message()))?;
            log::trace!("[音频捕获线程] 音频客户端已成功停止。");
            Ok(())
        } // unsafe 块结束
    }
}

// --- AudioCapturer 的 Drop 实现 ---

/// `AudioCapturer` 的 `Drop` 实现，确保在 `AudioCapturer` 对象被丢弃时，
/// 音频捕获线程会被正确地请求停止并等待其结束。
/// 这是为了防止线程泄漏或不正确的资源释放。
impl Drop for AudioCapturer {
    fn drop(&mut self) {
        log::debug!("[音频捕获器] AudioCapturer 实例正在被 Drop，将确保停止捕获线程...");
        self.stop_capture(); // 调用 stop_capture 来处理线程的停止和清理
    }
}
