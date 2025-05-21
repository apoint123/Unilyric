use crate::metadata_processor::MetadataStore;
use crate::types::{ConvertError, TtmlParagraph};
use std::fmt::Write;

pub fn generate_yrc_from_ttml_data(
    paragraphs: &[TtmlParagraph],
    _metadata_store: &MetadataStore,
) -> Result<String, ConvertError> {
    let mut yrc_output = String::new();

    for para in paragraphs {
        if para.main_syllables.is_empty() {
            if para.p_end_ms > para.p_start_ms {
                let line_duration_ms = para.p_end_ms.saturating_sub(para.p_start_ms);
                if line_duration_ms > 0 {
                    writeln!(yrc_output, "[{},{}]", para.p_start_ms, line_duration_ms)?;
                }
            }
            continue;
        }

        let line_start_ms = para.p_start_ms;
        let mut line_duration_ms = para.p_end_ms.saturating_sub(para.p_start_ms);

        if line_duration_ms == 0 && !para.main_syllables.is_empty() {
            let first_syl_start_opt = para.main_syllables.first().map(|s| s.start_ms);
            let last_syl_end_opt = para.main_syllables.last().map(|s| s.end_ms);
            if let (Some(syl_start), Some(syl_end)) = (first_syl_start_opt, last_syl_end_opt) {
                let syl_based_duration = syl_end.saturating_sub(syl_start);
                line_duration_ms = syl_based_duration;
                if line_duration_ms == 0 && para.main_syllables.iter().any(|s| !s.text.is_empty()) {
                    line_duration_ms = 1;
                }
            }
        }

        if line_duration_ms == 0 && para.main_syllables.is_empty() {
            continue;
        }
        write!(yrc_output, "[{},{}]", line_start_ms, line_duration_ms)?;

        let num_main_syllables = para.main_syllables.len();
        for (idx, ttml_syl) in para.main_syllables.iter().enumerate() {
            let syl_duration_ms = ttml_syl.end_ms.saturating_sub(ttml_syl.start_ms);

            if !ttml_syl.text.is_empty() {
                let actual_syl_duration = syl_duration_ms;
                write!(
                    yrc_output,
                    "({},{},0){}",
                    ttml_syl.start_ms, actual_syl_duration, ttml_syl.text
                )?;
            } else if syl_duration_ms > 0 {
                write!(yrc_output, "({},{},0)", ttml_syl.start_ms, syl_duration_ms)?;
            }

            if ttml_syl.ends_with_space && idx < num_main_syllables - 1 {
                write!(yrc_output, "(0,0,0)  ")?;
            }
        }
        writeln!(yrc_output)?;
    }

    let final_output = yrc_output.trim_end_matches('\n');
    Ok(if final_output.is_empty() {
        String::new()
    } else {
        format!("{}\n", final_output)
    })
}
