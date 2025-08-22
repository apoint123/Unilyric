//! # TTML 生成器 - 轨道渲染模块
//!
//! 该模块负责将 `LyricTrack` 数据结构渲染为具体的 `<span>` XML 元素，
//! 包括主歌词、背景人声、翻译和罗马音等。

use crate::utils::normalize_text_whitespace;
use lyrics_helper_core::{
    AnnotatedTrack, ConvertError, LyricSyllable, LyricTrack, TrackMetadataKey,
    TtmlGenerationOptions, TtmlTimingMode,
};
use quick_xml::{
    Writer,
    events::{BytesText, Event},
};

use super::{splitting::write_syllable_with_optional_splitting, utils::format_ttml_time};

/// 写入单个音节span
pub(super) fn write_single_syllable_span<W: std::io::Write>(
    writer: &mut Writer<W>,
    syl: &LyricSyllable,
    options: &TtmlGenerationOptions,
) -> Result<(), ConvertError> {
    let builder = writer
        .create_element("span")
        .with_attribute(("begin", format_ttml_time(syl.start_ms).as_str()))
        .with_attribute((
            "end",
            format_ttml_time(syl.end_ms.max(syl.start_ms)).as_str(),
        ));

    if options.format && syl.ends_with_space {
        let text_with_space = format!("{} ", syl.text);
        builder.write_text_content(BytesText::new(&text_with_space))?;
    } else {
        builder.write_text_content(BytesText::new(&syl.text))?;
    }
    Ok(())
}

/// 将一个完整的轨道渲染为一系列的 `<span>` 标签。
pub(super) fn write_track_as_spans<W: std::io::Write>(
    writer: &mut Writer<W>,
    track: &LyricTrack,
    options: &TtmlGenerationOptions,
) -> Result<(), ConvertError> {
    let mut syllables_iter = track.words.iter().flat_map(|w| &w.syllables).peekable();

    while let Some(syl) = syllables_iter.next() {
        write_syllable_with_optional_splitting(writer, syl, options)?;

        if syl.ends_with_space && syllables_iter.peek().is_some() && !options.format {
            writer.write_event(Event::Text(BytesText::new(" ")))?;
        }
    }
    Ok(())
}

/// 将辅助轨道（如翻译、罗马音）作为内联的 `<span>` 写入。
pub(super) fn write_inline_auxiliary_track<W: std::io::Write>(
    writer: &mut Writer<W>,
    track: &LyricTrack,
    role: &str,
    options: &TtmlGenerationOptions,
) -> Result<(), ConvertError> {
    let mut element_builder = writer
        .create_element("span")
        .with_attribute(("ttm:role", role));

    if let Some(lang) = track.metadata.get(&TrackMetadataKey::Language)
        && !lang.is_empty()
    {
        element_builder = element_builder.with_attribute(("xml:lang", lang.as_str()));
    }

    let all_syllables: Vec<_> = track.words.iter().flat_map(|w| &w.syllables).collect();
    if all_syllables.is_empty() {
        return Ok(());
    }

    let is_timed = all_syllables.iter().any(|s| s.end_ms > s.start_ms);
    let has_multiple_syllables = all_syllables.len() > 1;

    let write_as_nested_timed_spans =
        is_timed && options.timing_mode == TtmlTimingMode::Word && has_multiple_syllables;

    if write_as_nested_timed_spans {
        let start_ms = all_syllables.iter().map(|s| s.start_ms).min().unwrap_or(0);
        let end_ms = all_syllables.iter().map(|s| s.end_ms).max().unwrap_or(0);

        element_builder
            .with_attribute(("begin", format_ttml_time(start_ms).as_str()))
            .with_attribute(("end", format_ttml_time(end_ms).as_str()))
            .write_inner_content(|writer| Ok(write_track_as_spans(writer, track, options)?))?;
    } else {
        let full_text = all_syllables
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(if options.format { " " } else { "" });

        let normalized_text = normalize_text_whitespace(&full_text);
        if !normalized_text.is_empty() {
            element_builder.write_text_content(BytesText::new(&normalized_text))?;
        }
    }

    Ok(())
}

/// 将所有背景人声轨道写入一个大的 `x-bg` 角色 `<span>` 容器中。
pub(super) fn write_background_tracks<W: std::io::Write>(
    writer: &mut Writer<W>,
    bg_annotated_tracks: &[&AnnotatedTrack],
    options: &TtmlGenerationOptions,
) -> Result<(), ConvertError> {
    let mut syllables_iter = bg_annotated_tracks
        .iter()
        .flat_map(|at| at.content.words.iter().flat_map(|w| &w.syllables))
        .peekable();

    if syllables_iter.peek().is_none() {
        return Ok(());
    }

    let all_syls: Vec<_> = syllables_iter.collect();

    let start_ms = all_syls.iter().map(|s| s.start_ms).min().unwrap_or(0);
    let end_ms = all_syls.iter().map(|s| s.end_ms).max().unwrap_or(0);

    writer
        .create_element("span")
        .with_attribute(("ttm:role", "x-bg"))
        .with_attribute(("begin", format_ttml_time(start_ms).as_str()))
        .with_attribute(("end", format_ttml_time(end_ms).as_str()))
        .write_inner_content(|writer| {
            let mut is_first = true;
            let mut iter = all_syls.into_iter().peekable();

            while let Some(syl_bg) = iter.next() {
                let text_with_parens = if syl_bg.text.trim().is_empty() {
                    syl_bg.text.clone()
                } else {
                    match (is_first, iter.peek().is_none()) {
                        (true, true) => format!("({})", syl_bg.text), // 唯一元素
                        (true, false) => format!("({}", syl_bg.text), // 第一个元素
                        (false, true) => format!("{})", syl_bg.text), // 最后一个元素
                        (false, false) => syl_bg.text.clone(),        // 中间元素
                    }
                };
                is_first = false;

                let temp_syl = LyricSyllable {
                    text: text_with_parens,
                    start_ms: syl_bg.start_ms,
                    end_ms: syl_bg.end_ms,
                    duration_ms: syl_bg.duration_ms,
                    ends_with_space: syl_bg.ends_with_space,
                };

                write_syllable_with_optional_splitting(writer, &temp_syl, options)?;

                if syl_bg.ends_with_space && iter.peek().is_some() && !options.format {
                    writer.write_event(Event::Text(BytesText::new(" ")))?;
                }
            }

            for at in bg_annotated_tracks {
                for track in &at.translations {
                    write_inline_auxiliary_track(writer, track, "x-translation", options)?;
                }
                for track in &at.romanizations {
                    write_inline_auxiliary_track(writer, track, "x-roman", options)?;
                }
            }

            Ok(())
        })?;
    Ok(())
}
