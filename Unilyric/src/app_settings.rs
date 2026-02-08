use directories::ProjectDirs;
use log::LevelFilter;
use lyrics_helper_core::{LyricFormat, MetadataStripperOptions, SyllableSmoothingOptions};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::amll_connector::types::ConnectorMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogSettings {
    pub enable_file_log: bool,
    pub file_log_level: LevelFilter,
    pub console_log_level: LevelFilter,
}

impl Default for LogSettings {
    fn default() -> Self {
        LogSettings {
            enable_file_log: false,
            file_log_level: LevelFilter::Info,
            console_log_level: LevelFilter::Info,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AppAmllMirror {
    #[default]
    GitHub,
    Dimeta,
    Bikonoo,
    Custom {
        index_url: String,
        lyrics_url_template: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub log_settings: LogSettings,
    pub smtc_time_offset_ms: i64,
    pub amll_connector_enabled: bool,
    pub amll_connector_websocket_url: String,
    pub amll_connector_mode: ConnectorMode,
    pub amll_connector_server_port: u16,
    pub always_search_all_sources: bool,
    pub last_selected_smtc_session_id: Option<String>,
    pub selected_font_family: Option<String>,

    pub use_provider_subset: bool,
    pub auto_search_provider_subset: Vec<String>,
    pub prioritize_amll_db: bool,

    pub enable_t2s_for_auto_search: bool,

    pub last_source_format: LyricFormat,
    pub last_target_format: LyricFormat,
    pub send_audio_data_to_player: bool,

    pub metadata_stripper: MetadataStripperOptions,
    pub syllable_smoothing: SyllableSmoothingOptions,
    pub auto_apply_metadata_stripper: bool,
    pub auto_apply_agent_recognizer: bool,
    pub amll_mirror: AppAmllMirror,
    pub auto_cache: bool,
    pub auto_cache_max_count: usize,
    pub enable_cover_cache_cleanup: bool,
    pub max_cover_cache_files: usize,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            log_settings: LogSettings::default(),
            smtc_time_offset_ms: 0,
            amll_connector_enabled: false,
            amll_connector_websocket_url: "ws://localhost:11444".to_string(),
            amll_connector_mode: ConnectorMode::Client,
            amll_connector_server_port: 11455,
            always_search_all_sources: true,
            last_selected_smtc_session_id: None,
            selected_font_family: None,
            enable_t2s_for_auto_search: true,
            send_audio_data_to_player: true,
            use_provider_subset: false,
            auto_search_provider_subset: vec![],
            prioritize_amll_db: true,

            last_source_format: LyricFormat::Ass,
            last_target_format: LyricFormat::Ttml,
            metadata_stripper: Default::default(),
            syllable_smoothing: Default::default(),
            auto_apply_metadata_stripper: true,
            auto_apply_agent_recognizer: true,
            amll_mirror: AppAmllMirror::default(),
            auto_cache: false,
            auto_cache_max_count: 500,
            enable_cover_cache_cleanup: true,
            max_cover_cache_files: 500,
        }
    }
}

impl AppSettings {
    pub fn config_dir() -> Option<PathBuf> {
        if let Some(proj_dirs) = ProjectDirs::from("com", "Unilyric", "Unilyric") {
            let config_dir = proj_dirs.data_dir();
            if !config_dir.exists()
                && let Err(e) = fs::create_dir_all(config_dir)
            {
                tracing::error!("无法创建配置目录 {config_dir:?}: {e}");
                return None;
            }
            Some(config_dir.to_path_buf())
        } else {
            tracing::error!("无法获取项目配置目录路径。");
            None
        }
    }

    fn config_file_path() -> Option<PathBuf> {
        Self::config_dir().map(|dir| dir.join("unilyric.json"))
    }

    pub fn load() -> Self {
        if let Some(path) = Self::config_file_path() {
            if path.exists() {
                tracing::info!("[Settings] 尝试从 {path:?} 加载 JSON 配置文件。");
                match fs::read_to_string(&path) {
                    Ok(content) => match serde_json::from_str(&content) {
                        Ok(settings) => return settings,
                        Err(e) => {
                            tracing::error!(
                                "[Settings] 解析 JSON 配置文件 {path:?} 失败: {e}。将使用默认配置。"
                            );
                        }
                    },
                    Err(e) => {
                        tracing::error!(
                            "[Settings] 读取配置文件 {path:?} 失败: {e}。将使用默认配置。"
                        );
                    }
                }
            } else {
                tracing::info!("[Settings] 配置文件 {path:?} 未找到。将创建并使用默认配置。");
            }
        }

        let default_settings = AppSettings::default();
        if let Err(e) = default_settings.save() {
            tracing::error!("[Settings] 无法保存初始默认配置文件: {e}");
        }
        default_settings
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        if let Some(path) = Self::config_file_path() {
            match serde_json::to_string_pretty(self) {
                Ok(json_string) => {
                    fs::write(&path, json_string)?;
                    tracing::info!("[Settings] 设置已成功保存到 {path:?}");
                    Ok(())
                }
                Err(e) => {
                    tracing::error!("[Settings] 序列化设置为 JSON 失败: {e}");
                    Err(std::io::Error::other(e))
                }
            }
        } else {
            let err_msg = "[Settings] 无法确定配置文件路径，保存失败。";
            tracing::error!("{err_msg}");
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, err_msg))
        }
    }
}
