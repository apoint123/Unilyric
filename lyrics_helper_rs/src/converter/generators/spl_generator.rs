//! SPL (Salt Player Lyric) 歌词格式生成器

use std::fmt::Write;

use lyrics_helper_core::{ConvertError, LyricLine, LyricSyllable, MetadataStore};

/// 将毫秒时间格式化为 SPL 时间戳字符串 `[分:秒.厘秒]` 或 `<分:秒.厘秒>`。
///
/// # 参数
/// * `ms` - 时间的毫秒数。
/// * `use_angle_brackets` - 布尔值，如果为 `true`，使用尖括号 `<>`；否则使用方括号 `[]`。
///
/// # 返回
/// 返回格式化后的时间戳字符串。
fn format_spl_timestamp(ms: u64, use_angle_brackets: bool) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    let centiseconds = (ms % 1000) / 10;

    let open = if use_angle_brackets { '<' } else { '[' };
    let close = if use_angle_brackets { '>' } else { ']' };

    format!("{open}{minutes:02}:{seconds:02}.{centiseconds:02}{close}")
}

/// SPL 生成的主入口函数。
pub fn generate_spl(
    lines: &[LyricLine],
    _metadata_store: &MetadataStore,
) -> Result<String, ConvertError> {
    let mut spl_output = String::new();

    for (i, line) in lines.iter().enumerate() {
        write!(spl_output, "{}", format_spl_timestamp(line.start_ms, false))?;

        let main_syllables: Vec<&LyricSyllable> = line
            .main_track()
            .map(|track| {
                track
                    .content
                    .words
                    .iter()
                    .flat_map(|word| &word.syllables)
                    .collect()
            })
            .unwrap_or_default();
        let is_word_timed = !main_syllables.is_empty();

        if is_word_timed {
            let mut last_ts = line.start_ms;
            for syl in &main_syllables {
                if syl.start_ms > last_ts {
                    write!(spl_output, "{}", format_spl_timestamp(syl.start_ms, true))?;
                }
                write!(spl_output, "{}", syl.text)?;
                write!(spl_output, "{}", format_spl_timestamp(syl.end_ms, true))?;
                last_ts = syl.end_ms;
            }
            if line.end_ms > last_ts {
                write!(spl_output, "{}", format_spl_timestamp(line.end_ms, false))?;
            }
        } else {
            if let Some(text) = line.main_text() {
                write!(spl_output, "{text}")?;
            }

            let needs_explicit_end_tag = lines
                .get(i + 1)
                .is_none_or(|next_line| line.end_ms != next_line.start_ms);

            if needs_explicit_end_tag {
                write!(spl_output, "{}", format_spl_timestamp(line.end_ms, false))?;
            }
        }
        writeln!(spl_output)?;

        // 为了简单和兼容，也为翻译行也生成相同的时间戳
        if let Some(main_track) = line.main_track() {
            for track in &main_track.translations {
                for word in &track.words {
                    for syllable in &word.syllables {
                        writeln!(
                            spl_output,
                            "{}{}",
                            format_spl_timestamp(line.start_ms, false),
                            syllable.text
                        )?;
                    }
                }
            }
        }
    }

    Ok(spl_output)
}
