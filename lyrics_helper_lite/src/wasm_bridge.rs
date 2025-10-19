use std::collections::HashMap;

use crate::client::create_http_client;
use crate::error::Result;
use crate::providers::{RequestBodyFormat, TrackQuery, get_all_providers, get_provider};
use crate::search::matcher::aggregate_and_sort_results;
use futures::future;
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

#[derive(Serialize)]
struct ProviderInfo {
    id: &'static str,
    name: &'static str,
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

/// Searches for a song across all available providers and returns a single, aggregated list.
///
/// This function is ideal for scenarios without CORS restrictions.
///
/// # Arguments
///
/// * `track_query_json` - A JSON string representing the `TrackQuery` object.
///
/// # Returns
///
/// A `Promise` that resolves to a JS object: `{ results: [SearchResult, ...] }`,
/// with results from all providers sorted together by relevance.
#[wasm_bindgen]
pub async fn unified_search_songs(track_query_json: &str) -> std::result::Result<JsValue, JsValue> {
    set_panic_hook();
    let result: Result<JsValue> = async {
        let query: TrackQuery = serde_json::from_str(track_query_json)?;
        let providers = get_all_providers();
        let client = create_http_client();

        let search_futures = providers.into_values().map(|provider| {
            let query_ref = &query;
            let client_ref = &client;
            async move {
                let req_info = provider.prepare_search_request(query_ref)?;
                let body_str = req_info.body.as_deref().unwrap_or("");

                let response_text = match req_info.method.as_str() {
                    "POST" => match req_info.body_format {
                        RequestBodyFormat::Json => {
                            client_ref.post_json(&req_info.url, body_str).await?
                        }
                        RequestBodyFormat::UrlEncodedForm => {
                            client_ref.post_form(&req_info.url, body_str).await?
                        }
                    },
                    "GET" => client_ref.get(&req_info.url).await?,
                    _ => {
                        return Err(crate::error::FetcherError::InvalidInput(format!(
                            "Unsupported HTTP method: {}",
                            req_info.method
                        )));
                    }
                };
                provider.handle_search_response(&response_text, query_ref)
            }
        });

        let results: Vec<Vec<SearchResult>> = future::join_all(search_futures)
            .await
            .into_iter()
            .filter_map(std::result::Result::ok)
            .collect();

        let aggregated_results =
            query.with_track(|track| aggregate_and_sort_results(&track, results));

        let wrapper = SearchResultsWrapper {
            results: aggregated_results,
        };
        Ok(serde_wasm_bindgen::to_value(&wrapper)?)
    }
    .await;

    result.map_err(Into::into)
}

/// Generates the necessary HTTP request details for all providers for a unified search.
///
/// This is part of the low-level API. Ideal for WASM.
///
/// # Returns
///
/// A JS object mapping provider names to their request details.
/// Example: `{ "qq": { url, ... }, "netease": { url, ... } }`
#[wasm_bindgen]
pub fn prepare_unified_search_requests(
    track_query_json: &str,
) -> std::result::Result<JsValue, JsValue> {
    set_panic_hook();
    let result: Result<JsValue> = (|| {
        let query: TrackQuery = serde_json::from_str(track_query_json)?;
        let providers = get_all_providers();
        let requests: HashMap<_, _> = providers
            .into_iter()
            .filter_map(|(name, provider)| {
                provider
                    .prepare_search_request(&query)
                    .ok()
                    .map(|req| (name, req))
            })
            .collect();
        Ok(serde_wasm_bindgen::to_value(&requests)?)
    })();
    result.map_err(Into::into)
}

/// Parses and aggregates raw search responses from multiple providers.
///
/// This is part of the low-level API. Ideal for WASM.
///
/// # Arguments
///
/// * `responses_json` - A JSON string mapping provider names to their raw response strings.
///   Example: `{ "qq": "...", "netease": "..." }`
/// * `track_query_json` - The original JSON string for the `TrackQuery`.
///
/// # Returns
///
/// A JS object: `{ results: [SearchResult, ...] }`, sorted by relevance.
#[wasm_bindgen]
pub fn handle_unified_search_responses(
    responses_json: &str,
    track_query_json: &str,
) -> std::result::Result<JsValue, JsValue> {
    set_panic_hook();
    let result: Result<JsValue> = (|| {
        let query: TrackQuery = serde_json::from_str(track_query_json)?;
        let responses: HashMap<String, String> = serde_json::from_str(responses_json)?;

        let mut all_results = Vec::new();
        for (provider_name, response_text) in responses {
            if let Ok(provider) = get_provider(&provider_name)
                && let Ok(results) = provider.handle_search_response(&response_text, &query)
            {
                all_results.push(results);
            }
        }

        let aggregated_results =
            query.with_track(|track| aggregate_and_sort_results(&track, all_results));

        let wrapper = SearchResultsWrapper {
            results: aggregated_results,
        };
        Ok(serde_wasm_bindgen::to_value(&wrapper)?)
    })();
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

/// Returns a list of all available provider names.
///
/// # Returns
///
/// A `Promise` that resolves to a JS array of strings, e.g., `["qq", "netease"]`.
#[wasm_bindgen]
pub fn get_available_providers() -> std::result::Result<JsValue, JsValue> {
    set_panic_hook();
    let result: Result<JsValue> = (|| {
        let providers = get_all_providers();
        let provider_info: Vec<ProviderInfo> = providers
            .keys()
            .map(|&name| ProviderInfo { id: name, name })
            .collect();
        Ok(serde_wasm_bindgen::to_value(&provider_info)?)
    })();
    result.map_err(Into::into)
}
