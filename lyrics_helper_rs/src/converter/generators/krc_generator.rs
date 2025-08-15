//! KRC 格式生成器

use std::fmt::Write;

use lyrics_helper_core::{ContentType, ConvertError, LyricLine, MetadataStore};

/// KRC 生成的主入口函数。
pub fn generate_krc(
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
) -> Result<String, ConvertError> {
    let mut krc_output = String::new();

    writeln!(krc_output, "{}", metadata_store.generate_lrc_header())?;

    for line in lines {
        // KRC 不支持背景人声
        if let Some(main_track) = line
            .tracks
            .iter()
            .find(|t| t.content_type == ContentType::Main)
        {
            let syllables: Vec<_> = main_track
                .content
                .words
                .iter()
                .flat_map(|w| &w.syllables)
                .collect();

            if syllables.is_empty() {
                continue;
            }

            let line_duration = line.end_ms.saturating_sub(line.start_ms);
            write!(krc_output, "[{},{}]", line.start_ms, line_duration)?;

            for syl in syllables {
                let offset_ms = syl.start_ms.saturating_sub(line.start_ms);
                let duration_ms = syl.end_ms.saturating_sub(syl.start_ms);

                write!(krc_output, "<{},{},0>{}", offset_ms, duration_ms, syl.text)?;
            }

            writeln!(krc_output)?;
        }
    }

    Ok(krc_output)
}
