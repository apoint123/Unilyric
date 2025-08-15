//! # LQE 解析器

use std::collections::HashMap;
use tracing::warn;

use lyrics_helper_core::{
    ConversionOptions, ConvertError, InputFile, LyricFormat, LyricLine, ParsedSourceData,
};

#[derive(Clone, Copy)]
enum ParseState {
    Header,
    Lyrics,
    Translation,
    Pronunciation,
}

/// 解析 LQR 格式内容到 `ParsedSourceData` 结构。
pub fn parse_lqe(
    content: &str,
    options: &ConversionOptions,
) -> Result<ParsedSourceData, ConvertError> {
    if !content.trim_start().starts_with("[Lyricify Quick Export]") {
        return Err(ConvertError::InvalidLyricFormat(
            "文件缺少 [Lyricify Quick Export] 头部标记。".to_string(),
        ));
    }

    let mut main_source: Option<ParsedSourceData> = None;
    let mut translation_sources: Vec<(Vec<LyricLine>, ParsedSourceData, Option<String>)> =
        Vec::new();
    let mut romanization_sources: Vec<(Vec<LyricLine>, ParsedSourceData, Option<String>)> =
        Vec::new();
    let mut raw_metadata: HashMap<String, Vec<String>> = HashMap::new();

    let mut current_state = ParseState::Header;
    let mut current_block_content = String::new();
    let mut current_block_format = LyricFormat::Lrc;
    let mut current_block_lang: Option<String> = None;

    for line in content.lines() {
        if let Some(captures) = line
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .and_then(|s| s.split_once(':'))
        {
            let key = captures.0.trim();
            if ![
                "lyrics",
                "translation",
                "pronunciation",
                "Lyricify Quick Export",
                "version",
            ]
            .contains(&key)
            {
                raw_metadata
                    .entry(key.to_string())
                    .or_default()
                    .push(captures.1.trim().to_string());
            }
        }

        if line.starts_with("[lyrics:")
            || line.starts_with("[translation:")
            || line.starts_with("[pronunciation:")
        {
            if let Some(mut parsed_data) = process_block(
                &current_block_content,
                current_block_format,
                current_block_lang.clone(),
                options,
            )? {
                match current_state {
                    ParseState::Lyrics => main_source = Some(parsed_data),
                    ParseState::Translation => translation_sources.push((
                        std::mem::take(&mut parsed_data.lines),
                        parsed_data,
                        current_block_lang.clone(),
                    )),
                    ParseState::Pronunciation => romanization_sources.push((
                        std::mem::take(&mut parsed_data.lines),
                        parsed_data,
                        current_block_lang.clone(),
                    )),
                    ParseState::Header => {}
                }
            }
            current_block_content.clear();
            let (format, lang) = parse_section_header(line);
            current_block_format = format;
            current_block_lang = lang;
            current_state = if line.starts_with("[lyrics:") {
                ParseState::Lyrics
            } else if line.starts_with("[translation:") {
                ParseState::Translation
            } else {
                ParseState::Pronunciation
            };
        } else if !line.starts_with("[Lyricify Quick Export]") {
            current_block_content.push_str(line);
            current_block_content.push('\n');
        }
    }

    if let Some(mut parsed_data) = process_block(
        &current_block_content,
        current_block_format,
        current_block_lang.clone(),
        options,
    )? {
        match current_state {
            ParseState::Lyrics => main_source = Some(parsed_data),
            ParseState::Translation => translation_sources.push((
                std::mem::take(&mut parsed_data.lines),
                parsed_data,
                current_block_lang,
            )),
            ParseState::Pronunciation => romanization_sources.push((
                std::mem::take(&mut parsed_data.lines),
                parsed_data,
                current_block_lang,
            )),
            ParseState::Header => {}
        }
    }

    let mut result = main_source.unwrap_or_default();
    result.source_format = LyricFormat::Lqe;
    result.raw_metadata.extend(raw_metadata);

    crate::converter::merge_tracks(
        &mut result.lines,
        &translation_sources,
        &romanization_sources,
        options.matching_strategy,
    );

    for (_, source, _) in translation_sources
        .iter()
        .chain(romanization_sources.iter())
    {
        result.raw_metadata.extend(source.raw_metadata.clone());
    }

    Ok(result)
}

fn parse_section_header(header_line: &str) -> (LyricFormat, Option<String>) {
    let mut format = LyricFormat::Lrc;
    let mut lang = None;
    if let Some(params_str) = header_line
        .split_once(':')
        .and_then(|(_, rest)| rest.strip_suffix(']'))
    {
        for param in params_str.split(',').map(str::trim) {
            if let Some((key, value)) = param.split_once('@') {
                match key.trim() {
                    "format" => {
                        format = LyricFormat::from_string(value.trim()).unwrap_or_else(|| {
                            warn!("未知的 LQE 区块格式 '{}', 将回退到 LRC", value);
                            LyricFormat::Lrc
                        });
                    }
                    "language" => lang = Some(value.trim().to_string()),
                    _ => {}
                }
            }
        }
    }
    (format, lang)
}

fn process_block(
    content: &str,
    format: LyricFormat,
    lang: Option<String>,
    options: &ConversionOptions,
) -> Result<Option<ParsedSourceData>, ConvertError> {
    if content.trim().is_empty() {
        return Ok(None);
    }
    let input_file = InputFile::new(content.to_string(), format, lang, None);
    crate::converter::parse_input_file(&input_file, options).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lqe_simple_parse() {
        let content = "[Lyricify Quick Export]\n[version:1.0]\n[lyrics:format@lrc]\n[00:10.00]Hello world\n[00:12.50]Next line";
        let options = ConversionOptions::default();
        let parsed_data = parse_lqe(content, &options).unwrap();
        assert_eq!(parsed_data.lines.len(), 2);
        assert_eq!(parsed_data.source_format, LyricFormat::Lqe);
    }

    #[test]
    fn test_lqe_with_translation() {
        let content = "[Lyricify Quick Export]\n[lyrics:format@lrc]\n[00:10.00]Hello world\n[translation:format@lrc,language@zh-Hans]\n[00:10.00]你好世界";
        let options = ConversionOptions::default();
        let parsed_data = parse_lqe(content, &options).unwrap();
        assert_eq!(parsed_data.lines.len(), 1);

        let line = &parsed_data.lines[0];
        let main_annotated_track = line
            .tracks
            .iter()
            .find(|at| at.content_type == lyrics_helper_core::ContentType::Main)
            .expect("应该找到主内容轨道");

        let translation_track = main_annotated_track.translations.first();

        assert!(translation_track.is_some(), "主内容轨道中应该包含翻译");
        assert_eq!(
            translation_track.unwrap().words[0].syllables[0].text,
            "你好世界"
        );
    }
}
