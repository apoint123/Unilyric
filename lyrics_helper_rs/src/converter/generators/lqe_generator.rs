//! # Lyricify Quick Export 格式生成器

use std::fmt::Write;

use crate::converter::generators;

use lyrics_helper_core::{
    AnnotatedTrack, ContentType, ConvertError, LqeGenerationOptions, LyricFormat, LyricLine,
    MetadataStore, TrackMetadataKey,
};

/// LQE 生成的主入口函数。
pub fn generate_lqe(
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
    options: &LqeGenerationOptions,
) -> Result<String, ConvertError> {
    let mut writer = String::new();

    writer.push_str("[Lyricify Quick Export]\n");
    writer.push_str("[version:1.0]\n");

    let lrc_header = metadata_store.generate_lrc_header();
    if !lrc_header.is_empty() {
        writer.push_str(&lrc_header);
        writer.push('\n');
    }

    let main_lang =
        metadata_store.get_single_value(&lyrics_helper_core::CanonicalMetadataKey::Language);
    let lang_attr = main_lang.map_or("und", |s| s.as_str());

    writeln!(
        writer,
        "[lyrics: format@{}, language@{}]",
        options.main_lyric_format.to_extension_str(),
        lang_attr
    )?;

    let main_content = generate_sub_format(lines, metadata_store, options.main_lyric_format)?;
    writer.push_str(&main_content);
    writer.push_str("\n\n");

    // 翻译区块
    let translation_lines: Vec<LyricLine> = lines
        .iter()
        .filter_map(|line| {
            let trans_tracks: Vec<_> = line
                .tracks
                .iter()
                .flat_map(|at| at.translations.iter().cloned())
                .collect();

            if trans_tracks.is_empty() {
                None
            } else {
                let annotated_tracks = trans_tracks
                    .into_iter()
                    .map(|track| AnnotatedTrack {
                        content_type: ContentType::Main,
                        content: track,
                        translations: vec![],
                        romanizations: vec![],
                    })
                    .collect();
                Some(LyricLine {
                    tracks: annotated_tracks,
                    ..line.clone()
                })
            }
        })
        .collect();

    if !translation_lines.is_empty() {
        let trans_lang = translation_lines
            .iter()
            .find_map(|l| {
                l.tracks
                    .first()
                    .and_then(|t| t.content.metadata.get(&TrackMetadataKey::Language))
            })
            .map_or("und", |s| s.as_str());

        writeln!(
            writer,
            "[translation: format@{}, language@{}]",
            options.auxiliary_format.to_extension_str(),
            trans_lang
        )?;

        let translation_content =
            generate_sub_format(&translation_lines, metadata_store, options.auxiliary_format)?;
        writer.push_str(&translation_content);
        writer.push_str("\n\n");
    }

    // 罗马音区块
    let romanization_lines: Vec<LyricLine> = lines
        .iter()
        .filter_map(|line| {
            let roma_tracks: Vec<_> = line
                .tracks
                .iter()
                .flat_map(|at| at.romanizations.iter().cloned())
                .collect();

            if roma_tracks.is_empty() {
                None
            } else {
                let annotated_tracks = roma_tracks
                    .into_iter()
                    .map(|track| AnnotatedTrack {
                        content_type: ContentType::Main,
                        content: track,
                        translations: vec![],
                        romanizations: vec![],
                    })
                    .collect();
                Some(LyricLine {
                    tracks: annotated_tracks,
                    ..line.clone()
                })
            }
        })
        .collect();

    if !romanization_lines.is_empty() {
        let roma_lang = romanization_lines
            .iter()
            .find_map(|l| {
                l.tracks
                    .first()
                    .and_then(|t| t.content.metadata.get(&TrackMetadataKey::Language))
            })
            .map_or("romaji", |s| s.as_str());

        writeln!(
            writer,
            "[pronunciation: format@{}, language@{}]",
            options.auxiliary_format.to_extension_str(),
            roma_lang
        )?;

        let romanization_content = generate_sub_format(
            &romanization_lines,
            metadata_store,
            options.auxiliary_format,
        )?;
        writer.push_str(&romanization_content);
    }

    Ok(writer.trim().to_string())
}

fn generate_sub_format(
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
    format: LyricFormat,
) -> Result<String, ConvertError> {
    let dummy_options = lyrics_helper_core::ConversionOptions::default();

    match format {
        LyricFormat::Lrc => {
            generators::lrc_generator::generate_lrc(lines, metadata_store, &dummy_options.lrc)
        }
        LyricFormat::EnhancedLrc => generators::enhanced_lrc_generator::generate_enhanced_lrc(
            lines,
            metadata_store,
            &dummy_options.lrc,
        ),
        LyricFormat::Lys => generators::lys_generator::generate_lys(lines, metadata_store),
        _ => Err(ConvertError::Internal(format!(
            "LQE 生成器不支持将内部区块格式化为 '{format:?}'"
        ))),
    }
}
