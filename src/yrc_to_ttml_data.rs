use crate::types::{AssMetadata, ConvertError, QrcLine, TtmlParagraph};
use crate::utils::{post_process_ttml_syllable_line_spacing, process_parsed_syllables_to_ttml};

pub fn convert_yrc_to_ttml_data(
    yrc_lines: &[QrcLine],
    yrc_metadata: Vec<AssMetadata>,
) -> Result<(Vec<TtmlParagraph>, Vec<AssMetadata>), ConvertError> {
    let mut ttml_paragraphs: Vec<TtmlParagraph> = Vec::new();

    for yrc_line in yrc_lines.iter() {
        let mut current_ttml_syllables =
            process_parsed_syllables_to_ttml(&yrc_line.syllables, "YRC");

        post_process_ttml_syllable_line_spacing(&mut current_ttml_syllables);

        if !current_ttml_syllables.is_empty()
            || (yrc_line
                .line_start_ms
                .saturating_add(yrc_line.line_duration_ms)
                > yrc_line.line_start_ms)
        {
            let paragraph = TtmlParagraph {
                p_start_ms: yrc_line.line_start_ms,
                p_end_ms: yrc_line
                    .line_start_ms
                    .saturating_add(yrc_line.line_duration_ms),
                agent: "v1".to_string(),
                main_syllables: current_ttml_syllables,
                background_section: None,
                translation: None,
                romanization: None,
                song_part: None,
            };
            ttml_paragraphs.push(paragraph);
        } else {
            log::warn!(
                "[YRC -> TTML] YRC 行 (开始时间: {}) 无音节且无时长，已跳过",
                yrc_line.line_start_ms
            );
        }
    }
    Ok((ttml_paragraphs, yrc_metadata))
}
