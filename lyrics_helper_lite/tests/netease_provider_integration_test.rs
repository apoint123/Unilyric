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
    let raw_lyrics = provider
        .handle_lyrics_response(LYRICS_RESPONSE_JSON)
        .unwrap();

    assert!(!raw_lyrics.content.is_empty());
    assert!(raw_lyrics.content.contains("](") && raw_lyrics.content.contains(",0)")); // 检查 YRC 歌词
    assert!(raw_lyrics.translation.is_none() || raw_lyrics.translation.as_deref() == Some(""));
}
