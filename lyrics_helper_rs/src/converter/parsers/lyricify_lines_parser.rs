//! # Lyricify Lines 格式解析器

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::converter::{ utils::normalize_text_whitespace};

use lyrics_helper_core::{
    AnnotatedTrack, ContentType, ConvertError, LyricFormat, LyricLine, LyricLineBuilder,
    LyricSyllableBuilder, LyricTrack, ParsedSourceData, Word,
};

static LYL_LINE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[(\d+),(\d+)](.*)$").expect("编译 LYL_LINE_REGEX 失败"));

/// 解析 LYL 格式内容到 `ParsedSourceData` 结构。
pub fn parse_lyl(content: &str) -> Result<ParsedSourceData, ConvertError> {
    let mut lines: Vec<LyricLine> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for (i, line_str) in content.lines().enumerate() {
        let line_num = i + 1;
        let trimmed_line = line_str.trim();

        if trimmed_line.is_empty() || trimmed_line.eq_ignore_ascii_case("[type:LyricifyLines]") {
            continue;
        }

        if let Some(caps) = LYL_LINE_REGEX.captures(trimmed_line) {
            let start_ms: u64 = caps[1].parse()?;
            let end_ms: u64 = caps[2].parse()?;
            let raw_text = caps.get(3).map_or("", |m| m.as_str());
            let text = normalize_text_whitespace(raw_text);

            if text.is_empty() {
                continue;
            }

            if end_ms < start_ms {
                warnings.push(format!(
                    "第 {line_num} 行: 结束时间 {end_ms}ms 在开始时间 {start_ms}ms 之前。"
                ));
            }

            let main_content_track = LyricTrack {
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
            };

            let annotated_track = AnnotatedTrack {
                content_type: ContentType::Main,
                content: main_content_track,
                translations: vec![],
                romanizations: vec![],
            };

            let line = LyricLineBuilder::default()
                .track(annotated_track)
                .start_ms(start_ms)
                .end_ms(end_ms)
                .build()
                .unwrap();
            lines.push(line);
        } else {
            warnings.push(format!("第 {line_num} 行: 未能识别的行格式。"));
        }
    }

    Ok(ParsedSourceData {
        lines,
        raw_metadata: HashMap::new(),
        warnings,
        source_format: LyricFormat::Lyl,
        is_line_timed_source: true,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lyl_simple_parsing() {
        let content = "[type:LyricifyLines]\n[1000,3000]Hello world\n[3500,5000]Next line";
        let parsed_data = parse_lyl(content).unwrap();
        assert_eq!(parsed_data.lines.len(), 2);

        let line1 = &parsed_data.lines[0];
        assert_eq!(line1.start_ms, 1000);
        assert_eq!(line1.end_ms, 3000);

        let main_track = &line1.tracks[0];
        assert_eq!(main_track.content_type, ContentType::Main);
        assert_eq!(main_track.content.words[0].syllables.len(), 1);
        assert_eq!(main_track.content.words[0].syllables[0].text, "Hello world");
    }

    #[test]
    fn test_lyl_empty_lines_and_warnings() {
        let content = "[type:LyricifyLines]\n[1000,3000]Hello\n\n[4000,3000]Invalid time";
        let parsed_data = parse_lyl(content).unwrap();
        assert_eq!(parsed_data.lines.len(), 2);
        assert_eq!(parsed_data.warnings.len(), 1);
        assert!(parsed_data.warnings[0].contains("结束时间"));
    }
}
