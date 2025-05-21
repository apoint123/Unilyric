use thiserror::Error;

#[derive(Error, Debug)]
pub enum KugouError {
    #[error("网络错误: {0}")]
    Network(#[from] reqwest::Error),
    #[error("JSON反序列化失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Base64解密失败: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("Zlib解压失败: {0}")]
    Decompression(std::io::Error),
    #[error("UTF-8转换失败: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("未找到歌词或API返回错误：{0}")]
    LyricsNotFound(String),
    #[error("无效的KRC数据: {0}")]
    InvalidKrcData(String),
    #[error("未找到合适的歌词")]
    NoCandidatesFound,
    #[error("缺少下载歌词所需的信息")]
    MissingCredentials,
    #[error("返回的歌词内容为空")]
    EmptyLyricContent,
}

pub type Result<T> = std::result::Result<T, KugouError>;
