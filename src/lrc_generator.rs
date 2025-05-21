use crate::metadata_processor::MetadataStore;
use crate::types::{ConvertError, LrcContentType, LrcLine, TtmlParagraph};
use std::fmt::Write as FmtWrite; // 用于向 String 写入格式化文本 // 用于主歌词LRC的元数据生成

/// 从 TtmlParagraph 列表生成翻译或罗马音的 LRC 格式字符串。
///
/// # Arguments
/// * `paragraphs` - 一个包含 TtmlParagraph 结构体的切片，代表歌词的段落。
/// * `content_type` - 指示要生成的内容类型（翻译或罗马音）。
/// * `_metadata_store` - 元数据存储（当前在此函数中未使用，保留供未来扩展）。
///
/// # Returns
/// Result<String, ConvertError> - 成功时返回生成的LRC字符串，失败时返回错误。
pub fn generate_lrc_from_paragraphs(
    paragraphs: &[TtmlParagraph],
    content_type: LrcContentType,
) -> Result<String, ConvertError> {
    let mut lrc_lines: Vec<LrcLine> = Vec::new(); // 用于收集所有LRC行（包括主歌词和背景）

    for p in paragraphs {
        match content_type {
            LrcContentType::Translation => {
                // 1. 处理主歌词的翻译
                // 如果存在翻译字段（即使文本为空或仅空格），则收集它
                if let Some((text, _lang_code)) = &p.translation {
                    lrc_lines.push(LrcLine {
                        timestamp_ms: p.p_start_ms,    // 主歌词翻译使用段落的起始时间
                        text: text.trim().to_string(), // 存储trim后的文本，可能为空
                    });
                }

                // 2. 处理背景的翻译
                if let Some(bg_section) = &p.background_section {
                    if let Some((bg_text, _bg_lang_code)) = &bg_section.translation {
                        lrc_lines.push(LrcLine {
                            timestamp_ms: bg_section.start_ms, // 背景翻译使用背景段落的起始时间
                            text: bg_text.trim().to_string(),  // 存储trim后的文本，可能为空
                        });
                    }
                }
            }
            LrcContentType::Romanization => {
                // 1. 处理主歌词的罗马音
                if let Some(roma_text) = &p.romanization {
                    lrc_lines.push(LrcLine {
                        timestamp_ms: p.p_start_ms,
                        text: roma_text.trim().to_string(), // 存储trim后的文本，可能为空
                    });
                }
                // 2. 处理背景的罗马音
                if let Some(bg_section) = &p.background_section {
                    if let Some(bg_roma_text) = &bg_section.romanization {
                        lrc_lines.push(LrcLine {
                            timestamp_ms: bg_section.start_ms,
                            text: bg_roma_text.trim().to_string(), // 存储trim后的文本，可能为空
                        });
                    }
                }
            }
        }
    }

    // 按时间戳对所有收集到的LRC行进行排序
    lrc_lines.sort_by_key(|line| line.timestamp_ms);

    // 移除时间戳重复的行。
    // `dedup_by_key` 会保留每个重复时间戳的第一个遇到的LrcLine。
    // 如果在同一时间戳，主歌词翻译和背景翻译都被收集（可能一个是空文本，一个不是），
    // 这里的去重逻辑会保留先被收集到的那个。
    // 如果需要更复杂的合并逻辑（例如，如果一个有文本一个为空，则保留有文本的），
    // 则此处的去重逻辑需要调整。
    lrc_lines.dedup_by_key(|line| line.timestamp_ms);

    let mut lrc_output = String::new();
    for line in lrc_lines {
        let time_str = crate::utils::format_lrc_time_ms(line.timestamp_ms);
        writeln!(lrc_output, "{}{}", time_str, line.text)?;
    }

    // 清理末尾可能多余的换行符
    let trimmed_output = lrc_output.trim_end_matches('\n');
    // 如果trimmed_output为空（例如，没有任何有效的翻译或罗马音行被收集和输出），
    // 则返回一个空字符串，而不是一个单独的换行符。
    if trimmed_output.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("{}\n", trimmed_output))
    }
}

/// 从 TtmlParagraph 列表生成主歌词的 LRC 格式字符串。
///
/// # Arguments
/// * `paragraphs` - 包含 TtmlParagraph 结构体的切片。
/// * `metadata_store` - 包含元数据，用于在LRC文件头部生成元数据标签。
///
/// # Returns
/// Result<String, ConvertError> - 成功时返回生成的LRC字符串，失败时返回错误。
pub fn generate_main_lrc_from_paragraphs(
    paragraphs: &[TtmlParagraph],
    metadata_store: &MetadataStore,
) -> Result<String, ConvertError> {
    let mut lrc_output = String::new();
    let mut previous_line_end_ms: Option<u64> = None;

    // 写入LRC元数据标签 (如 [ti:], [ar:], [al:])
    lrc_output.push_str(&metadata_store.generate_lrc_metadata_string());

    // 过滤掉那些实际上没有可显示文本内容的段落
    let relevant_paragraphs: Vec<&TtmlParagraph> = paragraphs
        .iter()
        .filter(|p| {
            // 只有当音节列表非空，并且组合音节文本trim后非空时，才认为是相关段落
            !p.main_syllables.is_empty() && {
                let line_text = p
                    .main_syllables
                    .iter()
                    .enumerate()
                    .map(|(idx, syl)| {
                        if syl.ends_with_space && idx < p.main_syllables.len() - 1 {
                            format!("{} ", syl.text) // 如果音节后有空格且不是最后一个音节，则附加空格
                        } else {
                            syl.text.clone()
                        }
                    })
                    .collect::<String>();
                !line_text.trim().is_empty()
            }
        })
        .collect();

    for p in relevant_paragraphs {
        let current_line_start_ms = p.p_start_ms;

        // 在两行歌词之间如果间隔过长（例如超过5秒），插入一个空行时间戳
        if let Some(prev_end_ms) = previous_line_end_ms {
            if current_line_start_ms > prev_end_ms {
                let gap_ms = current_line_start_ms.saturating_sub(prev_end_ms);
                const LONG_GAP_THRESHOLD_MS: u64 = 5000; // 5秒阈值
                if gap_ms > LONG_GAP_THRESHOLD_MS {
                    // 使用前一行的结束时间或当前行开始时间减去一个微小量作为空行时间戳
                    let blank_line_time_str = crate::utils::format_lrc_time_ms(prev_end_ms);
                    writeln!(lrc_output, "{}", blank_line_time_str)?;
                }
            }
        }

        // 从 main_syllables 构建LRC行文本
        let full_line_text = p
            .main_syllables
            .iter()
            .enumerate()
            .map(|(idx, syl)| {
                if syl.ends_with_space && idx < p.main_syllables.len() - 1 {
                    format!("{} ", syl.text)
                } else {
                    syl.text.clone()
                }
            })
            .collect::<String>()
            .trim() // 对整行文本进行trim，移除可能因最后一个音节是空格导致的多余行尾空格
            .to_string();

        let time_str = crate::utils::format_lrc_time_ms(current_line_start_ms);
        writeln!(lrc_output, "{}{}", time_str, full_line_text)?;

        // 更新上一行的结束时间，使用段落的 p_end_ms
        previous_line_end_ms = Some(p.p_end_ms);
    }

    // 清理末尾可能多余的换行符，并确保输出以换行符结尾
    let trimmed_output = lrc_output.trim_end_matches('\n');
    Ok(format!("{}\n", trimmed_output))
}
