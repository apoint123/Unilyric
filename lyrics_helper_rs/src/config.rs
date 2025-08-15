//! 负责处理应用的持久化配置。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tracing::info;

/// 获取应用配置目录下指定文件的完整路径。
///
/// # 参数
/// * `filename` - 目标配置文件的名称，例如 "`kugou_config.json`"。
pub(crate) fn get_config_file_path(filename: &str) -> Result<PathBuf, std::io::Error> {
    if let Some(mut config_dir) = dirs::config_dir() {
        config_dir.push("lyrics-helper");
        fs::create_dir_all(&config_dir)?;
        config_dir.push(filename);
        Ok(config_dir)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "无法找到用户配置目录",
        ))
    }
}

/// AMLL 数据库的镜像源配置。
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[derive(Default)]
pub enum AmllMirror {
    /// 默认 GitHub 源。
    GitHub,
    #[default]
    /// Dimeta 镜像 (dimeta.top) By Luorix。
    Dimeta,
    /// Bikonoo 镜像 (bikonoo.com) By cybaka520。
    Bikonoo,
    /// 自定义镜像。
    /// `lyrics_url_template` 应包含 `{song_id}` 占位符。
    Custom {
        /// 指向 raw-lyrics-index.jsonl 文件的完整 URL。
        ///
        /// 示例：`https://your.mirror.com/path/to/raw-lyrics-index.jsonl`
        index_url: String,
        /// 指向歌词文件的 URL 模板。会自动将 `{song_id}` 占位符替换为实际的文件名
        ///
        /// 示例：`https://your.mirror.com/path/to/raw-lyrics/{song_id}`
        lyrics_url_template: String,
    },
}

/// AMLL TTML Database 的配置项。
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct AmllConfig {
    #[serde(default)]
    /// AMLL 数据库的镜像源配置。
    pub mirror: AmllMirror,
}

/// 从文件加载 AMLL 的配置。如果文件不存在，则返回默认配置。
pub fn load_amll_config() -> Result<AmllConfig, Box<dyn std::error::Error>> {
    let config_path = get_config_file_path("amll_config.json")?;
    match fs::read_to_string(config_path) {
        Ok(content) => {
            let config: AmllConfig = serde_json::from_str(&content)?;
            info!("已加载 AMLL 镜像配置。");
            Ok(config)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!("未找到 AMLL 配置文件，使用默认源。");
            Ok(AmllConfig::default())
        }
        Err(e) => Err(e.into()),
    }
}

/// 通用的、带时间戳的缓存配置结构。
#[derive(Serialize, Deserialize, Debug)]
pub struct CachedConfig<T> {
    /// 缓存的数据。
    pub data: T,
    /// 最后更新的时间戳。
    pub last_updated: DateTime<Utc>,
}

/// 从文件加载带时间戳的缓存配置。
pub fn load_cached_config<T: for<'de> Deserialize<'de>>(
    filename: &str,
) -> Result<CachedConfig<T>, Box<dyn std::error::Error + Send + Sync>> {
    let config_path = get_config_file_path(filename)?;
    let content = fs::read_to_string(config_path)?;
    let config: CachedConfig<T> = serde_json::from_str(&content)?;
    Ok(config)
}

/// 将数据封装在带时间戳的结构中，并保存到文件。
pub fn save_cached_config<T: Serialize>(
    filename: &str,
    data: &T,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config_path = get_config_file_path(filename)?;
    let cached_config = CachedConfig {
        data,
        last_updated: Utc::now(),
    };
    let content = serde_json::to_string_pretty(&cached_config)?;
    fs::write(config_path, content)?;
    Ok(())
}
