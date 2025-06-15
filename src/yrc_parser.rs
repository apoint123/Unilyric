use crate::types::{AssMetadata, ConvertError, LysSyllable, QrcLine};
use once_cell::sync::Lazy;
use regex::Regex;

static YRC_LINE_TIMESTAMP_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\[(?P<start>\d+),(?P<duration>\d+)\]").expect("未能编译 YRC_LINE_TIMESTAMP_REGEX")
});

static YRC_SYLLABLE_TIMESTAMP_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\((?P<start>\d+),(?P<duration>\d+),(?P<zero>0)\)")
        .expect("未能编译 YRC_SYLLABLE_TIMESTAMP_REGEX")
});

pub fn parse_yrc_line(line_str: &str, line_num: usize) -> Result<QrcLine, ConvertError> {
    let line_ts_cap = YRC_LINE_TIMESTAMP_REGEX.captures(line_str).ok_or_else(|| {
        ConvertError::InvalidQrcFormat {
            line_num,
            message: "行首缺少行时间戳标记 [start,duration]".to_string(),
        }
    })?;

    let line_start_ms_str = line_ts_cap.name("start").unwrap().as_str();
    let line_duration_ms_str = line_ts_cap.name("duration").unwrap().as_str();

    let line_start_ms: u64 = line_start_ms_str.parse().map_err(ConvertError::ParseInt)?;
    let line_duration_ms: u64 = line_duration_ms_str
        .parse()
        .map_err(ConvertError::ParseInt)?;

    let content_after_line_ts = &line_str[line_ts_cap.get(0).unwrap().end()..];
    let mut syllables: Vec<LysSyllable> = Vec::new();

    let mut timestamps_info = Vec::new();
    for ts_cap in YRC_SYLLABLE_TIMESTAMP_REGEX.captures_iter(content_after_line_ts) {
        let ts_match = ts_cap.get(0).unwrap();
        let start_ms_str = ts_cap.name("start").unwrap().as_str();
        let duration_ms_str = ts_cap.name("duration").unwrap().as_str();

        let start_ms: u64 = start_ms_str.parse().map_err(ConvertError::ParseInt)?;
        let duration_ms: u64 = duration_ms_str.parse().map_err(ConvertError::ParseInt)?;
        timestamps_info.push((ts_match.start(), ts_match.end(), start_ms, duration_ms));
    }

    for i in 0..timestamps_info.len() {
        let (_ts_match_start, ts_match_end, syl_start_ms, syl_duration_ms) = timestamps_info[i];

        let text_start_pos = ts_match_end;
        let text_end_pos = if i + 1 < timestamps_info.len() {
            timestamps_info[i + 1].0
        } else {
            content_after_line_ts.len()
        };

        let text_slice = &content_after_line_ts[text_start_pos..text_end_pos];

        syllables.push(LysSyllable {
            text: text_slice.to_string(),
            start_ms: syl_start_ms,
            duration_ms: syl_duration_ms,
        });
    }

    if syllables.is_empty()
        && !content_after_line_ts.trim().is_empty()
        && timestamps_info.is_empty()
    {
        log::warn!(
            "行 {line_num}: 内容 '{content_after_line_ts}' 中未找到有效的YRC音节时间戳，但内容非空。"
        );
    }

    Ok(QrcLine {
        line_start_ms,
        line_duration_ms,
        syllables,
    })
}

pub fn load_yrc_from_string(
    yrc_content: &str,
) -> Result<(Vec<QrcLine>, Vec<AssMetadata>), ConvertError> {
    let mut yrc_lines_vec: Vec<QrcLine> = Vec::new();
    let metadata_vec: Vec<AssMetadata> = Vec::new();

    for (i, line_str_raw) in yrc_content.lines().enumerate() {
        let line_num = i + 1;
        let trimmed_line = line_str_raw.trim();

        if trimmed_line.is_empty() {
            continue;
        }

        if trimmed_line.starts_with("{\"t\":")
            && trimmed_line.contains("\"c\":")
            && trimmed_line.ends_with('}')
        {
            continue;
        }

        if YRC_LINE_TIMESTAMP_REGEX.is_match(trimmed_line) {
            match parse_yrc_line(trimmed_line, line_num) {
                Ok(parsed_line) => {
                    yrc_lines_vec.push(parsed_line);
                }
                Err(e) => {
                    log::error!("解析 YRC 行 {line_num} ('{trimmed_line}') 失败: {e}");
                }
            }
        } else {
            log::warn!("行 {line_num}: 无法识别的 YRC 行格式: '{trimmed_line}'");
        }
    }
    Ok((yrc_lines_vec, metadata_vec))
}
