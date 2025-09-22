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
    let parsed_data = provider
        .handle_lyrics_response(LYRICS_RESPONSE_XML)
        .unwrap();

    assert!(
        parsed_data.raw_metadata.contains_key("ti"),
        "Metadata should contain title [ti]"
    );
    assert!(
        parsed_data.raw_metadata.contains_key("ar"),
        "Metadata should contain artist [ar]"
    );

    assert!(
        !parsed_data.lines.is_empty(),
        "Parsed lyric lines should not be empty"
    );

    let has_translation = parsed_data.lines.iter().any(|line| {
        line.main_track()
            .is_some_and(|t| !t.translations.is_empty())
    });

    assert!(
        has_translation,
        "A translation should be present and merged into the lyric lines"
    );

    let first_translation_text = parsed_data.lines[0].main_track().unwrap().translations[0].text();
    assert!(
        !first_translation_text.is_empty(),
        "First translation text should not be empty"
    );
}
