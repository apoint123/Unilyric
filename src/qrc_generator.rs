// 导入标准库的 Write trait，用于向字符串写入格式化文本
use std::fmt::Write;
// 导入项目中定义的类型：
// ConvertError: 错误处理枚举
// LysSyllable: 用于表示音节的结构体 (QRC与LYS, KRC共用此结构体进行中间处理)
// TtmlParagraph: TTML段落结构体，作为歌词数据的主要内部表示
// TtmlSyllable: TTML音节结构体
use crate::types::{ConvertError, LysSyllable, TtmlParagraph, TtmlSyllable};
// 导入元数据处理器，用于获取和格式化元数据
use crate::metadata_processor::MetadataStore;

/// (此函数在当前版本的 qrc_generator.rs 中未被直接调用，但保留以供参考其逻辑)
/// 清理文本两端的括号。
/// 例如，"(背景)" 会变成 "背景"。
#[allow(dead_code)] // 允许未使用此函数，避免编译器警告
fn strip_outer_parentheses(text: &str) -> String {
    let mut char_iter = text.char_indices().peekable(); // 创建字符及其字节索引的迭代器
    let mut start_byte_idx = 0; // 清理后文本的起始字节索引
    let mut end_byte_idx = text.len(); // 清理后文本的结束字节索引

    // 检查第一个字符是否是开括号
    if let Some((_, first_char)) = char_iter.peek()
        && (*first_char == '(' || *first_char == '（')
    {
        // 支持半角和全角括号
        char_iter.next(); // 消耗掉开括号
        // 更新起始索引为开括号之后的字符的索引
        start_byte_idx = char_iter.peek().map_or(text.len(), |(idx, _)| *idx);
    }

    // 检查最后一个字符是否是闭括号
    if !text.is_empty()
        && let Some((idx, char_val)) = text.char_indices().next_back()
    {
        // 从后向前迭代
        if char_val == ')' || char_val == '）' {
            end_byte_idx = idx; // 更新结束索引为闭括号之前的字符的索引
        }
    }

    // 如果起始索引大于或等于结束索引（例如，原文本是 "()" 或 ""），则返回空字符串
    if start_byte_idx >= end_byte_idx {
        String::new()
    } else {
        // 返回去除括号后的子字符串
        text[start_byte_idx..end_byte_idx].to_string()
    }
}

/// (此函数在当前版本的 qrc_generator.rs 中未被直接调用，但保留以供参考其逻辑)
/// 将 TTML 音节转换为内部用于 QRC 生成的 LysSyllable 结构。
/// 主要处理背景音节的括号。
#[allow(dead_code)] // 允许未使用此函数
fn convert_ttml_syllable_to_qrc_syllable_internal(
    ttml_syl: &TtmlSyllable,         //输入的 TTML 音节
    is_background_syllable: bool,    //是否为背景音节
    is_first_bg_syllable: bool,      //是否为当前行第一个背景音节
    is_last_bg_syllable: bool,       //是否为当前行最后一个背景音节
    num_bg_syllables_in_line: usize, //当前行背景音节总数
) -> LysSyllable {
    // 计算音节持续时间
    let duration_ms = ttml_syl.end_ms.saturating_sub(ttml_syl.start_ms);
    let mut current_text = ttml_syl.text.clone(); // 克隆音节文本

    // 如果是背景音节，特殊处理括号
    if is_background_syllable {
        // 先移除已有的外部括号（如果有）
        let cleaned_text = strip_outer_parentheses(&current_text);

        if num_bg_syllables_in_line == 1 {
            // 如果行内只有一个背景音节，则用 () 包裹
            current_text = format!("({cleaned_text})");
        } else if is_first_bg_syllable {
            // 如果是第一个背景音节（且行内不止一个），则在前面加 (
            current_text = format!("({cleaned_text}");
        } else if is_last_bg_syllable {
            // 如果是最后一个背景音节（且行内不止一个），则在后面加 )
            current_text = format!("{cleaned_text})");
        } else {
            // 中间的背景音节，直接使用清理后的文本
            current_text = cleaned_text;
        }
    }

    // 返回 LysSyllable 结构
    LysSyllable {
        text: current_text,
        start_ms: ttml_syl.start_ms, // QRC 音节时间戳是绝对时间
        duration_ms,
    }
}

/// 从 TTML 段落数据生成 QRC 格式的字符串。
///
/// # Arguments
/// * `paragraphs` - 一个包含 `TtmlParagraph` 结构体的切片，代表歌词的段落。
/// * `metadata_store` - 一个 `MetadataStore` 的引用，用于获取和格式化元数据。
///
/// # Returns
/// `Result<String, ConvertError>` - 如果成功，返回生成的 QRC 字符串；否则返回错误。
pub fn generate_qrc_from_ttml_data(
    paragraphs: &[TtmlParagraph],
    metadata_store: &MetadataStore,
) -> Result<String, ConvertError> {
    let mut qrc_output = String::new(); // 初始化输出字符串

    // 写入元数据头部，例如 [ti:歌曲名], [ar:歌手名] 等
    // generate_qrc_krc_yrc_metadata_string 方法会从 metadata_store 中提取相关信息并格式化
    qrc_output.push_str(&metadata_store.generate_qrc_krc_metadata_string());

    // 遍历每个 TTML 段落 (通常一个段落对应 QRC 的一行歌词)
    for para in paragraphs {
        // 处理主歌词音节
        if !para.main_syllables.is_empty() {
            // QRC 行的开始时间是该行第一个音节的开始时间
            let line_start_ms = para.main_syllables.first().unwrap().start_ms;
            // QRC 行的结束时间是该行最后一个音节的结束时间
            let line_end_ms = para.main_syllables.last().unwrap().end_ms;
            // 计算行持续时间
            let line_duration_ms = line_end_ms.saturating_sub(line_start_ms);

            // 写入行级别的时间戳：[开始时间,持续时间]
            write!(qrc_output, "[{line_start_ms},{line_duration_ms}]")?;

            let num_main_syllables = para.main_syllables.len();
            // 遍历该行中的所有主音节
            for (idx, ttml_syl) in para.main_syllables.iter().enumerate() {
                let syl_text = &ttml_syl.text; // 音节文本
                // QRC 音节的开始时间是相对于歌曲开始的绝对时间
                let syl_start_ms_abs = ttml_syl.start_ms;
                // 音节的持续时间
                let syl_duration_ms = ttml_syl.end_ms.saturating_sub(ttml_syl.start_ms);

                // 只有当音节有文本或有持续时间时才写入
                if !syl_text.is_empty() || syl_duration_ms > 0 {
                    // 写入音节：文本(开始时间,持续时间)
                    write!(
                        qrc_output,
                        "{syl_text}({syl_start_ms_abs},{syl_duration_ms})"
                    )?;
                }

                // 如果音节后标记需要空格，并且不是当前行的最后一个音节，则添加一个 (0,0) 时间戳表示空格
                if ttml_syl.ends_with_space && idx < num_main_syllables - 1 {
                    qrc_output.push_str(" (0,0)");
                }
            }
            writeln!(qrc_output)?; // 每行歌词结束后换行
        }

        // 处理背景歌词部分 (如果存在)
        if let Some(bg_section) = &para.background_section
            && !bg_section.syllables.is_empty()
        {
            // 背景歌词也按主歌词的方式处理行时间和音节时间
            let line_start_ms = bg_section.syllables.first().unwrap().start_ms;
            let line_end_ms = bg_section.syllables.last().unwrap().end_ms;
            let line_duration_ms = line_end_ms.saturating_sub(line_start_ms);

            write!(qrc_output, "[{line_start_ms},{line_duration_ms}]")?;

            let num_bg_syllables = bg_section.syllables.len();
            for (idx, ttml_syl_bg) in bg_section.syllables.iter().enumerate() {
                // 背景音节的文本通常已经包含了括号，例如 "(背景词)"
                // 这是由 convert_ttml_syllable_to_qrc_syllable_internal (如果被调用) 或其他转换逻辑处理的
                // 在当前版本的 qrc_generator.rs 中，背景文本的括号应由上游（如TTML解析或转换到TTML的逻辑）处理好。
                let syl_text_bg = &ttml_syl_bg.text;
                let syl_start_ms_abs_bg = ttml_syl_bg.start_ms;
                let syl_duration_ms_bg = ttml_syl_bg.end_ms.saturating_sub(ttml_syl_bg.start_ms);

                if !syl_text_bg.is_empty() || syl_duration_ms_bg > 0 {
                    write!(
                        qrc_output,
                        "{syl_text_bg}({syl_start_ms_abs_bg},{syl_duration_ms_bg})"
                    )?;
                }
                if ttml_syl_bg.ends_with_space && idx < num_bg_syllables - 1 {
                    qrc_output.push_str(" (0,0)");
                }
            }
            writeln!(qrc_output)?; // 背景歌词行结束后换行
        }
    }
    // 移除字符串末尾可能多余的换行符
    let final_output = qrc_output.trim_end_matches('\n');
    // 如果最终输出为空，则返回空字符串，否则确保末尾有一个换行符
    if final_output.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("{final_output}\n"))
    }
}
