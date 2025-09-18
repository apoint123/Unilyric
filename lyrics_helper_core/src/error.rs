use std::{error::Error as StdError, fmt, io};

use thiserror::Error;

/// 定义歌词转换和处理过程中可能发生的各种错误。
#[derive(Error, Debug)]
pub enum ConvertError {
    /// 通用的解析错误。
    #[error("解析失败: {0}")]
    Parse(Box<dyn StdError + Send + Sync>),

    /// 整数解析错误。
    #[error("解析整数失败: {0}")]
    ParseInt(#[from] std::num::ParseIntError),

    /// 无效的时间格式字符串。
    #[error("无效的时间格式: {0}")]
    InvalidTime(String),

    /// 字符串格式化错误。
    #[error("格式化失败: {0}")]
    Format(#[from] fmt::Error),

    /// 内部逻辑错误或未明确分类的错误。
    #[error("内部错误: {0}")]
    Internal(String),

    /// 文件读写等IO错误。
    #[error("IO 错误: {0}")]
    Io(#[from] io::Error),

    /// JSON 解析错误。
    #[error("解析 JSON 内容 '{context}' 失败: {source}")]
    JsonParse {
        /// 底层 `serde_json` 错误
        #[source]
        source: serde_json::Error,
        /// 有关错误发生位置的上下文信息。
        context: String,
    },

    /// JSON 结构不符合预期。
    #[error("JSON 结构无效: {0}")]
    InvalidJsonStructure(String),

    /// 从字节序列转换为 UTF-8 字符串失败。
    #[error("UTF-8 转换错误: {0}")]
    FromUtf8(#[from] std::string::FromUtf8Error),

    /// 无效的歌词格式。
    #[error("无效的歌词格式: {0}")]
    InvalidLyricFormat(String),

    /// 词组边界检测错误
    #[error("词组边界检测失败: {0}")]
    WordBoundaryDetection(String),

    /// 振假名解析错误
    #[error("振假名解析失败: {0}")]
    FuriganaParsingError(String),

    /// 轨道合并错误
    #[error("轨道合并失败: {0}")]
    TrackMergeError(String),
}

impl From<ConvertError> for std::io::Error {
    fn from(err: ConvertError) -> Self {
        std::io::Error::other(err)
    }
}

impl ConvertError {
    /// 创建一个带有上下文的 `JsonParse` 错误。
    #[must_use]
    pub fn json_parse(source: serde_json::Error, context: String) -> Self {
        Self::JsonParse { source, context }
    }

    /// 创建一个通用的 Parse 错误，用于包装任何具体的解析错误。
    pub fn new_parse<E>(error: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        Self::Parse(Box::new(error))
    }
}

/// 定义从字符串解析 `CanonicalMetadataKey` 时可能发生的错误。
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ParseCanonicalMetadataKeyError(pub String);

impl fmt::Display for ParseCanonicalMetadataKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "未知或无效的元数据键: {}", self.0)
    }
}
impl StdError for ParseCanonicalMetadataKeyError {}
