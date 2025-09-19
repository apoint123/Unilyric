mod decoder;
mod models;

use crate::error::{FetcherError, Result};
use crate::providers::qq::decoder::decrypt_qrc;
use crate::providers::{
    LyricProvider, RequestBodyFormat, RequestInfo, TrackQuery, checked_json_parser,
};
use crate::search::matcher::sort_and_rate_results;
use lyrics_helper_core::MatchType;
use lyrics_helper_core::model::generic::Artist as CoreArtist;
use lyrics_helper_core::{RawLyrics, SearchResult};
use quick_xml::Reader;
use quick_xml::events::Event;
use serde_json::json;

pub struct QQProvider;

impl LyricProvider for QQProvider {
    fn prepare_search_request(&self, query: &TrackQuery) -> Result<RequestInfo> {
        let keyword = format!("{} {}", query.title, query.artists.join(" "));

        let request_body = json!({
            "req_1": {
                "method": "DoSearchForQQMusicDesktop",
                "module": "music.search.SearchCgiService",
                "param": {
                    "num_per_page": 20,
                    "page_num": 1,
                    "query": keyword,
                    "search_type": 0,
                }
            }
        });

        Ok(RequestInfo {
            url: "https://u.y.qq.com/cgi-bin/musicu.fcg".to_string(),
            method: "POST".to_string(),
            headers: None,
            body: Some(request_body.to_string()),
            body_format: RequestBodyFormat::Json,
        })
    }

    fn handle_search_response(
        &self,
        response_text: &str,
        query: &TrackQuery,
    ) -> Result<Vec<SearchResult>> {
        let v: serde_json::Value = serde_json::from_str(response_text)?;
        let req_1 = v
            .get("req_1")
            .ok_or_else(|| FetcherError::Provider("Missing 'req_1' in response".to_string()))?;

        if let Some(code) = req_1.get("code").and_then(serde_json::Value::as_i64)
            && code != 0
        {
            return Err(FetcherError::Provider(format!(
                "QQ API returned error code: {code}"
            )));
        }

        let song_list: models::SongData =
            checked_json_parser(response_text, 0, &["req_1", "data", "body", "song"])?;

        let candidates = song_list
            .list
            .into_iter()
            .map(|item| {
                let artists = item
                    .singer
                    .into_iter()
                    .map(|s| CoreArtist {
                        id: s.mid,
                        name: s.name,
                    })
                    .collect();
                SearchResult {
                    title: item.name,
                    artists,
                    album: Some(item.album.name),
                    album_id: Some(item.album.mid),
                    duration: Some(item.interval * 1000),
                    provider_id: item.mid,
                    provider_name: "qq".to_string(),
                    provider_id_num: Some(item.id),
                    match_type: MatchType::None,
                    cover_url: None,
                    language: None,
                }
            })
            .collect();

        let sorted_results = query.with_track(|track| sort_and_rate_results(&track, candidates));

        Ok(sorted_results)
    }

    fn prepare_lyrics_request(&self, search_result: &SearchResult) -> Result<RequestInfo> {
        let music_id = search_result.provider_id_num.ok_or_else(|| {
            FetcherError::InvalidInput(
                "Missing 'provider_id_num' (musicid) for fetching lyrics".to_string(),
            )
        })?;

        Ok(RequestInfo {
            url: "https://c.y.qq.com/qqmusic/fcgi-bin/lyric_download.fcg".to_string(),
            method: "POST".to_string(),
            headers: Some(vec![(
                "Referer".to_string(),
                "https://y.qq.com/".to_string(),
            )]),
            body: Some(format!(
                "version=15&miniversion=82&lrctype=4&musicid={music_id}"
            )),
            body_format: RequestBodyFormat::UrlEncodedForm,
        })
    }

    fn handle_lyrics_response(&self, response_text: &str) -> Result<RawLyrics> {
        let xml_content = response_text
            .trim()
            .strip_prefix("<!--")
            .unwrap_or(response_text)
            .strip_suffix("-->")
            .unwrap_or(response_text)
            .trim();

        if xml_content.is_empty() {
            return Err(FetcherError::Provider(
                "Empty lyric XML response".to_string(),
            ));
        }

        let mut reader = Reader::from_str(xml_content);

        let mut buf = Vec::new();
        let mut main_lyrics_encrypted = String::new();
        let mut trans_lyrics_encrypted = String::new();
        let mut roma_lyrics_encrypted = String::new();
        let mut current_tag = String::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    current_tag = String::from_utf8(e.name().as_ref().to_vec()).unwrap_or_default();
                }
                Ok(Event::CData(e)) => {
                    let text = String::from_utf8(e.into_inner().to_vec()).unwrap_or_default();
                    if !text.trim().is_empty() {
                        match current_tag.as_str() {
                            "content" => main_lyrics_encrypted = text,
                            "contentts" => trans_lyrics_encrypted = text,
                            "contentroma" => roma_lyrics_encrypted = text,
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    return Err(FetcherError::Provider(format!(
                        "Failed to parse lyrics XML: {e}"
                    )));
                }
                Ok(Event::Eof) => break,
                _ => {}
            }
            buf.clear();
        }

        if main_lyrics_encrypted.is_empty() {
            return Err(FetcherError::Provider(
                "Lyric not found in XML response".to_string(),
            ));
        }

        let main_lyrics = decrypt_qrc(&main_lyrics_encrypted)?;
        let translation = decrypt_qrc(&trans_lyrics_encrypted).ok();
        let romanization = decrypt_qrc(&roma_lyrics_encrypted).ok();

        Ok(RawLyrics {
            format: "QRC".to_string(),
            content: main_lyrics,
            translation,
            romanization,
        })
    }
}
