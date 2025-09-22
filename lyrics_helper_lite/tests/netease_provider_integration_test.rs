use lyrics_helper_lite::providers::{LyricProvider, TrackQuery, netease::NeteaseProvider};

const SEARCH_RESPONSE_JSON: &str = include_str!("test_data/netease_search_response.json");
const LYRICS_RESPONSE_JSON: &str = include_str!("test_data/netease_lyrics_response.json");

#[test]
fn test_handle_netease_search_response() {
    let provider = NeteaseProvider;
    let mock_query = TrackQuery {
        title: "富士山下".to_string(),
        artists: vec!["陈奕迅".to_string()],
        album: Some("What's Going On…?".to_string()),
        duration: Some(258_902),
    };

    let results = provider
        .handle_search_response(SEARCH_RESPONSE_JSON, &mock_query)
        .unwrap();

    assert!(!results.is_empty());

    let result = &results[0];
    assert_eq!(result.title, "富士山下");
    assert_eq!(result.artists.len(), 1);
    assert_eq!(result.artists[0].name, "陈奕迅");
    assert_eq!(result.album.as_deref(), Some("What's Going On…?"));
    assert_eq!(result.provider_id, "65766");
    assert!(result.match_type == lyrics_helper_core::MatchType::Perfect);
}

#[test]
fn test_handle_netease_lyrics_response() {
    let provider = NeteaseProvider;
    let parsed_data = provider
        .handle_lyrics_response(LYRICS_RESPONSE_JSON)
        .unwrap();

    assert!(
        !parsed_data.lines.is_empty(),
        "There should be parsed lyric lines"
    );

    let first_line = &parsed_data.lines[0];
    assert!(
        first_line.start_ms > 0,
        "First line should have a start time"
    );
    assert!(
        first_line.main_text().is_some(),
        "First line should have main text"
    );

    let has_translation = parsed_data.lines.iter().any(|line| {
        line.main_track()
            .is_some_and(|t| !t.translations.is_empty())
    });
    assert!(
        !has_translation,
        "Translation should not be present in this test case"
    );
}
