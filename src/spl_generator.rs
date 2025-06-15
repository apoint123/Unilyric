// 导入标准库的 Write trait，用于向字符串写入格式化文本
use std::fmt::Write;
// 导入项目中定义的类型：ConvertError (错误处理枚举) 和 TtmlParagraph (TTML段落结构)
use crate::types::{ConvertError, TtmlParagraph};
// 导入元数据处理器 (尽管SPL不使用头部元数据，但函数签名保持一致性)
use crate::metadata_processor::MetadataStore;

/// 将毫秒时间格式化为 SPL 时间戳字符串 `[分:秒.毫秒]` 或 `<分:秒.毫秒>`。
///
/// # Arguments
/// * `total_ms` - 总毫秒数。
/// * `use_angle_brackets` - 布尔值，如果为 `true`，则使用尖括号 `< >`；否则使用方括号 `[ ]`。
///
/// # Returns
/// `String` - 格式化后的 SPL 时间戳字符串。
fn format_spl_timestamp_from_total_ms(total_ms: u64, use_angle_brackets: bool) -> String {
    let millis_part = total_ms % 1000; // 提取毫秒部分
    let total_seconds = total_ms / 1000; // 计算总秒数
    let seconds_part = total_seconds % 60; // 提取秒部分 (0-59)
    let minutes_part = total_seconds / 60; // 提取分钟部分

    // 根据参数选择括号类型
    let open_bracket = if use_angle_brackets { '<' } else { '[' };
    let close_bracket = if use_angle_brackets { '>' } else { ']' };

    // 格式化输出，例如 "[01:23.456]" 或 "<01:23.456>"
    format!("{open_bracket}{minutes_part:02}:{seconds_part:02}.{millis_part:03}{close_bracket}")
}

/// 从 TTML 段落数据生成 SPL (Salt Player Lyric) 格式的字符串。
///
/// # Arguments
/// * `paragraphs` - 一个包含 `TtmlParagraph` 结构体的切片，代表歌词的段落。
/// * `_metadata_store` - `MetadataStore` 的引用。SPL 格式不使用文件头部元数据，
///   因此此参数当前未使用，用 `_` 前缀标记。
///
/// # Returns
/// `Result<String, ConvertError>` - 如果成功，返回生成的 SPL 字符串；否则返回错误。
pub fn generate_spl_from_ttml_data(
    paragraphs: &[TtmlParagraph],
    _metadata_store: &MetadataStore, // SPL不使用头部元数据
) -> Result<String, ConvertError> {
    let mut spl_output = String::new(); // 初始化输出字符串

    // SPL 格式不支持头部元数据标签，所以 _metadata_store 在这里不被使用。

    // 遍历每个 TTML 段落 (通常一个段落对应 SPL 的一行或多行歌词)
    for (para_idx, para) in paragraphs.iter().enumerate() {
        // 检查段落是否包含有效的主歌词音节（有文本或有非零时长）
        let has_main_syllables = !para.main_syllables.is_empty()
            && para
                .main_syllables
                .iter()
                .any(|s| !s.text.trim().is_empty() || s.end_ms > s.start_ms);
        // 检查段落是否有有效的翻译文本
        let has_translation = para
            .translation
            .as_ref()
            .is_some_and(|(t, _)| !t.trim().is_empty());

        // 如果既没有主歌词音节也没有翻译，但段落本身有明确的非零时长，
        // 则输出一个带开始和结束时间戳的空行（表示静默或纯音乐）。
        if !has_main_syllables && !has_translation {
            if para.p_end_ms > para.p_start_ms {
                // 确保有持续时间
                write!(
                    spl_output,
                    "{}",
                    format_spl_timestamp_from_total_ms(para.p_start_ms, false)
                )?;
                // 只有当结束时间确实大于开始时间时才添加结束标记，避免 [ts][ts]
                if para.p_end_ms > para.p_start_ms {
                    write!(
                        spl_output,
                        "{}",
                        format_spl_timestamp_from_total_ms(para.p_end_ms, false)
                    )?;
                }
                writeln!(spl_output)?;
            } else {
                // 如果段落无内容也无时长，记录警告并跳过
                log::warn!("[SPL 生成] 跳过第 {para_idx} 个段落：无主音节、无翻译且无时长。");
            }
            continue; // 处理下一个段落
        }

        // --- 处理主歌词行 ---
        if has_main_syllables {
            // 写入行开始时间戳 (使用方括号)
            let line_start_ts_str = format_spl_timestamp_from_total_ms(para.p_start_ms, false);
            write!(spl_output, "{line_start_ts_str}")?;

            // 判断当前行是否为“卡拉OK行”（即需要逐字时间戳）
            // 一个行被视为卡拉OK行，如果：
            // 1. 它包含多个音节。
            // 2. 或者它只包含一个音节，但该音节的开始/结束时间与整个段落的开始/结束时间不完全吻合，
            //    这意味着音节前后有静默期，需要用逐字时间戳来精确表示。
            let is_karaoke_line = if para.main_syllables.len() > 1 {
                true
            } else if let Some(syl) = para.main_syllables.first() {
                // 检查单个音节是否完全填满段落时间
                syl.start_ms != para.p_start_ms || syl.end_ms != para.p_end_ms
            } else {
                false // 没有音节，不可能是卡拉OK行 (理论上已被 has_main_syllables 过滤)
            };

            if is_karaoke_line {
                // --- 生成卡拉OK行 (带内联逐字时间戳) ---
                // 如果第一个音节不是从段落开始时间立即开始 (有前导静默)
                if let Some(first_syl) = para.main_syllables.first()
                    && first_syl.start_ms > para.p_start_ms
                {
                    // 写入第一个音节的开始时间作为内联时间戳 (使用尖括号)
                    write!(
                        spl_output,
                        "{}",
                        format_spl_timestamp_from_total_ms(first_syl.start_ms, true)
                    )?;
                }

                // 遍历所有音节
                for (s_idx, syl) in para.main_syllables.iter().enumerate() {
                    // 如果音节文本为空但有时长 (例如，一个由空格音节转换来的静默)，
                    // 只写入其结束时间戳作为下一个音节的开始。
                    if syl.text.is_empty() && syl.end_ms > syl.start_ms {
                        write!(
                            spl_output,
                            "{}",
                            format_spl_timestamp_from_total_ms(syl.end_ms, true)
                        )?;
                        continue;
                    }
                    // 写入音节文本
                    write!(spl_output, "{}", syl.text)?;
                    if syl.ends_with_space {
                        // 如果音节后有空格
                        write!(spl_output, " ")?;
                    }

                    // 判断是否为最后一个音节
                    let is_last_syllable_in_para = s_idx == para.main_syllables.len() - 1;
                    if is_last_syllable_in_para {
                        // 如果是最后一个音节，其后的时间戳是整个行的结束时间 (使用方括号)
                        write!(
                            spl_output,
                            "{}",
                            format_spl_timestamp_from_total_ms(para.p_end_ms, false)
                        )?;
                    } else {
                        // 如果不是最后一个音节，其后的时间戳是当前音节的结束时间（也是下一个音节的开始时间，使用尖括号）
                        write!(
                            spl_output,
                            "{}",
                            format_spl_timestamp_from_total_ms(syl.end_ms, true)
                        )?;
                    }
                }
            } else {
                // --- 生成非卡拉OK行 (整行文本，可能带单个行尾结束时间戳) ---
                let mut line_full_text = String::new(); // 用于构建整行文本
                for (s_idx, syl) in para.main_syllables.iter().enumerate() {
                    line_full_text.push_str(&syl.text);
                    // 如果音节后有空格且不是最后一个音节，则添加空格
                    if syl.ends_with_space && s_idx < para.main_syllables.len() - 1 {
                        line_full_text.push(' ');
                    }
                }
                let trimmed_line_full_text = line_full_text.trim_end(); // 去除可能因最后一个音节带空格而产生的行尾空格
                write!(spl_output, "{trimmed_line_full_text}")?;

                // 决定是否需要显式的行尾结束时间戳
                let mut needs_explicit_line_end_tag = false;
                // 行必须有实际内容（文本或时长）才考虑结束标签
                let line_has_substance =
                    !trimmed_line_full_text.is_empty() || (para.p_end_ms > para.p_start_ms);

                if line_has_substance {
                    if para_idx == paragraphs.len() - 1 {
                        // 如果是最后一段歌词，总是需要显式结束时间戳
                        needs_explicit_line_end_tag = true;
                    } else {
                        // 如果不是最后一段，检查其结束时间是否与下一段的开始时间不同
                        let next_para_start_ms = paragraphs[para_idx + 1].p_start_ms;
                        if para.p_end_ms != next_para_start_ms {
                            // 如果时间不连续，则需要显式结束时间戳
                            needs_explicit_line_end_tag = true;
                        }
                        // 如果 para.p_end_ms == next_para_start_ms，则是隐式结尾，不需要标签
                    }
                }

                // 如果确定需要显式结束时间戳，并且行文本非空（避免为纯时间戳空行再加结束戳）
                if needs_explicit_line_end_tag && !trimmed_line_full_text.is_empty() {
                    write!(
                        spl_output,
                        "{}",
                        format_spl_timestamp_from_total_ms(para.p_end_ms, false)
                    )?;
                } else if needs_explicit_line_end_tag
                    && trimmed_line_full_text.is_empty()
                    && (para.p_end_ms > para.p_start_ms)
                {
                    // 特殊情况：如果行文本为空，但行本身有持续时间，并且需要显式结束（例如是最后一行），
                    // 也应该添加结束时间戳，形成如 [start_time][end_time] 的空行。
                    // 检查是否已经有开始时间戳了，避免重复。
                    // format_spl_timestamp_from_total_ms(para.p_start_ms, false) 已经写入。
                    // 如果 start_ms != end_ms，则写入结束时间戳。
                    if para.p_start_ms != para.p_end_ms {
                        write!(
                            spl_output,
                            "{}",
                            format_spl_timestamp_from_total_ms(para.p_end_ms, false)
                        )?;
                    }
                }
            }
            writeln!(spl_output)?; // 主歌词行结束后换行
        }

        // --- 处理翻译行 ---
        if let Some((trans_text, _lang_code)) = &para.translation
            && !trans_text.trim().is_empty()
        {
            // 确保翻译文本非空
            // SPL规范允许多行翻译，如果TTML中的翻译文本包含换行符，则拆分为多行SPL翻译
            let trans_lines = trans_text.split('\n').filter(|s| !s.trim().is_empty());
            for single_trans_line in trans_lines {
                // 每行翻译都使用主歌词行的开始时间戳（同时间戳翻译）
                // 或者，如果主歌词行本身是空的（只有时间），翻译行也只输出时间戳+文本
                if has_main_syllables {
                    // 如果主歌词行有内容
                    writeln!(
                        spl_output,
                        "{}{}",
                        format_spl_timestamp_from_total_ms(para.p_start_ms, false),
                        single_trans_line.trim() // 翻译文本
                    )?;
                } else {
                    // 如果主歌词行是空的（例如只有时间戳的静默行），翻译行也对应这个时间戳
                    writeln!(
                        spl_output,
                        "{}{}",
                        format_spl_timestamp_from_total_ms(para.p_start_ms, false),
                        single_trans_line.trim()
                    )?;
                }
                // SPL规范中，隐式翻译（无时间戳，紧跟主歌词）也是支持的。
                // 当前生成逻辑总是为翻译行添加与主歌词相同的开始时间戳。
                // 如果要生成隐式翻译，这里的逻辑需要调整。
            }
        }
    }

    // 移除字符串末尾可能多余的换行符
    let final_output = spl_output.trim_end_matches('\n');
    // 如果最终输出为空，则返回空字符串，否则确保末尾有一个换行符
    Ok(if final_output.is_empty() {
        String::new()
    } else {
        format!("{final_output}\n")
    })
}
