mod crypto;
mod models;

use self::models::LyricApiResponse;
use crate::error::{FetcherError, Result};
use crate::parser::{merge_lyric_lines, parse_lrc, parse_yrc};
use crate::providers::{
    LyricProvider, RequestBodyFormat, RequestInfo, TrackQuery, checked_json_parser,
};
use crate::search::matcher::sort_and_rate_results;
use lyrics_helper_core::ParsedSourceData;
use lyrics_helper_core::{MatchType, SearchResult, model::generic::Artist as CoreArtist};
use serde_json::json;

const SEARCH_CLOUDSEARCH_PC_PATH: &str = "/api/cloudsearch/pc";
const SEARCH_CLOUDSEARCH_PC_URL: &str = "https://interface.music.163.com/eapi/cloudsearch/pc";

const SONG_LYRIC_V1_PATH: &str = "/api/song/lyric/v1";
const SONG_LYRIC_V1_URL: &str = "https://interface3.music.163.com/eapi/song/lyric/v1";

pub struct NeteaseProvider;

impl LyricProvider for NeteaseProvider {
    fn prepare_search_request(&self, query: &TrackQuery) -> Result<RequestInfo> {
        let keyword = format!("{} {}", query.title, query.artists.join(" "));

        let payload = json!({
            "s": keyword,
            "type": "1",
            "limit": 30,
            "offset": 0,
            "total": true
        });

        let encrypted_params = crypto::prepare_eapi_params(SEARCH_CLOUDSEARCH_PC_PATH, &payload)?;

        Ok(RequestInfo {
            url: SEARCH_CLOUDSEARCH_PC_URL.to_string(),
            method: "POST".to_string(),
            headers: None,
            body: Some(format!("params={encrypted_params}")),
            body_format: RequestBodyFormat::UrlEncodedForm,
        })
    }

    fn handle_search_response(
        &self,
        response_text: &str,
        query: &TrackQuery,
    ) -> Result<Vec<SearchResult>> {
        let api_response: models::SearchResultData =
            checked_json_parser(response_text, 200, &["result"])?;

        let candidates = api_response
            .songs
            .into_iter()
            .map(|item| {
                let artists = item
                    .artists
                    .into_iter()
                    .map(|a| CoreArtist {
                        id: a.id.to_string(),
                        name: a.name,
                    })
                    .collect();
                SearchResult {
                    title: item.name,
                    artists,
                    album: Some(item.album.name),
                    album_id: Some(item.album.id.to_string()),
                    duration: Some(item.duration),
                    provider_id: item.id.to_string(),
                    provider_name: "netease".to_string(),
                    provider_id_num: Some(item.id),
                    cover_url: item.album.pic_url,
                    match_type: MatchType::None,
                    language: None,
                }
            })
            .collect();

        let sorted_results = query.with_track(|track| sort_and_rate_results(&track, candidates));

        Ok(sorted_results)
    }

    fn prepare_lyrics_request(&self, search_result: &SearchResult) -> Result<RequestInfo> {
        let song_id = &search_result.provider_id;

        let payload = json!({
            "id": song_id,
            "cp": "false",
            "lv": "0",
            "kv": "0",
            "tv": "0",
            "rv": "0",
            "yv": "0",
            "ytv": "0",
            "yrv": "0",
            "csrf_token": ""
        });

        let encrypted_params = crypto::prepare_eapi_params(SONG_LYRIC_V1_PATH, &payload)?;

        Ok(RequestInfo {
            url: SONG_LYRIC_V1_URL.to_string(),
            method: "POST".to_string(),
            headers: None,
            body: Some(format!("params={encrypted_params}")),
            body_format: RequestBodyFormat::UrlEncodedForm,
        })
    }

    fn handle_lyrics_response(&self, response_text: &str) -> Result<ParsedSourceData> {
        let api_response: LyricApiResponse = checked_json_parser(response_text, 200, &[])?;

        let main_parsed_result = api_response
            .yrc
            .as_ref()
            .and_then(|y| y.lyric.as_ref())
            .filter(|s| !s.is_empty())
            .map(|content| parse_yrc(content))
            .or_else(|| {
                api_response
                    .lrc
                    .as_ref()
                    .and_then(|l| l.lyric.as_ref())
                    .filter(|s| !s.is_empty())
                    .map(|content| parse_lrc(content))
            })
            .ok_or_else(|| FetcherError::Provider("Lyric not found in API response".to_string()))?;

        let mut main_parsed = main_parsed_result?;

        let translation_content = api_response
            .tlyric
            .and_then(|t| t.lyric)
            .filter(|s| !s.is_empty());

        let romanization_content = api_response
            .romalrc
            .and_then(|r| r.lyric)
            .filter(|s| !s.is_empty());

        let translation_lines = translation_content
            .map(|t| parse_lrc(&t).map(|p| p.lines))
            .transpose()?;

        let romanization_lines = romanization_content
            .map(|r| parse_lrc(&r).map(|p| p.lines))
            .transpose()?;

        let merged_lines =
            merge_lyric_lines(main_parsed.lines, translation_lines, romanization_lines);
        main_parsed.lines = merged_lines;

        main_parsed.source_name = "netease".to_string();

        Ok(main_parsed)
    }
}
