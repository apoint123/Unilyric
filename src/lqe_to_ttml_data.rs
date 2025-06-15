use crate::types::{
    self, ConvertError, LqeSection, LrcLine, LyricFormat, ParsedLqeData, ParsedSourceData,
    TtmlParagraph,
};
use crate::{
    lrc_parser, lyricify_lines_parser, lyricify_lines_to_ttml_data, lys_parser, lys_to_ttml_data,
    spl_parser, spl_to_ttml_data,
};

fn parse_lyrics(section: &LqeSection) -> Result<Vec<TtmlParagraph>, ConvertError> {
    match section.format {
        Some(LyricFormat::Lys) => {
            let (lys_lines, _meta) = lys_parser::load_lys_from_string(&section.content)?;
            let (paragraphs, _) = lys_to_ttml_data::convert_lys_to_ttml_data(&lys_lines)?;
            Ok(paragraphs)
        }
        Some(LyricFormat::Lyl) => {
            let parsed_lines = lyricify_lines_parser::parse_lyricify_lines(&section.content)?;
            let (paragraphs, _meta) =
                lyricify_lines_to_ttml_data::convert_lyricify_to_ttml_data(&parsed_lines)?;
            Ok(paragraphs)
        }
        Some(LyricFormat::Spl) => {
            let (raw_spl_lines, _spl_meta) = spl_parser::load_spl_from_string(&section.content)?;
            let (paragraphs, _processed_metadata) =
                spl_to_ttml_data::convert_spl_to_ttml_data(&raw_spl_lines, Vec::new())?;
            Ok(paragraphs)
        }
        Some(fmt) => Err(ConvertError::Internal(format!("不支持的格式: {fmt:?}"))),
        None => Err(ConvertError::Internal("歌词格式未指定.".to_string())),
    }
}

fn parse_embedded_lrc(section_content: &str) -> Result<Vec<LrcLine>, ConvertError> {
    let (display_lrc_lines, _bilingual_translations, _lrc_meta) =
        lrc_parser::parse_lrc_text_to_lines(section_content)?;

    let lrc_lines: Vec<LrcLine> = display_lrc_lines
        .into_iter()
        .filter_map(|display_line| match display_line {
            types::DisplayLrcLine::Parsed(lrc_line) => Some(lrc_line),
            _ => None, // 忽略原始行或无法解析的行
        })
        .collect();
    Ok(lrc_lines)
}

pub fn convert_lqe_to_intermediate_data(
    lqe_data: &ParsedLqeData,
) -> Result<ParsedSourceData, ConvertError> {
    let mut intermediate_data = ParsedSourceData {
        general_metadata: lqe_data.global_metadata.clone(),
        ..Default::default()
    };

    if let Some(ref lyrics_sec) = lqe_data.lyrics_section {
        intermediate_data.paragraphs = parse_lyrics(lyrics_sec)?;
        if let Some(lang) = &lyrics_sec.language {
            intermediate_data.language_code = Some(lang.clone());
        }
        if let Some(fmt) = lyrics_sec.format {
            intermediate_data.is_line_timed_source =
                matches!(fmt, LyricFormat::Lyl | LyricFormat::Lrc);
            if fmt == LyricFormat::Spl {
                let mut is_spl_effectively_line_timed = true;
                if !intermediate_data.paragraphs.is_empty() {
                    for p in &intermediate_data.paragraphs {
                        if p.main_syllables.len() > 1 {
                            is_spl_effectively_line_timed = false;
                            break;
                        }
                    }
                }
                intermediate_data.is_line_timed_source = is_spl_effectively_line_timed;
            }
        }
    } else {
        log::info!("[LQE -> TTML] 未找到歌词部分");
    }

    if let Some(ref trans_sec) = lqe_data.translation_section {
        if trans_sec.format == Some(LyricFormat::Lrc) || trans_sec.format.is_none() {
            if intermediate_data.paragraphs.is_empty() {
                intermediate_data.lqe_extracted_translation_lrc_content =
                    Some(trans_sec.content.clone());
                intermediate_data.lqe_translation_language = trans_sec.language.clone();
            } else {
                let lrc_lines = parse_embedded_lrc(&trans_sec.content)?;
                let trans_lang = trans_sec.language.clone();
                for para in intermediate_data.paragraphs.iter_mut() {
                    if let Some(lrc_line) =
                        lrc_lines.iter().find(|l| l.timestamp_ms == para.p_start_ms)
                    {
                        para.translation = Some((lrc_line.text.clone(), trans_lang.clone()));
                    }
                }
            }
        } else {
            log::warn!(
                "[LQE -> TTML] 不支持的翻译格式: {:?}，应为LRC",
                trans_sec.format
            );
        }
    }

    if let Some(ref pron_sec) = lqe_data.pronunciation_section {
        if pron_sec.format == Some(LyricFormat::Lrc) || pron_sec.format.is_none() {
            if intermediate_data.paragraphs.is_empty() {
                intermediate_data.lqe_extracted_romanization_lrc_content =
                    Some(pron_sec.content.clone());
                intermediate_data.lqe_romanization_language = pron_sec.language.clone();
            } else {
                let lrc_lines = parse_embedded_lrc(&pron_sec.content)?;
                for para in intermediate_data.paragraphs.iter_mut() {
                    if let Some(lrc_line) =
                        lrc_lines.iter().find(|l| l.timestamp_ms == para.p_start_ms)
                    {
                        para.romanization = Some(lrc_line.text.clone());
                    }
                }
            }
        } else {
            log::warn!(
                "[LQE -> TTML] 不支持的音译格式: {:?}，应为LRC",
                pron_sec.format
            );
        }
    }

    for meta in &intermediate_data.general_metadata {
        match meta.key.to_lowercase().as_str() {
            "ti" | "title" => if intermediate_data.apple_music_id.is_empty() && meta.key == "ti" {},
            "ar" | "artist" | "artists" => {
                if !meta.value.is_empty() {
                    intermediate_data.songwriters.push(meta.value.clone());
                }
            }
            _ => {}
        }
    }
    intermediate_data.songwriters.sort();
    intermediate_data.songwriters.dedup();

    Ok(intermediate_data)
}
