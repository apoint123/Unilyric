//! # TTML 解析器的工具函数
//!
//! 该模块提供了一系列用于处理 TTML 特定数据格式的辅助函数，
//! 例如时间戳解析、属性提取和文本清理。

use lyrics_helper_core::ConvertError;
use quick_xml::{Reader, events::BytesStart};

/// 解析 TTML 时间字符串到毫秒。
pub(super) fn parse_ttml_time_to_ms(time_str: &str) -> Result<u64, ConvertError> {
    // 解析毫秒部分（.1, .12, .123）
    fn parse_decimal_ms_part(ms_str: &str, original_time_str: &str) -> Result<u64, ConvertError> {
        if ms_str.is_empty() || ms_str.len() > 3 || ms_str.chars().any(|c| !c.is_ascii_digit()) {
            return Err(ConvertError::InvalidTime(format!(
                "毫秒部分 '{ms_str}' 在时间戳 '{original_time_str}' 中无效或格式错误 (只支持最多3位数字)"
            )));
        }
        let val = ms_str.parse::<u64>().map_err(|e| {
            ConvertError::InvalidTime(format!(
                "无法解析时间戳 '{original_time_str}' 中的毫秒部分 '{ms_str}': {e}"
            ))
        })?;
        Ok(val * 10u64.pow(3 - u32::try_from(ms_str.len()).unwrap_or(3)))
    }

    // 解析 "SS.mmm" 或 "SS" 格式的字符串，返回秒和毫秒
    fn parse_seconds_and_decimal_ms_part(
        seconds_and_ms_str: &str,
        original_time_str: &str,
    ) -> Result<(u64, u64), ConvertError> {
        let mut dot_parts = seconds_and_ms_str.splitn(2, '.');
        let seconds_str = dot_parts.next().unwrap(); // 肯定有

        if seconds_str.is_empty() {
            // 例如 ".5s" 或 "MM:.5"
            return Err(ConvertError::InvalidTime(format!(
                "时间格式 '{original_time_str}' 的秒部分为空 (例如 '.mmm')"
            )));
        }

        let seconds = seconds_str.parse::<u64>().map_err(|e| {
            ConvertError::InvalidTime(format!(
                "在时间戳 '{original_time_str}' 中解析秒 '{seconds_str}' 失败: {e}"
            ))
        })?;

        let milliseconds = if let Some(ms_str) = dot_parts.next() {
            parse_decimal_ms_part(ms_str, original_time_str)?
        } else {
            0
        };

        Ok((seconds, milliseconds))
    }

    // 格式："12.345s"
    if let Some(stripped) = time_str.strip_suffix('s') {
        if stripped.is_empty() || stripped.starts_with('.') || stripped.ends_with('.') {
            return Err(ConvertError::InvalidTime(format!(
                "时间戳 '{time_str}' 包含无效的秒格式"
            )));
        }
        if stripped.starts_with('-') {
            return Err(ConvertError::InvalidTime(format!(
                "时间戳不能为负: '{time_str}'"
            )));
        }

        let (seconds, milliseconds) = parse_seconds_and_decimal_ms_part(stripped, time_str)?;

        Ok(seconds * 1000 + milliseconds)
    } else {
        // 格式："HH:MM:SS.mmm", "MM:SS.mmm", "SS.mmm"
        // 从后往前解析以简化逻辑
        let mut parts_iter = time_str.split(':').rev(); // 倒序迭代

        let mut total_ms: u64 = 0;

        // 解析最后一个部分 (SS.mmm 或 SS)
        let current_part_str = parts_iter.next().ok_or_else(|| {
            ConvertError::InvalidTime(format!("时间格式 '{time_str}' 无效或为空"))
        })?;

        if current_part_str.starts_with('-') {
            // 检查负数
            return Err(ConvertError::InvalidTime(format!(
                "时间戳不能为负: '{time_str}'"
            )));
        }

        let (seconds, milliseconds) =
            parse_seconds_and_decimal_ms_part(current_part_str, time_str)?;
        total_ms += seconds * 1000 + milliseconds;

        // 解析倒数第二个部分 (分钟 MM)
        if let Some(minutes_str) = parts_iter.next() {
            let minutes = minutes_str.parse::<u64>().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "在 '{time_str}' 中解析分钟 '{minutes_str}' 失败: {e}"
                ))
            })?;
            if minutes >= 60 {
                return Err(ConvertError::InvalidTime(format!(
                    "分钟值 '{minutes}' (应 < 60) 在时间戳 '{time_str}' 中无效"
                )));
            }
            total_ms += minutes * 60_000;
        }

        // 解析倒数第三个部分 (小时 HH)
        if let Some(hours_str) = parts_iter.next() {
            let hours = hours_str.parse::<u64>().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "在 '{time_str}' 中解析小时 '{hours_str}' 失败: {e}"
                ))
            })?;
            total_ms += hours * 3_600_000;
        }

        if parts_iter.next().is_some() {
            return Err(ConvertError::InvalidTime(format!(
                "时间格式 '{time_str}' 包含过多部分，格式无效。"
            )));
        }

        // 如果是单独的 "SS.mmm" 格式，秒数可以大于59。
        // 否则（HH:MM:SS 或 MM:SS），秒数必须小于60。
        let num_colon_parts = time_str.chars().filter(|&c| c == ':').count();
        if num_colon_parts > 0 && seconds >= 60 {
            return Err(ConvertError::InvalidTime(format!(
                "秒值 '{seconds}' (应 < 60) 在时间戳 '{time_str}' 中无效"
            )));
        }

        Ok(total_ms)
    }
}

/// 清理文本两端的括号（单个或成对）
pub(super) fn clean_parentheses_from_bg_text_into(text: &str, output: &mut String) {
    output.clear();
    let trimmed = text
        .trim()
        .trim_start_matches(['(', '（'])
        .trim_end_matches([')', '）'])
        .trim();
    output.push_str(trimmed);
}

/// 规范化文本中的空白字符
pub(super) fn normalize_text_whitespace_into(input: &str, output: &mut String) {
    output.clear();
    let mut first = true;
    for word in input.split_whitespace() {
        if !first {
            output.push(' ');
        }
        output.push_str(word);
        first = false;
    }
}

/// 从给定的属性名列表中获取第一个找到的属性，并将其转换为目标类型。
///
/// # 参数
/// * `e` - `BytesStart` 事件，代表一个 XML 标签的开始。
/// * `reader` - XML 读取器，用于解码。
/// * `attr_names` - 一个字节切片数组，包含所有要尝试的属性名（包括别名）。
/// * `processor` - 一个闭包，接收解码后的字符串值，并返回 `Result<T, ConvertError>`。
///
/// # 返回
/// * `Result<Option<T>, ConvertError>` - 成功时返回一个包含转换后值的 Option，如果找不到任何属性则返回 `None`。
pub(super) fn get_attribute_with_aliases<T, F>(
    e: &BytesStart,
    reader: &Reader<&[u8]>,
    attr_names: &[&[u8]],
    processor: F,
) -> Result<Option<T>, ConvertError>
where
    F: Fn(&str) -> Result<T, ConvertError>,
{
    let mut found_attr = None;
    for &name in attr_names {
        if let Some(attr) = e.try_get_attribute(name)? {
            found_attr = Some(attr);
            break;
        }
    }

    found_attr
        .map(|attr| {
            let decoded_value = attr.decode_and_unescape_value(reader.decoder())?;
            processor(&decoded_value)
        })
        .transpose()
}

/// 获取字符串类型的属性值。
pub(super) fn get_string_attribute(
    e: &BytesStart,
    reader: &Reader<&[u8]>,
    attr_names: &[&[u8]],
) -> Result<Option<String>, ConvertError> {
    get_attribute_with_aliases(e, reader, attr_names, |s| Ok(s.to_owned()))
}

/// 获取并解析为毫秒的时间戳属性值。
pub(super) fn get_time_attribute(
    e: &BytesStart,
    reader: &Reader<&[u8]>,
    attr_names: &[&[u8]],
    warnings: &mut Vec<String>,
) -> Result<Option<u64>, ConvertError> {
    (get_string_attribute(e, reader, attr_names)?).map_or(Ok(None), |value_str| {
        match parse_ttml_time_to_ms(&value_str) {
            Ok(ms) => Ok(Some(ms)),
            Err(err) => {
                warnings.push(format!(
                    "时间戳 '{value_str}' 解析失败 ({err}). 该时间戳将被忽略."
                ));
                Ok(None)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ttml_time_to_ms() {
        assert_eq!(parse_ttml_time_to_ms("7.1s").unwrap(), 7100);
        assert_eq!(parse_ttml_time_to_ms("7.12s").unwrap(), 7120);
        assert_eq!(parse_ttml_time_to_ms("7.123s").unwrap(), 7123);
        assert_eq!(parse_ttml_time_to_ms("99999.123s").unwrap(), 99_999_123);
        assert_eq!(parse_ttml_time_to_ms("01:02:03.456").unwrap(), 3_723_456);
        assert_eq!(parse_ttml_time_to_ms("05:10.1").unwrap(), 310_100);
        assert_eq!(parse_ttml_time_to_ms("05:10.12").unwrap(), 310_120);
        assert_eq!(parse_ttml_time_to_ms("7.123").unwrap(), 7123);
        assert_eq!(parse_ttml_time_to_ms("7").unwrap(), 7000);
        assert_eq!(parse_ttml_time_to_ms("15.5s").unwrap(), 15500);
        assert_eq!(parse_ttml_time_to_ms("15s").unwrap(), 15000);

        assert_eq!(parse_ttml_time_to_ms("0").unwrap(), 0);
        assert_eq!(parse_ttml_time_to_ms("0.0s").unwrap(), 0);
        assert_eq!(parse_ttml_time_to_ms("00:00:00.000").unwrap(), 0);
        assert_eq!(parse_ttml_time_to_ms("99:59:59.999").unwrap(), 359_999_999);
        assert_eq!(parse_ttml_time_to_ms("60").unwrap(), 60000);
        assert_eq!(parse_ttml_time_to_ms("123.456").unwrap(), 123_456);

        assert!(matches!(
            parse_ttml_time_to_ms("abc"),
            Err(ConvertError::InvalidTime(_))
        ));
        assert!(matches!(
            parse_ttml_time_to_ms("1:2:3:4"),
            Err(ConvertError::InvalidTime(_))
        ));
        assert!(matches!(
            parse_ttml_time_to_ms("01:60:00.000"),
            Err(ConvertError::InvalidTime(_))
        ));
        assert!(matches!(
            parse_ttml_time_to_ms("01:00:60.000"),
            Err(ConvertError::InvalidTime(_))
        ));
        assert!(matches!(
            parse_ttml_time_to_ms("-10s"),
            Err(ConvertError::InvalidTime(_))
        ));
        assert!(matches!(
            parse_ttml_time_to_ms("-01:00:00.000"),
            Err(ConvertError::InvalidTime(_))
        ));
        assert!(matches!(
            parse_ttml_time_to_ms("10.s"),
            Err(ConvertError::InvalidTime(_))
        ));
        assert!(matches!(
            parse_ttml_time_to_ms(".5s"),
            Err(ConvertError::InvalidTime(_))
        ));
        assert!(matches!(
            parse_ttml_time_to_ms("s"),
            Err(ConvertError::InvalidTime(_))
        ));
        assert!(matches!(
            parse_ttml_time_to_ms("10.1234s"),
            Err(ConvertError::InvalidTime(_))
        ));
        assert!(matches!(
            parse_ttml_time_to_ms("10.abcs"),
            Err(ConvertError::InvalidTime(_))
        ));
        assert!(matches!(
            parse_ttml_time_to_ms("10.1234"),
            Err(ConvertError::InvalidTime(_))
        ));
        assert!(matches!(
            parse_ttml_time_to_ms("10.abc"),
            Err(ConvertError::InvalidTime(_))
        ));
        assert!(matches!(
            parse_ttml_time_to_ms("01:00:.000"),
            Err(ConvertError::InvalidTime(_))
        ));
    }

    #[test]
    fn test_normalize_text_whitespace() {
        let mut buffer = String::new();

        normalize_text_whitespace_into("  hello   world  ", &mut buffer);
        assert_eq!(buffer, "hello world");

        normalize_text_whitespace_into("\n\t  foo \r\n bar\t", &mut buffer);
        assert_eq!(buffer, "foo bar");

        normalize_text_whitespace_into("single", &mut buffer);
        assert_eq!(buffer, "single");

        normalize_text_whitespace_into("   ", &mut buffer);
        assert_eq!(buffer, "");

        normalize_text_whitespace_into("", &mut buffer);
        assert_eq!(buffer, "");
    }

    #[test]
    fn test_clean_parentheses_from_bg_text() {
        fn clean_parentheses_from_bg_text_into_owned(text: &str) -> String {
            let mut buf = String::new();
            clean_parentheses_from_bg_text_into(text, &mut buf);
            buf
        }

        assert_eq!(
            clean_parentheses_from_bg_text_into_owned("(hello)"),
            "hello"
        );
        assert_eq!(
            clean_parentheses_from_bg_text_into_owned("（hello）"),
            "hello"
        );
        assert_eq!(
            clean_parentheses_from_bg_text_into_owned(" ( hello world ) "),
            "hello world"
        );
        assert_eq!(
            clean_parentheses_from_bg_text_into_owned("(unmatched"),
            "unmatched"
        );
        assert_eq!(
            clean_parentheses_from_bg_text_into_owned("unmatched)"),
            "unmatched"
        );
        assert_eq!(
            clean_parentheses_from_bg_text_into_owned("no parentheses"),
            "no parentheses"
        );
    }
}
