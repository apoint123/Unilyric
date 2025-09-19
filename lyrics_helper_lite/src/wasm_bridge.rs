use crate::client::create_http_client;
use crate::error::Result;
use crate::providers::{RequestBodyFormat, TrackQuery, get_provider};
use lyrics_helper_core::SearchResult;
use serde::Serialize;
use wasm_bindgen::prelude::*;

fn set_panic_hook() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// A wrapper for search results to ensure the JS side receives a structured object.
#[derive(Serialize)]
struct SearchResultsWrapper {
    results: Vec<SearchResult>,
}

/// Searches for a song using the high-level API, which handles HTTP requests internally.
///
/// This function is ideal for scenarios without CORS restrictions.
///
/// # Arguments
///
/// * `provider_name` - The name of the provider (e.g., "qq", "netease").
/// * `track_query_json` - A JSON string representing the `TrackQuery` object.
///   Example: `{"title": "Song Title", "artists": ["Artist Name"], "album": "Album Name", "duration_ms": 200000}`
///
/// # Returns
///
/// A `Promise` that resolves to a JS object: `{ results: [SearchResult, ...] }`.
/// The `results` array is sorted by relevance.
#[wasm_bindgen]
pub async fn search_songs(
    provider_name: &str,
    track_query_json: &str,
) -> std::result::Result<JsValue, JsValue> {
    set_panic_hook();
    let result: Result<JsValue> = async {
        let provider = get_provider(provider_name)?;
        let query: TrackQuery = serde_json::from_str(track_query_json)?;
        let req_info = provider.prepare_search_request(&query)?;

        let client = create_http_client();
        let body_str = req_info.body.as_deref().unwrap_or("");

        let response_text = match req_info.method.as_str() {
            "POST" => match req_info.body_format {
                RequestBodyFormat::Json => client.post_json(&req_info.url, body_str).await?,
                RequestBodyFormat::UrlEncodedForm => {
                    client.post_form(&req_info.url, body_str).await?
                }
            },
            "GET" => client.get(&req_info.url).await?,
            _ => {
                return Err(crate::error::FetcherError::InvalidInput(format!(
                    "Unsupported HTTP method: {}",
                    req_info.method
                )));
            }
        };

        let search_results = provider.handle_search_response(&response_text, &query)?;
        let wrapper = SearchResultsWrapper {
            results: search_results,
        };
        Ok(serde_wasm_bindgen::to_value(&wrapper)?)
    }
    .await;

    result.map_err(Into::into)
}

/// Fetches lyrics for a specific search result using the high-level API.
///
/// This function is ideal for scenarios without CORS restrictions.
///
/// # Arguments
///
/// * `provider_name` - The name of the provider (e.g., "qq", "netease").
/// * `search_result_json` - A JSON string representing a single `SearchResult` object
///   obtained from `search_songs`.
///
/// # Returns
///
/// A `Promise` that resolves to a `RawLyrics` object.
#[wasm_bindgen]
pub async fn get_lyrics(
    provider_name: &str,
    search_result_json: &str,
) -> std::result::Result<JsValue, JsValue> {
    set_panic_hook();
    let result: Result<JsValue> = async {
        let provider = get_provider(provider_name)?;
        let search_result: SearchResult = serde_json::from_str(search_result_json)?;
        let req_info = provider.prepare_lyrics_request(&search_result)?;

        let client = create_http_client();
        let response_text = client
            .post_form(&req_info.url, req_info.body.as_deref().unwrap_or(""))
            .await?;

        let raw_lyrics = provider.handle_lyrics_response(&response_text)?;

        Ok(serde_wasm_bindgen::to_value(&raw_lyrics)?)
    }
    .await;

    result.map_err(Into::into)
}

/// Generates the necessary HTTP request details for searching a song.
///
/// This is part of the low-level API designed for web frontends to bypass CORS issues.
/// You need to make the actual request by yourself.
///
/// # Arguments
///
/// * `provider_name` - The name of the provider.
/// * `track_query_json` - A JSON string representing the `TrackQuery` object.
///
/// # Returns
///
/// A JS object containing HTTP request details (`{ url, method, headers, body }`).
#[wasm_bindgen]
pub fn prepare_search_request(
    provider_name: &str,
    track_query_json: &str,
) -> std::result::Result<JsValue, JsValue> {
    set_panic_hook();
    let result: Result<JsValue> = (|| {
        let provider = get_provider(provider_name)?;
        let query: TrackQuery = serde_json::from_str(track_query_json)?;
        let req_info = provider.prepare_search_request(&query)?;
        Ok(serde_wasm_bindgen::to_value(&req_info)?)
    })();
    result.map_err(Into::into)
}

/// Parses the raw search response string (fetched by yourself).
///
/// # Arguments
///
/// * `provider_name` - The name of the provider.
/// * `response_text` - The raw response body you got.
/// * `track_query_json` - The original JSON string for the `TrackQuery` used to generate the request.
///
/// # Returns
///
/// A JS object: `{ results: [SearchResult, ...] }`, sorted by relevance.
#[wasm_bindgen]
pub fn handle_search_response(
    provider_name: &str,
    response_text: &str,
    track_query_json: &str,
) -> std::result::Result<JsValue, JsValue> {
    set_panic_hook();
    let result: Result<JsValue> = (|| {
        let provider = get_provider(provider_name)?;
        let query: TrackQuery = serde_json::from_str(track_query_json)?;
        let search_results = provider.handle_search_response(response_text, &query)?;
        let wrapper = SearchResultsWrapper {
            results: search_results,
        };
        Ok(serde_wasm_bindgen::to_value(&wrapper)?)
    })();
    result.map_err(Into::into)
}

/// Generates the HTTP request details for fetching lyrics.
///
/// This is part of the low-level API designed for web frontends to bypass CORS issues.
/// You need to make the actual request by yourself.
///
/// # Arguments
///
/// * `provider_name` - The name of the provider.
/// * `search_result_json` - A JSON string representing a single `SearchResult` object.
///
/// # Returns
///
/// A JS object containing HTTP request details (`{ url, method, headers, body }`).
#[wasm_bindgen]
pub fn prepare_lyrics_request(
    provider_name: &str,
    search_result_json: &str,
) -> std::result::Result<JsValue, JsValue> {
    set_panic_hook();
    let result: Result<JsValue> = (|| {
        let provider = get_provider(provider_name)?;
        let search_result: SearchResult = serde_json::from_str(search_result_json)?;
        let req_info = provider.prepare_lyrics_request(&search_result)?;
        Ok(serde_wasm_bindgen::to_value(&req_info)?)
    })();
    result.map_err(Into::into)
}

/// Parses the raw lyrics response string (fetched by yourself).
///
/// # Arguments
///
/// * `provider_name` - The name of the provider.
/// * `response_text` - The raw response body you got.
///
/// # Returns
///
/// A `RawLyrics` object.
#[wasm_bindgen]
pub fn handle_lyrics_response(
    provider_name: &str,
    response_text: &str,
) -> std::result::Result<JsValue, JsValue> {
    set_panic_hook();
    let result: Result<JsValue> = (|| {
        let provider = get_provider(provider_name)?;
        let raw_lyrics = provider.handle_lyrics_response(response_text)?;
        Ok(serde_wasm_bindgen::to_value(&raw_lyrics)?)
    })();
    result.map_err(Into::into)
}
