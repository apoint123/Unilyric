use lyrics_helper_core::{ContentType, LyricLine, LyricSyllable};
use regex::Regex;
use std::{collections::HashMap, sync::LazyLock};

static METADATA_TAG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[(?P<key>[a-zA-Z]+):(?P<value>.*)]$").expect("编译 METADATA_TAG_REGEX 失败")
});

#[must_use]
pub fn normalize_text_whitespace(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed.split_whitespace().collect::<Vec<&str>>().join(" ")
}

pub fn parse_and_store_metadata(
    line: &str,
    raw_metadata: &mut HashMap<String, Vec<String>>,
) -> bool {
    if let Some(caps) = METADATA_TAG_REGEX.captures(line)
        && let (Some(key_match), Some(value_match)) = (caps.name("key"), caps.name("value"))
    {
        let key = key_match.as_str().trim();
        if !key.is_empty() {
            let normalized_value = normalize_text_whitespace(value_match.as_str());
            raw_metadata
                .entry(key.to_string())
                .or_default()
                .push(normalized_value);
            return true;
        }
    }
    false
}

pub fn process_syllable_text(
    raw_text_slice: &str,
    syllables: &mut [LyricSyllable],
) -> Option<(String, bool)> {
    let has_leading_space = raw_text_slice.starts_with(char::is_whitespace);
    let has_trailing_space = raw_text_slice.ends_with(char::is_whitespace);
    let clean_text = raw_text_slice.trim();

    if has_leading_space && let Some(last_syllable) = syllables.last_mut() {
        last_syllable.ends_with_space = true;
    }

    if clean_text.is_empty() {
        None
    } else {
        Some((clean_text.to_string(), has_trailing_space))
    }
}

const TOLERANCE_MS: u64 = 50;

/// Merges translation and romanization lines into mian lyrics
pub fn merge_lyric_lines(
    mut main_lines: Vec<LyricLine>,
    translation_lines: Option<Vec<LyricLine>>,
    romanization_lines: Option<Vec<LyricLine>>,
) -> Vec<LyricLine> {
    if translation_lines.is_none() && romanization_lines.is_none() {
        return main_lines;
    }

    let mut trans_iter = translation_lines
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .peekable();
    let mut roman_iter = romanization_lines
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .peekable();

    for main_line in main_lines.iter_mut() {
        if let Some(main_annotated_track) = main_line
            .tracks
            .iter_mut()
            .find(|at| at.content_type == ContentType::Main)
        {
            // Match translations
            while let Some(trans_line) = trans_iter.peek() {
                if trans_line.start_ms + TOLERANCE_MS < main_line.start_ms {
                    trans_iter.next(); // This translation line is too old, skip
                } else {
                    break; // Reached the potential matching window
                }
            }
            while let Some(trans_line) = trans_iter.peek() {
                if trans_line.start_ms.abs_diff(main_line.start_ms) <= TOLERANCE_MS {
                    if let Some(track_to_add) = trans_line.main_track() {
                        main_annotated_track
                            .translations
                            .push(track_to_add.content.clone());
                    }
                    trans_iter.next();
                } else {
                    break; // This translation line is for a future main line
                }
            }

            // Match romanizations
            while let Some(roman_line) = roman_iter.peek() {
                if roman_line.start_ms + TOLERANCE_MS < main_line.start_ms {
                    roman_iter.next();
                } else {
                    break;
                }
            }
            while let Some(roman_line) = roman_iter.peek() {
                if roman_line.start_ms.abs_diff(main_line.start_ms) <= TOLERANCE_MS {
                    if let Some(track_to_add) = roman_line.main_track() {
                        main_annotated_track
                            .romanizations
                            .push(track_to_add.content.clone());
                    }
                    roman_iter.next();
                } else {
                    break;
                }
            }
        }
    }
    main_lines
}
