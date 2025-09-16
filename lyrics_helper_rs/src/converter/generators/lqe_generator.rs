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

    write_main_lyric_block(&mut writer, lines, metadata_store, options)?;

    write_auxiliary_block(
        &mut writer,
        lines,
        metadata_store,
        options,
        &AuxiliaryTrackType::Translation,
    )?;

    write_auxiliary_block(
        &mut writer,
        lines,
        metadata_store,
        options,
        &AuxiliaryTrackType::Romanization,
    )?;

    Ok(writer.trim().to_string())
}

enum AuxiliaryTrackType {
    Translation,
    Romanization,
}

fn extract_and_promote_lines(
    lines: &[LyricLine],
    track_type: &AuxiliaryTrackType,
) -> Vec<LyricLine> {
    lines
        .iter()
        .filter_map(|line| {
            let auxiliary_tracks: Vec<_> = line
                .tracks
                .iter()
                .flat_map(|at| match track_type {
                    AuxiliaryTrackType::Translation => at.translations.iter().cloned(),
                    AuxiliaryTrackType::Romanization => at.romanizations.iter().cloned(),
                })
                .collect();

            if auxiliary_tracks.is_empty() {
                None
            } else {
                let annotated_tracks = auxiliary_tracks
                    .into_iter()
                    .map(|track| AnnotatedTrack {
                        content_type: ContentType::Main,
                        content: track,
                        ..Default::default()
                    })
                    .collect();

                Some(LyricLine {
                    tracks: annotated_tracks,
                    ..line.clone()
                })
            }
        })
        .collect()
}

fn write_main_lyric_block(
    writer: &mut String,
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
    options: &LqeGenerationOptions,
) -> Result<(), ConvertError> {
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

    Ok(())
}

fn write_auxiliary_block(
    writer: &mut String,
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
    options: &LqeGenerationOptions,
    track_type: &AuxiliaryTrackType,
) -> Result<(), ConvertError> {
    let auxiliary_lines = extract_and_promote_lines(lines, track_type);
    if auxiliary_lines.is_empty() {
        return Ok(());
    }

    let lang = auxiliary_lines
        .iter()
        .find_map(|l| {
            l.tracks
                .first()?
                .content
                .metadata
                .get(&TrackMetadataKey::Language)
        })
        .map(String::as_str);

    let (block_name, default_lang) = match track_type {
        AuxiliaryTrackType::Translation => ("translation", "und"),
        AuxiliaryTrackType::Romanization => ("pronunciation", "romaji"),
    };

    let final_lang = lang.unwrap_or(default_lang);

    writeln!(
        writer,
        "[{}: format@{}, language@{}]",
        block_name,
        options.auxiliary_format.to_extension_str(),
        final_lang
    )?;

    let content = generate_sub_format(&auxiliary_lines, metadata_store, options.auxiliary_format)?;
    writer.push_str(&content);
    writer.push_str("\n\n");

    Ok(())
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
