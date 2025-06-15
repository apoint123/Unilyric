//
//
//
//  AI 创作声明
//
//  大部分注释是使用 AI 生成的，虽然直接用 AI 来写核心逻辑还不太行，但用来写注释真是太合适不过了
//  大部分辅助的函数和逻辑也是由 AI 生成的，这上面 AI 还是写得比我好
//
//

// 这行是一个条件编译属性。
// 它表示：如果不是在调试模式下编译 (debug_assertions 为 false),
// 那么将使用 "windows" 子系统。这通常用于在 Windows 上创建没有控制台窗口的 GUI 应用程序。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// 模块声明区域
// Rust 使用 `mod` 关键字来声明模块。每个模块通常对应一个文件或一个目录。
// 这些模块包含了应用程序的不同功能部分。

mod types; // 定义应用程序中使用的各种数据类型。
#[macro_use] // 这个属性表示 `utils` 模块中定义的宏将被导入到当前作用域，可以直接使用。
mod utils; // 包含一些工具函数或宏，例如 `log_info!` 可能在这里定义。
mod amll_connector;
mod amll_lyrics_fetcher;
mod app; // 包含应用程序核心逻辑和 UI 定义的模块。
mod app_definition;
pub mod app_fetch_core;
mod app_settings; // 用于管理应用程序设置的模块。
mod app_ui; // 包含应用程序用户界面 (UI) 相关逻辑的模块。
mod app_update;
mod ass_generator; // 用于生成 ASS 字幕格式的模块。
mod ass_parser; // 用于解析 ASS (Advanced SubStation Alpha) 字幕格式的模块。
mod io;
mod json_parser; // 用于解析 JSON 数据的模块。
mod krc_generator; // 用于生成 KRC 歌词格式的模块。
mod krc_parser; // 用于解析 KRC (酷狗歌词) 格式的模块。
mod kugou_lyrics_fetcher; // 用于从酷狗音乐获取歌词的模块。
mod logger; // 用于日志记录的模块。
mod lqe_generator; // 用于生成 LQE 歌词格式的模块。
mod lqe_parser; // 用于解析 LQE 歌词格式的模块。
mod lqe_to_ttml_data; // 用于将 LQE 格式数据转换为 TTML 格式数据的模块。
mod lrc_generator; // 用于生成 LRC 歌词格式的模块。
mod lrc_parser; // 用于解析 LRC (LyRiCs) 歌词格式的模块。
mod lyric_processor;
mod lyricify_lines_generator; // 用于生成 Lyricify 逐行歌词格式的模块。
mod lyricify_lines_parser; // 用于解析 Lyricify 逐行歌词格式的模块。
mod lyricify_lines_to_ttml_data; // 用于将 Lyricify 逐行歌词格式数据转换为 TTML 格式数据的模块。
mod lyrics_merger;
mod lys_generator; // 用于生成 LYS 格式的模块。
mod lys_parser; // 用于解析 LYS 格式的模块。
mod lys_to_ttml_data; // 用于将 LYS 格式数据转换为 TTML 格式数据的模块。
mod metadata_processor; // 用于处理元数据（例如歌曲信息）的模块。
mod netease_lyrics_fetcher; // 用于从网易云音乐获取歌词的模块。
mod qq_lyrics_fetcher; // 用于从 QQ 音乐获取歌词的模块。
mod qrc_generator; // 用于生成 QRC 格式的模块。
mod qrc_parser; // 用于解析 QRC (QQ 音乐歌词) 格式的模块。
mod qrc_to_ttml_data; // 用于将 QRC 格式数据转换为 TTML 格式数据的模块。
mod spl_generator; // 用于生成 SPL 格式的模块。
mod spl_parser; // 用于解析 SPL 格式的模块。
mod spl_to_ttml_data; // 用于将 SPL 格式数据转换为 TTML 格式数据的模块。
mod ttml_generator; // 用于生成 TTML 字幕格式的模块。
mod ttml_parser; // 用于解析 TTML (Timed Text Markup Language) 字幕格式的模块。
mod websocket_server;
mod yrc_generator; // 用于生成 YRC (网易云音乐歌词) 格式的模块。
mod yrc_parser; // 用于解析 YRC (网易云音乐歌词) 格式的模块。
mod yrc_to_ttml_data; // 用于将 YRC 格式数据转换为 TTML 格式数据的模块。

// 从 `app_settings` 模块导入 `AppSettings` 结构体。
use app_settings::AppSettings;
// 从标准库的 `sync::mpsc` 模块导入必要的组件。
// `mpsc` 代表 "multiple producer, single consumer" (多生产者，单消费者) 队列，常用于线程间通信。
use std::sync::mpsc;

// Rust 程序的入口点。
fn main() {
    // 加载应用程序设置。
    // `AppSettings::load()` 从配置文件或其他持久化存储中读取设置。
    let app_settings = AppSettings::load();

    // 创建一个用于在 UI 线程和其他线程之间传递日志条目的通道。
    // `mpsc::channel()` 返回一个元组，包含一个发送者 (Sender) 和一个接收者 (Receiver)。
    let (ui_log_sender, ui_log_receiver): (
        mpsc::Sender<logger::LogEntry>, // 发送者，用于发送 `LogEntry` 类型的日志条目。
        mpsc::Receiver<logger::LogEntry>, // 接收者，用于接收 `LogEntry` 类型的日志条目。
    ) = mpsc::channel();

    // 初始化全局日志记录器。
    // `ui_log_sender` 被传递给日志记录器，以便将日志条目发送到 UI 线程进行显示。
    // `app_settings.log_settings.enable_file_log` 用于控制是否启用文件日志记录。
    logger::init_global_logger(ui_log_sender, app_settings.log_settings.enable_file_log);

    // 使用 `log` crate (一个流行的 Rust 日志库) 记录一条信息。
    // `target: "unilyric_main"` 指定了日志消息的来源模块或目标。
    log::info!(target: "unilyric_main", "应用程序已启动。");

    // 配置 eframe (egui 的原生后端) 的原生选项。
    let native_options = eframe::NativeOptions {
        // 设置窗口视口的构建器。
        viewport: egui::ViewportBuilder::default()
            // 设置窗口的初始内部大小 (宽度 1024.0, 高度 768.0)。
            .with_inner_size([1024.0, 768.0])
            // 设置窗口的最小内部大小 (宽度 800.0, 高度 600.0)。
            .with_min_inner_size([800.0, 600.0]),
        // `..Default::default()` 表示使用 `NativeOptions` 其他字段的默认值。
        ..Default::default()
    };

    // 运行 eframe 原生应用程序。
    // `eframe::run_native` 是启动基于 egui 的 GUI 应用程序的函数。
    // 它接收三个参数：
    // 1. 应用程序的名称 ("UniLyric")。
    // 2. 上面定义的原生选项 (`native_options`)。
    // 3. 一个闭包 (Box::new(move |cc| ...))，该闭包在应用程序启动时被调用，用于创建应用程序实例。
    //    `move` 关键字表示闭包会获取其捕获的变量的所有权。
    //    `cc` 是 `eframe::CreationContext`，包含了创建应用程序所需的一些上下文信息。
    if let Err(e) = eframe::run_native(
        "UniLyric",     // 应用程序标题
        native_options, // 窗口和应用程序的配置选项
        // 这个 Box<dyn FnOnce(&eframe::CreationContext<'_>) -> Result<Box<dyn eframe::App>, eframe::Error>>
        // 是一个创建应用程序实例的工厂函数。
        Box::new(move |cc| {
            // 创建 UniLyricApp 的实例。
            // `app_settings.clone()` 创建设置的副本，因为 `app_settings` 可能会在其他地方被使用。
            // `ui_log_receiver` 被传递给应用程序实例，以便它可以接收和显示日志。
            let app_instance =
                crate::app_definition::UniLyricApp::new(cc, app_settings.clone(), ui_log_receiver);
            // 返回一个包装好的应用程序实例。
            // `Ok(Box::new(app_instance))` 表示成功创建了应用程序实例。
            // `Box<dyn eframe::App>` 是一个trait object，表示任何实现了 `eframe::App` trait 的类型。
            Ok(Box::new(app_instance))
        }),
    ) {
        // 如果 `eframe::run_native` 返回错误 (Err)，则记录错误信息。
        log::error!(target: "unilyric_main", "Eframe 运行错误: {e}");
    }
}
