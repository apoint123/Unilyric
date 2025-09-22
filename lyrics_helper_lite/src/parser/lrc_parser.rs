use crate::error::{FetcherError, Result};
use lyrics_helper_core::{
    AnnotatedTrack, ContentType, LyricFormat, LyricLine, LyricSyllable, LyricTrack,
    ParsedSourceData, Word,
};
use regex::Regex;
use std::{collections::HashMap, sync::LazyLock};

use super::utils::{normalize_text_whitespace, parse_and_store_metadata};

static LRC_LINE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^((?:\[\d{2,}:\d{2}[.:]\d{2,3}])+)(.*)$")
        .expect("Failed to compile LRC_LINE_REGEX")
});
static LRC_TIMESTAMP_EXTRACT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[(\d{2,}):(\d{2})[.:](\d{2,3})]")
        .expect("Failed to compile LRC_TIMESTAMP_EXTRACT_REGEX")
});

const DEFAULT_LAST_LINE_DURATION_MS: u64 = 10000;

/// Parses a raw LRC format string into `ParsedSourceData`.
pub fn parse_lrc(content: &str) -> Result<ParsedSourceData> {
    let mut main_parse_result = parse_lines_to_temp_entries(content)?;
    main_parse_result.entries.sort_by_key(|e| e.timestamp_ms);
    let final_lyric_lines = process_timestamp_groups(&main_parse_result.entries);

    Ok(ParsedSourceData {
        lines: final_lyric_lines,
        raw_metadata: main_parse_result.metadata,
        source_format: LyricFormat::Lrc,
        is_line_timed_source: true,
        warnings: main_parse_result.warnings,
        ..Default::default()
    })
}

struct TempLrcEntry {
    timestamp_ms: u64,
    text: String,
}

struct InitialParseResult {
    entries: Vec<TempLrcEntry>,
    metadata: HashMap<String, Vec<String>>,
    warnings: Vec<String>,
}

fn parse_lines_to_temp_entries(content: &str) -> Result<InitialParseResult> {
    let mut entries = Vec::new();
    let mut metadata = HashMap::new();
    let mut warnings = Vec::new();

    for (line_num, line_str) in content.lines().enumerate() {
        let line_str_trimmed = line_str.trim();
        if line_str_trimmed.is_empty() || parse_and_store_metadata(line_str_trimmed, &mut metadata)
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
                let milliseconds: u64 = match fraction_str.len() {
                    2 => fraction_str.parse::<u64>().map(|f| f * 10)?,
                    3 => fraction_str.parse::<u64>()?,
                    _ => {
                        return Err(FetcherError::InvalidTime(format!(
                            "Invalid millisecond part length: {}",
                            fraction_str.len()
                        )));
                    }
                };

                if seconds >= 60 {
                    warnings.push(format!(
                        "Invalid seconds count (>= 60) in timestamp on line {}: '{}'",
                        line_num + 1,
                        line_str_trimmed
                    ));
                    continue;
                }

                entries.push(TempLrcEntry {
                    timestamp_ms: (minutes * 60 + seconds) * 1000 + milliseconds,
                    text: text_part.clone(),
                });
            }
        }
    }

    Ok(InitialParseResult {
        entries,
        metadata,
        warnings,
    })
}

fn process_timestamp_groups(temp_entries: &[TempLrcEntry]) -> Vec<LyricLine> {
    let mut final_lyric_lines: Vec<LyricLine> = Vec::new();
    let mut i = 0;
    while i < temp_entries.len() {
        let start_ms = temp_entries[i].timestamp_ms;

        let group_end_index = temp_entries[i..]
            .iter()
            .position(|e| e.timestamp_ms != start_ms)
            .map_or(temp_entries.len(), |pos| i + pos);

        let group_lines = &temp_entries[i..group_end_index];
        let end_ms = temp_entries
            .get(group_end_index)
            .map_or(start_ms + DEFAULT_LAST_LINE_DURATION_MS, |next| {
                next.timestamp_ms.max(start_ms)
            });

        let meaningful_lines: Vec<_> = group_lines.iter().filter(|e| !e.text.is_empty()).collect();
        if meaningful_lines.is_empty() {
            i = group_end_index;
            continue;
        }

        let main_entry = meaningful_lines[0];
        let translations_entries = &meaningful_lines[1..];

        let main_track = new_line_timed_track(main_entry.text.clone(), start_ms, end_ms);
        let translations = translations_entries
            .iter()
            .map(|entry| new_line_timed_track(entry.text.clone(), start_ms, end_ms))
            .collect();

        let line = LyricLine {
            start_ms,
            end_ms,
            tracks: vec![AnnotatedTrack {
                content_type: ContentType::Main,
                content: main_track,
                translations,
                ..Default::default()
            }],
            ..Default::default()
        };
        final_lyric_lines.push(line);
        i = group_end_index;
    }
    final_lyric_lines
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
