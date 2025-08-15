//! # TTML 生成器 - 自动分词模块
//!
//! 该模块负责处理自动将单个歌词音节（syllable）拆分为更小的词元（token），
//! 并根据权重重新分配时间。

use std::sync::LazyLock;

use super::track::write_single_syllable_span;
use super::utils::format_ttml_time;
use hyphenation::{Hyphenator, Language, Load, Standard};
use lyrics_helper_core::{ConvertError, LyricSyllable, TtmlGenerationOptions};
use quick_xml::{Writer, events::BytesText};
use unicode_segmentation::UnicodeSegmentation;

static ENGLISH_HYPHENATOR: LazyLock<Standard> = LazyLock::new(|| {
    // 从嵌入的资源中加载美式英语词典
    Standard::from_embedded(Language::EnglishUS)
        .expect("Failed to load embedded English hyphenation dictionary.")
});

/// 根据选项写入音节，如果启用了自动分词则先进行分词。
pub(super) fn write_syllable_with_optional_splitting<W: std::io::Write>(
    writer: &mut Writer<W>,
    syl: &LyricSyllable,
    options: &TtmlGenerationOptions,
) -> Result<(), ConvertError> {
    if options.auto_word_splitting && syl.text.trim().chars().count() > 1 {
        let tokens = auto_tokenize(&syl.text);

        let last_visible_token_index = tokens.iter().rposition(|token| {
            get_char_type(token.chars().next().unwrap_or(' ')) != CharType::Whitespace
        });

        let total_weight: f64 = tokens
            .iter()
            .map(|token| {
                let first_char = token.chars().next().unwrap_or(' ');
                match get_char_type(first_char) {
                    CharType::Latin | CharType::Numeric | CharType::Cjk => {
                        let char_count = token.chars().count();
                        let safe_count: u32 = char_count.try_into().unwrap_or(1_000_000);
                        f64::from(safe_count)
                    }
                    CharType::Other => options.punctuation_weight,
                    CharType::Whitespace => 0.0,
                }
            })
            .sum();

        if total_weight > 0.0 {
            let total_duration = syl.end_ms.saturating_sub(syl.start_ms);
            let safe_duration: u32 = total_duration.try_into().unwrap_or(2_000_000_000);
            let duration_per_weight = f64::from(safe_duration) / total_weight;

            let mut current_token_start_ms = syl.start_ms;
            let mut accumulated_weight = 0.0;

            for (token_idx, token) in tokens.iter().enumerate() {
                let first_char = token.chars().next().unwrap_or(' ');
                let char_type = get_char_type(first_char);

                if char_type == CharType::Whitespace {
                    continue;
                }

                let token_weight = match char_type {
                    CharType::Latin | CharType::Numeric | CharType::Cjk => {
                        let char_count = token.chars().count();
                        let safe_count: u32 = char_count.try_into().unwrap_or(1_000_000);
                        f64::from(safe_count)
                    }
                    CharType::Other => options.punctuation_weight,
                    CharType::Whitespace => 0.0,
                };

                accumulated_weight += token_weight;

                let offset_ms = (accumulated_weight * duration_per_weight).round();
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let safe_offset = if (0.0..=1_000_000_000.0).contains(&offset_ms) {
                    offset_ms as u64
                } else if offset_ms > 1_000_000_000.0 {
                    1_000_000_000
                } else {
                    0
                };
                let token_end_ms = if Some(token_idx) == last_visible_token_index {
                    syl.end_ms
                } else {
                    syl.start_ms.saturating_add(safe_offset)
                };

                let text_to_write = if options.format
                    && syl.ends_with_space
                    && Some(token_idx) == last_visible_token_index
                {
                    format!("{token} ")
                } else {
                    token.clone()
                };

                writer
                    .create_element("span")
                    .with_attribute(("begin", format_ttml_time(current_token_start_ms).as_str()))
                    .with_attribute(("end", format_ttml_time(token_end_ms).as_str()))
                    .write_text_content(BytesText::new(&text_to_write))?;

                current_token_start_ms = token_end_ms;
            }
        } else {
            write_single_syllable_span(writer, syl, options)?;
        }
    } else {
        write_single_syllable_span(writer, syl, options)?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum CharType {
    Cjk,
    Latin,
    Numeric,
    Whitespace,
    Other,
}

fn get_char_type(c: char) -> CharType {
    if c.is_whitespace() {
        CharType::Whitespace
    } else if c.is_ascii_alphabetic() {
        CharType::Latin
    } else if c.is_ascii_digit() {
        CharType::Numeric
    } else if (0x4E00..=0x9FFF).contains(&(c as u32))
        || (0x3040..=0x309F).contains(&(c as u32))
        || (0x30A0..=0x30FF).contains(&(c as u32))
        || (0xAC00..=0xD7AF).contains(&(c as u32))
    {
        CharType::Cjk
    } else {
        CharType::Other
    }
}

fn auto_tokenize(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut tokens = Vec::new();
    let mut current_token = String::new();
    let mut last_char_type: Option<CharType> = None;

    for grapheme in text.graphemes(true) {
        let first_char = grapheme.chars().next().unwrap_or(' ');
        let current_char_type = get_char_type(first_char);

        if let Some(last_type) = last_char_type {
            let should_break = !matches!(
                (last_type, current_char_type),
                (CharType::Latin, CharType::Latin)
                    | (CharType::Numeric, CharType::Numeric)
                    | (CharType::Whitespace, CharType::Whitespace)
            );

            if should_break && !current_token.is_empty() {
                // 如果刚刚结束的 token 是一个拉丁词，并且长度大于1，就尝试按音节拆分
                if last_type == CharType::Latin && current_token.chars().count() > 1 {
                    // 拆分为多个部分
                    tokens.extend(
                        ENGLISH_HYPHENATOR
                            .hyphenate(&current_token)
                            .into_iter()
                            .segments()
                            .map(String::from),
                    );
                } else {
                    // 对于非拉丁词（如数字、单个字符）或未拆分的词，直接推入
                    tokens.push(current_token);
                }
                current_token = String::new();
            }
        }
        current_token.push_str(grapheme);
        last_char_type = Some(current_char_type);
    }

    // 处理循环结束后的最后一个 token
    if !current_token.is_empty() {
        if last_char_type == Some(CharType::Latin) && current_token.chars().count() > 1 {
            tokens.extend(
                ENGLISH_HYPHENATOR
                    .hyphenate(&current_token)
                    .into_iter()
                    .segments()
                    .map(String::from),
            );
        } else {
            tokens.push(current_token);
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_tokenize() {
        assert_eq!(auto_tokenize("Hello world"), vec!["Hello", " ", "world"]);
        assert_eq!(auto_tokenize("你好世界"), vec!["你", "好", "世", "界"]);
        assert_eq!(auto_tokenize("Hello你好"), vec!["Hello", "你", "好"]);
        assert_eq!(auto_tokenize("word123"), vec!["word", "123"]);
        assert_eq!(
            auto_tokenize("你好-世界"),
            vec!["你", "好", "-", "世", "界"]
        );
        assert_eq!(auto_tokenize("Hello  world"), vec!["Hello", "  ", "world"]);
        assert_eq!(auto_tokenize(""), Vec::<String>::new());
        assert_eq!(
            auto_tokenize("OK, Let's GO! 走吧123"),
            vec![
                "OK", ",", " ", "Let", "'", "s", " ", "GO", "!", " ", "走", "吧", "123"
            ]
        );
    }

    #[test]
    fn test_auto_tokenize_with_syllables() {
        assert_eq!(
            auto_tokenize("hyphenation"),
            vec!["hy", "phen", "a", "tion"]
        );
        assert_eq!(auto_tokenize("Amazing!"), vec!["Amaz", "ing", "!",]);
        assert_eq!(
            auto_tokenize("wonderful世界"),
            vec!["won", "der", "ful", "世", "界"]
        );
    }
}
