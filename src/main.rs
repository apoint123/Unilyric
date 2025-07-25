#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod types;
#[macro_use]
mod utils;
mod amll_connector;
mod app;
mod app_definition;
pub mod app_fetch_core;
mod app_settings;
mod app_ui;
mod app_update;
mod io;
mod logger;
mod websocket_server;

use app_settings::AppSettings;
use std::sync::mpsc;

fn main() {
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
