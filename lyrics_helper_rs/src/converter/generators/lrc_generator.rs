//! LRC 格式生成器

use std::fmt::Write as FmtWrite;

use lyrics_helper_core::{
    ContentType, ConvertError, LrcEndTimeOutputMode, LrcGenerationOptions, LrcSubLinesOutputMode,
    LyricLine, LyricTrack, MetadataStore,
};

/// LRC 生成的主入口函数。
pub fn generate_lrc(
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
    options: &LrcGenerationOptions,
) -> Result<String, ConvertError> {
    let mut lrc_output = String::with_capacity(lines.len() * 50);

    let lrc_header = metadata_store.generate_lrc_header();
    if !lrc_header.is_empty() {
        writeln!(lrc_output, "{}", lrc_header.trim_end_matches('\n'))?;
    }

    for (i, line) in lines.iter().enumerate() {
        let main_annotated_track = line
            .tracks
            .iter()
            .find(|t| t.content_type == ContentType::Main);
        let bg_annotated_track = line
            .tracks
            .iter()
            .find(|t| t.content_type == ContentType::Background);

        match options.sub_lines_output_mode {
            LrcSubLinesOutputMode::Ignore => {
                if let Some(track) = main_annotated_track {
                    write_track_as_line(&mut lrc_output, line.start_ms, &track.content)?;
                }
            }
            LrcSubLinesOutputMode::MergeWithParentheses => {
                write_merged_line(
                    &mut lrc_output,
                    line.start_ms,
                    main_annotated_track.map(|t| &t.content),
                    bg_annotated_track.map(|t| &t.content),
                )?;
            }
            LrcSubLinesOutputMode::SeparateLines => {
                if let Some(track) = main_annotated_track {
                    write_track_as_line(&mut lrc_output, line.start_ms, &track.content)?;
                }
                if let Some(track) = bg_annotated_track {
                    let bg_start_ms = track
                        .content
                        .words
                        .iter()
                        .flat_map(|w| &w.syllables)
                        .map(|s| s.start_ms)
                        .min()
                        .unwrap_or(line.start_ms);
                    write_track_as_line(&mut lrc_output, bg_start_ms, &track.content)?;
                }
            }
        }

        let next_line_start_ms = lines.get(i + 1).map(|l| l.start_ms);
        handle_end_time_output(
            &mut lrc_output,
            line,
            options.end_time_output_mode,
            next_line_start_ms,
        )?;
    }

    let trimmed_output = lrc_output.trim_end();
    Ok(format!("{trimmed_output}\n"))
}

/// 从轨道中提取纯文本。
fn get_text_from_track(track: &LyricTrack) -> String {
    let line_text = track
        .words
        .iter()
        .flat_map(|w| &w.syllables)
        .map(|syl| {
            if syl.ends_with_space {
                format!("{} ", syl.text)
            } else {
                syl.text.clone()
            }
        })
        .collect::<String>();

    // .collect() 可能会在最后留下一个多余的空格，最好 trim 一下
    line_text.trim_end().to_string()
}

/// 将一个轨道作为简单的 LRC 行写入。
fn write_track_as_line(
    output: &mut String,
    start_ms: u64,
    track: &LyricTrack,
) -> Result<(), std::fmt::Error> {
    let text = get_text_from_track(track);
    if !text.trim().is_empty() {
        writeln!(output, "{}{}", format_lrc_time_ms(start_ms), text)?;
    }
    Ok(())
}

/// 将主轨道和背景轨道合并为一行写入。
fn write_merged_line(
    output: &mut String,
    line_start_ms: u64,
    main_track: Option<&LyricTrack>,
    bg_track: Option<&LyricTrack>,
) -> Result<(), std::fmt::Error> {
    let main_text = main_track.map(get_text_from_track);
    let bg_text = bg_track.map(get_text_from_track);

    match (main_text, bg_text) {
        (Some(mt), Some(bt)) if !mt.trim().is_empty() && !bt.trim().is_empty() => {
            let merged_text = format!("{} ({})", mt.trim(), bt.trim());
            writeln!(
                output,
                "{}{}",
                format_lrc_time_ms(line_start_ms),
                merged_text
            )?;
        }
        (Some(mt), _) if !mt.trim().is_empty() => {
            writeln!(output, "{}{}", format_lrc_time_ms(line_start_ms), mt)?;
        }
        (_, Some(bt)) if !bt.trim().is_empty() => {
            let merged_text = format!("({})", bt.trim());
            writeln!(
                output,
                "{}{}",
                format_lrc_time_ms(line_start_ms),
                merged_text
            )?;
        }
        _ => {}
    }
    Ok(())
}

/// 根据选项处理是否输出行结束时间戳。
fn handle_end_time_output(
    output: &mut String,
    current_line: &LyricLine,
    mode: LrcEndTimeOutputMode,
    next_line_start_ms: Option<u64>,
) -> Result<(), std::fmt::Error> {
    if current_line.end_ms == 0 {
        return Ok(());
    }

    match mode {
        LrcEndTimeOutputMode::Never => { /* 什么也不做 */ }
        LrcEndTimeOutputMode::Always => {
            writeln!(output, "{}", format_lrc_time_ms(current_line.end_ms))?;
        }
        LrcEndTimeOutputMode::OnLongPause { threshold_ms } => {
            if let Some(next_start) = next_line_start_ms {
                if next_start.saturating_sub(current_line.end_ms) > threshold_ms {
                    writeln!(output, "{}", format_lrc_time_ms(current_line.end_ms))?;
                }
            } else {
                writeln!(output, "{}", format_lrc_time_ms(current_line.end_ms))?;
            }
        }
    }
    Ok(())
}

/// 将毫秒时间格式化为 LRC 时间字符串 `[mm:ss.xxx]` 或 `[mm:ss.xx]`。
/// 此函数输出毫秒 (xxx)。
///
/// # 参数
/// * `ms` - 需要格式化的总毫秒数。
///
/// # 返回
/// `String` - 格式化后的 LRC 时间标签字符串。
#[must_use]
pub fn format_lrc_time_ms(ms: u64) -> String {
    let minutes = ms / 60000;
    let seconds = (ms % 60000) / 1000;
    let milliseconds = ms % 1000;
    format!("[{minutes:02}:{seconds:02}.{milliseconds:03}]")
}
