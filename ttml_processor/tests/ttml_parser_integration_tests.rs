use std::path::Path;

use lyrics_helper_core::*;
use ttml_processor::{generate_ttml, parse_ttml};

fn load_test_data(filename: &str) -> String {
    let path = Path::new("tests/test_data").join(filename);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取测试文件 '{}' 失败: {e}", path.display()))
}

fn get_line_text(line: &LyricLine) -> Option<String> {
    let text = line
        .tracks
        .iter()
        .filter(|at| at.content_type == ContentType::Main)
        .flat_map(|at| at.content.words.iter().flat_map(|w| &w.syllables))
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() { None } else { Some(text) }
}

fn get_track_text(track: &LyricTrack) -> Option<String> {
    let text = track
        .words
        .iter()
        .flat_map(|w| &w.syllables)
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() { None } else { Some(text) }
}

fn get_syllables_from_line(line: &LyricLine, content_type: ContentType) -> Vec<&LyricSyllable> {
    line.tracks
        .iter()
        .filter(|at| at.content_type == content_type)
        .flat_map(|at| at.content.words.iter().flat_map(|w| &w.syllables))
        .collect()
}

#[test]
fn test_parse_line_timed_basic() {
    let content = load_test_data("line_timed_basic.ttml");
    let result = parse_ttml(&content, &TtmlParsingOptions::default()).unwrap();

    assert!(result.is_line_timed_source, "应该检测到逐行歌词");

    assert_eq!(result.lines.len(), 2, "应该解析两行歌词");

    let first_line = &result.lines[0];
    assert_eq!(first_line.start_ms, 10000);
    assert_eq!(first_line.end_ms, 15500);
    assert_eq!(get_line_text(first_line).as_deref(), Some("这是一行歌词."));
}

#[test]
fn test_parse_word_timed_basic() {
    let content = load_test_data("word_timed_basic.ttml");
    let result = parse_ttml(&content, &TtmlParsingOptions::default()).unwrap();

    assert!(!result.is_line_timed_source, "应该检测到逐字歌词");
    assert_eq!(result.lines.len(), 1);

    let line = &result.lines[0];
    let main_syllables = get_syllables_from_line(line, ContentType::Main);
    assert_eq!(main_syllables.len(), 2, "应该有两个音节");

    let first_syl = &main_syllables[0];
    assert_eq!(first_syl.text, "Hello");
    assert_eq!(first_syl.start_ms, 5000);
    assert_eq!(first_syl.end_ms, 5500);
    assert!(first_syl.ends_with_space, "第一个音节后面应该有空格");

    let second_syl = &main_syllables[1];
    assert_eq!(second_syl.text, "world");
    assert_eq!(second_syl.start_ms, 5600);
    assert_eq!(second_syl.end_ms, 6200);
    assert!(!second_syl.ends_with_space);
}

#[test]
fn test_metadata_extraction() {
    let content = load_test_data("full_metadata.ttml");
    let result = parse_ttml(&content, &TtmlParsingOptions::default()).unwrap();

    let metadata = &result.raw_metadata;

    assert_eq!(metadata.get("title").unwrap()[0], "My Awesome Song");
    assert_eq!(metadata.get("artist").unwrap()[0], "The Rustaceans");

    assert_eq!(metadata.get("songwriters").unwrap().len(), 2);
    assert_eq!(metadata.get("songwriters").unwrap()[0], "作曲者1号");

    let agent_store = &result.agents;
    assert_eq!(agent_store.agents_by_id.len(), 2, "应该解析出两个 aget");

    let agent_v1 = agent_store
        .agents_by_id
        .get("v1")
        .expect("应该找到 ID 为 v1 的 agent");
    assert_eq!(agent_v1.name.as_deref(), Some("演唱者1号"));
    assert_eq!(agent_v1.agent_type, AgentType::Person);

    let agent_v2 = agent_store
        .agents_by_id
        .get("v2")
        .expect("应该找到 ID 为 v2 的 agent");
    assert_eq!(agent_v2.name.as_deref(), Some("演唱者2号"));
    assert_eq!(agent_v2.agent_type, AgentType::Person);
}

#[test]
fn test_parse_word_timed_with_background() {
    let content = load_test_data("background_vocals.ttml");
    let result = parse_ttml(&content, &TtmlParsingOptions::default()).unwrap();

    assert!(!result.is_line_timed_source);

    let line_with_bg = result
        .lines
        .iter()
        .find(|l| {
            l.tracks
                .iter()
                .any(|at| at.content_type == ContentType::Background)
        })
        .expect("应该找到一行有背景人声的歌词");

    let bg_track = line_with_bg
        .tracks
        .iter()
        .find(|at| at.content_type == ContentType::Background)
        .map(|at| &at.content)
        .expect("背景轨道内容应存在");

    let bg_syllables: Vec<_> = bg_track.words.iter().flat_map(|w| &w.syllables).collect();

    let bg_start_ms = bg_syllables.iter().map(|s| s.start_ms).min().unwrap_or(0);
    let bg_end_ms = bg_syllables.iter().map(|s| s.end_ms).max().unwrap_or(0);

    assert_eq!(bg_start_ms, 15000);
    assert_eq!(bg_end_ms, 17500);
    assert_eq!(bg_syllables.len(), 2);
    assert_eq!(bg_syllables[0].text, "ooh");
    assert_eq!(bg_syllables[1].text, "aah");
}

#[test]
fn test_warning_generation_for_recoverable_issues() {
    let content = load_test_data("malformed_but_recoverable.ttml");
    let result = parse_ttml(&content, &TtmlParsingOptions::default()).unwrap();

    assert!(!result.warnings.is_empty(), "应该产生警告");

    assert!(
        result.warnings.iter().any(|w| w.contains("<br/>")),
        "应该警告 br 标签"
    );

    assert!(
        result.warnings.iter().any(|w| w.contains("时间戳无效")),
        "应该警告时间戳无效"
    );
}

#[test]
fn test_round_trip() {
    let content = load_test_data("real_world.ttml");
    let parsed_data = parse_ttml(&content, &TtmlParsingOptions::default()).unwrap();

    let mut metadata_store = MetadataStore::new();
    for (raw_key, values) in &parsed_data.raw_metadata {
        for value in values {
            metadata_store.add(raw_key, &value.clone()).unwrap();
        }
    }

    let timing_mode = if parsed_data.is_line_timed_source {
        TtmlTimingMode::Line
    } else {
        TtmlTimingMode::Word
    };
    let options = TtmlGenerationOptionsBuilder::default()
        .timing_mode(timing_mode)
        .format(true)
        .build()
        .unwrap();

    let agent_store = MetadataStore::to_agent_store(&metadata_store);
    let generated_ttml_output =
        generate_ttml(&parsed_data.lines, &metadata_store, &agent_store, &options).unwrap();

    insta::assert_snapshot!(generated_ttml_output);
}

#[test]
#[allow(clippy::too_many_lines)]
fn test_parse_formatted_ttml() {
    // 格式化TTML，<span>之间只有换行符。预期：无空格。
    let formatted_no_space_content = r#"
<tt xmlns="http://www.w3.org/ns/ttml" itunes:timing="word" xmlns:itunes="http://itunes.apple.com/lyric-ttml-extensions">
<body>
  <p begin="0s" end="2s">
    <span begin="0s" end="1s">Hello</span>
    <span begin="1s" end="2s">World</span>
  </p>
</body>
</tt>
"#;
    let result1 = parse_ttml(formatted_no_space_content, &TtmlParsingOptions::default()).unwrap();

    assert!(
        result1.detected_formatted_ttml_input.unwrap_or(false),
        "场景1: 应该检测到格式化输入"
    );
    let line1 = &result1.lines[0];
    let main_syllables1: Vec<&LyricSyllable> = line1
        .main_track()
        .map(|track| {
            track
                .content
                .words
                .iter()
                .flat_map(|word| &word.syllables)
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(main_syllables1.len(), 2);
    assert!(
        !main_syllables1[0].ends_with_space,
        "场景1: 'Hello' 后面不应该有空格"
    );

    // 格式化TTML，<span>之间有一个明确的空格。预期：有空格。
    let formatted_with_space_content = r#"
<tt xmlns="http://www.w3.org/ns/ttml" itunes:timing="word" xmlns:itunes="http://itunes.apple.com/lyric-ttml-extensions">
<body>
  <p begin="0s" end="2s">
    <span begin="0s" end="1s">Hello</span> <span begin="1s" end="2s">World</span>
  </p>
</body>
</tt>
"#;
    let result2 = parse_ttml(formatted_with_space_content, &TtmlParsingOptions::default()).unwrap();

    assert!(
        result2.detected_formatted_ttml_input.unwrap_or(false),
        "场景2: 应该检测到格式化输入"
    );
    let line2 = &result2.lines[0];
    let main_syllables2: Vec<&LyricSyllable> = line2
        .main_track()
        .map(|track| {
            track
                .content
                .words
                .iter()
                .flat_map(|word| &word.syllables)
                .collect()
        })
        .unwrap_or_default();
    assert!(
        main_syllables2[0].ends_with_space,
        "场景2: 'Hello' 后面应该有空格"
    );

    // 未格式化的TTML，<span>之间有空格。预期：有空格。
    let unformatted_with_space_content = r#"<tt xmlns="http://www.w3.org/ns/ttml" itunes:timing="word" xmlns:itunes="http://itunes.apple.com/lyric-ttml-extensions"><body><p begin="0s" end="2s"><span begin="0s" end="1s">Hello</span> <span begin="1s" end="2s">World</span></p></body></tt>"#;
    let result3 = parse_ttml(
        unformatted_with_space_content,
        &TtmlParsingOptions::default(),
    )
    .unwrap();

    let line3 = &result3.lines[0];
    let main_syllables3: Vec<&LyricSyllable> = line3
        .main_track()
        .map(|track| {
            track
                .content
                .words
                .iter()
                .flat_map(|word| &word.syllables)
                .collect()
        })
        .unwrap_or_default();
    assert!(
        main_syllables3[0].ends_with_space,
        "场景3: 在未格式化输入中，'Hello' 后面应该有空格"
    );

    // 混合了紧邻和非紧邻<span>的格式化文件。预期：精确识别空格。
    let mixed_formatted_content = r#"
<tt xmlns="http://www.w3.org/ns/ttml" itunes:timing="word" xmlns:itunes="http://itunes.apple.com/lyric-ttml-extensions">
<body>
  <p begin="31s" end="36s">
    <span begin="31s" end="32s">1</span
    ><span begin="32s" end="33s">2</span>
    <span begin="34s" end="35s">3</span>
  </p>
</body>
</tt>
"#;
    let result4 = parse_ttml(mixed_formatted_content, &TtmlParsingOptions::default()).unwrap();

    assert!(
        result4.detected_formatted_ttml_input.unwrap_or(false),
        "场景4: 应该检测到格式化输入"
    );
    let line4 = &result4.lines[0];
    let main_syllables4: Vec<&LyricSyllable> = line4
        .main_track()
        .map(|track| {
            track
                .content
                .words
                .iter()
                .flat_map(|word| &word.syllables)
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(main_syllables4.len(), 3);
    assert!(
        !main_syllables4[0].ends_with_space,
        "场景4: '1' 后面不应该有空格 (紧邻)"
    );
    assert!(
        !main_syllables4[1].ends_with_space,
        "场景4: '2' 后面不应该有空格 (换行分隔)"
    );
}

#[test]
fn test_fatal_error_on_invalid_xml() {
    let content = load_test_data("invalid_xml.ttml");
    let result = parse_ttml(&content, &TtmlParsingOptions::default());

    assert!(result.is_err(), "解析XML应该报错");

    assert!(
        matches!(result.unwrap_err(), ConvertError::Xml(_)),
        "错误类型应该为 ConvertError::Xml"
    );
}

#[test]
fn test_parse_line_timed_translation() {
    let content = r#"
<tt xmlns="http://www.w3.org/ns/ttml" xmlns:itunes="http://music.apple.com/lyric-ttml-internal" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xml:lang="ja">
<head>
  <metadata>
    <iTunesMetadata>
      <translations>
        <translation xml:lang="zh-Hans">
          <text for="L1">第一行翻译</text>
          <text for="L2">第二行翻译</text>
        </translation>
      </translations>
    </iTunesMetadata>
  </metadata>
</head>
<body>
  <p begin="1.0s" end="2.0s" itunes:key="L1"><span begin="1.0s" end="2.0s">一行目</span></p>
  <p begin="3.0s" end="4.0s" itunes:key="L2"><span begin="3.0s" end="4.0s">二行目</span></p>
</body>
</tt>
"#;
    let result = parse_ttml(content, &TtmlParsingOptions::default()).unwrap();

    assert_eq!(result.lines.len(), 2, "应该解析出两行歌词");

    let line1 = &result.lines[0];
    let translation_tracks1: Vec<_> = line1
        .tracks
        .iter()
        .flat_map(|at| &at.translations)
        .collect();
    assert_eq!(translation_tracks1.len(), 1, "第一行应该有一条翻译轨道");
    let track1 = translation_tracks1[0];
    assert_eq!(
        get_track_text(track1).as_deref(),
        Some("第一行翻译"),
        "第一行翻译文本不匹配"
    );
    assert!(
        track1
            .words
            .iter()
            .flat_map(|w| &w.syllables)
            .all(|s| s.duration_ms.is_none()),
        "逐行翻译不应有时间戳"
    );

    let line2 = &result.lines[1];
    let translation_tracks2: Vec<_> = line2
        .tracks
        .iter()
        .flat_map(|at| &at.translations)
        .collect();
    assert_eq!(translation_tracks2.len(), 1, "第二行应该有一条翻译轨道");
    assert_eq!(
        get_track_text(translation_tracks2[0]).as_deref(),
        Some("第二行翻译")
    );
}

#[test]
fn test_parse_timed_romanization() {
    let content = r#"
<tt xmlns="http://www.w3.org/ns/ttml" xmlns:itunes="http://music.apple.com/lyric-ttml-internal" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xml:lang="ja">
  <head>
    <metadata>
      <iTunesMetadata>
        <transliterations>
          <transliteration xml:lang="ja-Latn">
            <text for="L1">
              <span begin="1.0s" end="1.5s">Asa</span>
              <span begin="1.6s" end="2.0s">mo</span>
            </text>
          </transliteration>
        </transliterations>
      </iTunesMetadata>
    </metadata>
  </head>
  <body>
    <p begin="1.0s" end="2.0s" itunes:key="L1">
        <span begin="1.0s" end="1.5s">朝</span>
        <span begin="1.6s" end="2.0s">も</span>
    </p>
  </body>
</tt>
"#;
    let result = parse_ttml(content, &TtmlParsingOptions::default()).unwrap();

    assert_eq!(result.lines.len(), 1, "应该解析出一行歌词");
    let line = &result.lines[0];

    let romanization_tracks: Vec<&LyricTrack> = line
        .main_track()
        .map(|track| track.romanizations.iter().collect())
        .unwrap_or_default();
    assert_eq!(romanization_tracks.len(), 1, "应该有一组逐字罗马音");
    let romanization_track = romanization_tracks[0];
    let romanization_syllables: Vec<_> = romanization_track
        .words
        .iter()
        .flat_map(|w| &w.syllables)
        .collect();

    assert_eq!(
        romanization_track
            .metadata
            .get(&TrackMetadataKey::Language)
            .map(std::string::String::as_str),
        Some("ja-Latn"),
        "罗马音语言应为 ja-Latn"
    );
    assert_eq!(romanization_syllables.len(), 2, "罗马音应该有两个音节");

    assert_eq!(romanization_syllables[0].text, "Asa");
    assert_eq!(romanization_syllables[0].start_ms, 1000);
    assert_eq!(romanization_syllables[1].text, "mo");
    assert_eq!(romanization_syllables[1].end_ms, 2000);
}

#[test]
fn test_parse_timed_translation() {
    let content = r#"
<tt xmlns="http://www.w3.org/ns/ttml" xmlns:itunes="http://music.apple.com/lyric-ttml-internal" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xml:lang="zh-Hant">
  <head>
    <metadata>
      <iTunesMetadata>
        <translations>
          <translation xml:lang="zh-Hans">
            <text for="L1">
              <span begin="28.140s" end="28.922s">钟声响起</span>
              <span begin="28.922s" end="29.582s">归家</span>
            </text>
          </translation>
        </translations>
      </iTunesMetadata>
    </metadata>
  </head>
  <body>
    <p begin="28.140s" end="29.582s" itunes:key="L1">
        <span begin="28.140s" end="29.582s">鐘聲響起歸家</span>
    </p>
  </body>
</tt>
"#;
    let result = parse_ttml(content, &TtmlParsingOptions::default()).unwrap();

    assert_eq!(result.lines.len(), 1, "应该解析出一行歌词");
    let line = &result.lines[0];

    let translation_tracks: Vec<&LyricTrack> = line
        .main_track()
        .map(|track| track.translations.iter().collect())
        .unwrap_or_default();
    assert_eq!(translation_tracks.len(), 1, "应该有一组逐字翻译");
    let translation_track = translation_tracks[0];
    let translation_syllables: Vec<_> = translation_track
        .words
        .iter()
        .flat_map(|w| &w.syllables)
        .collect();

    assert_eq!(
        translation_track
            .metadata
            .get(&TrackMetadataKey::Language)
            .map(std::string::String::as_str),
        Some("zh-Hans"),
        "翻译语言应为 zh-Hans"
    );
    assert_eq!(translation_syllables.len(), 2, "翻译应该有两个音节");

    assert_eq!(translation_syllables[0].text, "钟声响起");
    assert_eq!(translation_syllables[0].start_ms, 28140);
    assert_eq!(translation_syllables[1].text, "归家");
    assert_eq!(translation_syllables[1].end_ms, 29582);
}

#[test]
fn test_parse_apple_music_timed_auxiliary_tracks() {
    let content = r#"
<tt xmlns="http://www.w3.org/ns/ttml" xmlns:itunes="http://music.apple.com/lyric-ttml-internal" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xml:lang="ko">
<head>
  <metadata>
    <iTunesMetadata>
      <translations>
        <translation xml:lang="en-US">
          <text for="L1">
            <span begin="10.0s" end="10.8s">I'm not afraid</span>
            <span ttm:role="x-bg" begin="11.0s" end="11.8s">(Just interesting)</span>
          </text>
        </translation>
      </translations>
      <transliterations>
        <transliteration xml:lang="ko-Latn">
          <text for="L1">
            <span begin="10.0s" end="10.8s">duryeopjineun ana</span>
            <span ttm:role="x-bg">
              <span begin="11.0s" end="11.4s">heungmiroul</span>
              <span begin="11.4s" end="11.8s">ppun</span>
            </span>
          </text>
        </transliteration>
      </transliterations>
    </iTunesMetadata>
  </metadata>
</head>
<body>
  <p begin="10.0s" end="12.0s" itunes:key="L1">
      <span begin="10.0s" end="10.8s">두렵지는 않아</span>
      <span ttm:role="x-bg">
          <span begin="11.0s" end="11.8s">(흥미로울 뿐)</span>
      </span>
  </p>
</body>
</tt>
"#;
    let result = parse_ttml(content, &TtmlParsingOptions::default()).unwrap();

    assert_eq!(result.lines.len(), 1, "应该解析出一行歌词");
    let line = &result.lines[0];

    let main_at = line
        .tracks
        .iter()
        .find(|at| at.content_type == ContentType::Main)
        .expect("应找到主轨道");
    assert_eq!(main_at.translations.len(), 1);
    assert_eq!(
        get_track_text(&main_at.translations[0]).as_deref(),
        Some("I'm not afraid")
    );
    assert_eq!(main_at.romanizations.len(), 1);
    assert_eq!(
        get_track_text(&main_at.romanizations[0]).as_deref(),
        Some("duryeopjineun ana")
    );

    let bg_at = line
        .tracks
        .iter()
        .find(|at| at.content_type == ContentType::Background)
        .expect("应该找到背景内容轨道");
    assert_eq!(bg_at.translations.len(), 1);
    assert_eq!(
        get_track_text(&bg_at.translations[0]).as_deref(),
        Some("Just interesting")
    );
    assert_eq!(bg_at.romanizations.len(), 1);
    let bg_roman_syls: Vec<_> = bg_at.romanizations[0]
        .words
        .iter()
        .flat_map(|w| &w.syllables)
        .collect();

    assert_eq!(bg_roman_syls.len(), 2);
    assert_eq!(bg_roman_syls[0].text, "heungmiroul");
    assert_eq!(bg_roman_syls[1].text, "ppun");
}

#[test]
fn test_complex_round_trip() {
    let content = load_test_data("complex_round_trip.ttml");
    let parsed_data = parse_ttml(&content, &TtmlParsingOptions::default()).unwrap();

    let agent_store = &parsed_data.agents;

    let metadata_store = MetadataStore::from(&parsed_data);

    let options = TtmlGenerationOptionsBuilder::default()
        .timing_mode(TtmlTimingMode::Word)
        .format(true)
        .build()
        .unwrap();

    let generated_ttml_output =
        generate_ttml(&parsed_data.lines, &metadata_store, agent_store, &options).unwrap();

    insta::assert_snapshot!(generated_ttml_output);
}
