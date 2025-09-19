use lyrics_helper_core::SearchResult;
use lyrics_helper_lite::providers::{LyricProvider, TrackQuery, qq::QQProvider};

const SEARCH_RESPONSE_JSON: &str = include_str!("test_data/qq_search_response.json");
const LYRICS_RESPONSE_XML: &str = include_str!("test_data/qq_lyrics_response.xml");

#[test]
fn test_prepare_search_request() {
    let provider = QQProvider;
    let query = TrackQuery {
        title: "内向都是作曲家".to_string(),
        artists: vec!["Yunomi".to_string(), "nicamoq".to_string()],
        album: None,
        duration: None,
    };

    let request_info = provider.prepare_search_request(&query).unwrap();
    assert_eq!(request_info.url, "https://u.y.qq.com/cgi-bin/musicu.fcg");
    assert!(
        request_info
            .body
            .unwrap()
            .contains("\"query\":\"内向都是作曲家 Yunomi nicamoq\"")
    );
}

#[test]
fn test_handle_search_response() {
    let provider = QQProvider;

    let mock_query = TrackQuery {
        title: "インドア系ならトラックメイカー".to_string(),
        artists: vec!["Yunomi".to_string(), "nicamoq".to_string()],
        album: Some("インドア系ならトラックメイカー".to_string()),
        duration: None,
    };

    let results = provider
        .handle_search_response(SEARCH_RESPONSE_JSON, &mock_query)
        .unwrap();

    assert!(!results.is_empty());

    let result = &results[0];
    assert_eq!(result.title, "インドア系ならトラックメイカー");
    assert_eq!(result.artists.len(), 2);
    assert_eq!(result.artists[0].name, "Yunomi");
    assert_eq!(
        result.album.as_deref(),
        Some("インドア系ならトラックメイカー")
    );
    assert_eq!(result.provider_id_num, Some(108_030_645));
    assert!(result.match_type == lyrics_helper_core::MatchType::Perfect);
}

#[test]
fn test_prepare_lyrics_request() {
    let provider = QQProvider;
    let search_result = SearchResult {
        provider_id_num: Some(108_030_645),
        ..Default::default()
    };

    let request_info = provider.prepare_lyrics_request(&search_result).unwrap();
    assert_eq!(
        request_info.url,
        "https://c.y.qq.com/qqmusic/fcgi-bin/lyric_download.fcg"
    );
    assert!(request_info.body.unwrap().contains("musicid=108030645"));
}

#[test]
fn test_handle_lyrics_response() {
    let provider = QQProvider;
    let raw_lyrics = provider
        .handle_lyrics_response(LYRICS_RESPONSE_XML)
        .unwrap();

    assert!(
        !raw_lyrics.content.is_empty(),
        "Decrypted lyric content should not be empty"
    );
    assert!(
        raw_lyrics.content.contains("[ti:"),
        "Lyric should contain metadata tags"
    );
    assert!(
        raw_lyrics.content.contains("[ar:"),
        "Lyric should contain metadata tags"
    );
    assert!(
        raw_lyrics.translation.is_some(),
        "Translation should be present"
    );
    assert!(
        !raw_lyrics.translation.as_ref().unwrap().is_empty(),
        "Translation should not be empty"
    );
}
