// 导入标准库的 Write trait，用于向字符串写入格式化文本
use std::fmt::Write;
// 导入项目中定义的类型：
// ConvertError: 错误处理枚举
// TtmlParagraph: TTML段落结构体，作为歌词数据的主要内部表示
use crate::types::{ConvertError, TtmlParagraph};
// 导入元数据处理器，用于获取和格式化元数据
use crate::metadata_processor::MetadataStore;

/// 从 TTML 段落数据生成 KRC 格式的字符串。
///
/// KRC 格式特点：
/// - 行时间戳：`[行开始绝对时间, 行总持续时间]`
/// - 音节时间戳：`<音节相对行首的偏移时间, 音节持续时间, 0>`
/// - 音节间的空格通常用 `<0,0,0>  ` (两个空格) 表示。
///
/// # Arguments
/// * `paragraphs` - 一个包含 `TtmlParagraph` 结构体的切片，代表歌词的段落。
/// * `metadata_store` - 一个 `MetadataStore` 的引用，用于获取和格式化元数据。
///
/// # Returns
/// `Result<String, ConvertError>` - 如果成功，返回生成的 KRC 字符串；否则返回错误。
pub fn generate_krc_from_ttml_data(
    paragraphs: &[TtmlParagraph],
    metadata_store: &MetadataStore,
) -> Result<String, ConvertError> {
    let mut krc_output = String::new(); // 初始化输出字符串

    // 写入元数据头部，例如 [ti:歌曲名], [ar:歌手名] 等
    // generate_qrc_krc_yrc_metadata_string 方法会从 metadata_store 中提取相关信息并格式化
    // KRC, QRC使用相同的元数据标签格式
    krc_output.push_str(&metadata_store.generate_qrc_krc_metadata_string());

    // 遍历每个 TTML 段落 (通常一个段落对应 KRC 的一行歌词)
    for para in paragraphs {
        // 如果当前段落没有主歌词音节
        if para.main_syllables.is_empty() {
            // 但如果段落本身有明确的开始和结束时间（表示这是一个有持续时间的空行或纯音乐段）
            if para.p_end_ms > para.p_start_ms {
                let line_duration_ms = para.p_end_ms.saturating_sub(para.p_start_ms);
                if line_duration_ms > 0 {
                    // 写入一个只有行时间戳的空行
                    writeln!(krc_output, "[{},{}]", para.p_start_ms, line_duration_ms)?;
                }
            }
            continue; // 处理下一个段落
        }

        // KRC 行的开始时间直接使用段落的 p_start_ms (通常是该行第一个音节的绝对开始时间)
        let line_tag_start_ms = para.p_start_ms;
        // KRC 行的持续时间是段落的 p_end_ms 减去 p_start_ms
        let mut line_tag_duration_ms = para.p_end_ms.saturating_sub(para.p_start_ms);

        // 如果行持续时间为0，但确实有音节内容，则尝试根据音节的实际跨度计算一个持续时间
        if line_tag_duration_ms == 0 && !para.main_syllables.is_empty() {
            let first_syl_abs_start = para.main_syllables.first().map_or(0, |s| s.start_ms);
            let last_syl_abs_end = para.main_syllables.last().map_or(0, |s| s.end_ms);
            let syl_based_duration = last_syl_abs_end.saturating_sub(first_syl_abs_start);
            line_tag_duration_ms = syl_based_duration;
        }

        // 写入行级别的时间戳：[开始时间,持续时间]
        write!(
            krc_output,
            "[{},{}]",
            line_tag_start_ms, line_tag_duration_ms
        )?;

        // 获取该行第一个音节的实际开始时间，用于计算后续音节的相对偏移
        // 如果段落的 p_start_ms 早于第一个音节的 start_ms，则以第一个音节的 start_ms 为基准计算偏移
        let first_syllable_actual_start_ms = para
            .main_syllables
            .first()
            .map_or(line_tag_start_ms, |s| s.start_ms); // 如果没有音节，则使用行开始时间（虽然前面已判断非空）

        let num_main_syllables = para.main_syllables.len();
        // 遍历该行中的所有主音节
        for (idx, ttml_syl) in para.main_syllables.iter().enumerate() {
            // 计算音节相对于行内第一个音节实际开始时间的偏移量
            let syl_offset_ms = ttml_syl
                .start_ms
                .saturating_sub(first_syllable_actual_start_ms);
            // 计算音节的持续时间
            let syl_duration_ms = ttml_syl.end_ms.saturating_sub(ttml_syl.start_ms);
            let effective_text = ttml_syl.text.clone(); // 获取音节文本

            // 只有当音节有文本或有持续时间时才写入
            if !effective_text.is_empty() {
                // 写入音节：<偏移时间,持续时间,0>文本
                // 第三个参数固定为0
                write!(
                    krc_output,
                    "<{},{},0>{}",
                    syl_offset_ms, syl_duration_ms, effective_text
                )?;
            } else if syl_duration_ms > 0 {
                // 如果音节无文本但有持续时间（例如，静默或纯空格音节），也写入时间戳
                write!(krc_output, "<{},{},0>", syl_offset_ms, syl_duration_ms)?;
            }

            // 如果音节后标记需要空格 (ends_with_space is true)
            if ttml_syl.ends_with_space {
                // 并且不是当前行的最后一个音节
                if idx < num_main_syllables - 1 {
                    // 使用 "<0,0,0> " 来表示音节间的空格
                    write!(krc_output, "<0,0,0> ")?;
                }
                // 如果是最后一个音节且 ends_with_space 为 true，KRC 通常不追加空格标记，
                // 因为行尾的空格没有实际显示意义。
            }
        }
        writeln!(krc_output)?; // 每行歌词结束后换行
    }

    // 移除字符串末尾可能多余的换行符
    let final_output = krc_output.trim_end_matches('\n');
    // 如果最终输出为空，则返回空字符串，否则确保末尾有一个换行符
    Ok(if final_output.is_empty() {
        String::new()
    } else {
        format!("{}\n", final_output)
    })
}
