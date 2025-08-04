use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum AppError {
    #[error("核心库错误: {0}")]
    LyricsHelper(Arc<lyrics_helper_rs::error::LyricsHelperError>),

    #[error("IO 错误: {0}")]
    Io(Arc<std::io::Error>),

    #[error("JSON 序列化/反序列化错误: {0}")]
    Json(Arc<serde_json::Error>),

    #[error("图片处理错误: {0}")]
    Image(Arc<image::ImageError>),

    #[error("剪贴板错误: {0}")]
    Clipboard(Arc<arboard::Error>),

    #[error("任务间通信失败: {0}")]
    Channel(String),

    #[error("错误: {0}")]
    Custom(String),
}

impl From<lyrics_helper_rs::error::LyricsHelperError> for AppError {
    fn from(err: lyrics_helper_rs::error::LyricsHelperError) -> Self {
        Self::LyricsHelper(Arc::new(err))
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(Arc::new(err))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(Arc::new(err))
    }
}

impl From<image::ImageError> for AppError {
    fn from(err: image::ImageError) -> Self {
        Self::Image(Arc::new(err))
    }
}

impl From<arboard::Error> for AppError {
    fn from(err: arboard::Error) -> Self {
        Self::Clipboard(Arc::new(err))
    }
}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for AppError {
    fn from(err: tokio::sync::mpsc::error::SendError<T>) -> Self {
        AppError::Channel(format!("Tokio channel send error: {}", err))
    }
}

pub type AppResult<T> = std::result::Result<T, AppError>;
