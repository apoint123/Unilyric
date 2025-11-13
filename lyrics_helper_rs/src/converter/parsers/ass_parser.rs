//! ASS 格式解析器

use std::collections::HashMap;

use regex::Regex;
use std::sync::LazyLock;

use lyrics_helper_core::{
    Agent, AgentStore, AgentType, AnnotatedTrack, ContentType, ConvertError, LyricFormat,
    LyricLine, LyricSyllable, LyricSyllableBuilder, LyricTrack, ParsedSourceData, TrackMetadataKey,
    Word,
};

use crate::converter::utils::process_syllable_text;

struct ParserState {
    lines: Vec<LyricLine>,
    warnings: Vec<String>,
    agents: AgentStore,
    raw_metadata: HashMap<String, Vec<String>>,
    has_karaoke_tags: bool,
}

impl ParserState {
    fn new(has_karaoke_tags: bool) -> Self {
        Self {
            lines: Vec::new(),
            warnings: Vec::new(),
            agents: AgentStore::new(),
            raw_metadata: HashMap::new(),
            has_karaoke_tags,
        }
    }
}

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

static AGENT_DEF_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^v(\d+):([^,]+)(?:,(person|group|grp|other|oth))?$")
        .expect("编译 AGENT_DEF_REGEX 失败")
});

/// 存储从 Actor 字段解析出的临时信息。
#[derive(Debug, Default)]
struct ParsedActorInfo {
    agent: Option<String>,
    song_part: Option<String>,
    lang_code: Option<String>,
    is_background: bool,
    is_marker: bool,
    agent_type: AgentType,
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
            current_time_ms += syllable_duration_ms;
        } else if let Some((clean_text, ends_with_space)) =
            process_syllable_text(text_slice, &mut syllables)
        {
            let syllable_end_ms = current_time_ms + syllable_duration_ms;
            let syllable = LyricSyllableBuilder::default()
                .text(clean_text)
                .start_ms(current_time_ms)
                .end_ms(syllable_end_ms)
                .duration_ms(syllable_duration_ms)
                .ends_with_space(ends_with_space)
                .build()
                .unwrap();
            syllables.push(syllable);
            current_time_ms = syllable_end_ms;
        } else {
            current_time_ms += syllable_duration_ms;
        }

        max_end_time_ms = max_end_time_ms.max(current_time_ms);
        previous_duration_cs = current_k_duration_cs;
        current_char_pos = tag_match.end();
    }

    // 处理最后一个 `\k` 标签后的文本
    let remaining_text_slice = &text[current_char_pos..];
    let syllable_duration_ms = u64::from(previous_duration_cs) * 10;

    if let Some((clean_text, _)) = process_syllable_text(remaining_text_slice, &mut syllables) {
        let syllable_end_ms = current_time_ms + syllable_duration_ms;
        let syllable = LyricSyllableBuilder::default()
            .text(clean_text)
            .start_ms(current_time_ms)
            .end_ms(syllable_end_ms)
            .duration_ms(syllable_duration_ms)
            .ends_with_space(false) // 最后一个音节通常不应该有尾随空格
            .build()
            .unwrap();
        syllables.push(syllable);
        current_time_ms = syllable_end_ms;
    } else {
        // 结尾只有空格或无内容，只需将最后一段时长加上
        current_time_ms += syllable_duration_ms;
    }
    max_end_time_ms = max_end_time_ms.max(current_time_ms);

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

    let mut role_tags_found: Vec<(&str, &str, AgentType)> = Vec::new();

    const V1_TAGS: &[&str] = &["左", "v1"];
    const V2_TAGS: &[&str] = &["右", "x-duet", "x-anti", "v2"];
    const CHORUS_TAGS: &[&str] = &["合", "v1000"];

    for tag in actor_str.split_whitespace() {
        if tag.starts_with("x-lang:") {
            let is_aux_style =
                style == "ts" || style == "trans" || style == "roma" || style.contains("bg-");
            if !is_aux_style {
                warnings.push(format!(
                "第 {line_num} 行: 在非辅助行 (样式: '{style}') 上发现了 'x-lang:' 标签，该标签将被忽略。"
            ));
                continue;
            }

            if info.lang_code.is_some() {
                warnings.push(format!(
                    "第 {line_num} 行: 发现多个 'x-lang:' 标签，将使用最后一个。"
                ));
            }
            info.lang_code = Some(tag.trim_start_matches("x-lang:").to_string());
        } else if tag == "x-mark" {
            info.is_marker = true;
        } else if tag == "x-bg" {
            info.is_background = true;
        } else if V1_TAGS.contains(&tag) {
            role_tags_found.push((tag, "v1", AgentType::Person));
        } else if V2_TAGS.contains(&tag) {
            role_tags_found.push((tag, "v2", AgentType::Person));
        } else if CHORUS_TAGS.contains(&tag) {
            role_tags_found.push((tag, "v1000", AgentType::Group));
        } else if let Some(caps) = AGENT_V_TAG_REGEX.captures(tag) {
            let agent_id = caps.get(0).unwrap().as_str();
            role_tags_found.push((tag, agent_id, AgentType::Person));
        }
    }

    let style_lower = style.to_lowercase();

    if style_lower == "orig" || style_lower == "default" {
        if role_tags_found.len() > 1 {
            let conflicting_tags: Vec<String> = role_tags_found
                .iter()
                .map(|(t, _, _)| (*t).to_string())
                .collect();
            warnings.push(format!(
                "第 {line_num} 行: 发现冲突的角色标签 {:?}，将使用第一个 ('{}')。",
                conflicting_tags, role_tags_found[0].0
            ));
        }

        if let Some((_, agent_id, agent_type)) = role_tags_found.first() {
            info.agent = Some((*agent_id).to_string());
            info.agent_type = agent_type.clone();
        } else if (style_lower == "ts" || style_lower == "trans" || style_lower == "roma")
            && info.lang_code.is_none()
        {
            info.agent = Some("v1".to_string());
            info.agent_type = AgentType::Person;
        }
    } else if (style == "ts" || style == "trans" || style == "roma") && info.lang_code.is_none() {
        warnings.push(format!(
            "第 {line_num} 行: 辅助行样式 '{style}' 缺少 'x-lang:' 标签，可能导致语言关联错误。"
        ));
    }

    info
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuxiliaryType {
    Translation,
    Romanization,
}

#[derive(Debug, Default)]
struct ParsedStyleInfo {
    is_background: bool,
    aux_type: Option<AuxiliaryType>,
}

fn parse_style_info(style: &str) -> ParsedStyleInfo {
    let mut info = ParsedStyleInfo::default();

    const TRANSLATION_KEYWORDS: &[&str] = &["trans", "ts"];
    const ROMANIZATION_KEYWORDS: &[&str] = &["roma"];
    const BACKGROUND_KEYWORDS: &[&str] = &["bg"];

    if BACKGROUND_KEYWORDS.iter().any(|&kw| style.contains(kw)) {
        info.is_background = true;
    }

    if TRANSLATION_KEYWORDS.iter().any(|&kw| style.contains(kw)) {
        info.aux_type = Some(AuxiliaryType::Translation);
    } else if ROMANIZATION_KEYWORDS.iter().any(|&kw| style.contains(kw)) {
        info.aux_type = Some(AuxiliaryType::Romanization);
    }

    info
}

fn build_words_for_track(
    syllables: Vec<LyricSyllable>,
    has_karaoke_tags: bool,
    text_content: &str,
    start_ms: u64,
) -> Vec<Word> {
    if syllables.is_empty() && !has_karaoke_tags {
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
    }
}

fn handle_main_lyric_line(
    state: &mut ParserState,
    has_karaoke_tags: bool,
    caps: &regex::Captures,
    actor_info: ParsedActorInfo,
    subtitle_line_num: usize,
) -> Result<(), ConvertError> {
    let start_ms = parse_ass_time(&caps["Start"], subtitle_line_num)?;
    let end_ms = parse_ass_time(&caps["End"], subtitle_line_num)?;
    let text_content = &caps["Text"];

    let content_type = if actor_info.is_background {
        ContentType::Background
    } else {
        ContentType::Main
    };

    let (annotated_track, calculated_end_ms) = if has_karaoke_tags {
        let (syllables, calculated_end_ms) =
            parse_karaoke_text(text_content, start_ms, subtitle_line_num)?;
        let words = build_words_for_track(syllables, true, text_content, start_ms);
        let annotated_track = AnnotatedTrack {
            content_type,
            content: LyricTrack {
                words,
                ..Default::default()
            },
            ..Default::default()
        };
        (annotated_track, end_ms.max(calculated_end_ms))
    } else {
        let words = build_words_for_track(Vec::new(), false, text_content, start_ms);
        let annotated_track = AnnotatedTrack {
            content_type,
            content: LyricTrack {
                words,
                ..Default::default()
            },
            ..Default::default()
        };
        (annotated_track, end_ms)
    };

    if actor_info.is_background {
        let last_main_line = state.lines.iter_mut().rev().find(|line| {
            line.tracks
                .iter()
                .any(|t| t.content_type == ContentType::Main)
        });

        if let Some(line) = last_main_line {
            line.add_track(annotated_track);
            line.end_ms = line.end_ms.max(calculated_end_ms);
        } else {
            let mut new_line = LyricLine::new(start_ms, calculated_end_ms);
            new_line.agent = actor_info.agent.filter(|_| !actor_info.is_background);
            new_line.song_part = actor_info.song_part.filter(|_| !actor_info.is_background);
            new_line.add_track(annotated_track);
            state.lines.push(new_line);
            state.warnings.push(format!(
                "第 {subtitle_line_num} 行: 背景人声行未找到可附加的主歌词行"
            ));
        }
    } else {
        let mut new_line = LyricLine::new(start_ms, calculated_end_ms);
        new_line.agent = actor_info.agent.filter(|_| !actor_info.is_background);
        new_line.song_part = actor_info.song_part.filter(|_| !actor_info.is_background);
        new_line.add_track(annotated_track);
        state.lines.push(new_line);
    }

    Ok(())
}

// 处理翻译、音译等辅助行
fn handle_aux_lyric_line(
    new_lines: &mut [LyricLine],
    has_karaoke_tags: bool,
    warnings: &mut Vec<String>,
    caps: &regex::Captures,
    actor_info: ParsedActorInfo,
    parsed_style: &ParsedStyleInfo,
    subtitle_line_num: usize,
) -> Result<(), ConvertError> {
    let aux_start_ms = parse_ass_time(&caps["Start"], subtitle_line_num)?;
    let text_content = &caps["Text"];

    let mut target_line = new_lines
        .iter_mut()
        .rev()
        .find(|line| line.start_ms == aux_start_ms);

    if target_line.is_none() {
        target_line = new_lines.last_mut();
    }

    if let Some(line) = target_line {
        let target_content_type = if parsed_style.is_background {
            ContentType::Background
        } else {
            ContentType::Main
        };

        let aux_type = parsed_style.aux_type.ok_or_else(|| {
            ConvertError::InvalidLyricFormat(format!("第 {subtitle_line_num} 行: 辅助行类型未知。"))
        })?;

        if has_karaoke_tags {
            let (syllables, calculated_end_ms) =
                parse_karaoke_text(text_content, line.start_ms, subtitle_line_num)?;
            let words = build_words_for_track(syllables, true, text_content, 0);
            let mut metadata = HashMap::new();
            if let Some(lang) = actor_info.lang_code {
                metadata.insert(TrackMetadataKey::Language, lang);
            }
            let aux_track = LyricTrack { words, metadata };

            if let Some(track_to_modify) = line
                .tracks
                .iter_mut()
                .find(|t| t.content_type == target_content_type)
            {
                match aux_type {
                    AuxiliaryType::Romanization => track_to_modify.romanizations.push(aux_track),
                    AuxiliaryType::Translation => track_to_modify.translations.push(aux_track),
                }
                line.end_ms = line.end_ms.max(calculated_end_ms);
            } else {
                warnings.push(format!(
                    "第 {subtitle_line_num} 行: 无法为样式找到匹配的 {target_content_type:?} 轨道进行附加，已忽略。"
                ));
            }
        } else {
            // 逐行歌词模式
            match aux_type {
                AuxiliaryType::Romanization => {
                    line.add_romanization(
                        target_content_type,
                        text_content,
                        actor_info.lang_code.as_deref(),
                    );
                }
                AuxiliaryType::Translation => {
                    line.add_translation(
                        target_content_type,
                        text_content,
                        actor_info.lang_code.as_deref(),
                    );
                }
            }
            let end_ms = parse_ass_time(&caps["End"], subtitle_line_num)?;
            line.end_ms = line.end_ms.max(end_ms);
        }
    } else {
        warnings.push(format!(
            "第 {subtitle_line_num} 行: 找到了一个辅助行，但它前面没有任何主歌词行可以附加，已忽略。"
        ));
    }
    Ok(())
}

fn process_dialogue_line(
    state: &mut ParserState,
    caps: &regex::Captures,
    subtitle_line_num: usize,
) -> Result<(), ConvertError> {
    let effect_raw = &caps["Effect"];
    if !effect_raw.is_empty() && !effect_raw.eq_ignore_ascii_case("karaoke") {
        return Ok(());
    }

    let style = &caps["Style"];
    let actor_raw = &caps["Actor"];

    let actor_info = parse_actor(actor_raw, style, subtitle_line_num, &mut state.warnings);

    if let Some(agent_id) = &actor_info.agent {
        state
            .agents
            .agents_by_id
            .entry(agent_id.clone())
            .or_insert_with(|| Agent {
                id: agent_id.clone(),
                name: None,
                agent_type: actor_info.agent_type.clone(),
            });
    }

    let style_lower = style.to_lowercase();
    if style_lower == "orig" || style_lower == "default" {
        handle_main_lyric_line(
            state,
            state.has_karaoke_tags,
            caps,
            actor_info,
            subtitle_line_num,
        )?;
    } else {
        let parsed_style = parse_style_info(&style_lower);
        if parsed_style.aux_type.is_some() {
            handle_aux_lyric_line(
                &mut state.lines,
                state.has_karaoke_tags,
                &mut state.warnings,
                caps,
                actor_info,
                &parsed_style,
                subtitle_line_num,
            )?;
        } else {
            state.warnings.push(format!(
                "第 {subtitle_line_num} 行: 样式 '{style}' 不受支持，已被忽略。"
            ));
        }
    }
    Ok(())
}

/// 解析ASS格式内容到 `ParsedSourceData` 结构。
pub fn parse_ass(content: &str) -> Result<ParsedSourceData, ConvertError> {
    let has_karaoke_tags = content.contains(r"{\k");
    let mut state = ParserState::new(has_karaoke_tags);
    let mut in_events_section = false;

    for (i, line_str_raw) in content.lines().enumerate() {
        let subtitle_line_num = i + 1;
        let line_str = line_str_raw.trim();

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
                if let Some(agent_caps) = AGENT_DEF_REGEX.captures(text_content) {
                    let agent_id = format!("v{}", &agent_caps[1]);
                    let agent_name = agent_caps[2].trim().to_string();
                    let agent_type = match agent_caps.get(3).map(|m| m.as_str()) {
                        Some("group" | "grp") => AgentType::Group,
                        Some("other" | "oth") => AgentType::Other,
                        _ => AgentType::Person,
                    };

                    state
                        .agents
                        .agents_by_id
                        .entry(agent_id.clone())
                        .and_modify(|agent| {
                            agent.name = Some(agent_name.clone());
                            agent.agent_type = agent_type.clone();
                        })
                        .or_insert_with(|| Agent {
                            id: agent_id,
                            name: Some(agent_name),
                            agent_type,
                        });
                } else if let Some((key, value)) = text_content.split_once(':') {
                    state
                        .raw_metadata
                        .entry(key.trim().to_string())
                        .or_default()
                        .push(value.trim().to_string());
                }
                continue;
            }

            if line_type == "Dialogue"
                && let Err(e) = process_dialogue_line(&mut state, &caps, subtitle_line_num)
            {
                state
                    .warnings
                    .push(format!("第 {subtitle_line_num} 行处理失败: {e}"));
            }
        } else if in_events_section {
            state.warnings.push(format!(
                "第 {subtitle_line_num} 行: 格式与预期的 ASS 事件格式不匹配，已跳过。"
            ));
        }
    }

    for (key, values) in &state.raw_metadata {
        if (AGENT_V_TAG_REGEX.is_match(key) || key == "v1000")
            && let Some(name) = values.first()
        {
            state
                .agents
                .agents_by_id
                .entry(key.clone())
                .and_modify(|agent| agent.name = Some(name.clone()))
                .or_insert_with(|| Agent {
                    id: key.clone(),
                    name: Some(name.clone()),
                    agent_type: if key == "v1000" {
                        AgentType::Group
                    } else {
                        AgentType::Person
                    },
                });
        }
    }

    Ok(ParsedSourceData {
        lines: state.lines,
        raw_metadata: state.raw_metadata,
        warnings: state.warnings,
        source_format: LyricFormat::Ass,
        is_line_timed_source: !state.has_karaoke_tags,
        agents: state.agents,
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
