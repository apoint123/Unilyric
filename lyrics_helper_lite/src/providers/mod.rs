use crate::error::{FetcherError, Result};
use lyrics_helper_core::{RawLyrics, SearchResult, Track};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Default)]
pub enum RequestBodyFormat {
    #[default]
    Json,
    UrlEncodedForm,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct RequestInfo {
    pub url: String,
    pub method: String, // "GET" or "POST"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<Vec<(String, String)>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default)]
    pub body_format: RequestBodyFormat,
}

#[derive(Debug, Deserialize)]
pub struct TrackQuery {
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration: Option<u64>,
}

impl TrackQuery {
    pub fn with_track<F, R>(&self, f: F) -> R
    where
        F: FnOnce(Track<'_>) -> R,
    {
        let artists_slice: Vec<&str> = self.artists.iter().map(AsRef::as_ref).collect();
        let track = Track {
            title: Some(&self.title),
            artists: Some(&artists_slice),
            album: self.album.as_deref(),
            duration: self.duration,
        };
        f(track)
    }
}

pub trait LyricProvider {
    fn prepare_search_request(&self, query: &TrackQuery) -> Result<RequestInfo>;
    fn handle_search_response(
        &self,
        response_text: &str,
        query: &TrackQuery,
    ) -> Result<Vec<SearchResult>>;
    fn prepare_lyrics_request(&self, search_result: &SearchResult) -> Result<RequestInfo>;
    fn handle_lyrics_response(&self, response_text: &str) -> Result<RawLyrics>;
}

pub mod netease;
pub mod qq;

pub fn get_provider(name: &str) -> Result<Box<dyn LyricProvider>> {
    match name {
        "qq" => Ok(Box::new(qq::QQProvider)),
        "netease" => Ok(Box::new(netease::NeteaseProvider)),
        _ => Err(FetcherError::InvalidInput(format!(
            "Provider '{name}' not supported"
        ))),
    }
}

pub fn checked_json_parser<T: serde::de::DeserializeOwned>(
    response_text: &str,
    success_code: i64,
    data_path: &[&str],
) -> Result<T> {
    let v: serde_json::Value = serde_json::from_str(response_text)?;

    if let Some(code) = v.get("code").and_then(serde_json::Value::as_i64)
        && code != success_code
    {
        let msg = v
            .get("msg")
            .or_else(|| v.get("message"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Unknown API error");
        return Err(FetcherError::Provider(format!(
            "API returned error code {code}: {msg}"
        )));
    }
    let mut data_value = &v;
    for key in data_path {
        data_value = data_value.get(key).ok_or_else(|| {
            FetcherError::Provider(format!("Missing key '{key}' in API response"))
        })?;
    }

    T::deserialize(data_value).map_err(|e| {
        FetcherError::Provider(format!("Failed to deserialize API data structure: {e}"))
    })
}
