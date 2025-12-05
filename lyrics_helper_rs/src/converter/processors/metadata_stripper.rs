//! 元数据行清理器。

use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use regex::{Regex, RegexBuilder};
use tracing::{debug, trace, warn};

use crate::converter::LyricLine;
use lyrics_helper_core::{MetadataStripperFlags, MetadataStripperOptions};

type RegexCacheKey = (String, bool); // (pattern, case_sensitive)
type RegexCacheMap = HashMap<RegexCacheKey, Regex>;

mod default_rules {
    use std::sync::OnceLock;

    use serde::Deserialize;

    #[derive(Deserialize)]
    struct DefaultStripperConfig {
        keywords: Vec<String>,
        regex_patterns: Vec<String>,
    }

    fn get_config() -> &'static DefaultStripperConfig {
        static CONFIG: OnceLock<DefaultStripperConfig> = OnceLock::new();
        CONFIG.get_or_init(|| {
            let config_str = include_str!("../../../assets/default_stripper_config.toml");
            toml::from_str(config_str).expect("Failed to parse default_stripper_config.toml")
        })
    }

    /// 获取默认的关键词列表
    pub(super) fn keywords() -> Vec<String> {
        get_config().keywords.clone()
    }

    /// 获取默认的正则表达式列表
    pub(super) fn regex_patterns() -> Vec<String> {
        get_config().regex_patterns.clone()
    }
}

fn get_regex_cache() -> &'static Mutex<RegexCacheMap> {
    static REGEX_CACHE: OnceLock<Mutex<RegexCacheMap>> = OnceLock::new();
    REGEX_CACHE.get_or_init(Default::default)
}

/// 编译或从缓存中获取一个（克隆的）Regex对象
fn get_cached_regex(pattern: &str, case_sensitive: bool) -> Option<Regex> {
    let key = (pattern.to_string(), case_sensitive);
    let cache_mutex = get_regex_cache();

    {
        let cache = cache_mutex.lock().unwrap();
        if let Some(regex) = cache.get(&key) {
            return Some(regex.clone());
        }
    }

    let Ok(new_regex) = RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
        .multi_line(false)
        .build()
    else {
        warn!("[MetadataStripper] 编译正则表达式 '{}' 失败", pattern);
        return None;
    };

    let mut cache = cache_mutex.lock().unwrap();
    Some(cache.entry(key).or_insert(new_regex).clone())
}

fn get_text(line: &LyricLine) -> String {
    line.main_text().unwrap_or_default()
}

struct StrippingRules<'a> {
    prepared_keywords: Cow<'a, [String]>,
    keyword_case_sensitive: bool,
    compiled_regexes: Vec<Regex>,
}

impl<'a> StrippingRules<'a> {
    fn new(options: &'a MetadataStripperOptions) -> Self {
        let compiled_regexes = if options
            .flags
            .contains(MetadataStripperFlags::ENABLE_REGEX_STRIPPING)
            && !options.regex_patterns.is_empty()
        {
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

        let keyword_case_sensitive = options
            .flags
            .contains(MetadataStripperFlags::KEYWORD_CASE_SENSITIVE);
        let prepared_keywords: Cow<'a, [String]> = if keyword_case_sensitive {
            Cow::Borrowed(&options.keywords)
        } else {
            Cow::Owned(options.keywords.iter().map(|k| k.to_lowercase()).collect())
        };

        Self {
            prepared_keywords,
            keyword_case_sensitive,
            compiled_regexes,
        }
    }

    fn has_rules(&self) -> bool {
        !self.prepared_keywords.is_empty() || !self.compiled_regexes.is_empty()
    }
}

fn clean_text_for_check(line_to_check: &str) -> &str {
    let mut text = line_to_check.trim();

    // 某些奇怪的歌词可能会在前面加上背景人声或者演唱者标记之类的东西
    // 通常不太可能又有这些东西又是元数据行
    if text.starts_with('(') && text.ends_with(')') {
        text = &text[1..text.len() - 1];
    } else if text.starts_with('(')
        && let Some(end_paren_idx) = text.find(')')
    {
        text = text[end_paren_idx + 1..].trim_start();
    }
    text
}

fn line_matches_rules(line_to_check: &str, rules: &StrippingRules) -> bool {
    let text_for_keyword_check = clean_text_for_check(line_to_check);

    if !rules.prepared_keywords.is_empty() {
        let prepared_line: Cow<str> = if rules.keyword_case_sensitive {
            Cow::Borrowed(text_for_keyword_check)
        } else {
            Cow::Owned(text_for_keyword_check.to_lowercase())
        };

        for keyword in rules.prepared_keywords.iter() {
            if let Some(stripped) = prepared_line.strip_prefix(keyword)
                && (stripped.trim_start().starts_with(':')
                    || stripped.trim_start().starts_with('：'))
            {
                return true;
            }
        }
    }

    if !rules.compiled_regexes.is_empty()
        && rules
            .compiled_regexes
            .iter()
            .any(|regex| regex.is_match(line_to_check))
    {
        return true;
    }

    false
}

fn line_looks_like_metadata(line_to_check: &str) -> bool {
    let text = clean_text_for_check(line_to_check);
    text.contains(':') || text.contains('：') || text.contains('-') // 一般是第一行的 歌曲名 - 歌手名 格式
}

// 强匹配行：匹配关键词加冒号，或者匹配正则表达式的行
// 弱匹配行：带有冒号，但不匹配关键词或正则表达式的行。如果夹在强匹配行之间，多半是元数据行但是没有对应的规则。但也有可能是演唱者标识，“男：...”这样的
// 真正的歌词行：既不匹配规则，又没有冒号的行，作为防火墙来阻止对之后行的移除。避免元数据在歌词中间，把中间的歌词也移除了

fn find_first_lyric_line_index(lines: &[LyricLine], rules: &StrippingRules, limit: usize) -> usize {
    let mut last_valid_metadata_index: Option<usize> = None;

    for (i, line_item) in lines.iter().enumerate().take(limit) {
        let line_text = get_text(line_item);

        let is_strict_match = line_matches_rules(&line_text, rules);
        let is_weak_match = line_looks_like_metadata(&line_text);

        if is_strict_match {
            last_valid_metadata_index = Some(i);
        } else if is_weak_match {
        } else {
            break;
        }
    }

    last_valid_metadata_index.map_or(0, |idx| idx + 1)
}

fn find_last_lyric_line_exclusive_index(
    lines: &[LyricLine],
    first_lyric_index: usize,
    rules: &StrippingRules,
    limit: usize,
) -> usize {
    if first_lyric_index >= lines.len() {
        return first_lyric_index;
    }

    let footer_scan_start_index = lines.len().saturating_sub(limit).max(first_lyric_index);
    let mut first_valid_footer_index: Option<usize> = None;

    for (i, line_item) in lines.iter().enumerate().rev() {
        if i < footer_scan_start_index {
            break;
        }

        let line_text = get_text(line_item);
        let is_strict_match = line_matches_rules(&line_text, rules);
        let is_weak_match = line_looks_like_metadata(&line_text);

        if is_strict_match {
            first_valid_footer_index = Some(i);
        } else if is_weak_match {
        } else {
            break;
        }
    }

    first_valid_footer_index.unwrap_or(lines.len())
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

    let options_to_use: Cow<MetadataStripperOptions> =
        if options.keywords.is_empty() && options.regex_patterns.is_empty() {
            debug!("[MetadataStripper] 未提供自定义规则，加载默认规则。");
            let mut temp_options = options.clone();
            temp_options.keywords = default_rules::keywords();
            temp_options.regex_patterns = default_rules::regex_patterns();
            Cow::Owned(temp_options)
        } else {
            Cow::Borrowed(options)
        };
    let rules = StrippingRules::new(&options_to_use);

    if lines.is_empty() || !rules.has_rules() {
        return;
    }

    let original_count = lines.len();

    let header_limit = options_to_use.header_scan_limit.calculate(original_count);
    let footer_limit = options_to_use.footer_scan_limit.calculate(original_count);

    let first_lyric_index = find_first_lyric_line_index(lines, &rules, header_limit);

    let last_lyric_exclusive_index =
        find_last_lyric_line_exclusive_index(lines, first_lyric_index, &rules, footer_limit);

    if first_lyric_index < last_lyric_exclusive_index {
        lines.drain(last_lyric_exclusive_index..);
        lines.drain(..first_lyric_index);
    } else if first_lyric_index > 0 || last_lyric_exclusive_index < original_count {
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

#[cfg(test)]
mod tests {
    use super::*;
    use lyrics_helper_core::{
        AnnotatedTrack, ContentType, LyricLine, LyricSyllable, LyricTrack, MetadataStripperFlags,
        MetadataStripperOptions, ScanLimitConfig, Word,
    };

    fn create_test_lines(texts: &[&str]) -> Vec<LyricLine> {
        texts
            .iter()
            .enumerate()
            .map(|(i, &text)| {
                let mut line = LyricLine::new(i as u64 * 1000, i as u64 * 1000 + 1000);

                let syllable = LyricSyllable {
                    text: text.to_string(),
                    ..Default::default()
                };
                let track = AnnotatedTrack {
                    content_type: ContentType::Main,
                    content: LyricTrack {
                        words: vec![Word {
                            syllables: vec![syllable],
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    ..Default::default()
                };
                line.add_track(track);
                line
            })
            .collect()
    }

    fn lines_to_texts(lines: &[LyricLine]) -> Vec<String> {
        lines
            .iter()
            .map(|line| line.main_text().unwrap_or_default())
            .collect()
    }

    #[test]
    fn test_stripper_disabled() {
        let mut lines = create_test_lines(&["Artist: Me", "Lyric line"]);
        let options = MetadataStripperOptions {
            flags: MetadataStripperFlags::empty(),
            keywords: vec!["Artist".to_string()],
            ..Default::default()
        };

        strip_descriptive_metadata_lines(&mut lines, &options);

        assert_eq!(lines_to_texts(&lines), vec!["Artist: Me", "Lyric line"]);
    }

    #[test]
    fn test_strip_header_keywords_basic() {
        let mut lines = create_test_lines(&["Artist: A", "Album: B", "Lyric 1", "Lyric 2"]);
        let options = MetadataStripperOptions {
            flags: MetadataStripperFlags::ENABLED,
            keywords: vec!["Artist".to_string(), "Album".to_string()],
            ..Default::default()
        };

        strip_descriptive_metadata_lines(&mut lines, &options);
        assert_eq!(lines_to_texts(&lines), vec!["Lyric 1", "Lyric 2"]);
    }

    #[test]
    fn test_keyword_case_insensitivity() {
        let mut lines = create_test_lines(&["artist: A", "Lyric 1"]);
        let options = MetadataStripperOptions {
            flags: MetadataStripperFlags::ENABLED,
            keywords: vec!["Artist".to_string()],
            ..Default::default()
        };

        strip_descriptive_metadata_lines(&mut lines, &options);
        assert_eq!(lines_to_texts(&lines), vec!["Lyric 1"]);
    }

    #[test]
    fn test_keywords_with_lrc_tags_and_whitespace() {
        let mut lines = create_test_lines(&["[ti:Title]", "[00:01.00] Artist : A", "Lyric 1"]);
        let options = MetadataStripperOptions {
            flags: MetadataStripperFlags::ENABLED,
            keywords: vec!["ti".to_string(), "Artist".to_string()],
            ..Default::default()
        };

        strip_descriptive_metadata_lines(&mut lines, &options);
        assert_eq!(lines_to_texts(&lines), vec!["Lyric 1"]);
    }

    #[test]
    fn test_keywords_with_full_width_colon() {
        let mut lines = create_test_lines(&["作曲：某人", "Lyric 1"]);
        let options = MetadataStripperOptions {
            flags: MetadataStripperFlags::ENABLED,
            keywords: vec!["作曲".to_string()],
            ..Default::default()
        };

        strip_descriptive_metadata_lines(&mut lines, &options);
        assert_eq!(lines_to_texts(&lines), vec!["Lyric 1"]);
    }

    #[test]
    fn test_regex_case_sensitivity() {
        let mut lines = create_test_lines(&["NOTE: important", "note: less important", "Lyric 1"]);
        let options = MetadataStripperOptions {
            flags: MetadataStripperFlags::ENABLED
                | MetadataStripperFlags::ENABLE_REGEX_STRIPPING
                | MetadataStripperFlags::REGEX_CASE_SENSITIVE,
            regex_patterns: vec![r"^NOTE:".to_string()],
            ..Default::default()
        };

        strip_descriptive_metadata_lines(&mut lines, &options);
        assert_eq!(
            lines_to_texts(&lines),
            vec!["note: less important", "Lyric 1"]
        );
    }

    #[test]
    fn test_all_lines_are_metadata() {
        let mut lines = create_test_lines(&["Artist: A", "Album: B", "Source: Web"]);
        let options = MetadataStripperOptions {
            flags: MetadataStripperFlags::ENABLED,
            keywords: vec![
                "Artist".to_string(),
                "Album".to_string(),
                "Source".to_string(),
            ],
            ..Default::default()
        };

        strip_descriptive_metadata_lines(&mut lines, &options);
        assert!(lines.is_empty(), "Expected lines to be empty");
    }

    #[test]
    fn test_empty_input_vec() {
        let mut lines = create_test_lines(&[]);
        let options = MetadataStripperOptions {
            flags: MetadataStripperFlags::ENABLED,
            keywords: vec!["Artist".to_string()],
            ..Default::default()
        };

        strip_descriptive_metadata_lines(&mut lines, &options);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_stripper_block_logic_header() {
        let texts = vec![
            "作词：A",
            "作曲：B",
            "不匹配规则的行：123",
            "编曲：C",
            "真正的歌词行", // 不包含冒号，预期应该在此处停止扫描
            "演唱者A：D",
        ];

        let mut lines = create_test_lines(&texts);

        let options = MetadataStripperOptions {
            flags: MetadataStripperFlags::ENABLED,
            keywords: vec![
                "作词".to_string(),
                "作曲".to_string(),
                "编曲".to_string(),
                "演唱者A".to_string(), // 意外情况：是真正的歌词，但是匹配规则
            ],
            // 避免尾部扫描把最后一行也移除了
            footer_scan_limit: ScanLimitConfig {
                ratio: 0.0,
                min_lines: 0,
                max_lines: 0,
            },
            ..Default::default()
        };

        strip_descriptive_metadata_lines(&mut lines, &options);

        assert_eq!(lines_to_texts(&lines), vec!["真正的歌词行", "演唱者A：D"]);
    }

    #[test]
    fn test_stripper_block_logic_footer() {
        let texts = vec![
            "Line 1",
            "Line 2",
            "Line 3",
            "制作人：X",
            "Extra Info: Y",
            "发行：Z",
        ];
        let mut lines = create_test_lines(&texts);
        let options = MetadataStripperOptions {
            flags: MetadataStripperFlags::ENABLED,
            keywords: vec!["制作人".to_string(), "发行".to_string()],
            ..Default::default()
        };

        strip_descriptive_metadata_lines(&mut lines, &options);

        assert_eq!(lines_to_texts(&lines), vec!["Line 1", "Line 2", "Line 3"]);
    }

    #[test]
    fn test_stripper_pure_gap_no_colon() {
        let texts = vec!["Title: A", "123", "Artist: B"];
        let mut lines = create_test_lines(&texts);

        let options = MetadataStripperOptions {
            flags: MetadataStripperFlags::ENABLED,
            keywords: vec!["Title".to_string(), "Artist".to_string()],
            footer_scan_limit: ScanLimitConfig {
                ratio: 0.0,
                min_lines: 0,
                max_lines: 0,
            },
            ..Default::default()
        };

        strip_descriptive_metadata_lines(&mut lines, &options);

        assert_eq!(lines_to_texts(&lines), vec!["123", "Artist: B"]);
    }
    #[test]
    fn test_default_config_parsing() {
        let keywords = default_rules::keywords();
        let regex_patterns = default_rules::regex_patterns();

        assert!(!keywords.is_empty(), "默认关键词不应为空");
        assert!(!regex_patterns.is_empty(), "默认正则表达式不应为空");
    }
}
