use crate::types::{AssMetadata, ConvertError, DisplayLrcLine, LrcLine};
use once_cell::sync::Lazy;
use regex::Regex;

static LRC_LINE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^((?:\[\d{2,}:\d{2}\.\d{2,3}\])+)(.*)$").expect("未能编译 LRC_LINE_REGEX")
});

static LRC_TIMESTAMP_EXTRACT_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\[(\d{2,}):(\d{2})\.(\d{2,3})\]").expect("未能编译 LRC_TIMESTAMP_EXTRACT_REGEX")
});

static LRC_METADATA_TAG_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\[(ti|ar|al|by|offset|length|re|ve):(.*?)\]$")
        .expect("未能编译 LRC_METADATA_TAG_REGEX")
});

pub type ParsedLrcCollection = (Vec<DisplayLrcLine>, Vec<LrcLine>, Vec<AssMetadata>);

/// 解析LRC文本内容。
///
/// # Arguments
/// * `content` - LRC文本内容。
///
/// # Returns
/// Result<(Vec<DisplayLrcLine>, Vec<LrcLine>, Vec<AssMetadata>), ConvertError>
/// 其中:
/// - `Vec<DisplayLrcLine>`: 主歌词行（对于双语LRC，这是第一行）和无法解析的原始行。
/// - `Vec<LrcLine>`: 从双语LRC中提取出的翻译行（双语LRC的第二行）。
/// - `Vec<AssMetadata>`: 解析出的元数据。
pub fn parse_lrc_text_to_lines(content: &str) -> Result<ParsedLrcCollection, ConvertError> {
    let mut parsed_lines_buffer: Vec<DisplayLrcLine> = Vec::new();
    let mut metadata: Vec<AssMetadata> = Vec::new();

    // 第一遍：解析所有行到 parsed_lines_buffer，包括元数据和原始行
    for line_str_raw in content.lines() {
        let line_str_trimmed = line_str_raw.trim();

        if line_str_trimmed.is_empty() {
            parsed_lines_buffer.push(DisplayLrcLine::Raw {
                original_text: line_str_raw.to_string(),
            });
            continue;
        }

        if let Some(meta_caps) = LRC_METADATA_TAG_REGEX.captures(line_str_trimmed) {
            let key = meta_caps.get(1).map_or("", |m| m.as_str()).to_string();
            let value = meta_caps
                .get(2)
                .map_or("", |m| m.as_str())
                .trim()
                .to_string();
            if !key.is_empty() {
                metadata.push(AssMetadata { key, value });
            }
            // 元数据行不加入 parsed_lines_buffer，它们被 metadata Vec 单独处理
            continue;
        }

        if let Some(line_caps) = LRC_LINE_REGEX.captures(line_str_trimmed) {
            let all_timestamps_str = line_caps.get(1).map_or("", |m| m.as_str());
            let text_part = line_caps.get(2).map_or("", |m| m.as_str()).to_string();
            let mut timestamps_on_this_line_valid = true;
            let mut parsed_timestamps_ms_for_current_physical_line: Vec<u64> = Vec::new();

            for ts_cap in LRC_TIMESTAMP_EXTRACT_REGEX.captures_iter(all_timestamps_str) {
                let minutes_str = ts_cap.get(1).map_or("0", |m| m.as_str());
                let seconds_str = ts_cap.get(2).map_or("0", |m| m.as_str());
                let fraction_str = ts_cap.get(3).map_or("0", |m| m.as_str());

                let minutes_res = minutes_str.parse::<u64>();
                let seconds_res = seconds_str.parse::<u64>();
                let fraction_val_res = match fraction_str.len() {
                    2 => fraction_str.parse::<u64>().map(|f| f * 10),
                    3 => fraction_str.parse::<u64>(),
                    _ => Err("invalid millisecond length".parse::<u64>().unwrap_err()),
                };

                if let (Ok(minutes), Ok(seconds), Ok(milliseconds)) =
                    (minutes_res, seconds_res, fraction_val_res)
                {
                    if minutes < 60 && seconds < 60 {
                        parsed_timestamps_ms_for_current_physical_line
                            .push((minutes * 60 + seconds) * 1000 + milliseconds);
                    } else {
                        timestamps_on_this_line_valid = false;
                        break;
                    }
                } else {
                    timestamps_on_this_line_valid = false;
                    break;
                }
            }

            if timestamps_on_this_line_valid
                && !parsed_timestamps_ms_for_current_physical_line.is_empty()
            {
                for ts_ms in parsed_timestamps_ms_for_current_physical_line {
                    parsed_lines_buffer.push(DisplayLrcLine::Parsed(LrcLine {
                        timestamp_ms: ts_ms,
                        text: text_part.clone(),
                    }));
                }
            } else {
                parsed_lines_buffer.push(DisplayLrcLine::Raw {
                    original_text: line_str_trimmed.to_string(),
                });
            }
        } else {
            parsed_lines_buffer.push(DisplayLrcLine::Raw {
                original_text: line_str_trimmed.to_string(),
            });
        }
    }

    // 第二遍：处理 parsed_lines_buffer，识别双语对
    let mut final_main_lines: Vec<DisplayLrcLine> = Vec::new();
    let mut extracted_translations: Vec<LrcLine> = Vec::new();
    let mut i = 0;
    while i < parsed_lines_buffer.len() {
        match &parsed_lines_buffer[i] {
            DisplayLrcLine::Parsed(current_lrc_line) => {
                // 检查下一行是否存在且为 Parsed 类型
                if i + 1 < parsed_lines_buffer.len()
                    && let DisplayLrcLine::Parsed(next_lrc_line) = &parsed_lines_buffer[i + 1]
                {
                    // 检查时间戳是否严格相同
                    if current_lrc_line.timestamp_ms == next_lrc_line.timestamp_ms {
                        // 认为是双语对
                        // 第一行作为主歌词
                        final_main_lines.push(DisplayLrcLine::Parsed(current_lrc_line.clone()));
                        // 第二行作为翻译提取
                        extracted_translations.push(LrcLine {
                            timestamp_ms: next_lrc_line.timestamp_ms,
                            text: next_lrc_line.text.clone(),
                        });
                        i += 2; // 跳过这两行
                        continue;
                    }
                }
                // 如果不是双语对中的第一行，或者没有匹配的下一行，则作为普通主歌词行
                final_main_lines.push(DisplayLrcLine::Parsed(current_lrc_line.clone()));
                i += 1;
            }
            DisplayLrcLine::Raw { original_text } => {
                // 原始行直接加入主歌词列表
                final_main_lines.push(DisplayLrcLine::Raw {
                    original_text: original_text.clone(),
                });
                i += 1;
            }
        }
    }

    Ok((final_main_lines, extracted_translations, metadata))
}
