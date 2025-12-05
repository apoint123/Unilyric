//! # LRC 格式解析器

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::converter::utils::{normalize_text_whitespace, parse_and_store_metadata};

use lyrics_helper_core::{
    AnnotatedTrack, ContentType, ConvertError, LrcLineRole, LrcParsingOptions,
    LrcSameTimestampStrategy, LyricFormat, LyricLine, LyricLineBuilder, LyricSyllable, LyricTrack,
    ParsedSourceData, Word,
};

/// 用于匹配一个完整的 LRC 歌词行，捕获时间戳部分和文本部分
static LRC_LINE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^((?:\[\d{2,}:\d{2}[.:]\d{2,3}])+)(.*)$").expect("未能编译 LRC_LINE_REGEX")
});

/// 用于从一个时间戳组中提取出单个时间戳
static LRC_TIMESTAMP_EXTRACT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[(\d{2,}):(\d{2})[.:](\d{2,3})]").expect("未能编译 LRC_TIMESTAMP_EXTRACT_REGEX")
});

struct TempLrcEntry {
    timestamp_ms: u64,
    text: String,
}

#[derive(Default)]
struct InitialParseResult {
    entries: Vec<TempLrcEntry>,
    metadata: HashMap<String, Vec<String>>,
    warnings: Vec<String>,
}

const DEFAULT_LAST_LINE_DURATION_MS: u64 = 10000;

/// 解析 LRC 格式内容到 `ParsedSourceData` 结构。
pub fn parse_lrc(
    content: &str,
    options: &LrcParsingOptions,
) -> Result<ParsedSourceData, ConvertError> {
    let mut initial_result = parse_lines_to_temp_entries(content)?;

    initial_result.entries.sort_by_key(|e| e.timestamp_ms);

    let (final_lyric_lines, processing_warnings) =
        process_timestamp_groups(&initial_result.entries, options);

    initial_result.warnings.extend(processing_warnings);

    Ok(ParsedSourceData {
        lines: final_lyric_lines,
        raw_metadata: initial_result.metadata,
        source_format: LyricFormat::Lrc,
        is_line_timed_source: true,
        warnings: initial_result.warnings,
        ..Default::default()
    })
}

fn parse_lines_to_temp_entries(content: &str) -> Result<InitialParseResult, ConvertError> {
    let mut result = InitialParseResult::default();

    for (line_num, line_str) in content.lines().enumerate() {
        let line_str_trimmed = line_str.trim();
        if line_str_trimmed.is_empty()
            || parse_and_store_metadata(line_str_trimmed, &mut result.metadata)
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
                        result.entries.push(TempLrcEntry {
                            timestamp_ms: (minutes * 60 + seconds) * 1000 + ms,
                            text: text_part.clone(),
                        });
                    } else {
                        result.warnings.push(format!(
                            "LRC秒数无效 (行 {}): '{}'",
                            line_num + 1,
                            seconds
                        ));
                    }
                }
            }
        }
    }
    Ok(result)
}

fn process_timestamp_groups(
    temp_entries: &[TempLrcEntry],
    options: &LrcParsingOptions,
) -> (Vec<LyricLine>, Vec<String>) {
    let mut final_lyric_lines: Vec<LyricLine> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

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
        let (tracks, mut new_warnings) =
            handle_strategy_for_group(&group_lines, start_ms, end_ms, options);
        warnings.append(&mut new_warnings);

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

    (final_lyric_lines, warnings)
}

fn handle_first_is_main_strategy(
    group_lines: &[&TempLrcEntry],
    start_ms: u64,
    end_ms: u64,
) -> Vec<AnnotatedTrack> {
    let meaningful_lines: Vec<_> = group_lines
        .iter()
        .filter(|e| !e.text.is_empty())
        .copied()
        .collect();

    if meaningful_lines.is_empty() {
        return vec![];
    }

    let main_entry = meaningful_lines[0];
    let translations_entries = &meaningful_lines[1..];

    let main_track = new_line_timed_track(main_entry.text.clone(), start_ms, end_ms);
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

fn handle_all_are_main_strategy(
    group_lines: &[&TempLrcEntry],
    start_ms: u64,
    end_ms: u64,
) -> Vec<AnnotatedTrack> {
    group_lines
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
        .collect()
}

fn handle_use_role_order_strategy(
    group_lines: &[&TempLrcEntry],
    roles: &[LrcLineRole],
    start_ms: u64,
    end_ms: u64,
) -> (Vec<AnnotatedTrack>, Vec<String>) {
    let mut warnings = vec![];

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
            LrcLineRole::Translation => translations.push(track),
            LrcLineRole::Romanization => romanizations.push(track),
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

    let tracks = main_content.map_or_else(Vec::new, |main_track| {
        vec![AnnotatedTrack {
            content_type: ContentType::Main,
            content: main_track,
            translations,
            romanizations,
        }]
    });

    (tracks, warnings)
}

fn handle_strategy_for_group(
    group_lines: &[&TempLrcEntry],
    start_ms: u64,
    end_ms: u64,
    options: &LrcParsingOptions,
) -> (Vec<AnnotatedTrack>, Vec<String>) {
    match &options.same_timestamp_strategy {
        LrcSameTimestampStrategy::FirstIsMain => (
            handle_first_is_main_strategy(group_lines, start_ms, end_ms),
            vec![],
        ),
        LrcSameTimestampStrategy::AllAreMain => (
            handle_all_are_main_strategy(group_lines, start_ms, end_ms),
            vec![],
        ),
        LrcSameTimestampStrategy::UseRoleOrder(roles) => {
            handle_use_role_order_strategy(group_lines, roles, start_ms, end_ms)
        }
    }
}

fn new_line_timed_track(text: String, start_ms: u64, end_ms: u64) -> LyricTrack {
    LyricTrack {
        words: vec![Word {
            syllables: vec![LyricSyllable {
                text,
                start_ms,
                end_ms,
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_track_text(track: &LyricTrack) -> String {
        track.text()
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
}
