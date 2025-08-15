//! LYS 歌词格式生成器

use std::fmt::Write;

use lyrics_helper_core::{
    ContentType, ConvertError, LyricLine, MetadataStore, Word, lys_properties,
};

/// LYS 生成的主入口函数。
pub fn generate_lys(
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
) -> Result<String, ConvertError> {
    let mut lys_output = String::with_capacity(lines.len() * 120);

    writeln!(lys_output, "{}", metadata_store.generate_lrc_header())?;

    for line in lines {
        // 主歌词行
        if let Some(main_track) = line
            .tracks
            .iter()
            .find(|t| t.content_type == ContentType::Main)
            && !main_track.content.words.is_empty()
        {
            let property = match line.agent.as_deref() {
                Some("v2") => lys_properties::MAIN_RIGHT,
                _ => lys_properties::MAIN_LEFT,
            };
            write!(lys_output, "[{property}]")?;
            write_words_to_lys_string(&mut lys_output, &main_track.content.words, false)?;
            writeln!(lys_output)?;
        }

        // 背景人声行
        if let Some(bg_track) = line
            .tracks
            .iter()
            .find(|t| t.content_type == ContentType::Background)
            && !bg_track.content.words.is_empty()
        {
            let bg_property = match line.agent.as_deref() {
                // agent "v2" -> 右。
                Some("v2") => lys_properties::BG_RIGHT,
                // 其他情况 -> 左。
                _ => lys_properties::BG_LEFT,
            };
            write!(lys_output, "[{bg_property}]")?;
            write_words_to_lys_string(&mut lys_output, &bg_track.content.words, true)?;
            writeln!(lys_output)?;
        }
    }

    Ok(lys_output)
}

fn write_words_to_lys_string(
    output: &mut String,
    words: &[Word],
    is_background: bool,
) -> Result<(), std::fmt::Error> {
    let syllables: Vec<_> = words.iter().flat_map(|w| &w.syllables).collect();

    for (i, syl) in syllables.iter().enumerate() {
        let duration_ms = syl.end_ms.saturating_sub(syl.start_ms);
        let mut text_to_write = syl.text.as_str();
        let temp_buffer;

        if is_background {
            let is_first = i == 0;
            let is_last = i == syllables.len() - 1;
            if is_first && is_last {
                temp_buffer = format!("({text_to_write})");
                text_to_write = &temp_buffer;
            } else if is_first {
                temp_buffer = format!("({text_to_write}");
                text_to_write = &temp_buffer;
            } else if is_last {
                temp_buffer = format!("{text_to_write})");
                text_to_write = &temp_buffer;
            }
        }

        write!(
            output,
            "{}({},{})",
            text_to_write, syl.start_ms, duration_ms
        )?;

        if syl.ends_with_space && i < syllables.len() - 1 {
            write!(output, " (0,0)")?;
        }
    }

    Ok(())
}
