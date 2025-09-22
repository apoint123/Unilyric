use crate::error::Result;
use lyrics_helper_core::{
    AnnotatedTrack, LyricFormat, LyricLine, LyricLineBuilder, LyricSyllable, LyricSyllableBuilder,
    LyricTrack, ParsedSourceData, Word,
};
use regex::Regex;
use std::{collections::HashMap, sync::LazyLock};

use super::utils::{parse_and_store_metadata, process_syllable_text};

static LYRIC_TOKEN_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<text>.*?)\((?P<start>\d+),(?P<duration>\d+)\)")
        .expect("编译 LYRIC_TOKEN_REGEX 失败")
});
static QRC_LINE_TIMESTAMP_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[\d+,\d+]").expect("编译 QRC_LINE_TIMESTAMP_REGEX 失败"));

pub fn parse_qrc(content: &str) -> Result<ParsedSourceData> {
    let mut raw_metadata: HashMap<String, Vec<String>> = HashMap::new();
    let mut final_lines: Vec<LyricLine> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for line_str in content.lines() {
        let trimmed_line = line_str.trim();
        if trimmed_line.is_empty() || parse_and_store_metadata(trimmed_line, &mut raw_metadata) {
            continue;
        }

        match parse_single_qrc_line(trimmed_line) {
            Ok(Some(line)) => final_lines.push(line),
            Ok(None) => (),
            Err(e) => warnings.push(e.to_string()),
        }
    }

    Ok(ParsedSourceData {
        lines: final_lines,
        raw_metadata,
        warnings,
        source_format: LyricFormat::Qrc,
        ..Default::default()
    })
}

fn parse_single_qrc_line(line_str: &str) -> Result<Option<LyricLine>> {
    let line_content = QRC_LINE_TIMESTAMP_REGEX.replace(line_str, "");
    let mut syllables: Vec<LyricSyllable> = Vec::new();

    for captures in LYRIC_TOKEN_REGEX.captures_iter(&line_content) {
        let raw_text = &captures["text"];
        if let Some((clean_text, ends_with_space)) = process_syllable_text(raw_text, &mut syllables)
        {
            // In most of cases, you should not use line-level timestamps
            // because in these cases (specially in QQMusic), the timestamps are
            // inaccurate and continuous (without a break).
            // But we respect the source file there, you may need to
            // recalculate the line-level timestamps based on the syllables.
            let start_ms: u64 = captures["start"].parse()?;
            let duration_ms: u64 = captures["duration"].parse()?;
            let syllable = LyricSyllableBuilder::default()
                .text(clean_text)
                .start_ms(start_ms)
                .end_ms(start_ms + duration_ms)
                .ends_with_space(ends_with_space)
                .build()
                .unwrap();
            syllables.push(syllable);
        }
    }

    if syllables.is_empty() {
        return Ok(None);
    }

    let start_ms = syllables.first().unwrap().start_ms;
    let end_ms = syllables.last().unwrap().end_ms;
    let words = vec![Word {
        syllables,
        ..Default::default()
    }];

    let line = LyricLineBuilder::default()
        .start_ms(start_ms)
        .end_ms(end_ms)
        .track(AnnotatedTrack {
            content: LyricTrack {
                words,
                ..Default::default()
            },
            ..Default::default()
        })
        .build()
        .unwrap();

    Ok(Some(line))
}
