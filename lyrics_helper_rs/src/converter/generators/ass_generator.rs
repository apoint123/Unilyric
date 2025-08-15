//! ASS 格式生成器

use std::fmt::Write;

use lyrics_helper_core::{
    AssGenerationOptions, ContentType, ConvertError, LyricLine, LyricTrack, MetadataStore,
    TrackMetadataKey, Word,
};

/// 将毫秒时间格式化为 ASS 时间字符串 `H:MM:SS.CS` (小时:分钟:秒.厘秒)。
fn format_ass_time(ms: u64) -> String {
    let total_cs = (ms + 5) / 10; // 四舍五入到厘秒
    let cs = total_cs % 100;
    let total_seconds = total_cs / 100;
    let seconds = total_seconds % 60;
    let total_minutes = total_seconds / 60;
    let minutes = total_minutes % 60;
    let hours = total_minutes / 60;
    format!("{hours}:{minutes:02}:{seconds:02}.{cs:02}")
}

/// 将毫秒时长四舍五入到厘秒 (cs)，用于 ASS 的 `\k` 标签。
fn round_duration_to_cs(duration_ms: u64) -> u32 {
    let cs = (duration_ms + 5) / 10;
    cs.try_into().unwrap_or(u32::MAX)
}

/// ASS 生成的主入口函数。
pub fn generate_ass(
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
    is_line_timed: bool,
    options: &AssGenerationOptions,
) -> Result<String, ConvertError> {
    let mut ass_content = String::with_capacity(lines.len() * 200 + 1024);

    // --- [Script Info] 部分 ---
    if let Some(custom_script_info) = &options.script_info {
        writeln!(ass_content, "{}", custom_script_info.trim())?;
    } else {
        writeln!(ass_content, "[Script Info]")?;
        writeln!(ass_content, "ScriptType: v4.00+")?;
        writeln!(ass_content, "PlayResX: 1920")?;
        writeln!(ass_content, "PlayResY: 1080")?;
    }
    writeln!(ass_content)?;

    // --- [V4+ Styles] 部分 ---
    if let Some(custom_styles) = &options.styles {
        writeln!(ass_content, "{}", custom_styles.trim())?;
    } else {
        writeln!(ass_content, "[V4+ Styles]")?;
        writeln!(
            ass_content,
            "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding"
        )?;
        writeln!(
            ass_content,
            "Style: Default,Arial,90,&H00FFFFFF,&H000000FF,&H00000000,&H99000000,0,0,0,0,100,100,0,0,1,3.5,1,2,10,10,10,1"
        )?; // 主歌词
        writeln!(
            ass_content,
            "Style: ts,Arial,55,&H00D3D3D3,&H000000FF,&H00000000,&H99000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,50,1"
        )?; // 翻译
        writeln!(
            ass_content,
            "Style: roma,Arial,55,&H00D3D3D3,&H000000FF,&H00000000,&H99000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,50,1"
        )?; // 罗马音
        writeln!(
            ass_content,
            "Style: bg-main,Arial,75,&H00E5E5E5,&H000000FF,&H00000000,&H99000000,0,0,0,0,100,100,0,0,1,3,1,8,10,10,10,1"
        )?; // 背景主歌词
        writeln!(
            ass_content,
            "Style: bg-ts,Arial,45,&H00A0A0A0,&H000000FF,&H00000000,&H99000000,0,0,0,0,100,100,0,0,1,1.5,1,8,10,10,55,1"
        )?; // 背景翻译
        writeln!(
            ass_content,
            "Style: bg-roma,Arial,45,&H00A0A0A0,&H000000FF,&H00000000,&H99000000,0,0,0,0,100,100,0,0,1,1.5,1,8,10,10,55,1"
        )?; // 背景罗马音
        writeln!(
            ass_content,
            "Style: meta,Arial,40,&H00C0C0C0,&H000000FF,&H00000000,&H99000000,0,0,0,0,100,100,0,0,1,1,0,5,10,10,10,1"
        )?; // 元数据
    }
    writeln!(ass_content)?;

    // --- [Events] 部分 ---
    writeln!(ass_content, "[Events]")?;
    writeln!(
        ass_content,
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text"
    )?;

    for (key, values) in metadata_store.get_all_data() {
        for value in values {
            writeln!(
                ass_content,
                "Comment: 0,0:00:00.00,0:00:00.00,meta,,0,0,0,,{key}: {value}"
            )?;
        }
    }

    for line in lines {
        for annotated_track in &line.tracks {
            let is_bg = annotated_track.content_type == ContentType::Background;
            let style = if is_bg { "bg-main" } else { "Default" };
            let mut actor_field = line.agent.clone().unwrap_or_default();
            if is_bg {
                actor_field = "x-bg".to_string();
            } else if let Some(part) = &line.song_part {
                write!(actor_field, r#" itunes:song-part="{part}""#)?;
            }

            write_dialogue_line(
                &mut ass_content,
                line,
                &annotated_track.content,
                style,
                &actor_field,
                is_line_timed,
            )?;

            let trans_style = if is_bg { "bg-ts" } else { "ts" };
            for trans_track in &annotated_track.translations {
                let actor = trans_track
                    .metadata
                    .get(&TrackMetadataKey::Language)
                    .map_or(String::new(), |l| format!("x-lang:{l}"));
                write_dialogue_line(
                    &mut ass_content,
                    line,
                    trans_track,
                    trans_style,
                    &actor,
                    is_line_timed,
                )?;
            }

            let roma_style = if is_bg { "bg-roma" } else { "roma" };
            for roma_track in &annotated_track.romanizations {
                let actor = roma_track
                    .metadata
                    .get(&TrackMetadataKey::Language)
                    .map_or(String::new(), |l| format!("x-lang:{l}"));
                write_dialogue_line(
                    &mut ass_content,
                    line,
                    roma_track,
                    roma_style,
                    &actor,
                    is_line_timed,
                )?;
            }
        }
    }

    Ok(ass_content)
}

fn write_dialogue_line(
    output: &mut String,
    line: &LyricLine,
    track: &LyricTrack,
    style: &str,
    actor: &str,
    is_line_timed: bool,
) -> Result<(), ConvertError> {
    let text_field = if is_line_timed {
        track
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
            .collect::<String>()
            .trim_end()
            .to_string()
    } else {
        build_karaoke_text(&track.words)?
    };

    if !text_field.trim().is_empty() {
        writeln!(
            output,
            "Dialogue: 0,{},{},{},{},0,0,0,,{}",
            format_ass_time(line.start_ms),
            format_ass_time(line.end_ms),
            style,
            actor.trim(),
            text_field
        )?;
    }
    Ok(())
}

/// 辅助函数，构建带 `\k` 标签的文本
fn build_karaoke_text(words: &[Word]) -> Result<String, ConvertError> {
    let syllables: Vec<_> = words.iter().flat_map(|w| &w.syllables).collect();
    if syllables.is_empty() {
        return Ok(String::new());
    }

    let mut text_builder = String::new();
    let mut previous_syllable_end_ms = syllables.first().map_or(0, |s| s.start_ms);

    for syl in syllables {
        // 计算音节间的间隙
        if syl.start_ms > previous_syllable_end_ms {
            let gap_centiseconds = round_duration_to_cs(syl.start_ms - previous_syllable_end_ms);
            if gap_centiseconds > 0 {
                write!(text_builder, "{{\\k{gap_centiseconds}}}")?;
            }
        }

        // 计算音节本身的时长
        let syllable_duration_ms = syl.end_ms.saturating_sub(syl.start_ms);
        let mut syllable_cs = round_duration_to_cs(syllable_duration_ms);
        // 对于非常短的音节，确保其至少有1cs
        if syllable_cs == 0 && syllable_duration_ms > 0 {
            syllable_cs = 1;
        }

        if syllable_cs > 0 {
            write!(text_builder, "{{\\k{syllable_cs}}}")?;
        }

        text_builder.push_str(&syl.text);

        if syl.ends_with_space {
            text_builder.push(' ');
        }
        previous_syllable_end_ms = syl.end_ms;
    }

    Ok(text_builder.trim_end().to_string())
}
