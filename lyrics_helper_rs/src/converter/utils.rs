//! 包含一些工具函数的模块。

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

use lyrics_helper_core::{LyricLine, LyricSyllable};

/// 辅助函数，用于安全地将偏移量应用到 u64 时间戳上
const fn offset_timestamp(timestamp: u64, offset: i64) -> u64 {
    if offset >= 0 {
        timestamp.saturating_add(offset.saturating_abs() as u64)
    } else {
        timestamp.saturating_sub(offset.saturating_abs() as u64)
    }
}

/// 对歌词行向量应用一个时间偏移。
///
/// 此函数会就地修改传入的 `LyricLine` 向量，调整其中所有的时间戳。
///
/// # 参数
/// * `lines` - 一个可变的 `LyricLine` 切片。
/// * `offset_ms` - 要应用的偏移量（毫秒）。正数表示延迟歌词，负数表示提前歌词。
pub fn apply_offset(lines: &mut [LyricLine], offset_ms: i64) {
    if offset_ms == 0 {
        return;
    }

    for line in lines.iter_mut() {
        // 调整行本身的时间戳
        line.start_ms = offset_timestamp(line.start_ms, offset_ms);
        line.end_ms = offset_timestamp(line.end_ms, offset_ms);

        for track in &mut line.tracks {
            for word in &mut track.content.words {
                for syl in &mut word.syllables {
                    syl.start_ms = offset_timestamp(syl.start_ms, offset_ms);
                    syl.end_ms = offset_timestamp(syl.end_ms, offset_ms);
                }
            }
        }
    }
}

static METADATA_TAG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\[(?P<key>[a-zA-Z]+):(?P<value>.*)]$").expect("编译 METADATA_TAG_REGEX 失败")
});

/// 尝试将一行文本解析为 LRC 风格的 `[key:value]` 元数据。
/// 如果成功，则将结果存入 `raw_metadata` 并返回 `true`。
///
/// # 返回
/// `true` - 如果该行是有效的元数据标签并已处理。
/// `false` - 如果该行不是元数据标签。
pub(crate) fn parse_and_store_metadata(
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

/// 处理音节的原始文本，从中分离出空格信息和干净的文本。
///
/// # 参数
/// * `raw_text_slice` - 两个时间戳之间的原始文本切片。
/// * `syllables` - 正在构建的音节列表，此函数可能会修改其中最后一个元素（为其添加 `ends_with_space` 标记）。
///
/// # 返回
/// * `Some((clean_text, ends_with_space))` - 如果原始文本包含有效内容。元组中第一个元素是
///   去除了所有前后空格的纯净文本，第二个元素是一个布尔值，代表这个纯净文本在原始文本中
///   是否有尾随空格。
/// * `None` - 如果原始文本只包含空格，意味着它已被作为前一个音节的尾随空格处理完毕。
pub(crate) fn process_syllable_text(
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

/// 规范化文本中的空白字符
pub(crate) fn normalize_text_whitespace(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed.split_whitespace().collect::<Vec<&str>>().join(" ")
}

#[cfg(test)]
mod tests {
    use lyrics_helper_core::LyricSyllableBuilder;

    use super::*;

    // 处理一个正常的音节
    #[test]
    fn test_process_basic_syllable_no_spaces() {
        let mut syllables: Vec<LyricSyllable> = Vec::new();
        let raw_text = "word";

        let result = process_syllable_text(raw_text, &mut syllables);

        assert_eq!(result, Some(("word".to_string(), false)));
        assert!(syllables.is_empty(), "不应修改音节列表");
    }

    // 处理一个带前导空格的音节
    #[test]
    fn test_process_syllable_with_leading_space() {
        let mut syllables = vec![
            LyricSyllableBuilder::default()
                .text("previous")
                .build()
                .unwrap(),
        ];

        let raw_text = " word";

        let result = process_syllable_text(raw_text, &mut syllables);

        assert_eq!(result, Some(("word".to_string(), false)));
        assert_eq!(syllables.len(), 1);
        assert!(
            syllables[0].ends_with_space,
            "前一个音节应被标记为有尾随空格"
        );
    }

    // 处理一个带尾随空格的音节
    #[test]
    fn test_process_syllable_with_trailing_space() {
        let mut syllables: Vec<LyricSyllable> = Vec::new();
        let raw_text = "word ";

        let result = process_syllable_text(raw_text, &mut syllables);

        assert_eq!(result, Some(("word".to_string(), true)));
        assert!(syllables.is_empty(), "不应修改音节列表");
    }

    // 处理一个同时带前导和尾随空格的音节
    #[test]
    fn test_process_syllable_with_both_spaces() {
        let mut syllables = vec![
            LyricSyllableBuilder::default()
                .text("previous")
                .build()
                .unwrap(),
        ];
        let raw_text = " word ";

        let result = process_syllable_text(raw_text, &mut syllables);

        assert_eq!(result, Some(("word".to_string(), true)));
        assert_eq!(syllables.len(), 1);
        assert!(
            syllables[0].ends_with_space,
            "前一个音节应被标记为有尾随空格"
        );
    }

    // 处理纯空格
    #[test]
    fn test_pure_whitespace_slice_modifies_previous() {
        let mut syllables = vec![
            LyricSyllableBuilder::default()
                .text("previous")
                .build()
                .unwrap(),
        ];
        let raw_text = "   ";

        let result = process_syllable_text(raw_text, &mut syllables);

        assert_eq!(result, None, "纯空格不应返回任何新音节");
        assert_eq!(syllables.len(), 1);
        assert!(
            syllables[0].ends_with_space,
            "前一个音节应被标记为有尾随空格"
        );
    }

    // 处理空的文本
    #[test]
    fn test_empty_slice_returns_none() {
        let mut syllables: Vec<LyricSyllable> = Vec::new();
        let raw_text = "";

        let result = process_syllable_text(raw_text, &mut syllables);

        assert_eq!(result, None);
        assert!(syllables.is_empty());
    }

    // 处理前导空格，但音节列表为空
    #[test]
    fn test_leading_space_with_empty_syllables_list() {
        let mut syllables: Vec<LyricSyllable> = Vec::new();
        let raw_text = " word";

        let result = process_syllable_text(raw_text, &mut syllables);

        // 确保不会崩溃
        assert_eq!(result, Some(("word".to_string(), false)));
        assert!(syllables.is_empty());
    }

    // 处理纯空格，但音节列表为空
    #[test]
    fn test_pure_whitespace_with_empty_syllables_list() {
        let mut syllables: Vec<LyricSyllable> = Vec::new();
        let raw_text = " ";

        let result = process_syllable_text(raw_text, &mut syllables);

        // 确保不会崩溃
        assert_eq!(result, None);
        assert!(syllables.is_empty());
    }

    // 对一个已经有尾随空格的音节，再次添加空格
    #[test]
    fn test_space_is_idempotent() {
        let mut syllables = vec![
            LyricSyllableBuilder::default()
                .text("previous")
                .ends_with_space(true)
                .build()
                .unwrap(),
        ];

        let raw_text = " ";

        let result = process_syllable_text(raw_text, &mut syllables);

        assert_eq!(result, None);
        assert_eq!(syllables.len(), 1);
        assert!(syllables[0].ends_with_space, "尾随空格标志应保持不变");
    }
}
