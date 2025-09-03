//! 增强型 LRC (Enhanced LRC) 格式解析器。
//!
//! 支持两种逐字时间戳格式：
//! 1. `[line_time]...<word_time>word<word_time>word<end_time>`
//! 2. `[line_time]<word_time>word<word_time>word...` (无末尾时间戳)
//!
//! 同时处理行起始时间与第一个词起始时间不一致的情况，并以后者为准。

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::converter::utils::{normalize_text_whitespace, parse_and_store_metadata};

use lyrics_helper_core::{
    AnnotatedTrack, ContentType, ConvertError, LyricLine, LyricLineBuilder, LyricSyllable,
    LyricSyllableBuilder, LyricTrack, ParsedSourceData, Word,
};

/// 用于匹配行时间标签，例如 [00:12.34]
static LINE_TIME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[(\d{2,}):(\d{2})[.:](\d{2,3})]").unwrap());
/// 用于匹配逐字时间标签，例如 <00:12.34>
static WORD_TIME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(\d{2,}):(\d{2})[.:](\d{2,3})>").unwrap());
/// 用于匹配 [mm:ss.xx] 或 <mm:ss.xx> 格式，并捕获其中的数字。
static TIME_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[<\[](\d{2,}):(\d{2})[.:](\d{2,3})[>\]]").unwrap());

const DEFAULT_LINE_DURATION_MS: u64 = 5000;

/// 解析增强型 LRC 格式内容到 `ParsedSourceData` 结构。
pub fn parse_enhanced_lrc(content: &str) -> Result<ParsedSourceData, ConvertError> {
    let mut lines: Vec<LyricLine> = Vec::new();
    let mut raw_metadata: HashMap<String, Vec<String>> = HashMap::new();
    let mut warnings: Vec<String> = Vec::new();

    for (line_num, line_str) in content.lines().enumerate() {
        let line_num_one_based = line_num + 1;
        let line_str_trimmed = line_str.trim();

        if line_str_trimmed.is_empty()
            || parse_and_store_metadata(line_str_trimmed, &mut raw_metadata)
        {
            continue;
        }

        if let Some(line_time_match) = LINE_TIME_RE.find(line_str_trimmed) {
            let Ok(Some(line_start_ms)) = parse_lrc_time_tag(line_time_match.as_str()) else {
                warnings.push(format!(
                    "第 {line_num_one_based} 行: 无法解析行时间戳，已跳过。"
                ));
                continue;
            };

            let line_content = &line_str_trimmed[line_time_match.end()..];
            let syllables = parse_syllables_from_line(
                line_content,
                line_start_ms,
                &mut warnings,
                line_num_one_based,
            );

            let lyric_line = if !syllables.is_empty() {
                // 如果行开始时间与第一个词的开始时间不同，使用第一个词的开始时间
                let final_line_start_ms = syllables.first().map_or(line_start_ms, |s| s.start_ms);
                let main_track = LyricTrack {
                    words: vec![Word {
                        syllables,
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                LyricLineBuilder::default()
                    .start_ms(final_line_start_ms)
                    .track(AnnotatedTrack {
                        content_type: ContentType::Main,
                        content: main_track,
                        ..Default::default()
                    })
                    .build()
                    .unwrap()
            } else if !line_content.trim().is_empty() {
                let main_track = LyricTrack {
                    words: vec![Word {
                        syllables: vec![
                            LyricSyllableBuilder::default()
                                .text(normalize_text_whitespace(line_content))
                                .start_ms(line_start_ms)
                                .build()
                                .unwrap(),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                LyricLineBuilder::default()
                    .start_ms(line_start_ms)
                    .track(AnnotatedTrack {
                        content_type: ContentType::Main,
                        content: main_track,
                        ..Default::default()
                    })
                    .build()
                    .unwrap()
            } else {
                continue;
            };
            lines.push(lyric_line);
        }
    }

    // 第二遍处理：填充行和音节的结束时间
    finalize_end_times(&mut lines, &mut warnings);

    Ok(ParsedSourceData {
        lines,
        raw_metadata,
        warnings,
        source_format: lyrics_helper_core::LyricFormat::EnhancedLrc,
        is_line_timed_source: false,
        ..Default::default()
    })
}

/// 从单行文本中解析出所有音节
fn parse_syllables_from_line(
    line_content: &str,
    line_start_ms: u64,
    warnings: &mut Vec<String>,
    line_num: usize,
) -> Vec<LyricSyllable> {
    let time_tags: Vec<(u64, std::ops::Range<usize>)> = WORD_TIME_RE
        .find_iter(line_content)
        .filter_map(|mat| {
            parse_lrc_time_tag(mat.as_str())
                .ok()
                .flatten()
                .map(|time| (time, mat.range()))
        })
        .collect();

    if time_tags.is_empty() {
        return Vec::new();
    }

    if let Some((first_word_time, _)) = time_tags.first()
        && line_start_ms != *first_word_time
    {
        warnings.push(format!(
                "第 {line_num} 行: 行时间戳 [{line_start_ms}] 与第一个音节时间戳 <{first_word_time}> 不匹配，已以后者为准。"
            ));
    }

    let mut syllables = Vec::new();
    for i in 0..time_tags.len() {
        let (current_time, current_range) = (&time_tags[i].0, &time_tags[i].1);

        let text_start = current_range.end;
        let text_end = time_tags
            .get(i + 1)
            .map_or(line_content.len(), |next_tag| next_tag.1.start);

        let raw_text_slice = &line_content[text_start..text_end];

        let ends_with_space = raw_text_slice.ends_with(' ');
        let text = normalize_text_whitespace(raw_text_slice);

        if !text.is_empty() {
            let next_time = time_tags.get(i + 1).map(|t| t.0);

            if let Some(nt) = next_time
                && nt < *current_time
            {
                warnings.push(format!(
                    "第 {line_num} 行: 检测到时间戳乱序或回溯 (<{current_time}> -> <{nt}>)。"
                ));
            }

            let end_ms = next_time.unwrap_or(0);
            let duration_ms = if end_ms > 0 {
                Some(end_ms.saturating_sub(*current_time))
            } else {
                None
            };

            let mut builder = LyricSyllableBuilder::default();
            builder
                .text(text)
                .start_ms(*current_time)
                .end_ms(end_ms)
                .ends_with_space(ends_with_space);

            if let Some(d) = duration_ms {
                builder.duration_ms(d);
            }

            syllables.push(builder.build().unwrap());
        }
    }
    syllables
}

/// 第二遍处理，修正所有行和音节的结束时间
fn finalize_end_times(lines: &mut [LyricLine], _warnings: &mut [String]) {
    // 首先按开始时间排序，确保时间线是正确的
    lines.sort_by_key(|line| line.start_ms);
    for i in 0..lines.len() {
        let next_line_start_ms = lines.get(i + 1).map(|l| l.start_ms);
        let current_line = &mut lines[i];

        if let Some(main_track) = current_line
            .tracks
            .iter_mut()
            .find(|t| t.content_type == ContentType::Main)
            && let Some(word) = main_track.content.words.first_mut()
        {
            if let Some(last_syllable) = word.syllables.last_mut() {
                if last_syllable.end_ms == 0 {
                    let end_ms = next_line_start_ms
                        .unwrap_or_else(|| last_syllable.start_ms + DEFAULT_LINE_DURATION_MS);
                    last_syllable.end_ms = end_ms.max(last_syllable.start_ms);
                    last_syllable.duration_ms =
                        Some(last_syllable.end_ms.saturating_sub(last_syllable.start_ms));
                }
                current_line.end_ms = last_syllable.end_ms;
            } else {
                current_line.end_ms = next_line_start_ms
                    .unwrap_or_else(|| current_line.start_ms + DEFAULT_LINE_DURATION_MS)
                    .max(current_line.start_ms);
            }
        }
    }
}

/// 解析 LRC 格式的时间标签（[mm:ss.xx] 或 <mm:ss.xx>），并返回总毫秒数。
///
/// # 参数
/// * `tag_str` - 包含时间标签的字符串，例如 "[01:23.45]" 或 "<01:23.456>"。
///
/// # 返回
/// * `Ok(Some(u64))` - 成功解析，返回总毫秒数。
/// * `Ok(None)` - 输入字符串不匹配时间标签格式。
/// * `Err(ConvertError)` - 标签内的数字解析失败，这是一个内部错误。
pub fn parse_lrc_time_tag(tag_str: &str) -> Result<Option<u64>, ConvertError> {
    if let Some(caps) = TIME_TAG_RE.captures(tag_str) {
        let minutes: u64 = caps.get(1).map_or(Ok(0), |m| m.as_str().parse())?;
        let seconds: u64 = caps.get(2).map_or(Ok(0), |m| m.as_str().parse())?;
        let fraction_str = caps.get(3).map_or("0", |m| m.as_str());
        let mut fraction: u64 = fraction_str.parse()?;

        // 如果是厘秒 (xx)，则乘以10转换为毫秒 (xxx)
        if fraction_str.len() == 2 {
            fraction *= 10;
        }
        Ok(Some(minutes * 60 * 1000 + seconds * 1000 + fraction))
    } else {
        // 如果正则表达式不匹配，则这不是一个有效的时间标签
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enhanced_lrc_parsing() {
        let content =
            "[00:10.00]<00:10.00>Hello <00:10.50>world\n[00:12.50]<00:12.50>Next <00:13.00>line";
        let data = parse_enhanced_lrc(content).unwrap();
        assert_eq!(data.lines.len(), 2);

        let line1 = &data.lines[0];
        assert_eq!(line1.start_ms, 10000);

        let main_track = &line1.tracks[0];
        let syls1: Vec<_> = main_track
            .content
            .words
            .iter()
            .flat_map(|w| &w.syllables)
            .collect();
        assert_eq!(syls1.len(), 2);
        assert_eq!(syls1[0].text, "Hello");
        assert_eq!(syls1[0].start_ms, 10000);
        assert_eq!(syls1[1].text, "world");
        assert_eq!(syls1[1].start_ms, 10500);
        assert_eq!(line1.end_ms, 12500);
    }
}
