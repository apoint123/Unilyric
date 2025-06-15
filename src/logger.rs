use chrono::{DateTime, Local};
use directories::ProjectDirs;
use fern::Dispatch;
use log::{Level, LevelFilter};
use once_cell::sync::OnceCell;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{SendError, Sender};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Success,
    Marker,
}

impl From<log::Level> for LogLevel {
    fn from(level: log::Level) -> Self {
        match level {
            log::Level::Error => LogLevel::Error,
            log::Level::Warn => LogLevel::Warn,
            log::Level::Info => LogLevel::Info,
            log::Level::Debug | log::Level::Trace => LogLevel::Info,
        }
    }
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Info => "提示",
            LogLevel::Warn => "警告",
            LogLevel::Error => "错误",
            LogLevel::Success => "成功",
            LogLevel::Marker => "标记",
        }
    }

    pub fn color(&self) -> egui::Color32 {
        match self {
            LogLevel::Info => egui::Color32::from_rgb(100, 180, 255),
            LogLevel::Warn => egui::Color32::from_rgb(255, 200, 0),
            LogLevel::Error => egui::Color32::from_rgb(255, 100, 100),
            LogLevel::Success => egui::Color32::from_rgb(100, 220, 100),
            LogLevel::Marker => egui::Color32::from_rgb(180, 160, 255),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
    pub timestamp: DateTime<Local>,
}

static UI_LOG_SENDER: OnceCell<Sender<LogEntry>> = OnceCell::new();

fn get_log_file_path() -> Result<PathBuf, String> {
    if let Some(proj_dirs) = ProjectDirs::from("com", "UniLyric", "UniLyric") {
        let log_dir = proj_dirs.data_local_dir();
        if !log_dir.exists() {
            fs::create_dir_all(log_dir)
                .map_err(|e| format!("无法创建日志目录 {log_dir:?}: {e}"))?;
        }
        Ok(log_dir.join("unilyric.log"))
    } else {
        let current_dir_log_path = PathBuf::from("unilyric.log");
        eprintln!("无法获取项目日志目录，将尝试在当前目录创建日志: {current_dir_log_path:?}");
        Ok(current_dir_log_path)
    }
}
pub fn init_global_logger(ui_sender: Sender<LogEntry>, enable_file_log_setting: bool) {
    if UI_LOG_SENDER.set(ui_sender).is_err() {
        eprintln!("UI Log Sender 已经被初始化过了!");
    }

    let log_file_path = match get_log_file_path() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("获取日志文件路径失败: {e}。将禁用文件日志记录。");
            PathBuf::from("unilyric_fallback.log")
        }
    };

    println!("[Logger Init] 日志文件将被写入: {log_file_path:?}");

    let base_dispatch = Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{}][{}] {}",
                Local::now().format("%Y-%m-%d %H:%M:%S.%3f"),
                record.level(),
                message
            ))
        })
        .level(LevelFilter::Info);

    let ui_dispatch = Dispatch::new()
        .filter(|metadata| {
            metadata.level() <= Level::Info || metadata.target().starts_with("app::")
        })
        .chain(fern::Output::call(|record| {
            if let Some(sender) = UI_LOG_SENDER.get() {
                let log_level = LogLevel::from(record.level());
                let entry = LogEntry {
                    level: log_level,
                    message: format!("{}", record.args()),
                    timestamp: Local::now(),
                };
                if let Err(SendError(failed_entry)) = sender.send(entry) {
                    eprintln!("[{}] {}", failed_entry.level.as_str(), failed_entry.message);
                }
            }
        }))
        .into_shared();

    let mut final_dispatch = base_dispatch.chain(ui_dispatch);

    if enable_file_log_setting {
        match fern::log_file(&log_file_path) {
            Ok(log_file) => {
                println!("[Logger Init] 文件日志已启用。日志文件将被写入: {log_file_path:?}");
                final_dispatch = final_dispatch.chain(log_file);
            }
            Err(e) => {
                eprintln!("无法打开日志文件 {log_file_path:?}: {e}。文件日志将被禁用。");
            }
        }
    } else {
        println!("[Logger Init] 文件日志已禁用 (根据设置)。");
    }

    if let Err(e) = final_dispatch.apply() {
        eprintln!("日志记录失败: {e}");
    } else {
        log::info!("日志记录器已初始化。日志路径: {log_file_path:?}");
    }
}
