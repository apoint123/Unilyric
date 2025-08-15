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
        let text = main_track
            .content
            .words
            .iter()
            .flat_map(|word| &word.syllables)
            .map(|syllable| syllable.text.as_str())
            .collect::<String>();
        return text.trim().to_string();
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

    let default_keywords: &[&str] = &[
        "作曲",
        "作词",
        "编曲",
        "演唱",
        "歌手",
        "歌名",
        "专辑",
        "发行",
        "出品",
        "监制",
        "录音",
        "混音",
        "母带",
        "吉他",
        "贝斯",
        "鼓",
        "键盘",
        "弦乐",
        "和声",
        "版权",
        "制作人",
        "原唱",
        "翻唱",
        "词",
        "曲",
        "发行人",
        "发行公司",
        "宣推",
        "录音制作",
        "制作发行",
        "制作团队",
        "音乐制作",
        "录音师",
        "混音工程师",
        "母带工程师",
        "制作统筹",
        "艺术指导",
        "出品团队",
        "发行方",
        "和声编写",
        "封面设计",
        "策划",
        "营销推广",
        "总策划",
        "特别鸣谢",
        "出品人",
        "出品公司",
        "联合出品",
        "词曲提供",
        "制作公司",
        "推广策划",
        "乐器演奏",
        "钢琴/合成器演奏",
        "钢琴演奏",
        "合成器演奏",
        "弦乐编写",
        "弦乐监制",
        "第一小提琴",
        "第二小提琴",
        "中提琴",
        "大提琴",
        "弦乐录音师",
        "弦乐录音室",
        "和声演唱",
        "录/混音",
        "制作助理",
        "和音",
        "乐队统筹",
        "维伴音乐",
        "灯光设计",
        "配唱制作人",
        "文案",
        "设计",
        "策划统筹",
        "企划宣传",
        "企划营销",
        "录音室",
        "混音室",
        "母带后期制作人",
        "母带后期处理工程师",
        "母带后期处理录音室",
        "鸣谢",
        "工作室",
        "特别企划",
        "音频编辑",
        "词曲协力",
        "制作公司/OP",
        "企划",
        "宣传",
        "统筹",
        "推广",
        "出品/发行",
        // --- 纯英文关键字 ---
        "OP",
        "SP",
        "Lyrics by",
        "Composed by",
        "Produced by",
        "Published by",
        "Vocals by",
        "Background Vocals by",
        "Additional Vocal by",
        "Mixing Engineer",
        "Mastered by",
        "Executive Producer",
        "Vocal Engineer",
        "Vocals Produced by",
        "Recorded at",
        "Repertoire Owner",
        "Co-Producer",
        "Mastering Engineer",
        "Written by",
        "Lyrics",
        "Composer",
        "Arranged By",
        "Record Producer",
        "Guitar",
        "Music Production",
        "Recording Engineer",
        "Backing Vocal",
        "Art Director",
        "Chief Producer",
        "Production Team",
        "Publisher",
        "Lyricist",
        "Arranger",
        "Producer",
        "Backing Vocals",
        "Backing Vocals Design",
        "Cover Design",
        "Planner",
        "Marketing Promotion",
        "Chref Planner",
        "Acknowledgement",
        "Production Company",
        "Jointly Produced by",
        "Co-production",
        "Presenter",
        "Presented by",
        "Co-produced by",
        "Lyrics and Composition Provided by",
        "Music and Lyrics Provided by",
        "Lyrics & Composition Provided by",
        "Words and Music by",
        "Distribution",
        "Release",
        "Distributed by",
        "Released by",
        "Produce Company",
        "Promotion Planning",
        "Marketing Strategy",
        "Promotion Strategy",
        "Strings",
        "First Violin",
        "Second Violin",
        "Viola",
        "Cello",
        "Vocal Producer",
        "Supervised production",
        "Copywriting",
        "Design",
        "Planner and coordinator",
        "Propaganda",
        "Arrangement",
        "Guitars",
        "Bass",
        "Drums",
        "Backing Vocal Arrangement",
        "Strings Arrangement",
        "Recording Studio",
        "OP/发行",
        "混音/母带工程师",
        "OP/SP",
        "词Lyrics",
        "曲Composer",
        "编曲Arranged By",
        "制作人Record Producer",
        "吉他Guitar",
        "音乐制作Music Production",
        "录音师Recording Engineer",
        "混音工程师Mixing Engineer",
        "母带工程师Mastering Engineer",
        "和声Backing Vocal",
        "制作统筹Executive Producer",
        "艺术指导Art Director",
        "监制Chief Producer",
        "出品团队Production Team",
        "发行方Publisher",
        "词Lyricist",
        "编曲Arranger",
        "制作人Producer",
        "和声Backing Vocals",
        "和声编写Backing Vocals Design",
        "混音Mixing Engineer",
        "封面设计Cover Design",
        "策划Planner",
        "营销推广Marketing Promotion",
        "总策划Chref Planner",
        "特别鸣谢Acknowledgement",
        "出品人Chief Producer",
        "出品公司Production Company",
        "联合出品Co-produced by",
        "联合出品Jointly Produced by",
        "联合出品Co-production",
        "出品方Presenter",
        "出品方Presented by",
        "词曲提供Lyrics and Composition Provided by",
        "词曲提供Music and Lyrics Provided by",
        "词曲提供Lyrics & Composition Provided by",
        "词曲提供Words and Music by",
        "发行Distribution",
        "发行Release",
        "发行Distributed by",
        "发行Released by",
        "制作公司Produce Company",
        "推广策划Promotion Planning",
        "推广策划Marketing Strategy",
        "推广策划Promotion Strategy",
        "弦乐 Strings",
        "第一小提琴 First Violin",
        "第二小提琴 Second Violin",
        "中提琴 Viola",
        "大提琴 Cello",
        "配唱制作人Vocal Producer",
        "监制Supervised production",
        "文案Copywriting",
        "设计Design",
        "策划统筹Planner and coordinator",
        "企划宣传Propaganda",
        "编曲Arrangement",
        "吉他Guitars",
        "贝斯Bass",
        "鼓Drums",
        "和声编写Backing Vocal Arrangement",
        "弦乐编写Strings Arrangement",
        "录音室Recording Studio",
        "混音室Mixing Studio",
        "母带后期制作人Mastering Producer",
        "母带后期处理工程师Mastering Engineer",
        "母带后期处理录音室Mastering Studio",
        "制作人Music Producer",
        "统筹Planning",
        "营销推广Marketing",
        "出品人Producer",
    ];
    let default_regex: &[&str] = &[
        "(?:【.*?未经.*?】|\\(.*?未经.*?\\)|「.*?未经.*?」|（.*?未经.*?）|『.*?未经.*?』)",
        "(?:【.*?音乐人.*?】|\\(.*?音乐人.*?\\)|「.*?音乐人.*?」|（.*?音乐人.*?）|『.*?音乐人.*?』)",
        ".*?未经.*?许可.*?不得.*?使用.*?",
        ".*?未经.*?许可.*?不得.*?方式.*?",
        "未经著作权人书面许可，\\s*不得以任何方式\\s*[(\\u{FF08}]包括.*?等[)\\u{FF09}]\\s*使用",
    ];

    // 根据选项决定是使用用户自定义的规则还是默认规则。
    let keywords_to_use: Cow<'_, [String]> = options.keywords.as_ref().map_or_else(
        || Cow::Owned(default_keywords.iter().map(|s| (*s).to_string()).collect()),
        |v| Cow::Borrowed(v.as_slice()),
    );

    let regex_to_use: Cow<'_, [String]> = options.regex_patterns.as_ref().map_or_else(
        || Cow::Owned(default_regex.iter().map(|s| (*s).to_string()).collect()),
        |v| Cow::Borrowed(v.as_slice()),
    );

    // 如果没有规则，直接返回
    if lines.is_empty()
        || (keywords_to_use.is_empty()
            && (!options
                .flags
                .contains(MetadataStripperFlags::ENABLE_REGEX_STRIPPING)
                || regex_to_use.is_empty()))
    {
        return;
    }

    let original_count = lines.len();

    // 基于关键词的移除
    if !keywords_to_use.is_empty() {
        let prepared_keywords: Cow<'_, [String]> = if options
            .flags
            .contains(MetadataStripperFlags::KEYWORD_CASE_SENSITIVE)
        {
            keywords_to_use
        } else {
            Cow::Owned(keywords_to_use.iter().map(|k| k.to_lowercase()).collect())
        };

        let line_matches_keyword_rule = |line_to_check: &str| -> bool {
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

            // 决定是否忽略大小写
            let prepared_line: Cow<str> = if options
                .flags
                .contains(MetadataStripperFlags::KEYWORD_CASE_SENSITIVE)
            {
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
            false
        };

        let mut last_matching_header_index: Option<usize> = None;
        let header_scan_limit = 20.min(lines.len());
        for (i, line_item) in lines.iter().enumerate().take(header_scan_limit) {
            let line_text = get_plain_text_from_new_lyric_line(line_item);
            if line_matches_keyword_rule(&line_text) {
                last_matching_header_index = Some(i);
            }
        }
        let first_lyric_line_index = last_matching_header_index.map_or(0, |idx| idx + 1);

        // 从歌词末尾反向扫描，找到第一个元数据行
        let mut last_lyric_line_exclusive_index = lines.len();
        if first_lyric_line_index < lines.len() {
            let end_lookback_count = 10;
            let footer_scan_start_index = lines
                .len()
                .saturating_sub(end_lookback_count)
                .max(first_lyric_line_index);
            for i in (footer_scan_start_index..lines.len()).rev() {
                let line_text = get_plain_text_from_new_lyric_line(&lines[i]);
                if line_matches_keyword_rule(&line_text) {
                    last_lyric_line_exclusive_index = i;
                } else {
                    break;
                }
            }
        } else {
            last_lyric_line_exclusive_index = first_lyric_line_index;
        }

        if first_lyric_line_index < last_lyric_line_exclusive_index {
            // 先移除尾部，这样不会影响头部的索引
            lines.drain(last_lyric_line_exclusive_index..);
            lines.drain(..first_lyric_line_index);
        } else if first_lyric_line_index > 0 || last_lyric_line_exclusive_index < original_count {
            // 如果头尾边界交叉或重合，说明所有行都是元数据，全部清空
            lines.clear();
        }

        if lines.len() < original_count {
            debug!("[MetadataStripper] 关键词移除后，剩余 {} 行。", lines.len());
        }
    }

    // 基于正则表达式的移除
    if options
        .flags
        .contains(MetadataStripperFlags::ENABLE_REGEX_STRIPPING)
        && !regex_to_use.is_empty()
        && !lines.is_empty()
    {
        let compiled_regexes: Vec<Regex> = regex_to_use
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
            .collect();

        if !compiled_regexes.is_empty() {
            let before_count = lines.len();
            lines.retain(|line| {
                let line_text = get_plain_text_from_new_lyric_line(line);
                !compiled_regexes
                    .iter()
                    .any(|regex| regex.is_match(&line_text))
            });
            let removed_count = before_count - lines.len();
            if removed_count > 0 {
                debug!("[MetadataStripper] 正则表达式移除了 {removed_count} 行。");
            }
        }
    }

    if lines.len() < original_count {
        debug!(
            "[MetadataStripper] 清理完成，总行数从 {} 变为 {}。",
            original_count,
            lines.len()
        );
    }
}
