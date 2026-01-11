use crate::{generator::utils::apply_parentheses_to_bg_text, utils::normalize_text_whitespace};
use lyrics_helper_core::{
    AnnotatedTrack, ConvertError, LyricSyllable, LyricTrack, TrackMetadataKey,
    TtmlGenerationOptions, TtmlTimingMode,
};
use quick_xml::{
    Writer,
    events::{BytesText, Event},
};

use super::utils::format_ttml_time;

pub(super) fn write_auxiliary_tracks<W: std::io::Write>(
    writer: &mut Writer<W>,
    annotated_tracks: &[&AnnotatedTrack],
    options: &TtmlGenerationOptions,
) -> Result<(), ConvertError> {
    for at in annotated_tracks {
        for track in &at.translations {
            if !options.use_apple_format_rules {
                write_inline_auxiliary_track(writer, track, "x-translation", options)?;
            }
        }

        for track in &at.romanizations {
            let is_timed = track.is_timed();

            if !is_timed && !options.use_apple_format_rules {
                write_inline_auxiliary_track(writer, track, "x-roman", options)?;
            }
        }
    }
    Ok(())
}

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

pub(super) fn write_track_as_spans<W: std::io::Write>(
    writer: &mut Writer<W>,
    track: &LyricTrack,
    options: &TtmlGenerationOptions,
) -> Result<(), ConvertError> {
    let mut syllables_iter = track.syllables().peekable();

    while let Some(syl) = syllables_iter.next() {
        write_single_syllable_span(writer, syl, options)?;

        if syl.ends_with_space && syllables_iter.peek().is_some() && !options.format {
            writer.write_event(Event::Text(BytesText::new(" ")))?;
        }
    }
    Ok(())
}

pub(super) fn write_inline_auxiliary_track<W: std::io::Write>(
    writer: &mut Writer<W>,
    track: &LyricTrack,
    role: &str,
    options: &TtmlGenerationOptions,
) -> Result<(), ConvertError> {
    if track.is_empty() {
        return Ok(());
    }

    let mut element_builder = writer
        .create_element("span")
        .with_attribute(("ttm:role", role));

    if let Some(lang) = track.metadata.get(&TrackMetadataKey::Language)
        && !lang.is_empty()
    {
        element_builder = element_builder.with_attribute(("xml:lang", lang.as_str()));
    }

    let is_timed = track.is_timed();
    let has_multiple_syllables = track.syllables().count() > 1;

    let write_as_nested_timed_spans = is_timed
        && options.timing_mode == TtmlTimingMode::Word
        && has_multiple_syllables
        && options.use_apple_format_rules;

    if write_as_nested_timed_spans {
        if let Some((start_ms, end_ms)) = track.time_range() {
            element_builder
                .with_attribute(("begin", format_ttml_time(start_ms).as_str()))
                .with_attribute(("end", format_ttml_time(end_ms).as_str()))
                .write_inner_content(|writer| Ok(write_track_as_spans(writer, track, options)?))?;
        }
    } else {
        let full_text = track.text();
        let normalized_text = normalize_text_whitespace(&full_text);
        if !normalized_text.is_empty() {
            element_builder.write_text_content(BytesText::new(&normalized_text))?;
        }
    }

    Ok(())
}

pub(super) fn write_background_tracks<W: std::io::Write>(
    writer: &mut Writer<W>,
    bg_annotated_tracks: &[&AnnotatedTrack],
    options: &TtmlGenerationOptions,
) -> Result<(), ConvertError> {
    let mut syllables_iter = bg_annotated_tracks
        .iter()
        .flat_map(|at| at.content.syllables())
        .peekable();

    if syllables_iter.peek().is_none() {
        return Ok(());
    }

    let all_syls: Vec<_> = syllables_iter.collect();

    let start_ms = all_syls.iter().map(|s| s.start_ms).min().unwrap_or(0);
    let end_ms = all_syls.iter().map(|s| s.end_ms).max().unwrap_or(0);

    let start_time_str = format_ttml_time(start_ms);
    let end_time_str = format_ttml_time(end_ms);

    let mut span_builder = writer
        .create_element("span")
        .with_attribute(("ttm:role", "x-bg"));

    if !options.use_apple_format_rules {
        span_builder = span_builder
            .with_attribute(("begin", start_time_str.as_str()))
            .with_attribute(("end", end_time_str.as_str()));
    }

    span_builder.write_inner_content(|writer| {
        let mut is_first = true;
        let mut iter = all_syls.into_iter().peekable();

        while let Some(syl_bg) = iter.next() {
            let is_last = iter.peek().is_none();
            let text_with_parens = apply_parentheses_to_bg_text(&syl_bg.text, is_first, is_last);

            is_first = false;

            let temp_syl = LyricSyllable {
                text: text_with_parens,
                start_ms: syl_bg.start_ms,
                end_ms: syl_bg.end_ms,
                duration_ms: syl_bg.duration_ms,
                ends_with_space: syl_bg.ends_with_space,
            };

            write_single_syllable_span(writer, &temp_syl, options)?;

            if syl_bg.ends_with_space && iter.peek().is_some() && !options.format {
                writer.write_event(Event::Text(BytesText::new(" ")))?;
            }
        }

        write_auxiliary_tracks(writer, bg_annotated_tracks, options)?;

        Ok(())
    })?;
    Ok(())
}
