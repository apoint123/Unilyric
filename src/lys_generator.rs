// 导入标准库的 Write trait，用于向字符串写入格式化文本
use std::fmt::Write;
// 导入项目中定义的类型：
// ConvertError: 错误处理枚举
// TtmlParagraph: TTML段落结构体，作为歌词数据的主要内部表示
use crate::types::{ConvertError, TtmlParagraph};
// 导入元数据处理器，用于获取和格式化元数据
use crate::metadata_processor::MetadataStore;

// 定义 LYS 行属性常量，用于根据 TTML 段落的 agent 和背景信息选择合适的属性值
// 这些常量代表 LYS 文件中行首 `[属性]` 的数字含义。
// 0: 未设置，非背景 (通常映射到主唱1)
// 4: 左/主唱1, 非背景
// 5: 右/主唱2, 非背景
// 7: 左/主唱1, 背景和声
// 8: 右/主唱2, 背景和声
const LYS_PROPERTY_UNSET: u8 = 0; // 对应默认情况或无法精确匹配的 agent
const LYS_PROPERTY_NO_BACK_LEFT: u8 = 4; // 主唱1，非背景
const LYS_PROPERTY_NO_BACK_RIGHT: u8 = 5; // 主唱2，非背景
const LYS_PROPERTY_BACK_LEFT: u8 = 7; // 主唱1的背景
const LYS_PROPERTY_BACK_RIGHT: u8 = 8; // 主唱2的背景

/// 从 TTML 段落数据生成 LYS (Lyricify Syllable) 格式的字符串。
///
/// LYS 格式特点：
/// - 行以属性标记 `[property]` 开头。
/// - 音节格式为 `文本(绝对开始时间,持续时间)`。
/// - 音节间的空格用 ` (0,0)` 表示。
///
/// # Arguments
/// * `paragraphs` - 一个包含 `TtmlParagraph` 结构体的切片，代表歌词的段落。
/// * `metadata_store` - 一个 `MetadataStore` 的引用，用于获取和格式化元数据。
/// * `include_metadata` - 布尔值，指示是否在输出的 LYS 字符串头部包含元数据。
///
/// # Returns
/// `Result<String, ConvertError>` - 如果成功，返回生成的 LYS 字符串；否则返回错误。
pub fn generate_lys_from_ttml_data(
    paragraphs: &[TtmlParagraph],
    metadata_store: &MetadataStore,
    include_metadata: bool, // 控制是否输出元数据头部
) -> Result<String, ConvertError> {
    let mut lys_output = String::new(); // 初始化输出字符串

    // 如果需要，写入元数据头部
    if include_metadata {
        // generate_lys_metadata_string 方法会从 metadata_store 中提取元数据
        // 并将其格式化为 LYS 文件头部所需的 [key:value] 标签。
        lys_output.push_str(&metadata_store.generate_lys_metadata_string());
    }

    // 遍历每个 TTML 段落 (通常一个段落对应 LYS 的一行或两行歌词：主歌词+背景)
    for para in paragraphs.iter() {
        // --- 处理主歌词音节 ---
        if !para.main_syllables.is_empty() {
            // 根据 TTML 段落的 agent 属性确定 LYS 行属性
            // LYS 属性 0, 1, 4 通常对应主唱1 (v1)
            // LYS 属性 2, 5 通常对应主唱2 (v2)
            // LYS 属性 3 (NO_BACK_UNSET) 较少见，这里映射到 UNSET
            // 这里选择 NO_BACK_LEFT/RIGHT 作为非背景主唱的默认属性
            let property = match para.agent.as_str() {
                "v1" | "V1" => LYS_PROPERTY_NO_BACK_LEFT,        // 主唱1
                "v2" | "V2" => LYS_PROPERTY_NO_BACK_RIGHT,       // 主唱2
                "v1000" | "chorus" => LYS_PROPERTY_NO_BACK_LEFT, // 合唱也视为v1
                _ => LYS_PROPERTY_UNSET,                         // 其他或空 agent 默认为 0 (主唱1)
            };
            // 写入行属性标记
            write!(lys_output, "[{property}]")?;

            let num_main_syllables = para.main_syllables.len();
            // 遍历该行的所有主音节
            for (idx, ttml_syl) in para.main_syllables.iter().enumerate() {
                let syl_text = &ttml_syl.text; // 音节文本
                // LYS 音节的第一个时间参数是绝对开始时间
                let syl_start_ms = ttml_syl.start_ms;
                // 音节的持续时间
                let syl_duration_ms = ttml_syl.end_ms.saturating_sub(ttml_syl.start_ms);

                // 特殊处理：如果音节文本是单个空格且时长为0，这通常是用于表示音节间空格的 (0,0) 标记
                if syl_text == " " && syl_duration_ms == 0 {
                    write!(lys_output, " (0,0)")?; // 直接写入 LYS 的空格表示
                }
                // 否则，只有当音节有文本或有持续时间时才写入
                else if !syl_text.is_empty() || syl_duration_ms > 0 {
                    // 写入音节：文本(绝对开始时间,持续时间)
                    write!(lys_output, "{syl_text}({syl_start_ms},{syl_duration_ms})")?;

                    // 如果音节后标记需要空格 (ends_with_space is true)
                    // 并且不是当前行的最后一个音节
                    if ttml_syl.ends_with_space && (idx < num_main_syllables - 1) {
                        // 检查下一个音节是否已经是显式的 (0,0) 空格标记
                        let mut already_has_explicit_space_after = false;
                        if idx + 1 < num_main_syllables {
                            let next_syl = &para.main_syllables[idx + 1];
                            if next_syl.text == " "
                                && next_syl.end_ms.saturating_sub(next_syl.start_ms) == 0
                            {
                                already_has_explicit_space_after = true;
                            }
                        }
                        // 如果下一个音节不是显式空格，则添加一个 (0,0) 来表示当前音节后的空格
                        if !already_has_explicit_space_after {
                            write!(lys_output, " (0,0)")?;
                        }
                    }
                }
            }
            writeln!(lys_output)?; // 主歌词行结束后换行
        }

        // --- 处理背景歌词部分 (如果存在) ---
        if let Some(bg_section) = &para.background_section
            && !bg_section.syllables.is_empty()
        {
            // 背景歌词的属性通常也基于主歌词的 agent，但使用表示“背景”的属性值
            let bg_property = match para.agent.as_str() {
                "v1" | "V1" | "v1000" | "chorus" | "" => LYS_PROPERTY_BACK_LEFT, // 主唱1的背景
                "v2" | "V2" => LYS_PROPERTY_BACK_RIGHT,                          // 主唱2的背景
                _ => LYS_PROPERTY_BACK_LEFT, // 其他情况默认背景为左 (主唱1)
            };
            write!(lys_output, "[{bg_property}]")?; // 写入背景行属性

            let num_bg_syllables = bg_section.syllables.len();
            // 遍历背景音节
            for (idx, ttml_syl_bg) in bg_section.syllables.iter().enumerate() {
                let syl_text_bg = &ttml_syl_bg.text; // 背景音节文本
                let syl_start_ms_bg = ttml_syl_bg.start_ms; // 绝对开始时间
                let syl_duration_ms_bg = ttml_syl_bg.end_ms.saturating_sub(ttml_syl_bg.start_ms); // 持续时间

                // 与主歌词音节类似的处理逻辑
                if syl_text_bg == " " && syl_duration_ms_bg == 0 {
                    write!(lys_output, " (0,0)")?;
                } else if !syl_text_bg.is_empty() || syl_duration_ms_bg > 0 {
                    write!(
                        lys_output,
                        "{syl_text_bg}({syl_start_ms_bg},{syl_duration_ms_bg})"
                    )?;
                    if ttml_syl_bg.ends_with_space && (idx < num_bg_syllables - 1) {
                        let mut already_has_explicit_space_after_bg = false;
                        if idx + 1 < num_bg_syllables {
                            let next_syl_bg = &bg_section.syllables[idx + 1];
                            if next_syl_bg.text == " "
                                && next_syl_bg.end_ms.saturating_sub(next_syl_bg.start_ms) == 0
                            {
                                already_has_explicit_space_after_bg = true;
                            }
                        }
                        if !already_has_explicit_space_after_bg {
                            write!(lys_output, " (0,0)")?;
                        }
                    }
                }
            }
            writeln!(lys_output)?; // 背景歌词行结束后换行
        }
    }

    // 移除字符串末尾可能多余的换行符
    let trimmed_output = lys_output.trim_end_matches('\n');
    // 如果最终输出为空，则返回空字符串，否则确保末尾有一个换行符
    Ok(if trimmed_output.is_empty() {
        String::new()
    } else {
        format!("{trimmed_output}\n")
    })
}
