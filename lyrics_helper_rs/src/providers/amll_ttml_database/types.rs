//! 此模块定义了与 AMLL TTML Database 提供商相关的所有数据结构和类型。

use serde::Deserialize;
use std::collections::HashMap;

/// 自定义反序列化模块，用于将 JSON 中的 `Vec<(String, Vec<String>)>` 高效地转换为 `HashMap`。
mod de_vec_to_map {
    use serde::{Deserializer, de};
    use std::collections::HashMap;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<HashMap<String, Vec<String>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let vec_of_tuples: Vec<(String, Vec<String>)> = de::Deserialize::deserialize(deserializer)?;
        Ok(vec_of_tuples.into_iter().collect())
    }
}

/// 代表从 `index.jsonl` 文件中解析出的单个索引条目。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexEntry {
    /// 歌词元数据，如歌曲名、艺术家、各种平台 ID 等。
    #[serde(with = "de_vec_to_map")]
    pub metadata: HashMap<String, Vec<String>>,
    /// 歌词文件在仓库中的文件名（例如 "12345.ttml"）。
    /// 这将作为此 provider 的唯一 `song_id`。
    pub raw_lyric_file: String,
}

impl IndexEntry {
    /// 辅助函数，方便地获取单个字符串类型的元数据值（如歌曲名）。
    /// 因为元数据的值部分总是一个 `Vec<String>`，此函数简化了取第一个元素的操作。
    pub fn get_meta_str(&self, key: &str) -> Option<&str> {
        self.metadata
            .get(key)
            .and_then(|v| v.first())
            .map(String::as_str)
    }

    /// 辅助函数，方便地获取字符串向量类型的元数据值（如艺术家列表）。
    pub fn get_meta_vec(&self, key: &str) -> Option<&Vec<String>> {
        self.metadata.get(key)
    }
}

/// 用于解析来自 GitHub API 的错误响应。
///
/// 主要用于识别和处理 API 速率限制的错误信息。
#[derive(Debug, Deserialize)]
pub struct GitHubErrorResponse {
    pub message: String,
}
