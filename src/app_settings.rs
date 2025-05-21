use directories::ProjectDirs;
use ini::Ini;
use log::LevelFilter;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

const PINNED_METADATA_SECTION: &str = "PinnedMetadata";
// 定义一个分隔符，用于在INI文件中连接和分割同一键的多个值
// 选择一个在元数据值中不太可能出现的字符串序列
const MULTI_VALUE_DELIMITER: &str = ";;;";

#[derive(Debug, Clone)]
pub struct LogSettings {
    pub enable_file_log: bool,
    pub file_log_level: LevelFilter,
    pub console_log_level: LevelFilter,
}

impl Default for LogSettings {
    fn default() -> Self {
        LogSettings {
            enable_file_log: false, // 默认不启用文件日志，与之前分析一致
            file_log_level: LevelFilter::Info,
            console_log_level: LevelFilter::Info,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AppSettings {
    pub log_settings: LogSettings,
    pub pinned_metadata: HashMap<String, Vec<String>>, // 键对应一个字符串向量，本身支持多值
}

impl AppSettings {
    fn config_path() -> Option<PathBuf> {
        if let Some(proj_dirs) = ProjectDirs::from("com", "Unilyric", "Unilyric") {
            let config_dir = proj_dirs.data_local_dir();
            if !config_dir.exists() {
                if let Err(e) = fs::create_dir_all(config_dir) {
                    log::error!("无法创建配置目录 {:?}: {}", config_dir, e);
                    return None;
                }
            }
            Some(config_dir.join("unilyric.ini"))
        } else {
            log::error!("无法获取项目配置目录路径。");
            None
        }
    }

    pub fn load() -> Self {
        if let Some(path) = Self::config_path() {
            if path.exists() {
                match Ini::load_from_file(&path) {
                    Ok(conf) => {
                        let log_section = conf.section(Some("Logging"));
                        let ls = LogSettings {
                            enable_file_log: log_section
                                .and_then(|s| s.get("EnableFileLog"))
                                .and_then(|s| s.parse::<bool>().ok())
                                .unwrap_or(LogSettings::default().enable_file_log),
                            file_log_level: log_section
                                .and_then(|s| s.get("FileLogLevel"))
                                .and_then(|s| LevelFilter::from_str(s).ok())
                                .unwrap_or(LogSettings::default().file_log_level),
                            console_log_level: log_section
                                .and_then(|s| s.get("ConsoleLogLevel"))
                                .and_then(|s| LevelFilter::from_str(s).ok())
                                .unwrap_or(LogSettings::default().console_log_level),
                        };

                        let mut loaded_pinned_metadata = HashMap::new();
                        if let Some(pinned_section) = conf.section(Some(PINNED_METADATA_SECTION)) {
                            for (key, single_value_str) in pinned_section.iter() {
                                // 将从INI读取的单个字符串按分隔符切分回 Vec<String>
                                let values_vec: Vec<String> = single_value_str
                                    .split(MULTI_VALUE_DELIMITER)
                                    .map(|s| s.to_string())
                                    .collect();
                                loaded_pinned_metadata.insert(key.to_string(), values_vec);
                            }
                        }
                        log::info!(
                            "从 {:?} 加载配置成功。加载了 {} 个固定元数据键。",
                            path,
                            loaded_pinned_metadata.len()
                        );
                        return AppSettings {
                            log_settings: ls,
                            pinned_metadata: loaded_pinned_metadata,
                        };
                    }
                    Err(e) => {
                        log::error!("加载配置文件 {:?} 失败: {}。将使用默认配置。", path, e);
                    }
                }
            } else {
                log::info!("配置文件 {:?} 未找到。将创建并使用默认配置。", path);
                let default_settings = AppSettings::default();
                if default_settings.save().is_err() {
                    log::error!("无法保存初始默认配置文件到 {:?}。", path);
                }
                return default_settings;
            }
        }
        log::warn!("无法确定配置文件路径。将使用运行时默认配置。");
        AppSettings::default()
    }

    pub fn save(&self) -> Result<(), ini::Error> {
        if let Some(path) = Self::config_path() {
            let mut conf = Ini::new();
            // 写入日志设置区域
            conf.with_section(Some("Logging"))
                .set(
                    "EnableFileLog",
                    self.log_settings.enable_file_log.to_string(),
                )
                .set("FileLogLevel", self.log_settings.file_log_level.to_string())
                .set(
                    "ConsoleLogLevel",
                    self.log_settings.console_log_level.to_string(),
                );

            conf.delete(Some(PINNED_METADATA_SECTION)); // 先删除旧区域，确保干净写入

            if !self.pinned_metadata.is_empty() {
                let mut section = conf.with_section(Some(PINNED_METADATA_SECTION));
                for (key, values_vec) in &self.pinned_metadata {
                    if !values_vec.is_empty() {
                        // 将 Vec<String> 用分隔符连接成单个字符串进行存储
                        let single_value_str = values_vec.join(MULTI_VALUE_DELIMITER);
                        section.set(key, single_value_str);
                    }
                }
            }

            match conf.write_to_file(&path) {
                Ok(_) => {
                    log::info!(
                        "配置已保存到 {:?}。保存了 {} 个固定元数据键。",
                        path,
                        self.pinned_metadata.len()
                    );
                    Ok(())
                }
                Err(write_error) => {
                    log::error!("保存配置到 {:?} 失败: {}", path, write_error);
                    Err(ini::Error::Io(write_error)) // ini::Error::Io 需要一个 std::io::Error
                }
            }
        } else {
            let err_msg = "无法确定配置文件路径，保存失败。".to_string();
            log::error!("{}", err_msg);
            Err(ini::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                err_msg,
            )))
        }
    }
}
