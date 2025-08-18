//! 负责处理应用的持久化配置。

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod native {
    use chrono::Utc;
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

    pub fn load_amll_config() -> Result<super::AmllConfig, Box<dyn std::error::Error>> {
        let config_path = get_config_file_path("amll_config.json")?;
        match fs::read_to_string(config_path) {
            Ok(content) => {
                let config: super::AmllConfig = serde_json::from_str(&content)?;
                info!("已加载 AMLL 镜像配置。");
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                info!("未找到 AMLL 配置文件，使用默认源。");
                Ok(super::AmllConfig::default())
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn load_cached_config<T: for<'de> Deserialize<'de>>(
        filename: &str,
    ) -> Result<super::CachedConfig<T>, Box<dyn std::error::Error + Send + Sync>> {
        let config_path = get_config_file_path(filename)?;
        let content = fs::read_to_string(config_path)?;
        let config: super::CachedConfig<T> = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn save_cached_config<T: Serialize>(
        filename: &str,
        data: &T,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let config_path = get_config_file_path(filename)?;
        let cached_config = super::CachedConfig {
            data,
            last_updated: Utc::now(),
        };
        let content = serde_json::to_string_pretty(&cached_config)?;
        fs::write(config_path, content)?;
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Serialize};
    use std::fmt;
    use tracing::info;
    use wasm_bindgen::JsValue;
    use web_sys::window;

    #[derive(Debug)]
    struct WasmConfigError(String);

    impl fmt::Display for WasmConfigError {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl std::error::Error for WasmConfigError {}

    fn js_to_error(e: JsValue) -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(WasmConfigError(format!("{:?}", e)))
    }
    fn str_to_error(s: &str) -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(WasmConfigError(s.to_string()))
    }

    fn get_local_storage() -> Result<web_sys::Storage, Box<dyn std::error::Error + Send + Sync>> {
        window()
            .ok_or_else(|| str_to_error("无法获取 window 对象"))?
            .local_storage()
            .map_err(js_to_error)?
            .ok_or_else(|| str_to_error("localStorage 不可用"))
    }

    fn build_key(filename: &str) -> String {
        format!("lyrics-helper-config:{}", filename)
    }

    pub fn load_amll_config() -> Result<super::AmllConfig, Box<dyn std::error::Error>> {
        match get_local_storage().and_then(|storage| {
            storage
                .get_item(&build_key("amll_config.json"))
                .map_err(js_to_error)
        }) {
            Ok(Some(content)) => {
                let config: super::AmllConfig = serde_json::from_str(&content)?;
                info!("已从 localStorage 加载 AMLL 镜像配置。");
                Ok(config)
            }
            _ => {
                info!("未在 localStorage 中找到 AMLL 配置，使用默认源。");
                Ok(super::AmllConfig::default())
            }
        }
    }

    pub fn load_cached_config<T: for<'de> Deserialize<'de>>(
        filename: &str,
    ) -> Result<super::CachedConfig<T>, Box<dyn std::error::Error + Send + Sync>> {
        let storage = get_local_storage()?;
        let key = build_key(filename);
        let content = storage
            .get_item(&key)
            .map_err(js_to_error)?
            .ok_or_else(|| str_to_error("在 localStorage 中未找到该配置"))?;
        let config: super::CachedConfig<T> = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn save_cached_config<T: Serialize>(
        filename: &str,
        data: &T,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let storage = get_local_storage()?;
        let key = build_key(filename);
        let cached_config = super::CachedConfig {
            data,
            last_updated: Utc::now(),
        };
        let content = serde_json::to_string(&cached_config)?;
        storage.set_item(&key, &content).map_err(js_to_error)?;
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{load_amll_config, load_cached_config, save_cached_config};
#[cfg(target_arch = "wasm32")]
pub use wasm::{load_amll_config, load_cached_config, save_cached_config};

// --- 共享的、与平台无关的结构体定义 ---
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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

/// 通用的、带时间戳的缓存配置结构。
#[derive(Serialize, Deserialize, Debug)]
pub struct CachedConfig<T> {
    /// 缓存的数据。
    pub data: T,
    /// 最后更新的时间戳。
    pub last_updated: DateTime<Utc>,
}
