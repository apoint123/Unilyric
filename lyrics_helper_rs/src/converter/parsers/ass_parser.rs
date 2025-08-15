//! ASS 格式解析器
//!
//! 此解析器强依赖于字幕行在文件中的物理顺序，若顺序错误，可能会导致辅助行关联错误。

use std::collections::HashMap;

use regex::Regex;
use std::sync::LazyLock;

use lyrics_helper_core::{
    AnnotatedTrack, ContentType, ConvertError, LyricFormat, LyricLine, LyricLineBuilder,
    LyricSyllable, LyricSyllableBuilder, LyricTrack, ParsedSourceData, TrackMetadataKey, Word,
};

/// 用于解析ASS时间戳字符串 (H:MM:SS.CS)
static ASS_TIME_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\d+):(\d{2}):(\d{2})\.(\d{2})").expect("编译 ASS_TIME_REGEX 失败")
});

/// 用于解析ASS文本中的 K 标签 `{\k[厘秒]}`
static KARAOKE_TAG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\\k([^}]+)}").expect("编译 KARAOKE_TAG_REGEX 失败"));

/// 用于解析ASS文件中 [Events] 部分的 Dialogue 或 Comment 行
static ASS_LINE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^(?P<Type>Comment|Dialogue):\s*",       // 行类型
        r"(?P<Layer>\d+)\s*,",                    // Layer
        r"(?P<Start>\d+:\d{2}:\d{2}\.\d{2})\s*,", // 开始时间
        r"(?P<End>\d+:\d{2}:\d{2}\.\d{2})\s*,",   // 结束时间
        r"(?P<Style>[^,]*?)\s*,",                 // 样式
        r"(?P<Actor>[^,]*?)\s*,",                 // 角色
        r"[^,]*,[^,]*,[^,]*,",                    // 忽略 MarginL, MarginR, MarginV
        r"(?P<Effect>[^,]*?)\s*,",                // 特效
        r"(?P<Text>.*?)\s*$"                      // 文本内容
    ))
    .expect("编译 ASS_LINE_REGEX 失败")
});

/// 用于从 Actor 字段中解析 iTunes 的歌曲组成部分
static SONG_PART_DIRECTIVE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"itunes:song-part=(?:"([^"]*)"|'([^']*)'|([^\s"']+))"#)
        .expect("编译 SONG_PART_DIRECTIVE_REGEX 失败")
});

/// 用于解析 v[数字] 格式的演唱者标签
static AGENT_V_TAG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^v(\d+)$").expect("编译 AGENT_V_TAG_REGEX 失败"));

/// 存储从 Actor 字段解析出的临时信息。
#[derive(Debug, Default)]
struct ParsedActorInfo {
    agent: Option<String>,
    song_part: Option<String>,
    lang_code: Option<String>,
    is_background: bool,
    is_marker: bool,
}

/// 解析 ASS 时间字符串 (H:MM:SS.CS) 并转换为毫秒。
fn parse_ass_time(time_str: &str, line_num: usize) -> Result<u64, ConvertError> {
    ASS_TIME_REGEX.captures(time_str).map_or_else(
        || {
            Err(ConvertError::InvalidTime(format!(
                "第 {line_num} 行时间格式错误: {time_str} "
            )))
        },
        |caps| {
            let h: u64 = caps[1].parse().map_err(ConvertError::ParseInt)?;
            let m: u64 = caps[2].parse().map_err(ConvertError::ParseInt)?;
            let s: u64 = caps[3].parse().map_err(ConvertError::ParseInt)?;
            let cs: u64 = caps[4].parse().map_err(ConvertError::ParseInt)?;
            Ok(h * 3_600_000 + m * 60_000 + s * 1000 + cs * 10)
        },
    )
}

/// 解析包含卡拉OK标签的ASS文本，分解为带时间信息的 `LyricSyllable`。
/// 返回音节列表和根据 `\k` 标签计算出的实际结束时间。
fn parse_karaoke_text(
    text: &str,
    line_start_ms: u64,
    line_num: usize,
) -> Result<(Vec<LyricSyllable>, u64), ConvertError> {
    let mut syllables: Vec<LyricSyllable> = Vec::new();
    let mut current_char_pos = 0;
    let mut current_time_ms = line_start_ms;
    let mut max_end_time_ms = line_start_ms;
    let mut previous_duration_cs: u32 = 0;

    for cap in KARAOKE_TAG_REGEX.captures_iter(text) {
        let tag_match = cap.get(0).ok_or_else(|| {
            ConvertError::InvalidLyricFormat(format!("第 {line_num} 行: 无法提取卡拉OK标签匹配项"))
        })?;
        let duration_cs_str = cap
            .get(1)
            .ok_or_else(|| {
                ConvertError::InvalidLyricFormat(format!(
                    "第 {line_num} 行: 无法从卡拉OK标签提取时长"
                ))
            })?
            .as_str();
        let current_k_duration_cs: u32 = duration_cs_str.parse().map_err(|_| {
            ConvertError::InvalidTime(format!(
                "第 {line_num} 行: 无效的卡拉OK时长值: {duration_cs_str}"
            ))
        })?;

        let text_slice = &text[current_char_pos..tag_match.start()];
        let syllable_duration_ms = u64::from(previous_duration_cs) * 10;

        if text_slice.is_empty() {
            // 如果上一个 \k 标签后没有内容（连续的 \k 标签）
            // 同样将时长累加到时间流中
            current_time_ms += syllable_duration_ms;
        } else {
            // 如果内容是纯空格，则执行合并逻辑
            if text_slice.trim().is_empty() {
                // 将这个纯空格音节的时长加到时间流中
                current_time_ms += syllable_duration_ms;
                // 并且将前一个有效音节标记为以空格结尾
                if let Some(last_syllable) = syllables.last_mut() {
                    last_syllable.ends_with_space = true;
                }
            } else {
                // 如果内容是有效文本
                let syllable_end_ms = current_time_ms + syllable_duration_ms;

                // 在创建音节时就直接处理尾随空格
                let mut text_to_store = text_slice.to_string();
                let mut ends_with_space = false;

                if text_to_store.ends_with(' ') {
                    text_to_store = text_to_store.trim_end().to_string();
                    ends_with_space = true;
                }

                // 只有当修剪后文本不为空时，才创建音节
                if !text_to_store.is_empty() {
                    let syllable = LyricSyllableBuilder::default()
                        .text(text_to_store)
                        .start_ms(current_time_ms)
                        .end_ms(syllable_end_ms)
                        .duration_ms(syllable_duration_ms)
                        .ends_with_space(ends_with_space)
                        .build()
                        .unwrap();
                    syllables.push(syllable);
                }
                // 推进时间
                current_time_ms = syllable_end_ms;
            }
        }

        max_end_time_ms = max_end_time_ms.max(current_time_ms);
        previous_duration_cs = current_k_duration_cs;
        current_char_pos = tag_match.end();
    }

    // 处理最后一个 `\k` 标签后的文本
    let remaining_text_slice = &text[current_char_pos..];
    if remaining_text_slice.is_empty() {
        max_end_time_ms =
            max_end_time_ms.max(current_time_ms + u64::from(previous_duration_cs) * 10);
    } else {
        let syllable_duration_ms = u64::from(previous_duration_cs) * 10;

        if remaining_text_slice.trim().is_empty() {
            if let Some(last_syllable) = syllables.last_mut() {
                last_syllable.ends_with_space = true;
            }
            current_time_ms += syllable_duration_ms;
        } else {
            let syllable_end_ms = current_time_ms + syllable_duration_ms;

            let mut text_to_store = remaining_text_slice.to_string();
            let mut ends_with_space = false;

            if text_to_store.ends_with(' ') {
                text_to_store = text_to_store.trim_end().to_string();
                ends_with_space = false;
            }

            if !text_to_store.is_empty() {
                let syllable = LyricSyllableBuilder::default()
                    .text(text_to_store)
                    .start_ms(current_time_ms)
                    .end_ms(syllable_end_ms)
                    .duration_ms(syllable_duration_ms)
                    .ends_with_space(ends_with_space)
                    .build()
                    .unwrap();
                syllables.push(syllable);
            }
            current_time_ms = syllable_end_ms;
        }
        max_end_time_ms = max_end_time_ms.max(current_time_ms);
    }

    Ok((syllables, max_end_time_ms))
}

/// 解析 Actor 字段以确定角色、语言等信息。
fn parse_actor(
    actor_str_input: &str,
    style: &str,
    line_num: usize,
    warnings: &mut Vec<String>,
) -> ParsedActorInfo {
    let mut actor_str = actor_str_input.to_string();
    let mut info = ParsedActorInfo::default();

    // 解析 itunes:song-part
    if let Some(caps) = SONG_PART_DIRECTIVE_REGEX.captures(&actor_str)
        && let Some(full_match) = caps.get(0)
    {
        let full_match_str = full_match.as_str();
        info.song_part = caps
            .get(1)
            .or_else(|| caps.get(2))
            .or_else(|| caps.get(3))
            .map(|m| m.as_str().to_string());
        actor_str = actor_str.replace(full_match_str, "");
    }

    // 解析剩余的标签
    let mut role_candidate: Option<String> = None;

    for tag in actor_str.split_whitespace() {
        if tag.starts_with("x-lang:") {
            if info.lang_code.is_some() {
                warnings.push(format!(
                    "第 {line_num} 行: 发现多个 'x-lang:' 标签，将使用最后一个。"
                ));
            }
            info.lang_code = Some(tag.trim_start_matches("x-lang:").to_string());
        } else if tag == "x-mark" {
            info.is_marker = true;
        } else if tag == "x-bg" || AGENT_V_TAG_REGEX.is_match(tag) {
            if let Some(existing_role) = &role_candidate {
                warnings.push(format!(
                    "第 {line_num} 行: 发现冲突的角色标签 '{existing_role}' 和 '{tag}'，将使用第一个 ('{existing_role}')。"
                ));
            } else {
                role_candidate = Some(tag.to_string());
            }
        }
    }

    // 对主歌词样式应用角色逻辑
    if style == "orig" || style == "default" {
        if let Some(role) = role_candidate {
            if role == "x-bg" {
                info.is_background = true;
                info.agent = None;
            } else {
                // 是 `v[数字]` 格式，则作为 agent
                info.agent = Some(role);
            }
        } else {
            // 没有指定角色，默认为 v1
            info.agent = Some("v1".to_string());
        }
    } else if (style == "ts" || style == "trans" || style == "roma") && info.lang_code.is_none() {
        warnings.push(format!(
            "第 {line_num} 行: 辅助行样式 '{style}' 缺少 'x-lang:' 标签，可能导致语言关联错误。"
        ));
    }

    info
}

/// 解析ASS格式内容到 `ParsedSourceData` 结构。
pub fn parse_ass(content: &str) -> Result<ParsedSourceData, ConvertError> {
    // 确定是逐字模式还是逐行模式
    let has_karaoke_tags = content.contains(r"{\k");

    let mut raw_metadata: HashMap<String, Vec<String>> = HashMap::new();
    let mut warnings: Vec<String> = Vec::new();

    let mut new_lines_internal: Vec<LyricLine> = Vec::new();
    let mut current_line: Option<LyricLine> = None;

    let mut in_events_section = false;
    let mut subtitle_line_num = 0;

    for line_str_raw in content.lines() {
        subtitle_line_num += 1;
        let line_str = line_str_raw.trim();

        // 寻找并进入 [Events] 区域
        if !in_events_section {
            if line_str.eq_ignore_ascii_case("[Events]") {
                in_events_section = true;
            }
            continue;
        }

        if line_str.starts_with("Format:") || line_str.is_empty() {
            continue;
        }

        if let Some(caps) = ASS_LINE_REGEX.captures(line_str) {
            let line_type = &caps["Type"];
            let style = &caps["Style"];
            let text_content = &caps["Text"];

            if text_content.is_empty() {
                continue;
            }

            if style == "meta" && line_type == "Comment" {
                if let Some((key, value)) = text_content.split_once(':') {
                    raw_metadata
                        .entry(key.trim().to_string())
                        .or_default()
                        .push(value.trim().to_string());
                }
                continue;
            }

            if line_type != "Dialogue" {
                continue;
            }

            let start_ms = parse_ass_time(&caps["Start"], subtitle_line_num)?;
            let actor_raw = &caps["Actor"];
            let actor_info = parse_actor(actor_raw, style, subtitle_line_num, &mut warnings);

            let style_lower = style.to_lowercase();

            // 主歌词行: 开启一个新的 LyricLine
            if style_lower == "orig" || style_lower == "default" {
                if let Some(completed_line) = current_line.take() {
                    new_lines_internal.push(completed_line);
                }

                let (syllables, calculated_end_ms) =
                    parse_karaoke_text(text_content, start_ms, subtitle_line_num)?;

                let content_type = if actor_info.is_background {
                    ContentType::Background
                } else {
                    ContentType::Main
                };

                let words = if syllables.is_empty() && !has_karaoke_tags {
                    // 对于逐行歌词，即使没有音节，也创建一个包含整行文本的Word
                    vec![Word {
                        syllables: vec![
                            LyricSyllableBuilder::default()
                                .text(text_content.to_string())
                                .start_ms(start_ms)
                                .end_ms(start_ms)
                                .build()
                                .unwrap(),
                        ],
                        ..Default::default()
                    }]
                } else if syllables.is_empty() {
                    vec![]
                } else {
                    vec![Word {
                        syllables,
                        furigana: None,
                    }]
                };

                let content_track = LyricTrack {
                    words,
                    metadata: HashMap::default(),
                };

                let annotated_track = AnnotatedTrack {
                    content_type,
                    content: content_track,
                    translations: vec![],
                    romanizations: vec![],
                };

                let mut new_line = LyricLineBuilder::default()
                    .start_ms(start_ms)
                    .end_ms(calculated_end_ms)
                    .agent(if actor_info.is_background {
                        None
                    } else {
                        actor_info.agent
                    })
                    .song_part(if actor_info.is_background {
                        None
                    } else {
                        actor_info.song_part
                    })
                    .tracks(vec![annotated_track])
                    .itunes_key(None)
                    .build()
                    .unwrap();

                // 对于逐行歌词，使用 dialogue 的结束时间
                if !has_karaoke_tags {
                    let end_ms = parse_ass_time(&caps["End"], subtitle_line_num)?;
                    new_line.end_ms = new_line.end_ms.max(end_ms);
                }

                current_line = Some(new_line);

            // 辅助行: 附加到当前的 LyricLine
            } else if ["ts", "trans", "roma"]
                .iter()
                .any(|&s| style_lower.contains(s))
            {
                if let Some(line) = current_line.as_mut() {
                    if let Some(main_annotated_track) = line.tracks.first_mut() {
                        let (syllables, calculated_end_ms) =
                            parse_karaoke_text(text_content, line.start_ms, subtitle_line_num)?;

                        let is_romanization = style_lower.contains("roma");

                        let words = if syllables.is_empty() && !has_karaoke_tags {
                            vec![Word {
                                syllables: vec![
                                    LyricSyllableBuilder::default()
                                        .text(text_content.to_string())
                                        .start_ms(start_ms)
                                        .end_ms(start_ms)
                                        .build()
                                        .unwrap(),
                                ],

                                ..Default::default()
                            }]
                        } else if syllables.is_empty() {
                            vec![]
                        } else {
                            vec![Word {
                                syllables,
                                furigana: None,
                            }]
                        };

                        let mut metadata = HashMap::new();
                        if let Some(lang) = actor_info.lang_code {
                            metadata.insert(TrackMetadataKey::Language, lang);
                        }

                        let aux_track = LyricTrack { words, metadata };

                        if is_romanization {
                            main_annotated_track.romanizations.push(aux_track);
                        } else {
                            main_annotated_track.translations.push(aux_track);
                        }
                        line.end_ms = line.end_ms.max(calculated_end_ms);

                        // 对于逐行歌词，使用 dialogue 的结束时间
                        if !has_karaoke_tags {
                            let end_ms = parse_ass_time(&caps["End"], subtitle_line_num)?;
                            line.end_ms = line.end_ms.max(end_ms);
                        }
                    } else {
                        warnings.push(format!(
                            "第 {subtitle_line_num} 行: 找到了一个辅助行，但当前主歌词行没有内容轨道可以附加，已忽略。"
                        ));
                    }
                } else {
                    warnings.push(format!(
                        "第 {subtitle_line_num} 行: 找到了一个翻译/音译行，但它前面没有任何主歌词行可以附加，已忽略。"
                    ));
                }
            } else {
                warnings.push(format!(
                    "第 {subtitle_line_num} 行: 样式 '{style}' 不受支持，已被忽略。"
                ));
            }
        } else {
            warnings.push(format!(
                "第 {subtitle_line_num} 行: 格式与预期的 ASS 事件格式不匹配，已跳过。"
            ));
        }
    }

    // 不要忘记保存最后正在处理的行
    if let Some(completed_line) = current_line.take() {
        new_lines_internal.push(completed_line);
    }

    Ok(ParsedSourceData {
        lines: new_lines_internal,
        raw_metadata,
        warnings,
        source_format: LyricFormat::Ass,
        is_line_timed_source: !has_karaoke_tags,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syl(text: &str, start_ms: u64, duration_ms: u64, ends_with_space: bool) -> LyricSyllable {
        LyricSyllable {
            text: text.to_string(),
            start_ms,
            end_ms: start_ms + duration_ms,
            duration_ms: Some(duration_ms),
            ends_with_space,
        }
    }

    #[test]
    fn test_normal_sentence() {
        let text = r"{\k20}你{\k30}好{\k50}世{\k40}界";
        let start_ms = 10000;
        let (syllables, end_ms) = parse_karaoke_text(text, start_ms, 1).unwrap();

        let expected_syllables = vec![
            syl("你", 10000, 200, false),
            syl("好", 10200, 300, false),
            syl("世", 10500, 500, false),
            syl("界", 11000, 400, false),
        ];

        assert_eq!(syllables, expected_syllables);
        assert_eq!(end_ms, 11400);
    }

    #[test]
    fn test_standalone_space_logic() {
        let text = r"{\k20}A{\k25} {\k30}B";
        let start_ms = 5000;
        let (syllables, end_ms) = parse_karaoke_text(text, start_ms, 1).unwrap();

        let expected_syllables = vec![
            syl("A", 5000, 200, true),
            syl("B", 5000 + 200 + 250, 300, false),
        ];

        assert_eq!(syllables, expected_syllables);
        assert_eq!(end_ms, 5750);
    }

    #[test]
    fn test_trailing_space_in_text_logic() {
        let text = r"{\k20}A {\k30}B";
        let start_ms = 5000;
        let (syllables, end_ms) = parse_karaoke_text(text, start_ms, 1).unwrap();

        let expected_syllables = vec![syl("A", 5000, 200, true), syl("B", 5200, 300, false)];

        assert_eq!(syllables, expected_syllables);
        assert_eq!(end_ms, 5500);
    }

    #[test]
    fn test_complex_mixed_spaces() {
        let text = r"{\k10}A {\k15} {\k20}B {\k22}C";
        let start_ms = 1000;
        let (syllables, end_ms) = parse_karaoke_text(text, start_ms, 1).unwrap();

        let expected_syllables = vec![
            syl("A", 1000, 100, true),
            syl("B", 1000 + 100 + 150, 200, true),
            syl("C", 1000 + 100 + 150 + 200, 220, false),
        ];

        assert_eq!(syllables, expected_syllables);
        assert_eq!(end_ms, 1670);
    }

    #[test]
    fn test_leading_text_before_first_k_tag() {
        let text = r"1{\k40}2";
        let start_ms = 2000;
        let (syllables, end_ms) = parse_karaoke_text(text, start_ms, 1).unwrap();

        let expected_syllables = vec![syl("1", 2000, 0, false), syl("2", 2000, 400, false)];

        assert_eq!(syllables, expected_syllables);
        assert_eq!(end_ms, 2400);
    }

    #[test]
    fn test_trailing_k_tag_at_end() {
        let text = r"{\k50}end{\k30}";
        let start_ms = 3000;
        let (syllables, end_ms) = parse_karaoke_text(text, start_ms, 1).unwrap();

        let expected_syllables = vec![syl("end", 3000, 500, false)];

        assert_eq!(syllables, expected_syllables);
        assert_eq!(end_ms, 3000 + 500 + 300);
    }

    #[test]
    fn test_only_k_tags() {
        let text = r"{\k10}{\k20}{\k30}";
        let start_ms = 1000;
        let (syllables, end_ms) = parse_karaoke_text(text, start_ms, 1).unwrap();

        assert!(syllables.is_empty());
        assert_eq!(end_ms, 1000 + 100 + 200 + 300);
    }

    #[test]
    fn test_empty_input_string() {
        let text = r"";
        let start_ms = 500;
        let (syllables, end_ms) = parse_karaoke_text(text, start_ms, 1).unwrap();

        assert!(syllables.is_empty());
        assert_eq!(end_ms, start_ms);
    }

    #[test]
    fn test_no_k_tags_at_all() {
        let text = r"完全没有K标签";
        let start_ms = 500;
        let (syllables, end_ms) = parse_karaoke_text(text, start_ms, 1).unwrap();

        let expected_syllables = vec![syl("完全没有K标签", 500, 0, false)];

        assert_eq!(syllables, expected_syllables);
        assert_eq!(end_ms, start_ms);
    }

    #[test]
    fn test_with_other_ass_tags() {
        let text = r"{\k20}你好{\b1}👋{\k30}世界";
        let start_ms = 1000;
        let (syllables, end_ms) = parse_karaoke_text(text, start_ms, 1).unwrap();

        let expected_syllables = vec![
            syl("你好{\\b1}👋", 1000, 200, false),
            syl("世界", 1200, 300, false),
        ];

        assert_eq!(syllables, expected_syllables);
        assert_eq!(end_ms, 1500);
    }

    #[test]
    fn test_invalid_k_tag_duration_should_error() {
        let text = r"{\k20}A{\kabc}B";
        let start_ms = 1000;
        let result = parse_karaoke_text(text, start_ms, 1);

        assert!(result.is_err(), "应该因无效的K时间报错");
        match result.err().unwrap() {
            ConvertError::InvalidTime(_) => { /* 预期的错误类型 */ }
            _ => panic!("预期InvalidTime错误，但报另一个不同的错误"),
        }
    }

    #[test]
    fn test_zero_duration_k_tags() {
        let text = r"{\k50}A{\k0}B{\k40}C";
        let start_ms = 2000;
        let (syllables, end_ms) = parse_karaoke_text(text, start_ms, 1).unwrap();

        let expected_syllables = vec![
            syl("A", 2000, 500, false),
            syl("B", 2500, 0, false),
            syl("C", 2500, 400, false),
        ];

        assert_eq!(syllables, expected_syllables);
        assert_eq!(end_ms, 2900);
    }

    #[test]
    fn test_leading_and_trailing_standalone_spaces() {
        let text = r" {\k10}A{\k20} ";
        let start_ms = 5000;
        let (syllables, end_ms) = parse_karaoke_text(text, start_ms, 1).unwrap();

        // 预期：
        // 1. 开头的空格因为前面没有音节，其时长(0)被累加，但不会标记任何东西。
        // 2. 音节"A"被创建。
        // 3. 结尾的空格会标记音节"A"为 ends_with_space=true，并累加其时长。
        let expected_syllables = vec![syl("A", 5000, 100, true)];

        assert_eq!(syllables, expected_syllables);
        // 总时长 = 5000(start) + 0(前导空格) + 100(A) + 200(尾随空格) = 5300
        assert_eq!(end_ms, 5300);
    }
}
