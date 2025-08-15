//! # LRC 格式解析器

use regex::Regex;
use std::cell::OnceCell;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::converter::utils::{normalize_text_whitespace, parse_and_store_metadata};

use lyrics_helper_core::{
    AnnotatedTrack, ContentType, ConvertError, LrcLineRole, LrcParsingOptions,
    LrcSameTimestampStrategy, LyricFormat, LyricLine, LyricLineBuilder, LyricSyllableBuilder,
    LyricTrack, ParsedSourceData, Word,
};

/// 用于匹配一个完整的 LRC 歌词行，捕获时间戳部分和文本部分
static LRC_LINE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^((?:\[\d{2,}:\d{2}[.:]\d{2,3}])+)(.*)$").expect("未能编译 LRC_LINE_REGEX")
});

/// 用于从一个时间戳组中提取出单个时间戳
static LRC_TIMESTAMP_EXTRACT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[(\d{2,}):(\d{2})[.:](\d{2,3})]").expect("未能编译 LRC_TIMESTAMP_EXTRACT_REGEX")
});

const DEFAULT_LAST_LINE_DURATION_MS: u64 = 10000;

/// 解析 LRC 格式内容到 `ParsedSourceData` 结构。
pub fn parse_lrc(
    content: &str,
    options: &LrcParsingOptions,
) -> Result<ParsedSourceData, ConvertError> {
    struct TempLrcEntry {
        timestamp_ms: u64,
        text: String,
    }

    let mut raw_metadata: HashMap<String, Vec<String>> = HashMap::new();
    let mut warnings: Vec<String> = Vec::new();

    let mut temp_entries: Vec<TempLrcEntry> = Vec::new();

    for (line_num, line_str) in content.lines().enumerate() {
        let line_str_trimmed = line_str.trim();
        if line_str_trimmed.is_empty()
            || parse_and_store_metadata(line_str_trimmed, &mut raw_metadata)
        {
            continue;
        }

        if let Some(line_caps) = LRC_LINE_REGEX.captures(line_str_trimmed) {
            let all_timestamps_str = line_caps.get(1).map_or("", |m| m.as_str());
            let raw_text_part = line_caps.get(2).map_or("", |m| m.as_str());
            let text_part = normalize_text_whitespace(raw_text_part);

            for ts_cap in LRC_TIMESTAMP_EXTRACT_REGEX.captures_iter(all_timestamps_str) {
                let minutes: u64 = ts_cap[1].parse()?;
                let seconds: u64 = ts_cap[2].parse()?;
                let fraction_str = &ts_cap[3];
                let milliseconds: Result<u64, ConvertError> = match fraction_str.len() {
                    2 => Ok(fraction_str.parse::<u64>().map(|f| f * 10)?),
                    3 => Ok(fraction_str.parse::<u64>()?),
                    _ => Err(ConvertError::InvalidTime(format!(
                        "无效的毫秒部分: {fraction_str}"
                    ))),
                };
                if let Ok(ms) = milliseconds {
                    if seconds < 60 {
                        temp_entries.push(TempLrcEntry {
                            timestamp_ms: (minutes * 60 + seconds) * 1000 + ms,
                            text: text_part.clone(),
                        });
                    } else {
                        warnings.push(format!("LRC秒数无效 (行 {}): '{}'", line_num + 1, seconds));
                    }
                }
            }
        }
    }

    temp_entries.sort_by_key(|e| e.timestamp_ms);

    let primary_language_cache: OnceCell<heuristic_analyzer::PrimaryLanguage> = OnceCell::new();

    let mut final_lyric_lines: Vec<LyricLine> = Vec::new();
    let mut i = 0;
    while i < temp_entries.len() {
        let start_ms = temp_entries[i].timestamp_ms;

        // 将所有具有相同时间戳的行分组
        let mut next_event_index = i;
        while let Some(next_entry) = temp_entries.get(next_event_index) {
            if next_entry.timestamp_ms == start_ms {
                next_event_index += 1;
            } else {
                break;
            }
        }
        let group_lines: Vec<&TempLrcEntry> = temp_entries[i..next_event_index].iter().collect();

        // 如果分组完全由空行组成, 它的作用只是结束标记, 跳过即可
        if group_lines.is_empty() || group_lines.iter().all(|e| e.text.is_empty()) {
            i = next_event_index;
            continue;
        }

        let end_ms = temp_entries
            .get(next_event_index)
            .map_or(start_ms + DEFAULT_LAST_LINE_DURATION_MS, |next| {
                next.timestamp_ms.max(start_ms)
            });

        // 根据所选策略处理分组
        let tracks: Vec<AnnotatedTrack> = match &options.same_timestamp_strategy {
            LrcSameTimestampStrategy::Heuristic => {
                let lang = *primary_language_cache.get_or_init(|| {
                    let all_text: Vec<&str> =
                        temp_entries.iter().map(|e| e.text.as_str()).collect();
                    heuristic_analyzer::determine_primary_language(&all_text)
                });

                let line_texts: Vec<&str> = group_lines.iter().map(|e| e.text.as_str()).collect();

                let assignments = heuristic_analyzer::assign_roles(&line_texts, lang);
                if let Some(track) =
                    heuristic_analyzer::build_annotated_track(&assignments, start_ms, end_ms)
                {
                    vec![track]
                } else {
                    vec![]
                }
            }
            LrcSameTimestampStrategy::FirstIsMain => {
                let meaningful_lines: Vec<_> = group_lines
                    .into_iter()
                    .filter(|e| !e.text.is_empty())
                    .collect();
                if meaningful_lines.is_empty() {
                    vec![]
                } else {
                    let main_entry = meaningful_lines[0];
                    let translations_entries = &meaningful_lines[1..];

                    let main_track =
                        new_line_timed_track(main_entry.text.clone(), start_ms, end_ms);

                    let translations = translations_entries
                        .iter()
                        .map(|entry| new_line_timed_track(entry.text.clone(), start_ms, end_ms))
                        .collect();

                    vec![AnnotatedTrack {
                        content_type: ContentType::Main,
                        content: main_track,
                        translations,
                        ..Default::default()
                    }]
                }
            }
            LrcSameTimestampStrategy::AllAreMain => group_lines
                .iter()
                .filter(|e| !e.text.is_empty())
                .map(|entry| {
                    let main_track = new_line_timed_track(entry.text.clone(), start_ms, end_ms);
                    AnnotatedTrack {
                        content_type: ContentType::Main,
                        content: main_track,
                        ..Default::default()
                    }
                })
                .collect(),
            LrcSameTimestampStrategy::UseRoleOrder(roles) => {
                if group_lines.len() != roles.len() {
                    warnings.push(format!(
                        "{}ms: 歌词行数（{}）与提供的角色数（{}）不匹配。",
                        start_ms,
                        group_lines.len(),
                        roles.len()
                    ));
                }

                let mut main_content: Option<LyricTrack> = None;
                let mut translations: Vec<LyricTrack> = vec![];
                let mut romanizations: Vec<LyricTrack> = vec![];

                let mut main_role_assigned = false;

                for (entry, role) in group_lines.iter().zip(roles.iter()) {
                    if entry.text.is_empty() {
                        continue; // 空行作为占位符, 直接跳过
                    }

                    let track = new_line_timed_track(entry.text.clone(), start_ms, end_ms);

                    match role {
                        LrcLineRole::Main => {
                            if main_role_assigned {
                                warnings.push(format!(
                                    "{start_ms}ms：指定了多个主歌词行。随后的主歌词行将被视为翻译行。"
                                ));
                                translations.push(track);
                            } else {
                                main_content = Some(track);
                                main_role_assigned = true;
                            }
                        }
                        LrcLineRole::Translation => {
                            translations.push(track);
                        }
                        LrcLineRole::Romanization => {
                            romanizations.push(track);
                        }
                    }
                }

                if main_content.is_none() && !group_lines.iter().all(|e| e.text.is_empty()) {
                    warnings.push(format!(
                        "{start_ms}ms: 未设置主歌词行。默认将第一行作为主歌词行。"
                    ));
                    if let Some(first_non_empty) = group_lines.iter().find(|e| !e.text.is_empty()) {
                        main_content = Some(new_line_timed_track(
                            first_non_empty.text.clone(),
                            start_ms,
                            end_ms,
                        ));
                    }
                }

                if let Some(main_track) = main_content {
                    vec![AnnotatedTrack {
                        content_type: ContentType::Main,
                        content: main_track,
                        translations,
                        romanizations,
                    }]
                } else {
                    vec![]
                }
            }
        };

        if !tracks.is_empty() {
            let line = LyricLineBuilder::default()
                .tracks(tracks)
                .start_ms(start_ms)
                .end_ms(end_ms)
                .build()
                .unwrap();
            final_lyric_lines.push(line);
        }

        i = next_event_index;
    }

    Ok(ParsedSourceData {
        lines: final_lyric_lines,
        raw_metadata,
        source_format: LyricFormat::Lrc,
        is_line_timed_source: true,
        warnings,
        ..Default::default()
    })
}

fn new_line_timed_track(text: String, start_ms: u64, end_ms: u64) -> LyricTrack {
    LyricTrack {
        words: vec![Word {
            syllables: vec![
                LyricSyllableBuilder::default()
                    .text(text)
                    .start_ms(start_ms)
                    .end_ms(end_ms)
                    .build()
                    .unwrap(),
            ],

            ..Default::default()
        }],
        ..Default::default()
    }
}

mod heuristic_analyzer {
    use fst::Set;
    use std::collections::{HashMap, HashSet};
    use std::sync::LazyLock;

    const DICTIONARY_FST_DATA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/dictionary.fst"));

    #[derive(Debug)]
    struct WordChecker {
        words: Set<&'static [u8]>,
    }

    impl WordChecker {
        fn new() -> Self {
            let words = Set::new(DICTIONARY_FST_DATA)
                .expect("Embedded FST data is malformed. This indicates a build-time error.");
            WordChecker { words }
        }

        fn is_english_word(&self, word: &str) -> bool {
            self.words.contains(word.to_lowercase())
        }
    }

    static WORD_CHECKER: LazyLock<WordChecker> = LazyLock::new(WordChecker::new);

    fn is_hiragana(c: char) -> bool {
        ('\u{3040}'..='\u{309F}').contains(&c)
    }
    fn is_katakana(c: char) -> bool {
        ('\u{30A0}'..='\u{30FF}').contains(&c)
    }
    fn is_japanese_kana(c: char) -> bool {
        is_hiragana(c) || is_katakana(c)
    }
    fn is_hangul(c: char) -> bool {
        ('\u{AC00}'..='\u{D7A3}').contains(&c)
    }
    fn is_cjk_ideograph(c: char) -> bool {
        ('\u{4E00}'..='\u{9FFF}').contains(&c)
    }
    fn is_latin(c: char) -> bool {
        c.is_ascii_alphabetic()
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PrimaryLanguage {
        Japanese,
        Korean,
        Chinese,
        LatinBased,
    }

    pub fn determine_primary_language(all_lines: &[&str]) -> PrimaryLanguage {
        let mut kana_count = 0;
        let mut hangul_count = 0;
        let mut cjk_count = 0;
        let mut latin_count = 0;

        for line in all_lines {
            for c in line.chars() {
                if is_japanese_kana(c) {
                    kana_count += 1;
                } else if is_hangul(c) {
                    hangul_count += 1;
                } else if is_cjk_ideograph(c) {
                    cjk_count += 1;
                } else if is_latin(c) {
                    latin_count += 1;
                }
            }
        }

        if kana_count > 0 {
            PrimaryLanguage::Japanese
        } else if hangul_count > 0 {
            PrimaryLanguage::Korean
        } else if latin_count > cjk_count && cjk_count < 10 {
            PrimaryLanguage::LatinBased
        } else {
            PrimaryLanguage::Chinese
        }
    }

    #[derive(Debug, Default, Clone)]
    struct LineScores {
        primary: f64,
        translation: f64,
        romanization: f64,
    }

    static ROMAJI_PARTICLES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
        HashSet::from(["no", "ga", "wa", "de", "wo", "ni", "to", "mo", "he", "ka"])
    });

    #[allow(clippy::cast_precision_loss)]
    fn calculate_scores(line: &str, lang: PrimaryLanguage, index: usize) -> LineScores {
        let mut scores = LineScores::default();
        if line.is_empty() {
            return scores;
        }

        let mut kana_count = 0;
        let mut hangul_count = 0;
        let mut cjk_count = 0;
        let mut latin_count = 0;

        for c in line.chars() {
            if is_japanese_kana(c) {
                kana_count += 1;
            } else if is_hangul(c) {
                hangul_count += 1;
            } else if is_cjk_ideograph(c) {
                cjk_count += 1;
            } else if is_latin(c) {
                latin_count += 1;
            }
        }

        let is_latin_dominant = latin_count > 0 && (cjk_count + hangul_count + kana_count == 0);

        if kana_count > 0 {
            scores.primary += 10.0;
        }
        if hangul_count > 0 {
            scores.primary += 10.0;
        }
        if cjk_count > 0 && kana_count == 0 && hangul_count == 0 {
            match lang {
                PrimaryLanguage::Japanese | PrimaryLanguage::Korean => scores.translation += 10.0,
                PrimaryLanguage::Chinese => scores.primary += 10.0,
                PrimaryLanguage::LatinBased => scores.translation += 7.0,
            }
        }

        if is_latin_dominant {
            let words: Vec<&str> = line
                .split_whitespace()
                .map(|w| w.trim_matches(|p: char| !p.is_alphanumeric()))
                .filter(|w| !w.is_empty())
                .collect();
            if !words.is_empty() {
                let matched_words = words
                    .iter()
                    .filter(|w| WORD_CHECKER.is_english_word(w))
                    .count();
                let match_rate = matched_words as f64 / words.len() as f64;

                if lang == PrimaryLanguage::LatinBased {
                    scores.primary += 10.0 * match_rate.max(0.1);
                } else {
                    scores.translation += 5.0;
                    scores.romanization += 5.0;

                    if match_rate > 0.5 {
                        scores.translation += 10.0 * match_rate;
                    }

                    let avg_word_len = words.iter().map(|w| w.chars().count()).sum::<usize>()
                        as f32
                        / words.len() as f32;
                    if avg_word_len < 4.5 {
                        scores.romanization += 5.0;
                    }
                    scores.romanization += words
                        .iter()
                        .filter(|w| ROMAJI_PARTICLES.contains(w.to_lowercase().as_str()))
                        .count() as f64;

                    let pinyin_tone_word_count = words
                        .iter()
                        .filter(|w| {
                            if let Some(last_char) = w.chars().last() {
                                last_char.is_ascii_digit()
                                    && w.chars().any(|c| c.is_ascii_alphabetic())
                            } else {
                                false
                            }
                        })
                        .count();
                    if pinyin_tone_word_count > 0 {
                        scores.romanization += pinyin_tone_word_count as f64 * 4.0;
                    }
                }
            }
        }

        match index {
            0 => scores.primary += 1.0,
            1 => scores.translation += 1.0,
            2 => scores.romanization += 1.0,
            _ => {}
        }

        scores
    }

    #[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
    pub enum Role {
        Primary,
        Translation,
        Romanization,
    }

    pub fn assign_roles<'a>(
        group_lines: &[&'a str],
        lang: PrimaryLanguage,
    ) -> HashMap<Role, &'a str> {
        if group_lines.is_empty() {
            return HashMap::new();
        }

        let mut potential_assignments = Vec::new();
        for (line_idx, &line_text) in group_lines.iter().enumerate() {
            if line_text.is_empty() {
                continue;
            }
            let scores = calculate_scores(line_text, lang, line_idx);
            potential_assignments.push((scores.primary, Role::Primary, line_idx));
            potential_assignments.push((scores.translation, Role::Translation, line_idx));
            potential_assignments.push((scores.romanization, Role::Romanization, line_idx));
        }

        potential_assignments
            .sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let mut final_assignments: HashMap<Role, &'a str> = HashMap::new();
        let mut assigned_indices: HashSet<usize> = HashSet::new();
        let mut assigned_roles: HashSet<Role> = HashSet::new();

        for (score, role, line_idx) in potential_assignments {
            if score > 0.0
                && !assigned_indices.contains(&line_idx)
                && !assigned_roles.contains(&role)
            {
                final_assignments.insert(role, group_lines[line_idx]);
                assigned_indices.insert(line_idx);
                assigned_roles.insert(role);
            }
        }
        final_assignments
    }

    pub fn build_annotated_track(
        assignments: &HashMap<Role, &str>,
        start_ms: u64,
        end_ms: u64,
    ) -> Option<super::AnnotatedTrack> {
        let main_text = assignments.get(&Role::Primary)?;

        let main_track = super::new_line_timed_track((*main_text).to_string(), start_ms, end_ms);

        let translations = if let Some(text) = assignments.get(&Role::Translation) {
            vec![super::new_line_timed_track(
                (*text).to_string(),
                start_ms,
                end_ms,
            )]
        } else {
            vec![]
        };

        let romanizations = if let Some(text) = assignments.get(&Role::Romanization) {
            vec![super::new_line_timed_track(
                (*text).to_string(),
                start_ms,
                end_ms,
            )]
        } else {
            vec![]
        };

        Some(super::AnnotatedTrack {
            content_type: super::ContentType::Main,
            content: main_track,
            translations,
            romanizations,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_track_text(track: &LyricTrack) -> String {
        track
            .words
            .iter()
            .flat_map(|w| &w.syllables)
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("")
    }

    fn get_optional_track_text(tracks: &[LyricTrack]) -> Option<String> {
        if tracks.is_empty() {
            None
        } else {
            Some(get_track_text(&tracks[0]))
        }
    }

    #[test]
    fn test_default_bilingual_lrc_parsing() {
        let content = "[00:20.00]Hello world\n[00:20.00]你好世界\n[00:22.00]Next line";
        let parsed_data = parse_lrc(content, &LrcParsingOptions::default()).unwrap();
        assert_eq!(parsed_data.lines.len(), 2);
        let track = &parsed_data.lines[0].tracks[0];
        assert_eq!(get_track_text(&track.content), "Hello world");
        assert_eq!(get_track_text(&track.translations[0]), "你好世界");
    }

    #[test]
    fn test_role_order_standard() {
        let content = "[00:20.00]Hello world\n[00:20.00]こんにちは\n[00:20.00]你好世界";
        let options = LrcParsingOptions {
            same_timestamp_strategy: LrcSameTimestampStrategy::UseRoleOrder(vec![
                LrcLineRole::Main,
                LrcLineRole::Romanization,
                LrcLineRole::Translation,
            ]),
        };
        let parsed_data = parse_lrc(content, &options).unwrap();
        let track = &parsed_data.lines[0].tracks[0];
        assert_eq!(get_track_text(&track.content), "Hello world");
        assert_eq!(get_track_text(&track.romanizations[0]), "こんにちは");
        assert_eq!(get_track_text(&track.translations[0]), "你好世界");
    }

    #[test]
    fn test_heuristic_japanese_song() {
        let content = "[00:15.50]君が好きだと叫びたい\n[00:15.50]想大声说我爱你\n[00:15.50]Kimi ga suki da to sakebitai";
        let options = LrcParsingOptions {
            same_timestamp_strategy: LrcSameTimestampStrategy::Heuristic,
        };
        let parsed_data = parse_lrc(content, &options).unwrap();
        assert_eq!(parsed_data.lines.len(), 1);

        let track = &parsed_data.lines[0].tracks[0];
        assert_eq!(
            get_optional_track_text(std::slice::from_ref(&track.content)),
            Some("君が好きだと叫びたい".to_string())
        );
        assert_eq!(
            get_optional_track_text(&track.translations),
            Some("想大声说我爱你".to_string())
        );
        assert_eq!(
            get_optional_track_text(&track.romanizations),
            Some("Kimi ga suki da to sakebitai".to_string())
        );
    }

    #[test]
    fn test_heuristic_chinese_song() {
        let content = "[01:05.10]能不能给我一首歌的时间\n[01:05.10]Can you give me the time of a song\n[01:05.10]Neng bu neng gei wo yi shou ge de shi jian";
        let options = LrcParsingOptions {
            same_timestamp_strategy: LrcSameTimestampStrategy::Heuristic,
        };
        let parsed_data = parse_lrc(content, &options).unwrap();
        assert_eq!(parsed_data.lines.len(), 1);

        let track = &parsed_data.lines[0].tracks[0];
        assert_eq!(
            get_optional_track_text(std::slice::from_ref(&track.content)),
            Some("能不能给我一首歌的时间".to_string())
        );
        assert_eq!(
            get_optional_track_text(&track.translations),
            Some("Can you give me the time of a song".to_string())
        );
        assert_eq!(
            get_optional_track_text(&track.romanizations),
            Some("Neng bu neng gei wo yi shou ge de shi jian".to_string())
        );
    }

    #[test]
    fn test_heuristic_korean_song() {
        let content = "[00:40.00]사랑해요\n[00:40.00]I love you\n[00:40.00]Saranghaeyo";
        let options = LrcParsingOptions {
            same_timestamp_strategy: LrcSameTimestampStrategy::Heuristic,
        };
        let parsed_data = parse_lrc(content, &options).unwrap();
        assert!(
            !parsed_data.lines.is_empty(),
            "Parsing should produce at least one line"
        );

        let track = &parsed_data.lines[0].tracks[0];
        assert_eq!(
            get_optional_track_text(std::slice::from_ref(&track.content)),
            Some("사랑해요".to_string())
        );
        assert_eq!(
            get_optional_track_text(&track.translations),
            Some("I love you".to_string())
        );
        assert_eq!(
            get_optional_track_text(&track.romanizations),
            Some("Saranghaeyo".to_string())
        );
    }

    #[test]
    fn test_heuristic_english_song() {
        let content = "[00:33.00]Never gonna give you up\n[00:33.00]绝不放弃你";
        let options = LrcParsingOptions {
            same_timestamp_strategy: LrcSameTimestampStrategy::Heuristic,
        };
        let parsed_data = parse_lrc(content, &options).unwrap();
        assert!(
            !parsed_data.lines.is_empty(),
            "Parsing should produce at least one line"
        );

        let track = &parsed_data.lines[0].tracks[0];
        assert_eq!(
            get_optional_track_text(std::slice::from_ref(&track.content)),
            Some("Never gonna give you up".to_string())
        );
        assert_eq!(
            get_optional_track_text(&track.translations),
            Some("绝不放弃你".to_string())
        );
        assert_eq!(get_optional_track_text(&track.romanizations), None);
    }

    #[test]
    fn test_heuristic_distinguishes_romaji_from_english() {
        let content = "[00:21.00]ありがとう\n[00:21.00]Thank you\n[00:21.00]Arigatou";
        let options = LrcParsingOptions {
            same_timestamp_strategy: LrcSameTimestampStrategy::Heuristic,
        };
        let parsed_data = parse_lrc(content, &options).unwrap();
        assert!(
            !parsed_data.lines.is_empty(),
            "Parsing should produce at least one line"
        );

        let track = &parsed_data.lines[0].tracks[0];
        assert_eq!(
            get_optional_track_text(std::slice::from_ref(&track.content)),
            Some("ありがとう".to_string())
        );
        assert_eq!(
            get_optional_track_text(&track.translations),
            Some("Thank you".to_string())
        );
        assert_eq!(
            get_optional_track_text(&track.romanizations),
            Some("Arigatou".to_string())
        );
    }
}
