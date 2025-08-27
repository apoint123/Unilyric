//! 元数据行清理器。

use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};

use regex::{Regex, RegexBuilder};
use tracing::{debug, trace, warn};

use crate::converter::LyricLine;
use lyrics_helper_core::{ContentType, MetadataStripperFlags, MetadataStripperOptions};

type RegexCacheKey = (String, bool); // (pattern, case_sensitive)
type RegexCacheMap = BTreeMap<RegexCacheKey, Regex>;

fn get_regex_cache() -> &'static Mutex<RegexCacheMap> {
    static REGEX_CACHE: OnceLock<Mutex<RegexCacheMap>> = OnceLock::new();
    REGEX_CACHE.get_or_init(Default::default)
}

/// 编译或从缓存中获取一个（克隆的）Regex对象
fn get_cached_regex(pattern: &str, case_sensitive: bool) -> Option<Regex> {
    let key = (pattern.to_string(), case_sensitive);
    let cache_mutex = get_regex_cache();
    let mut cache = cache_mutex.lock().unwrap();

    if let Some(regex) = cache.get(&key) {
        return Some(regex.clone());
    }

    let Ok(new_regex) = RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
        .multi_line(false)
        .build()
    else {
        warn!("[MetadataStripper] 编译正则表达式 '{}' 失败", pattern);
        return None;
    };

    let regex_to_return = new_regex.clone();
    cache.insert(key, new_regex);

    Some(regex_to_return)
}

/// 辅助函数：从 `LyricLine` 获取用于匹配的纯文本内容。
fn get_plain_text_from_new_lyric_line(line: &LyricLine) -> String {
    if let Some(main_track) = line
        .tracks
        .iter()
        .find(|t| t.content_type == ContentType::Main)
    {
        return main_track.content.text().trim().to_string();
    }
    String::new()
}

/// 从 `LyricLine` 列表中移除元数据行。
pub fn strip_descriptive_metadata_lines(
    lines: &mut Vec<LyricLine>,
    options: &MetadataStripperOptions,
) {
    if !options.flags.contains(MetadataStripperFlags::ENABLED) {
        trace!("[MetadataStripper] 功能被禁用，跳过处理。");
        return;
    }

    let keywords_to_use: &[String] = &options.keywords;
    let use_regex = options
        .flags
        .contains(MetadataStripperFlags::ENABLE_REGEX_STRIPPING)
        && !options.regex_patterns.is_empty();

    if lines.is_empty() || (keywords_to_use.is_empty() && !use_regex) {
        return;
    }

    let original_count = lines.len();

    let compiled_regexes: Vec<Regex> = if use_regex {
        options
            .regex_patterns
            .iter()
            .filter_map(|pattern_str| {
                if pattern_str.trim().is_empty() {
                    return None;
                }
                get_cached_regex(
                    pattern_str,
                    options
                        .flags
                        .contains(MetadataStripperFlags::REGEX_CASE_SENSITIVE),
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    let prepared_keywords: Cow<'_, [String]> = if options
        .flags
        .contains(MetadataStripperFlags::KEYWORD_CASE_SENSITIVE)
    {
        Cow::Borrowed(keywords_to_use)
    } else {
        Cow::Owned(keywords_to_use.iter().map(|k| k.to_lowercase()).collect())
    };
    let keyword_case_sensitive = options
        .flags
        .contains(MetadataStripperFlags::KEYWORD_CASE_SENSITIVE);

    let line_matches_any_rule = |line_to_check: &str| -> bool {
        if !keywords_to_use.is_empty() {
            let mut text_after_prefix = line_to_check.trim_start();
            if text_after_prefix.starts_with('[') {
                if let Some(end_bracket_idx) = text_after_prefix.find(']') {
                    text_after_prefix = text_after_prefix[end_bracket_idx + 1..].trim_start();
                }
            } else if text_after_prefix.starts_with('(')
                && let Some(end_paren_idx) = text_after_prefix.find(')')
            {
                text_after_prefix = text_after_prefix[end_paren_idx + 1..].trim_start();
            }

            let prepared_line: Cow<str> = if keyword_case_sensitive {
                Cow::Borrowed(text_after_prefix)
            } else {
                Cow::Owned(text_after_prefix.to_lowercase())
            };

            for keyword in prepared_keywords.iter() {
                if let Some(stripped) = prepared_line.strip_prefix(keyword)
                    && (stripped.trim_start().starts_with(':')
                        || stripped.trim_start().starts_with('：'))
                {
                    return true;
                }
            }
        }

        if !compiled_regexes.is_empty()
            && compiled_regexes
                .iter()
                .any(|regex| regex.is_match(line_to_check))
        {
            return true;
        }

        false
    };

    let mut last_matching_header_index: Option<usize> = None;
    let header_scan_limit = 20.min(lines.len());
    for (i, line_item) in lines.iter().enumerate().take(header_scan_limit) {
        let line_text = get_plain_text_from_new_lyric_line(line_item);
        if line_matches_any_rule(&line_text) {
            last_matching_header_index = Some(i);
        }
    }
    let first_lyric_line_index = last_matching_header_index.map_or(0, |idx| idx + 1);

    let mut last_lyric_line_exclusive_index = lines.len();
    if first_lyric_line_index < lines.len() {
        let end_lookback_count = 10;
        let footer_scan_start_index = lines
            .len()
            .saturating_sub(end_lookback_count)
            .max(first_lyric_line_index);
        for i in (footer_scan_start_index..lines.len()).rev() {
            let line_text = get_plain_text_from_new_lyric_line(&lines[i]);
            if line_matches_any_rule(&line_text) {
                last_lyric_line_exclusive_index = i;
            } else {
                break;
            }
        }
    } else {
        last_lyric_line_exclusive_index = first_lyric_line_index;
    }

    if first_lyric_line_index < last_lyric_line_exclusive_index {
        lines.drain(last_lyric_line_exclusive_index..);
        lines.drain(..first_lyric_line_index);
    } else if first_lyric_line_index > 0 || last_lyric_line_exclusive_index < original_count {
        lines.clear();
    }

    if lines.len() < original_count {
        debug!(
            "[MetadataStripper] 清理完成，总行数从 {} 变为 {}。",
            original_count,
            lines.len()
        );
    }
}
