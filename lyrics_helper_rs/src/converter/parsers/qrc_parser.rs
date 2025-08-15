//! # QRC 格式解析器
//!
//! 可以解析 Lyricify 标准的背景人声行，和 kana 标签中的振假名

use crate::converter::utils::{parse_and_store_metadata, process_syllable_text};
use lyrics_helper_core::{
    AnnotatedTrack, ContentType, ConvertError, FuriganaSyllable, LyricFormat, LyricLine,
    LyricLineBuilder, LyricSyllable, LyricSyllableBuilder, LyricTrack, ParsedSourceData, Word,
};
use regex::Regex;
use std::{collections::HashMap, sync::LazyLock};

static KANA_TAG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[kana:(?P<kana_stream>.*?)]").expect("编译 KANA_TAG_REGEX 失败")
});

static LYRIC_TOKEN_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<text>.+?)\((?P<start>\d+),(?P<duration>\d+)\)")
        .expect("编译 LYRIC_TOKEN_REGEX 失败")
});

static KANA_UNIT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<count>\d+)(?P<kana>[\p{Hiragana}\p{Katakana}ー]+)")
        .expect("编译 KANA_UNIT_REGEX 失败")
});

static HAS_KANJI_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\p{Han}").expect("编译 HAS_KANJI_REGEX 失败"));

static QRC_LINE_TIMESTAMP_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[\d+,\d+]").expect("编译 QRC_LINE_TIMESTAMP_REGEX 失败"));

#[derive(Debug)]
struct LyricToken {
    text: String,
    start_ms: u64,
    end_ms: u64,
}

#[derive(Debug)]
struct KanaToken {
    char_count: usize,
    syllables: Option<Vec<FuriganaSyllable>>,
}

struct MatchedWord {
    word: Word,
    line_index: usize,
}

/// 解析 QRC 格式内容到 `ParsedSourceData` 结构。
///
/// # Panics
///
/// 如果内部的 `KANA_TAG_REGEX` 被错误地修改，会触发 panic。
pub fn parse_qrc(content: &str) -> Result<ParsedSourceData, ConvertError> {
    let mut raw_metadata: HashMap<String, Vec<String>> = HashMap::new();
    let mut lyric_lines_str: Vec<&str> = Vec::new();

    for line_str in content.lines() {
        let trimmed_line = line_str.trim();
        if trimmed_line.is_empty() {
            continue;
        }
        if trimmed_line.starts_with("[kana:") {
            lyric_lines_str.push(trimmed_line);
            continue;
        }
        if !parse_and_store_metadata(trimmed_line, &mut raw_metadata) {
            lyric_lines_str.push(trimmed_line);
        }
    }

    let lyric_content = lyric_lines_str.join("\n");

    if let Some(kana_caps) = KANA_TAG_REGEX.captures(&lyric_content) {
        let kana_stream = kana_caps
            .name("kana_stream")
            .expect("`kana_stream` 捕获组在正则匹配成功时必然存在")
            .as_str();
        let (matched_words, warnings) = parse_furigana_qrc(&lyric_content, kana_stream)?;

        let lines = group_words_into_lines(matched_words);
        Ok(ParsedSourceData {
            lines,
            raw_metadata,
            warnings,
            source_format: LyricFormat::Qrc,
            ..Default::default()
        })
    } else {
        Ok(parse_standard_qrc(&lyric_content, raw_metadata))
    }
}

/// 解析包含 `[kana:...]` 标签的QRC内容。
fn parse_furigana_qrc(
    full_lyric_content: &str,
    kana_stream: &str,
) -> Result<(Vec<MatchedWord>, Vec<String>), ConvertError> {
    let kana_tokens = tokenize_kana(kana_stream)?;

    let mut lyric_tokens: Vec<(LyricToken, usize)> = Vec::new();
    let main_lyric_stream = KANA_TAG_REGEX.replace_all(full_lyric_content, "");

    for (line_index, line) in main_lyric_stream.lines().enumerate() {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            continue;
        }
        let line_content = QRC_LINE_TIMESTAMP_REGEX.replace(trimmed_line, "");

        for token in tokenize_lyrics(&line_content)? {
            lyric_tokens.push((token, line_index));
        }
    }

    let mut matched_words: Vec<MatchedWord> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut lyric_idx = 0;

    for kana_token in &kana_tokens {
        let mut found_match_for_current_kana = false;

        while lyric_idx < lyric_tokens.len() {
            let (current_lyric, line_idx) = &lyric_tokens[lyric_idx];
            let clean_text = current_lyric.text.trim();

            let is_match = !clean_text.is_empty()
                && HAS_KANJI_REGEX.is_match(clean_text)
                && clean_text.chars().count() == kana_token.char_count;

            if is_match {
                // 匹配成功
                process_lyric_token(
                    current_lyric,
                    *line_idx,
                    kana_token.syllables.clone(),
                    &mut matched_words,
                );
                lyric_idx += 1;
                found_match_for_current_kana = true;
                break;
            }
            // 不匹配
            process_lyric_token(current_lyric, *line_idx, None, &mut matched_words);
            lyric_idx += 1;
        }

        if !found_match_for_current_kana {
            let orphan_kana_text: String = kana_token
                .syllables
                .as_ref()
                .unwrap_or(&vec![])
                .iter()
                .map(|s| s.text.clone())
                .collect();

            warnings.push(format!(
                "注音 '{}' (对应 {} 个汉字) 未能在歌词中找到匹配项，已忽略。",
                orphan_kana_text, kana_token.char_count
            ));
        }
    }

    // 处理所有注音结束后，主歌词流中可能还有剩余的词元
    while lyric_idx < lyric_tokens.len() {
        let (current_lyric, line_idx) = &lyric_tokens[lyric_idx];
        process_lyric_token(current_lyric, *line_idx, None, &mut matched_words);
        lyric_idx += 1;
    }

    Ok((matched_words, warnings))
}

fn process_lyric_token(
    lyric_token: &LyricToken,
    line_idx: usize,
    furigana: Option<Vec<FuriganaSyllable>>,
    matched_words: &mut Vec<MatchedWord>,
) {
    let raw_text = &lyric_token.text;
    let has_leading_space = raw_text.starts_with(char::is_whitespace);
    let has_trailing_space = raw_text.ends_with(char::is_whitespace);
    let clean_text = raw_text.trim();

    if has_leading_space
        && let Some(last_word) = matched_words.last_mut()
        && let Some(last_syllable) = last_word.word.syllables.last_mut()
    {
        last_syllable.ends_with_space = true;
    }

    if clean_text.is_empty() {
        return;
    }

    matched_words.push(MatchedWord {
        word: Word {
            syllables: vec![
                LyricSyllableBuilder::default()
                    .text(clean_text)
                    .start_ms(lyric_token.start_ms)
                    .end_ms(lyric_token.end_ms)
                    .ends_with_space(has_trailing_space)
                    .build()
                    .unwrap(),
            ],
            furigana,
        },
        line_index: line_idx,
    });
}

/// 将一个扁平化的 `MatchedWord` 列表按行号分组为 `LyricLine` 向量。
fn group_words_into_lines(matched_words: Vec<MatchedWord>) -> Vec<LyricLine> {
    let mut lines: Vec<LyricLine> = Vec::new();
    if matched_words.is_empty() {
        return lines;
    }

    let mut line_words_map: std::collections::BTreeMap<usize, Vec<Word>> =
        std::collections::BTreeMap::new();
    for matched in matched_words {
        line_words_map
            .entry(matched.line_index)
            .or_default()
            .push(matched.word);
    }

    for (_, line_words) in line_words_map {
        let content_track = LyricTrack {
            words: line_words,
            ..Default::default()
        };
        let line = LyricLineBuilder::default()
            .start_ms(content_track.words.first().unwrap().syllables[0].start_ms)
            .end_ms(content_track.words.last().unwrap().syllables[0].end_ms)
            .track(AnnotatedTrack {
                content: content_track,
                ..Default::default()
            })
            .build()
            .unwrap();
        lines.push(line);
    }
    lines
}

/// 将主歌词流字符串解析为 `LyricToken` 向量。
fn tokenize_lyrics(single_line_content: &str) -> Result<Vec<LyricToken>, ConvertError> {
    let mut tokens = Vec::new();
    for caps in LYRIC_TOKEN_REGEX.captures_iter(single_line_content) {
        let start_ms: u64 = caps["start"].parse()?;
        let duration_ms: u64 = caps["duration"].parse()?;
        tokens.push(LyricToken {
            text: caps["text"].to_string(),
            start_ms,
            end_ms: start_ms + duration_ms,
        });
    }
    Ok(tokens)
}

/// 将 `[kana:...]` 流字符串解析为 `KanaToken` 向量。
fn tokenize_kana(kana_stream: &str) -> Result<Vec<KanaToken>, ConvertError> {
    let mut tokens = Vec::new();

    // 找到所有 "数字+假名" 组合的位置
    let matches: Vec<_> = KANA_UNIT_REGEX.find_iter(kana_stream).collect();

    for i in 0..matches.len() {
        let current_match = &matches[i];
        let caps = KANA_UNIT_REGEX.captures(current_match.as_str()).unwrap();

        let char_count: usize = caps["count"].parse()?;

        // 确定这个注音单元的完整文本范围（包括可能的时间戳）
        let content_start = current_match.start();
        let content_end = if i + 1 < matches.len() {
            matches[i + 1].start() // 到下一个匹配的开始位置
        } else {
            kana_stream.len() // 或到字符串末尾
        };

        let full_unit_text = &kana_stream[content_start..content_end];

        // 分离注音和时间戳
        let kana_text_with_timing = &full_unit_text[caps["count"].len()..];

        let mut syllables = Vec::new();
        if kana_text_with_timing.contains('(') {
            for inner_caps in LYRIC_TOKEN_REGEX.captures_iter(kana_text_with_timing) {
                let text = inner_caps["text"].to_string();
                let start_ms: u64 = inner_caps["start"].parse()?;
                let duration_ms: u64 = inner_caps["duration"].parse()?;
                syllables.push(FuriganaSyllable {
                    text,
                    timing: Some((start_ms, start_ms + duration_ms)),
                });
            }
        } else {
            // 如果没有时间戳，整个文本就是单个音节的文本
            let text = LYRIC_TOKEN_REGEX
                .replace_all(kana_text_with_timing.trim(), "${text}")
                .to_string();
            syllables.push(FuriganaSyllable { text, timing: None });
        }

        tokens.push(KanaToken {
            char_count,
            syllables: Some(syllables),
        });
    }

    Ok(tokens)
}

fn line_to_string(line: &LyricLine) -> String {
    line.tracks.first().map_or_else(
        || "<空行>".to_string(),
        |track| {
            track
                .content
                .words
                .iter()
                .flat_map(|w| &w.syllables)
                .map(|s| s.text.clone())
                .collect::<String>()
        },
    )
}

/// 解析单行QRC歌词，但不处理背景人声逻辑，仅返回原始行数据和是否像背景人声的标志。
fn parse_single_qrc_line(line_str: &str) -> Option<(LyricLine, bool)> {
    let trimmed_line = line_str.trim();
    if trimmed_line.is_empty() {
        return None;
    }

    let line_content = QRC_LINE_TIMESTAMP_REGEX.replace(trimmed_line, "");
    let mut syllables: Vec<LyricSyllable> = Vec::new();

    for captures in LYRIC_TOKEN_REGEX.captures_iter(&line_content) {
        let raw_text = &captures["text"];
        if let Some((clean_text, ends_with_space)) = process_syllable_text(raw_text, &mut syllables)
        {
            let start_ms: u64 = captures["start"].parse().ok()?;
            let duration_ms: u64 = captures["duration"].parse().ok()?;
            let syllable = LyricSyllableBuilder::default()
                .text(clean_text)
                .start_ms(start_ms)
                .end_ms(start_ms + duration_ms)
                .ends_with_space(ends_with_space)
                .duration_ms(duration_ms)
                .build()
                .unwrap();
            syllables.push(syllable);
        }
    }

    if syllables.is_empty() {
        return None;
    }

    let start_ms = syllables.first().unwrap().start_ms;
    let end_ms = syllables.last().unwrap().end_ms;
    let full_line_text: String = syllables.iter().map(|s| s.text.clone()).collect();
    let is_candidate = (full_line_text.starts_with('(') || full_line_text.starts_with('（'))
        && (full_line_text.ends_with(')') || full_line_text.ends_with('）'));

    let words = vec![Word {
        syllables,
        ..Default::default()
    }];
    let line = LyricLineBuilder::default()
        .start_ms(start_ms)
        .end_ms(end_ms)
        .track(AnnotatedTrack {
            content: LyricTrack {
                words,
                ..Default::default()
            },
            ..Default::default()
        })
        .build()
        .unwrap();

    Some((line, is_candidate))
}

/// 解析不含 `[kana:...]` 标签的标准QRC或罗马音QRC内容。
fn parse_standard_qrc(
    lyric_content: &str,
    raw_metadata: HashMap<String, Vec<String>>,
) -> ParsedSourceData {
    let mut warnings: Vec<String> = Vec::new();
    let mut final_lines: Vec<LyricLine> = Vec::new();
    let mut pending_bg_line: Option<LyricLine> = None;
    let mut last_pushed_was_candidate = false;

    let parsed_lines_iter = lyric_content.lines().filter_map(parse_single_qrc_line);

    for (current_line, is_candidate) in parsed_lines_iter {
        if is_candidate {
            if let Some(prev_bg_line) = pending_bg_line.take() {
                warnings.push(format!(
                    "行 '{}' 与另一背景人声行相邻，当作主歌词处理。",
                    line_to_string(&prev_bg_line)
                ));
                final_lines.push(prev_bg_line);
                last_pushed_was_candidate = true;
            }
            pending_bg_line = Some(current_line);
        } else {
            if let Some(mut bg_line) = pending_bg_line.take() {
                if let Some(last_line) = final_lines.last_mut() {
                    if let Some(track) = bg_line.tracks.first_mut() {
                        track.content_type = ContentType::Background;
                    }

                    for word in &mut bg_line.tracks[0].content.words {
                        for syl in &mut word.syllables {
                            syl.text = syl.text.trim_matches(['(', '（', ')', '）']).to_string();
                        }
                    }
                    last_line.tracks.push(bg_line.tracks.remove(0));
                } else {
                    warnings.push(format!(
                        "背景人声行 '{}' 无法关联到上一行，当作主歌词处理。",
                        line_to_string(&bg_line)
                    ));
                    final_lines.push(bg_line);
                }
            }
            final_lines.push(current_line);
            last_pushed_was_candidate = false;
        }
    }

    if let Some(mut bg_line) = pending_bg_line.take() {
        if !last_pushed_was_candidate && let Some(last_line) = final_lines.last_mut() {
            if let Some(track) = bg_line.tracks.first_mut() {
                track.content_type = ContentType::Background;

                for word in &mut track.content.words {
                    for syl in &mut word.syllables {
                        syl.text = syl.text.trim_matches(['(', '（', ')', '）']).to_string();
                    }
                }
            }
            last_line.tracks.push(bg_line.tracks.remove(0));
        } else {
            warnings.push(format!(
                "行 '{}' 与另一背景人声行相邻（或无法合并），当作主歌词处理。",
                line_to_string(&bg_line)
            ));
            final_lines.push(bg_line);
        }
    }

    final_lines.sort_by_key(|line| line.start_ms);

    ParsedSourceData {
        lines: final_lines,
        raw_metadata,
        warnings,
        source_format: LyricFormat::Qrc,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_furigana_qrc() {
        let content = include_str!("../../../tests/test_data/main.qrc");

        let result = parse_qrc(content).unwrap();

        assert_eq!(result.lines.len(), 106);

        let total_words = result
            .lines
            .iter()
            .flat_map(|l| &l.tracks)
            .flat_map(|t| &t.content.words)
            .count();
        let furigana_words = result
            .lines
            .iter()
            .flat_map(|l| &l.tracks)
            .flat_map(|t| &t.content.words)
            .filter(|w| w.furigana.is_some())
            .count();

        assert_eq!(total_words, 815);
        assert_eq!(furigana_words, 119);

        let target_line = result
            .lines
            .iter()
            .find(|l| l.start_ms == 41699)
            .expect("必须找到 start_ms 为 41699 的目标行");

        assert_eq!(target_line.end_ms, 42681);
        let words = &target_line.tracks[0].content.words;
        assert_eq!(words.len(), 5, "目标行应有5个词元");

        // 检查每个词元和它的注音情况
        assert_eq!(words[0].syllables[0].text, "納");
        assert!(words[0].furigana.is_some(), "“納”应有注音");
        assert_eq!(words[0].furigana.as_ref().unwrap()[0].text, "のう");

        assert_eq!(words[1].syllables[0].text, "期");
        assert!(words[1].furigana.is_some(), "“期”应有注音");
        assert_eq!(words[1].furigana.as_ref().unwrap()[0].text, "き");

        assert_eq!(words[2].syllables[0].text, "は");
        assert!(words[2].furigana.is_none(), "“は”不应有注音");

        assert_eq!(words[3].syllables[0].text, "明日");
        assert!(words[3].furigana.is_some(), "“明日”应有注音");
        assert_eq!(words[3].furigana.as_ref().unwrap()[0].text, "あ");

        assert_eq!(words[4].syllables[0].text, "だ");
        assert!(words[4].furigana.is_none(), "“だ”不应有注音");
    }

    #[test]
    fn test_standard_qrc_background_vocals() {
        let content = r"
[97648,4632]The (97648,384)scars (98032,565)of (98597,552)your (99149,581)love(99730,302)
[96826,3715](You're (96826,333)gonna (97159,299)wish (97458,435)you)(100143,398)
[102285,4362]They (102285,315)keep (102600,568)me (103168,568)thinking(103736,565)
[107000,1000]Consecutive(107000,1000)
[108000,1000](BG1)(108000,1000)
[109000,1000](BG2)(109000,1000)
    ";
        let result = parse_qrc(content).unwrap();

        assert_eq!(result.lines.len(), 5, "应有5行歌词");

        let line1 = &result.lines[0];
        assert_eq!(line1.start_ms, 97648, "第一行时间戳应为主歌词的");
        assert_eq!(line1.tracks.len(), 2, "第一行应有主歌词和背景歌词两个轨道");

        let main_track1 = line1
            .tracks
            .iter()
            .find(|t| t.content_type == ContentType::Main)
            .expect("第一行应有主轨道");
        let main_text1: String = main_track1.content.words[0]
            .syllables
            .iter()
            .map(|s| s.text.clone())
            .collect();
        assert!(main_text1.starts_with("The"), "主轨道内容应为 'The...'");

        let bg_track1 = line1
            .tracks
            .iter()
            .find(|t| t.content_type == ContentType::Background)
            .expect("第一行应有背景轨道");
        let bg_text1: String = bg_track1.content.words[0]
            .syllables
            .iter()
            .map(|s| s.text.clone())
            .collect();
        assert!(
            bg_text1.starts_with("You're"),
            "背景轨道内容应为 'You're...'"
        );
        assert!(!bg_text1.starts_with('('), "背景歌词的括号应被移除");

        let line2 = &result.lines[1];
        assert_eq!(line2.start_ms, 102_285);
        assert_eq!(line2.tracks.len(), 1, "第二行应只有1个轨道");

        let line4 = &result.lines[3];
        assert_eq!(line4.start_ms, 108_000);
        assert_eq!(line4.tracks.len(), 1, "第四行应只有1个轨道");
        assert_eq!(
            line4.tracks[0].content_type,
            ContentType::Main,
            "第四行应被视为主轨道"
        );
        let text4: String = line4.tracks[0].content.words[0]
            .syllables
            .iter()
            .map(|s| s.text.clone())
            .collect();
        assert_eq!(text4, "(BG1)", "第四行作为普通行，内容应保留括号");

        let line5 = &result.lines[4];
        assert_eq!(line5.start_ms, 109_000);
        assert_eq!(line5.tracks.len(), 1, "第五行应只有1个轨道");
        assert_eq!(
            line5.tracks[0].content_type,
            ContentType::Main,
            "第五行应被视为主轨道"
        );
        let text5: String = line5.tracks[0].content.words[0]
            .syllables
            .iter()
            .map(|s| s.text.clone())
            .collect();
        assert_eq!(text5, "(BG2)", "第五行作为普通行，内容应保留括号");
    }
}
