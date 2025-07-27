#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod amll_connector;
mod app;
mod app_actions;
mod app_definition;
pub mod app_fetch_core;
mod app_handlers;
mod app_settings;
mod app_ui;
mod app_update;
mod io;
mod types;
mod utils;

use app_settings::AppSettings;
use std::sync::mpsc;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Layer, fmt};

/// 一个自定义的 tracing Layer，用于将日志条目发送到UI线程。
struct UiLayer {
    sender: mpsc::Sender<types::LogEntry>,
}

impl<S> Layer<S> for UiLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        // 从事件元数据中创建 LogEntry
        let mut message = String::new();
        event.record(&mut MessageVisitor(&mut message));

        let entry = types::LogEntry {
            level: event.metadata().level().into(),
            message,
            timestamp: chrono::Local::now(),
            target: event.metadata().target().to_string(),
        };

        // 发送到UI，如果失败则打印到stderr
        if self.sender.send(entry).is_err() {
            eprintln!("UI日志通道已关闭。");
        }
    }
}

/// 用于从 tracing::Event 中提取消息的访问者。
struct MessageVisitor<'a>(&'a mut String);
impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0.push_str(&format!("{value:?}"));
        }
    }
}

fn setup_tracing(
    ui_log_sender: mpsc::Sender<types::LogEntry>,
    settings: &app_settings::LogSettings,
) {
    let our_crates_level = "debug".to_string();
    let console_filter_str = format!(
        "warn,Unilyric={our_crates_level},lyrics_helper_rs={our_crates_level},smtc_suite={our_crates_level},eframe={our_crates_level},egui_winit={our_crates_level},wgpu_core=warn,wgpu_hal=warn"
    );

    let console_filter = EnvFilter::new(console_filter_str);

    let file_filter = if settings.enable_file_log {
        let our_crates_file_level = settings.file_log_level.to_string().to_lowercase();
        let file_filter_str = format!(
            "warn,unilyric={our_crates_file_level},smtc_suite={our_crates_level},lyrics_helper_rs={our_crates_file_level}"
        );
        EnvFilter::new(file_filter_str)
    } else {
        EnvFilter::new("off")
    };

    let console_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .with_filter(console_filter);

    let ui_filter_str = format!(
        "warn,Unilyric={our_crates_level},lyrics_helper_rs={our_crates_level},smtc_suite={our_crates_level},eframe={our_crates_level},egui_winit={our_crates_level},wgpu_core=warn,wgpu_hal=warn"
    );
    let ui_filter = EnvFilter::new(ui_filter_str);

    let ui_layer = UiLayer {
        sender: ui_log_sender,
    }
    .with_filter(ui_filter);

    let file_layer = if settings.enable_file_log {
        match AppSettings::config_dir() {
            Some(config_dir) => {
                let log_dir = config_dir.join("logs");
                if let Err(e) = std::fs::create_dir_all(&log_dir) {
                    eprintln!("无法创建日志目录 {log_dir:?}: {e}");
                    None
                } else {
                    let file_appender = tracing_appender::rolling::daily(log_dir, "unilyric.log");
                    let (non_blocking_writer, guard) =
                        tracing_appender::non_blocking(file_appender);

                    static LOG_GUARD: once_cell::sync::Lazy<
                        std::sync::Mutex<Option<tracing_appender::non_blocking::WorkerGuard>>,
                    > = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));
                    *LOG_GUARD.lock().unwrap() = Some(guard);

                    Some(
                        fmt::layer()
                            .with_writer(non_blocking_writer)
                            .with_ansi(false)
                            .with_filter(file_filter),
                    )
                }
            }
            None => {
                eprintln!("无法获取配置目录，文件日志将被禁用");
                None
            }
        }
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(console_layer)
        .with(ui_layer)
        .with(file_layer)
        .init();
}

fn main() {
    let app_settings = AppSettings::load();
    let (ui_log_sender, ui_log_receiver) = mpsc::channel();

    setup_tracing(ui_log_sender, &app_settings.log_settings);

    tracing::info!(target: "unilyric_main", "应用程序已启动。");

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 768.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    if let Err(e) = eframe::run_native(
        "UniLyric",
        native_options,
        Box::new(move |cc| {
            let app_instance =
                crate::app_definition::UniLyricApp::new(cc, app_settings.clone(), ui_log_receiver);
            Ok(Box::new(app_instance))
        }),
    ) {
        tracing::error!(target: "unilyric_main", "Eframe 运行错误: {e}");
    }
}
