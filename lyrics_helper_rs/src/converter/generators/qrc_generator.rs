//! QRC (Lyricify 标准) 歌词格式生成器

use std::fmt::Write;

use lyrics_helper_core::{ContentType, ConvertError, LyricLine, MetadataStore, Word};

/// QRC 生成的主入口函数。
pub fn generate_qrc(
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
) -> Result<String, ConvertError> {
    let mut qrc_output = String::new();

    writeln!(qrc_output, "{}", metadata_store.generate_lrc_header())?;

    for line in lines {
        // 主歌词行
        if let Some(main_track) = line
            .tracks
            .iter()
            .find(|t| t.content_type == ContentType::Main)
            && !main_track.content.words.is_empty()
        {
            writeln!(
                qrc_output,
                "[{},{}]{}",
                line.start_ms,
                line.end_ms.saturating_sub(line.start_ms),
                build_qrc_text_from_words(&main_track.content.words, false)?
            )?;
        }
        // 背景人声行
        if let Some(bg_track) = line
            .tracks
            .iter()
            .find(|t| t.content_type == ContentType::Background)
            && !bg_track.content.words.is_empty()
        {
            let bg_start_ms = bg_track
                .content
                .words
                .iter()
                .flat_map(|w| &w.syllables)
                .map(|s| s.start_ms)
                .min()
                .unwrap_or(line.start_ms);
            let bg_end_ms = bg_track
                .content
                .words
                .iter()
                .flat_map(|w| &w.syllables)
                .map(|s| s.end_ms)
                .max()
                .unwrap_or(line.end_ms);

            writeln!(
                qrc_output,
                "[{},{}]{}",
                bg_start_ms,
                bg_end_ms.saturating_sub(bg_start_ms),
                build_qrc_text_from_words(&bg_track.content.words, true)?
            )?;
        }
    }

    Ok(qrc_output)
}

/// 辅助函数，将词语列表格式化为 QRC 行的文本部分。
fn build_qrc_text_from_words(
    words: &[Word],
    is_background: bool,
) -> Result<String, std::fmt::Error> {
    let mut output = String::new();
    let syllables: Vec<_> = words.iter().flat_map(|w| &w.syllables).collect();
    let total_syllable_count = syllables.len();

    for (i, syl) in syllables.iter().enumerate() {
        let duration_ms = syl.end_ms.saturating_sub(syl.start_ms);
        let is_first = i == 0;
        let is_last = i == total_syllable_count - 1;

        if is_background {
            if is_first && is_last {
                // 是背景行，且只有一个音节，则文本前后都加上括号
                write!(output, "({})( {},{})", syl.text, syl.start_ms, duration_ms)?;
            } else if is_first {
                // 第一个音节，在文本前加上左括号
                write!(output, "({}({},{})", syl.text, syl.start_ms, duration_ms)?;
            } else if is_last {
                // 最后一个音节，在文本后加上右括号
                write!(output, "{})( {},{})", syl.text, syl.start_ms, duration_ms)?;
            } else {
                // 背景行的中间音节，照常输出
                write!(output, "{}({},{})", syl.text, syl.start_ms, duration_ms)?;
            }
        } else {
            // 不是背景行，直接输出
            write!(output, "{}({},{})", syl.text, syl.start_ms, duration_ms)?;
        }

        if syl.ends_with_space && !is_last {
            write!(output, " (0,0)")?;
        }
    }

    Ok(output)
}
