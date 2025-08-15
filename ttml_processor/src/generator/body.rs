//! # TTML 生成器 - Body 处理模块
//!
//! 该模块负责生成 TTML 文件的 `<body>` 部分，包括对歌词行进行分组
//! 并写入 `<div>` 和 `<p>` 标签中。

use lyrics_helper_core::{
    ContentType, ConvertError, LyricLine, TtmlGenerationOptions, TtmlTimingMode,
};
use quick_xml::{Writer, events::BytesText, events::Event};

use crate::utils::normalize_text_whitespace;

use super::{
    track::{write_background_tracks, write_inline_auxiliary_track, write_track_as_spans},
    utils::format_ttml_time,
};

/// 写入 TTML 的 <body> 部分，包含所有歌词行。
pub(super) fn write_ttml_body<W: std::io::Write>(
    writer: &mut Writer<W>,
    lines: &[LyricLine],
    options: &TtmlGenerationOptions,
) -> Result<(), ConvertError> {
    let body_dur_ms = lines.iter().map(|line| line.end_ms).max().unwrap_or(0);
    let mut body_builder = writer.create_element("body");
    if body_dur_ms > 0 {
        body_builder = body_builder.with_attribute(("dur", format_ttml_time(body_dur_ms).as_str()));
    }

    if lines.is_empty() {
        body_builder.write_empty()?;
        return Ok(());
    }

    body_builder.write_inner_content(|writer| {
        let mut p_key_counter = 0;
        let mut current_div_lines: Vec<&LyricLine> = Vec::new();
        for current_line in lines {
            if !current_div_lines.is_empty() {
                let prev_line = *current_div_lines.last().unwrap();
                if prev_line.song_part != current_line.song_part {
                    write_div(writer, &current_div_lines, options, &mut p_key_counter)
                        .map_err(std::io::Error::other)?;
                    current_div_lines.clear();
                }
            }
            current_div_lines.push(current_line);
        }
        if !current_div_lines.is_empty() {
            write_div(writer, &current_div_lines, options, &mut p_key_counter)
                .map_err(std::io::Error::other)?;
        }
        Ok(())
    })?;
    Ok(())
}

/// 将歌词行写入一个 div 块
fn write_div<W: std::io::Write>(
    writer: &mut Writer<W>,
    part_lines: &[&LyricLine],
    options: &TtmlGenerationOptions,
    p_key_counter: &mut i32,
) -> Result<(), ConvertError> {
    if part_lines.is_empty() {
        return Ok(());
    }

    let div_start_ms = part_lines.first().unwrap().start_ms;
    let div_end_ms = part_lines
        .iter()
        .map(|l| l.end_ms)
        .max()
        .unwrap_or(div_start_ms);
    let song_part_key = &part_lines.first().unwrap().song_part;

    let mut div_builder = writer.create_element("div");
    div_builder = div_builder
        .with_attribute(("begin", format_ttml_time(div_start_ms).as_str()))
        .with_attribute(("end", format_ttml_time(div_end_ms).as_str()));

    if let Some(sp_val) = song_part_key.as_ref().filter(|s| !s.is_empty()) {
        div_builder = div_builder.with_attribute(("itunes:song-part", sp_val.as_str()));
    }

    div_builder.write_inner_content(|writer| {
        for line in part_lines {
            *p_key_counter += 1;

            let agent_id_to_set = line.agent.as_deref().unwrap_or("v1");

            writer
                .create_element("p")
                .with_attribute(("begin", format_ttml_time(line.start_ms).as_str()))
                .with_attribute(("end", format_ttml_time(line.end_ms).as_str()))
                .with_attribute(("itunes:key", format!("L{p_key_counter}").as_str()))
                .with_attribute(("ttm:agent", agent_id_to_set))
                .write_inner_content(|writer| {
                    write_p_content(writer, line, options).map_err(std::io::Error::other)
                })?;
        }
        Ok(())
    })?;
    Ok(())
}

/// 写入 <p> 标签的具体内容，包括主歌词、翻译、罗马音和背景人声。
fn write_p_content<W: std::io::Write>(
    writer: &mut Writer<W>,
    line: &LyricLine,
    options: &TtmlGenerationOptions,
) -> Result<(), ConvertError> {
    let main_content_tracks: Vec<_> = line
        .tracks
        .iter()
        .filter(|at| at.content_type == ContentType::Main)
        .collect();

    let background_annotated_tracks: Vec<_> = line
        .tracks
        .iter()
        .filter(|at| at.content_type == ContentType::Background)
        .collect();

    // 1. 处理主内容
    if options.timing_mode == TtmlTimingMode::Line {
        let line_text_to_write = main_content_tracks
            .iter()
            .flat_map(|at| at.content.words.iter().flat_map(|w| &w.syllables))
            .map(|syl| syl.text.clone())
            .collect::<Vec<_>>()
            .join(if options.format { " " } else { "" });

        if !line_text_to_write.is_empty() {
            writer.write_event(Event::Text(BytesText::new(&normalize_text_whitespace(
                &line_text_to_write,
            ))))?;
        }
    } else {
        for at in &main_content_tracks {
            write_track_as_spans(writer, &at.content, options)?;
        }
    }

    // 2. 处理内联辅助轨道 (用于主内容轨道)
    if !options.use_apple_format_rules {
        for at in &main_content_tracks {
            for track in &at.translations {
                write_inline_auxiliary_track(writer, track, "x-translation", options)?;
            }
            for track in &at.romanizations {
                write_inline_auxiliary_track(writer, track, "x-roman", options)?;
            }
        }
    }

    // 3. 处理背景内容
    if options.timing_mode == TtmlTimingMode::Word && !background_annotated_tracks.is_empty() {
        write_background_tracks(writer, &background_annotated_tracks, options)?;
    }

    Ok(())
}
