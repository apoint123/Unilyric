//! Lyricify Lines 格式生成器

use std::fmt::Write;

use lyrics_helper_core::{ContentType, ConvertError, LyricLine, MetadataStore};

/// LYL 生成的主入口函数。
pub fn generate_lyl(
    lines: &[LyricLine],
    _metadata_store: &MetadataStore,
) -> Result<String, ConvertError> {
    let mut output = String::with_capacity(lines.len() * 50);

    writeln!(output, "[type:LyricifyLines]")?;

    for line in lines {
        // 从主内容轨道提取文本
        if let Some(main_track) = line
            .tracks
            .iter()
            .find(|t| t.content_type == ContentType::Main)
        {
            let text_to_write: String = main_track
                .content
                .words
                .iter()
                .flat_map(|w| &w.syllables)
                .map(|s| s.text.as_str())
                .collect();

            if text_to_write.is_empty() {
                continue;
            }

            writeln!(
                output,
                "[{},{}]{}",
                line.start_ms, line.end_ms, text_to_write
            )?;
        }
    }

    Ok(output)
}
