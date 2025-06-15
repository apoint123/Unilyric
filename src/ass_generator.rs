// 导入标准库的 Write trait，用于向字符串写入格式化文本
use std::fmt::Write as FmtWrite;

// 导入项目中定义的类型：
// ConvertError: 错误处理枚举。
// TtmlParagraph: TTML段落结构，作为生成ASS内容的源数据。
use crate::types::{ConvertError, TtmlParagraph};
// 导入元数据处理器，用于从 MetadataStore 生成ASS事件中的元数据注释行。
use crate::metadata_processor::MetadataStore;
// 导入工具函数，例如清理背景文本中的括号。
use crate::utils::clean_parentheses_from_bg_text;

/// 将毫秒时间格式化为 ASS 时间字符串 `H:MM:SS.CS` (小时:分钟:秒.厘秒)。
/// ASS 时间戳精确到厘秒 (cs)。
///
/// # Arguments
/// * `ms` - u64 类型，表示总毫秒数。
///
/// # Returns
/// `String` - 格式化后的 ASS 时间字符串。
pub fn format_ass_time(ms: u64) -> String {
    // 将毫秒转换为总厘秒 (cs)，加5是为了进行四舍五入到最近的厘秒
    let total_cs = (ms + 5) / 10;
    // 提取厘秒部分 (0-99)
    let cs = total_cs % 100;
    // 计算总秒数
    let total_seconds = total_cs / 100;
    // 提取秒部分 (0-59)
    let seconds = total_seconds % 60;
    // 计算总分钟数
    let total_minutes = total_seconds / 60;
    // 提取分钟部分 (0-59)
    let minutes = total_minutes % 60;
    // 计算小时部分
    let hours = total_minutes / 60;
    // 格式化输出，例如 "0:01:23.45"
    format!("{hours}:{minutes:02}:{seconds:02}.{cs:02}")
}

/// 将毫秒时长转换为厘秒 (cs) 时长，用于 ASS 的 `\k` 标签。
/// 同样进行四舍五入。
///
/// # Arguments
/// * `duration_ms` - u64 类型，表示毫秒时长。
///
/// # Returns
/// `u32` - 对应的厘秒时长。
pub fn round_duration_to_cs(duration_ms: u64) -> u32 {
    // 加5后除以10，实现四舍五入到厘秒
    ((duration_ms + 5) / 10) as u32
}

/// 从 `TtmlParagraph` 数据和 `MetadataStore` 生成 ASS (Advanced SubStation Alpha) 格式的字符串。
///
/// # Arguments
/// * `paragraphs` - `Vec<TtmlParagraph>` 类型，包含所有歌词段落的数据。
/// * `metadata_store` - `&MetadataStore` 类型，包含从源文件解析出的元数据。
///
/// # Returns
/// `Result<String, ConvertError>` - 如果成功，返回生成的 ASS 格式字符串；否则返回错误。
pub fn generate_ass(
    paragraphs: Vec<TtmlParagraph>,
    metadata_store: &MetadataStore,
) -> Result<String, ConvertError> {
    // 初始化一个具有预估容量的字符串，以提高性能。
    // 假设每个段落平均生成约150个字符，再加上头部信息约500字符。
    let mut ass_content = String::with_capacity(paragraphs.len() * 150 + 500);

    // --- [Script Info] 部分 ---
    // 写入 ASS 文件脚本信息头。
    writeln!(ass_content, "[Script Info]")?;
    // ScriptType: 指定脚本格式版本，v4.00+ 是通用标准。
    writeln!(ass_content, "ScriptType: v4.00+")?;
    // PlayResX, PlayResY: 定义脚本的逻辑分辨率。字幕的坐标和大小会基于此分辨率。
    // Todo: 从设置加载样式
    writeln!(ass_content, "PlayResX: 1920")?;
    writeln!(ass_content, "PlayResY: 1440")?;
    writeln!(ass_content)?; // [Script Info] 结束后通常有一个空行

    // --- [V4+ Styles] 部分 ---
    // 写入样式定义头。
    writeln!(ass_content, "[V4+ Styles]")?;
    // Format 行定义了 Style 行中各个字段的顺序。
    writeln!(
        ass_content,
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding"
    )?;
    // 预定义一些样式。颜色格式为 &HBBGGRR (蓝绿红)。
    // Default: 默认样式。
    writeln!(
        ass_content,
        "Style: Default,Arial,100,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1"
    )?;
    // orig: 主歌词样式 (白色文字，黑色描边，轻微阴影)。
    writeln!(
        ass_content,
        "Style: orig,Arial,100,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1"
    )?;
    // ts: 翻译歌词样式 (灰色文字)。
    writeln!(
        ass_content,
        "Style: ts,Arial,60,&H00D3D3D3,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,1,2,10,10,60,1"
    )?;
    // roma: 罗马音样式 (与翻译类似)。
    writeln!(
        ass_content,
        "Style: roma,Arial,60,&H00D3D3D3,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,1,2,10,10,60,1"
    )?;
    // bg-ts: 背景人声翻译样式 (更暗的灰色，更靠上)。
    writeln!(
        ass_content,
        "Style: bg-ts,Arial,55,&H00A0A0A0,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,1,2,10,10,80,1"
    )?;
    // bg-roma: 背景人声罗马音样式。
    writeln!(
        ass_content,
        "Style: bg-roma,Arial,55,&H00A0A0A0,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,1,2,10,10,80,1"
    )?;
    // meta: 元数据注释行样式 (小号灰色文字，顶部对齐)。
    writeln!(
        ass_content,
        "Style: meta,Arial,50,&H00C0C0C0,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,1,0,0,0,1"
    )?;
    writeln!(ass_content)?; // [V4+ Styles] 结束后通常有一个空行

    // --- [Events] 部分 ---
    // 写入事件定义头。
    writeln!(ass_content, "[Events]")?;
    // Format 行定义了 Dialogue 或 Comment 行中各个字段的顺序。
    writeln!(
        ass_content,
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text"
    )?;

    // 将 MetadataStore 中存储的元数据作为 ASS Comment 事件写入。
    // 使用 "meta" 样式进行显示。
    ass_content.push_str(&metadata_store.generate_ass_event_comment_metadata_lines("meta"));

    // 遍历所有 TtmlParagraph，为每个段落生成对应的 Dialogue 事件行。
    for para in paragraphs.iter() {
        // --- 处理主歌词 ---
        if !para.main_syllables.is_empty() {
            // 根据 TtmlParagraph 中的 agent 字段确定 Actor (演唱者) 名称。
            // 这是为了在卡拉OK字幕中区分不同演唱者（如果源格式支持）。
            let actor = match para.agent.as_str() {
                "v2" | "V2" => "v2",           // 演唱者2
                "v1000" | "chorus" => "v1000", // 合唱
                "v1" | "V1" | "" => "v1",      // 演唱者1 (默认)
                other_agent => other_agent,    // 其他自定义演唱者名称
            };
            // 获取主歌词行的实际开始和结束时间（基于音节，如果音节时间与段落时间不完全一致）。
            let line_start_ms = para
                .main_syllables
                .first()
                .map_or(para.p_start_ms, |s| s.start_ms);
            let line_end_ms = para
                .main_syllables
                .last()
                .map_or(para.p_end_ms, |s| s.end_ms);
            // 将毫秒时间格式化为 ASS 时间字符串。
            let p_start_ass = format_ass_time(line_start_ms);
            let p_end_ass = format_ass_time(line_end_ms);

            let mut text_builder = String::new(); // 用于构建包含卡拉OK标签的文本。
            let mut previous_syllable_end_ms = line_start_ms; // 用于计算音节间隙。

            // 遍历主歌词的每个音节 (TtmlSyllable)。
            for syl in &para.main_syllables {
                // 如果当前音节的开始时间晚于前一个音节的结束时间，说明存在间隙。
                if syl.start_ms > previous_syllable_end_ms {
                    let gap_ms = syl.start_ms.saturating_sub(previous_syllable_end_ms);
                    if gap_ms > 0 {
                        let gap_cs = round_duration_to_cs(gap_ms);
                        if gap_cs > 0 {
                            // 写入间隙的卡拉OK标签 `{\k<duration_cs>}`，这部分通常是无文字的。
                            write!(text_builder, "{{\\k{gap_cs}}}")?;
                        }
                    }
                }
                // 计算音节的持续时间（毫秒和厘秒）。
                let syl_duration_ms = syl.end_ms.saturating_sub(syl.start_ms);
                let mut syl_duration_cs = round_duration_to_cs(syl_duration_ms);
                // 确保即使时长很短（小于5ms导致四舍五入为0cs），只要大于0ms，至少分配1cs。
                if syl_duration_cs == 0 && syl_duration_ms > 0 {
                    syl_duration_cs = 1;
                }

                if syl_duration_cs > 0 {
                    // 写入音节的卡拉OK标签 `{\k<duration_cs>}`。
                    write!(text_builder, "{{\\k{syl_duration_cs}}}")?;
                    if !syl.text.is_empty() {
                        // 附加音节文本。
                        text_builder.push_str(&syl.text);
                    }
                } else if !syl.text.is_empty() {
                    // 如果音节时长为0cs (例如格式问题)，但有文本，则直接附加文本。
                    text_builder.push_str(&syl.text);
                }

                // 如果音节后标记有空格，则添加一个实际的空格。
                if syl.ends_with_space {
                    text_builder.push(' ');
                }
                previous_syllable_end_ms = syl.end_ms; // 更新前一个音节的结束时间。
            }
            // 移除最终文本末尾可能多余的空格。
            let final_text = text_builder.trim_end().to_string();
            // 只有当最终文本非空时，才写入 Dialogue 行。
            if !final_text.is_empty() {
                writeln!(
                    ass_content,
                    "Dialogue: 0,{p_start_ass},{p_end_ass},orig,{actor},0,0,0,,{final_text}"
                )?;
            }
        }

        // --- 处理背景歌词 (如果存在) ---
        if let Some(ref bg_section) = para.background_section {
            if !bg_section.syllables.is_empty() {
                let actor_bg = "x-bg"; // 背景人声的 Actor 名称，可以自定义。
                // 获取背景歌词行的实际开始和结束时间。
                let bg_line_start_ms = bg_section
                    .syllables
                    .first()
                    .map_or(bg_section.start_ms, |s| s.start_ms);
                let bg_line_end_ms = bg_section
                    .syllables
                    .last()
                    .map_or(bg_section.end_ms, |s| s.end_ms);
                let bg_start_ass = format_ass_time(bg_line_start_ms);
                let bg_end_ass = format_ass_time(bg_line_end_ms);
                let mut text_builder_bg = String::new(); // 用于构建背景歌词的卡拉OK文本。
                let mut previous_syllable_end_ms_bg = bg_line_start_ms;

                // 遍历背景歌词的每个音节。
                for syl_bg in &bg_section.syllables {
                    // 处理音节间隙，同主歌词逻辑。
                    if syl_bg.start_ms > previous_syllable_end_ms_bg {
                        let gap_ms_bg = syl_bg.start_ms.saturating_sub(previous_syllable_end_ms_bg);
                        if gap_ms_bg > 0 {
                            let gap_cs_bg = round_duration_to_cs(gap_ms_bg);
                            if gap_cs_bg > 0 {
                                write!(text_builder_bg, "{{\\k{gap_cs_bg}}}")?;
                            }
                        }
                    }
                    // 处理音节时长和文本，同主歌词逻辑。
                    let syl_duration_ms_bg = syl_bg.end_ms.saturating_sub(syl_bg.start_ms);
                    let mut syl_duration_cs_bg = round_duration_to_cs(syl_duration_ms_bg);
                    if syl_duration_cs_bg == 0 && syl_duration_ms_bg > 0 {
                        syl_duration_cs_bg = 1;
                    }

                    // 清理背景音节文本中的括号（通常背景音是 (la la la) 形式）。
                    let cleaned_text_bg = clean_parentheses_from_bg_text(&syl_bg.text);

                    if syl_duration_cs_bg > 0 {
                        write!(text_builder_bg, "{{\\k{syl_duration_cs_bg}}}")?;
                        if !cleaned_text_bg.is_empty() {
                            text_builder_bg.push_str(&cleaned_text_bg);
                        }
                    } else if !cleaned_text_bg.is_empty() {
                        text_builder_bg.push_str(&cleaned_text_bg);
                    }

                    if syl_bg.ends_with_space {
                        text_builder_bg.push(' ');
                    }
                    previous_syllable_end_ms_bg = syl_bg.end_ms;
                }
                let final_text_bg = text_builder_bg.trim_end().to_string();
                if !final_text_bg.is_empty() {
                    // 使用 "orig" 样式（或其他专用背景样式）写入背景歌词 Dialogue 行。
                    writeln!(
                        ass_content,
                        "Dialogue: 0,{bg_start_ass},{bg_end_ass},orig,{actor_bg},0,0,0,,{final_text_bg}"
                    )?;
                }
            }
            // --- 处理背景部分的翻译和罗马音 (如果作为独立行输出) ---
            // 如果背景人声有翻译文本。
            if let Some((text, lang_code_opt)) = &bg_section.translation
                && !text.is_empty()
            {
                // Actor 字段可以用来携带语言代码信息，例如 "x-lang:en"。
                let actor_display = lang_code_opt
                    .as_ref()
                    .map_or_else(String::new, |c| format!("x-lang:{c}"));
                // 使用 "bg-ts" 样式写入背景翻译。
                writeln!(
                    ass_content,
                    "Dialogue: 0,{},{},bg-ts,{},0,0,0,,{}",
                    format_ass_time(bg_section.start_ms),
                    format_ass_time(bg_section.end_ms),
                    actor_display,
                    text
                )?;
            }
            // 如果背景人声有罗马音文本。
            if let Some(text) = &bg_section.romanization
                && !text.is_empty()
            {
                // 使用 "bg-roma" 样式写入背景罗马音。Actor 字段为空。
                writeln!(
                    ass_content,
                    "Dialogue: 0,{},{},bg-roma,,0,0,0,,{}",
                    format_ass_time(bg_section.start_ms),
                    format_ass_time(bg_section.end_ms),
                    text
                )?;
            }
        }

        // --- 处理主歌词的翻译和罗马音行 ---
        // 如果段落有翻译文本。
        if let Some((text, lang_code_opt)) = &para.translation
            && !text.is_empty()
        {
            let actor_display = lang_code_opt
                .as_ref()
                .map_or_else(String::new, |c| format!("x-lang:{c}"));
            // 使用 "ts" 样式写入主翻译。时间为整个段落的 p_start_ms 到 p_end_ms。
            writeln!(
                ass_content,
                "Dialogue: 0,{},{},ts,{},0,0,0,,{}",
                format_ass_time(para.p_start_ms),
                format_ass_time(para.p_end_ms),
                actor_display,
                text
            )?;
        }
        // 如果段落有罗马音文本。
        if let Some(text) = &para.romanization
            && !text.is_empty()
        {
            // 使用 "roma" 样式写入主罗马音。
            writeln!(
                ass_content,
                "Dialogue: 0,{},{},roma,,0,0,0,,{}",
                format_ass_time(para.p_start_ms),
                format_ass_time(para.p_end_ms),
                text
            )?;
        }
    } // 结束段落遍历
    Ok(ass_content) // 返回生成的完整 ASS 字符串
}
