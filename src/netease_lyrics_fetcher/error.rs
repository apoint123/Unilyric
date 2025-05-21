use thiserror::Error;

#[allow(dead_code)]
#[derive(Error, Debug)]
pub enum NeteaseError {
    #[error("网络请求失败: {0}")]
    Network(#[from] reqwest::Error),
    #[error("JSON反序列化失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("API返回错误码: {code}, 信息: {message:?}")]
    ApiError { code: i64, message: Option<String> },
    #[error("未找到歌词")]
    NoLyrics,
    #[error("歌曲未找到: {0}")]
    SongNotFound(String),
    #[error("响应中缺少必要的歌词字段")]
    MissingLyricField,
    #[error("加密/解密错误: {0}")]
    Crypto(String),
    #[error("内部错误: {0}")]
    Internal(String),
    #[error("URL解析失败: {0}")]
    UrlParse(#[from] url::ParseError),
}

pub type Result<T> = std::result::Result<T, NeteaseError>;
