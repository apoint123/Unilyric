//! ASS 格式生成器

use std::fmt::Write;

use lyrics_helper_core::{
    AgentStore, AssGenerationOptions, ContentType, ConvertError, LyricLine, LyricSyllable,
    LyricTrack, MetadataStore, TrackMetadataKey,
};

/// ASS 生成的主入口函数。
pub fn generate_ass(
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
    agents: &AgentStore,
    is_line_timed: bool,
    options: &AssGenerationOptions,
) -> Result<String, ConvertError> {
    let mut ass_content = String::with_capacity(lines.len() * 200 + 1024);

    write_ass_header(&mut ass_content, options)?;

    write_ass_events(
        &mut ass_content,
        lines,
        metadata_store,
        agents,
        is_line_timed,
    )?;

    Ok(ass_content)
}

fn write_ass_header(
    output: &mut String,
    options: &AssGenerationOptions,
) -> Result<(), ConvertError> {
    // --- [Script Info] 部分 ---
    if let Some(custom_script_info) = &options.script_info {
        writeln!(output, "{}", custom_script_info.trim())?;
    } else {
        writeln!(output, "[Script Info]")?;
        writeln!(output, "ScriptType: v4.00+")?;
        writeln!(output, "PlayResX: 1920")?;
        writeln!(output, "PlayResY: 1080")?;
    }
    writeln!(output)?;

    // --- [V4+ Styles] 部分 ---
    if let Some(custom_styles) = &options.styles {
        writeln!(output, "{}", custom_styles.trim())?;
    } else {
        writeln!(output, "[V4+ Styles]")?;
        writeln!(
            output,
            "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding"
        )?;
        writeln!(
            output,
            "Style: Default,Arial,100,&H00FFFFFF,&H003F3F3F,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1"
        )?; // 主歌词
        writeln!(
            output,
            "Style: Orig,Arial,100,&H00FFFFFF,&H003F3F3F,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1"
        )?; // 主歌词 (旧)
        writeln!(
            output,
            "Style: ts,Arial,55,&H00D3D3D3,&H000000FF,&H00000000,&H99000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,50,1"
        )?; // 翻译
        writeln!(
            output,
            "Style: roma,Arial,55,&H00D3D3D3,&H000000FF,&H00000000,&H99000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,50,1"
        )?; // 罗马音
        writeln!(
            output,
            "Style: bg-ts,Arial,45,&H00A0A0A0,&H000000FF,&H00000000,&H99000000,0,0,0,0,100,100,0,0,1,1.5,1,8,10,10,55,1"
        )?; // 背景翻译
        writeln!(
            output,
            "Style: bg-roma,Arial,45,&H00A0A0A0,&H000000FF,&H00000000,&H99000000,0,0,0,0,100,100,0,0,1,1.5,1,8,10,10,55,1"
        )?; // 背景罗马音
        writeln!(
            output,
            "Style: meta,Arial,40,&H00C0C0C0,&H000000FF,&H00000000,&H99000000,0,0,0,0,100,100,0,0,1,1,0,5,10,10,10,1"
        )?; // 元数据
    }
    writeln!(output)?;

    Ok(())
}

fn write_ass_events(
    output: &mut String,
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
    agents: &AgentStore,
    is_line_timed: bool,
) -> Result<(), ConvertError> {
    writeln!(output, "[Events]")?;
    writeln!(
        output,
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text"
    )?;

    for (key, values) in metadata_store.get_all_data() {
        for value in values {
            writeln!(
                output,
                "Comment: 0,0:00:00.00,0:00:00.00,meta,,0,0,0,,{key}: {value}"
            )?;
        }
    }

    for agent in agents.all_agents() {
        if let Some(name) = &agent.name
            && !name.is_empty()
        {
            writeln!(
                output,
                "Comment: 0,0:00:00.00,0:00:00.00,meta,,0,0,0,,{}: {}",
                agent.id, name
            )?;
        }
    }

    for line in lines {
        write_events_for_line(output, line, is_line_timed)?;
    }

    Ok(())
}

fn write_events_for_line(
    output: &mut String,
    line: &LyricLine,
    is_line_timed: bool,
) -> Result<(), ConvertError> {
    for annotated_track in &line.tracks {
        let is_bg = annotated_track.content_type == ContentType::Background;
        let style = "Default";
        let mut actor_field = line.agent.clone().unwrap_or_else(|| "v1".to_string());

        if is_bg {
            actor_field = "x-bg".to_string();
        } else if let Some(part) = &line.song_part {
            write!(actor_field, r#" itunes:song-part="{part}""#)?;
        }

        let track_start_ms;
        let track_end_ms;

        let syllables: Vec<&LyricSyllable> = annotated_track
            .content
            .words
            .iter()
            .flat_map(|w| &w.syllables)
            .collect();

        if syllables.is_empty() {
            track_start_ms = line.start_ms;
            track_end_ms = line.end_ms;
        } else {
            track_start_ms = syllables.first().unwrap().start_ms;
            track_end_ms = syllables.last().unwrap().end_ms;
        }

        write_dialogue_line(
            output,
            track_start_ms,
            track_end_ms,
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
                output,
                track_start_ms,
                track_end_ms,
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
                output,
                track_start_ms,
                track_end_ms,
                roma_track,
                roma_style,
                &actor,
                is_line_timed,
            )?;
        }
    }
    Ok(())
}

fn write_dialogue_line(
    output: &mut String,
    start_ms: u64,
    end_ms: u64,
    track: &LyricTrack,
    style: &str,
    actor: &str,
    is_line_timed: bool,
) -> Result<(), ConvertError> {
    let text_field = if is_line_timed {
        track.text()
    } else {
        let syllables: Vec<&LyricSyllable> =
            track.words.iter().flat_map(|w| &w.syllables).collect();
        build_karaoke_text(&syllables)?
    };

    if !text_field.trim().is_empty() {
        writeln!(
            output,
            "Dialogue: 0,{},{},{},{},0,0,0,,{}",
            format_ass_time(start_ms),
            format_ass_time(end_ms),
            style,
            actor.trim(),
            text_field
        )?;
    }
    Ok(())
}

/// 辅助函数，构建带 `\k` 标签的文本
fn build_karaoke_text(syllables: &[&LyricSyllable]) -> Result<String, ConvertError> {
    if syllables.is_empty() {
        return Ok(String::new());
    }

    let mut text_builder = String::new();
    let mut previous_syllable_end_ms = syllables.first().map_or(0, |s| s.start_ms);

    for &syl in syllables {
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

fn round_duration_to_cs(duration_ms: u64) -> u32 {
    let cs = (duration_ms + 5) / 10;
    cs.try_into().unwrap_or(u32::MAX)
}
