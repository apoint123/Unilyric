//! # YRC 格式解析器

use std::collections::HashMap;

use crate::converter::utils::process_syllable_text;
use lyrics_helper_core::{
    AnnotatedTrack, ContentType, ConvertError, LyricFormat, LyricLine, LyricLineBuilder,
    LyricSyllable, LyricSyllableBuilder, LyricTrack, ParsedSourceData, Word,
};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

/// 匹配 YRC 行级时间戳 `[start,duration]`
static YRC_LINE_TIMESTAMP_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[(?P<start>\d+),(?P<duration>\d+)]").expect("编译 YRC_LINE_TIMESTAMP_REGEX 失败")
});

/// 匹配 YRC 音节级时间戳 `(start,duration,0)`
static YRC_SYLLABLE_TIMESTAMP_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\((?P<start>\d+),(?P<duration>\d+),(?P<zero>0)\)")
        .expect("编译 YRC_SYLLABLE_TIMESTAMP_REGEX 失败")
});

/// 解析单行 YRC 歌词文本到 `LyricLine` 结构。
fn parse_yrc_line(line_str: &str, line_num: usize) -> Result<LyricLine, ConvertError> {
    let line_ts_cap = YRC_LINE_TIMESTAMP_REGEX.captures(line_str).ok_or_else(|| {
        ConvertError::InvalidLyricFormat(format!(
            "第 {line_num} 行: 行首缺少行时间戳标记 `[开始时间,总时长]`。",
        ))
    })?;

    let line_start_ms: u64 = line_ts_cap["start"].parse()?;
    let line_duration_ms: u64 = line_ts_cap["duration"].parse()?;

    let content_after_line_ts = &line_str[line_ts_cap.get(0).unwrap().end()..];

    let mut syllables: Vec<LyricSyllable> = Vec::new();
    let timestamp_matches: Vec<_> = YRC_SYLLABLE_TIMESTAMP_REGEX
        .find_iter(content_after_line_ts)
        .collect();

    for (i, ts_match) in timestamp_matches.iter().enumerate() {
        let text_start_pos = ts_match.end();
        let text_end_pos = if i + 1 < timestamp_matches.len() {
            timestamp_matches[i + 1].start()
        } else {
            content_after_line_ts.len()
        };
        let raw_text_slice = &content_after_line_ts[text_start_pos..text_end_pos];

        if let Some((clean_text, ends_with_space)) =
            process_syllable_text(raw_text_slice, &mut syllables)
        {
            let captures = YRC_SYLLABLE_TIMESTAMP_REGEX
                .captures(ts_match.as_str())
                .unwrap();
            let syl_start_ms: u64 = captures["start"].parse()?;
            let syl_duration_ms: u64 = captures["duration"].parse()?;

            let syllable = LyricSyllableBuilder::default()
                .text(clean_text)
                .start_ms(syl_start_ms)
                .end_ms(syl_start_ms + syl_duration_ms)
                .ends_with_space(ends_with_space)
                .build()
                .unwrap();
            syllables.push(syllable);
        }
    }

    let words = vec![Word {
        syllables,
        ..Default::default()
    }];

    let content_track = LyricTrack {
        words,
        ..Default::default()
    };

    let annotated_track = AnnotatedTrack {
        content_type: ContentType::Main,
        content: content_track,
        ..Default::default()
    };

    Ok(LyricLineBuilder::default()
        .start_ms(line_start_ms)
        .end_ms(line_start_ms + line_duration_ms)
        .track(annotated_track)
        .build()
        .unwrap())
}

/// 解析 YRC 格式内容到 `ParsedSourceData` 结构。
pub fn parse_yrc(content: &str) -> Result<ParsedSourceData, ConvertError> {
    let mut lines: Vec<LyricLine> = Vec::new();
    let mut raw_metadata: HashMap<String, Vec<String>> = HashMap::new();
    let mut warnings: Vec<String> = Vec::new();

    for (i, line_str_raw) in content.lines().enumerate() {
        let line_num = i + 1;
        let trimmed_line = line_str_raw.trim();

        if trimmed_line.is_empty() {
            continue;
        }

        // 解析 JSON 元数据行
        if trimmed_line.starts_with("{\"t\":") {
            if let Ok(json_value) = serde_json::from_str::<Value>(trimmed_line) {
                if let Some(c_array) = json_value.get("c").and_then(|v| v.as_array()) {
                    if c_array.is_empty() {
                        continue;
                    }
                    // 第一个 "tx" 元素通常是键，例如 "作词: "
                    let key_part = c_array[0]
                        .get("tx")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let key = key_part
                        .trim_end_matches(|c: char| c == ':' || c == '：' || c.is_whitespace())
                        .to_string();

                    // 后续的 "tx" 元素是值
                    let value = c_array
                        .iter()
                        .skip(1) // 跳过作为键的第一个元素
                        .filter_map(|item| item.get("tx").and_then(|v| v.as_str()))
                        .filter(|s| !s.trim().is_empty() && *s != "/") // 过滤掉空字符串和分隔符
                        .collect::<Vec<_>>()
                        .join(", "); // 用逗号连接多个作者

                    if !key.is_empty() && !value.is_empty() {
                        raw_metadata.entry(key).or_default().push(value);
                    }
                }
            } else {
                warnings.push(format!(
                    "第 {line_num} 行: 看起来像 JSON 元数据但解析失败，已跳过。"
                ));
            }
            continue;
        }

        if YRC_LINE_TIMESTAMP_REGEX.is_match(trimmed_line) {
            match parse_yrc_line(trimmed_line, line_num) {
                Ok(mut parsed_line) => {
                    parsed_line.agent = Some("v1".to_string());
                    lines.push(parsed_line);
                }
                Err(e) => {
                    warnings.push(format!("第 {line_num} 行: 解析歌词行失败。错误: {e}"));
                }
            }
        } else {
            warnings.push(format!("第 {line_num} 行: 未能识别的行格式。"));
        }
    }

    Ok(ParsedSourceData {
        lines,
        raw_metadata,
        warnings,
        source_format: LyricFormat::Yrc,
        is_line_timed_source: false,
        ..Default::default()
    })
}
