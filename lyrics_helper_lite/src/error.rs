use serde::{Deserialize, Serialize};
use std::num::ParseIntError;
use thiserror::Error;
use wasm_bindgen::prelude::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct WasmError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum FetcherError {
    #[error("Network request failed: {0}")]
    Network(String),

    #[error("JSON parsing failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("WASM serialization failed: {0}")]
    WasmSerialization(String),

    #[error("Provider API returned an error: {0}")]
    Provider(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Invalid time format: {0}")]
    InvalidTime(String),

    #[error("Lyric parsing failed: {0}")]
    Parse(String),
}

impl From<FetcherError> for JsValue {
    fn from(err: FetcherError) -> Self {
        let (code, message) = match &err {
            FetcherError::Network(msg) => ("NetworkError", msg.clone()),
            FetcherError::Json(_) => ("JsonError", err.to_string()),
            FetcherError::WasmSerialization(msg) => ("WasmSerializationError", msg.clone()),
            FetcherError::Provider(msg) => ("ProviderError", msg.clone()),
            FetcherError::InvalidInput(msg) => ("InvalidInput", msg.clone()),
            FetcherError::InvalidTime(msg) => ("InvalidTimeError", msg.clone()),
            FetcherError::Parse(msg) => ("ParseError", msg.clone()),
        };
        let error_obj = WasmError {
            code: code.to_string(),
            message,
        };
        serde_wasm_bindgen::to_value(&error_obj)
            .unwrap_or_else(|_| Self::from_str("Failed to serialize error"))
    }
}

impl From<reqwest::Error> for FetcherError {
    fn from(err: reqwest::Error) -> Self {
        Self::Network(err.to_string())
    }
}

impl From<serde_wasm_bindgen::Error> for FetcherError {
    fn from(err: serde_wasm_bindgen::Error) -> Self {
        Self::WasmSerialization(err.to_string())
    }
}

impl From<ParseIntError> for FetcherError {
    fn from(err: ParseIntError) -> Self {
        Self::InvalidTime(format!(
            "Failed to parse integer from time component: {err}"
        ))
    }
}

pub type Result<T> = std::result::Result<T, FetcherError>;
