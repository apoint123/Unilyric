use crate::types::{AssMetadata, ConvertError, LrcLine};
use once_cell::sync::Lazy;
use regex::Regex;

static LRC_LINE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\[(\d{2,}):(\d{2})\.(\d{2,3})\](.*)$").expect("未能编译 LRC_LINE_REGEX")
});

static LRC_METADATA_TAG_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\[(ti|ar|al|by|offset|length|re|ve):(.*?)\]$")
        .expect("未能编译 LRC_METADATA_TAG_REGEX")
});

pub fn parse_lrc_text_to_lines(
    content: &str,
) -> Result<(Vec<LrcLine>, Vec<AssMetadata>), ConvertError> {
    let mut lrc_lines: Vec<LrcLine> = Vec::new();
    let mut metadata: Vec<AssMetadata> = Vec::new();

    for (line_num, line_str_raw) in content.lines().enumerate() {
        let line_str = line_str_raw.trim();
        let mut matched_lyric = false;

        let captures_iter = LRC_LINE_REGEX.captures_iter(line_str);
        let mut text_content_for_multiple_ts: Option<String> = None;

        for caps in captures_iter {
            matched_lyric = true;
            let minutes_str = caps.get(1).map_or("0", |m| m.as_str());
            let seconds_str = caps.get(2).map_or("0", |m| m.as_str());
            let fraction_str = caps.get(3).map_or("0", |m| m.as_str());
            let current_text_content = caps.get(4).map_or("", |m| m.as_str()).to_string();
            if text_content_for_multiple_ts.is_none() {
                text_content_for_multiple_ts = Some(current_text_content);
            }

            let minutes: u64 = minutes_str.parse().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "LRC 行 {}: 无效的分钟 '{}': {}",
                    line_num + 1,
                    minutes_str,
                    e
                ))
            })?;
            let seconds: u64 = seconds_str.parse().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "LRC 行 {}: 无效的秒 '{}': {}",
                    line_num + 1,
                    seconds_str,
                    e
                ))
            })?;

            let milliseconds: u64 = match fraction_str.len() {
                2 => {
                    fraction_str.parse::<u64>().map_err(|e| {
                        ConvertError::InvalidTime(format!(
                            "LRC 行 {}: 无效的厘秒 '{}': {}",
                            line_num + 1,
                            fraction_str,
                            e
                        ))
                    })? * 10
                }
                3 => fraction_str.parse::<u64>().map_err(|e| {
                    ConvertError::InvalidTime(format!(
                        "LRC 行 {}: 无效的毫秒 '{}': {}",
                        line_num + 1,
                        fraction_str,
                        e
                    ))
                })?,
                _ => {
                    return Err(ConvertError::InvalidTime(format!(
                        "LRC 行 {}: 小数部分长度无效: '{}'",
                        line_num + 1,
                        fraction_str
                    )));
                }
            };

            let total_ms = (minutes * 60 + seconds) * 1000 + milliseconds;
            lrc_lines.push(LrcLine {
                timestamp_ms: total_ms,
                text: text_content_for_multiple_ts.clone().unwrap_or_default(),
            });
        }

        if !matched_lyric {
            if let Some(meta_caps) = LRC_METADATA_TAG_REGEX.captures(line_str) {
                let key = meta_caps.get(1).map_or("", |m| m.as_str()).to_string();
                let value = meta_caps
                    .get(2)
                    .map_or("", |m| m.as_str())
                    .trim()
                    .to_string();
                if !key.is_empty() {
                    metadata.push(AssMetadata { key, value });
                }
            } else if !line_str.is_empty() {
                log::info!(
                    "[LRC 处理] 行 {}: 跳过未识别的行: '{}'",
                    line_num + 1,
                    line_str
                );
            }
        }
    }

    lrc_lines.sort_by_key(|line| line.timestamp_ms);

    Ok((lrc_lines, metadata))
}
