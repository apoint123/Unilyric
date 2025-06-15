// 导入标准库的 Write trait，用于向字符串写入格式化文本
use std::fmt::Write;
// 导入项目中定义的类型：ConvertError (错误类型) 和 TtmlParagraph (TTML段落结构)
use crate::types::{ConvertError, TtmlParagraph};

/// 从 TTML 段落数据生成 Lyricify Lines (LYL) 格式的字符串。
///
/// LYL 格式为每行 `[开始时间,结束时间]歌词文本`。
/// 此函数会遍历输入的 `TtmlParagraph` 列表，将每个段落（通常代表一行歌词）
/// 转换回 LYL 格式。
///
/// # Arguments
/// * `paragraphs` - 一个包含 `TtmlParagraph` 结构体的切片，代表歌词的段落。
///   元数据（如歌曲名、歌手）不由此函数处理，因为 LYL 格式
///   本身不包含这类头部元数据标签。
///
/// # Returns
/// `Result<String, ConvertError>` - 如果成功，返回生成的 LYL 字符串；否则返回格式化错误。
pub fn generate_from_ttml_data(paragraphs: &[TtmlParagraph]) -> Result<String, ConvertError> {
    let mut output = String::new(); // 初始化输出字符串

    // LYL 文件通常以一个类型声明行开始
    if let Err(e) = writeln!(output, "[type:LyricifyLines]") {
        return Err(ConvertError::Format(e)); // 如果写入失败，返回格式化错误
    }

    // 遍历每个 TTML 段落
    for p in paragraphs {
        // 如果段落没有主歌词音节，则跳过该段落
        if p.main_syllables.is_empty() {
            continue;
        }

        // 从 TTML 段落的主音节列表中构建 LYL 行的完整文本
        // 这会连接所有音节的文本，并处理音节间的空格（如果 TtmlSyllable.ends_with_space 为 true）
        let mut line_text_parts: Vec<String> = Vec::new();
        for (idx, syl) in p.main_syllables.iter().enumerate() {
            line_text_parts.push(syl.text.clone()); // 添加音节文本
            // 如果音节标记其后有空格，并且不是该行的最后一个音节，则添加一个空格
            if syl.ends_with_space && idx < p.main_syllables.len() - 1 {
                line_text_parts.push(" ".to_string());
            }
        }
        // 将所有部分连接起来，并去除可能因最后一个音节是空格而产生的行尾多余空格
        let full_line_text = line_text_parts.join("").trim().to_string();

        // 如果处理后的完整行文本为空（例如，原段落只包含空音节或纯空格音节），则跳过
        // LYL 格式中，即使是空行（只有时间戳），文本部分也应该存在（即使是空字符串）
        // 但如果我们的目标是只输出有实际文本内容的行，这里的 continue 是合适的。
        // 如果需要输出时间戳标记的空行，这里的逻辑可能需要调整为不跳过，
        // 而是直接使用 full_line_text (即使它是空的)。
        // 当前实现：如果文本为空，则不输出该行。
        if full_line_text.is_empty() {
            continue;
        }

        // 获取段落的开始和结束时间
        let start_ms = p.p_start_ms;
        let end_ms = p.p_end_ms;

        // 对时间戳进行一些基本的健全性检查并记录警告
        // 检查1：开始和结束时间都为0，且只有一个音节（可能是无时间信息的静态文本）
        if start_ms == 0
            && end_ms == 0
            && p.main_syllables.len() == 1
            && p.main_syllables[0].text == full_line_text
        {
            log::warn!("[Lyricify Lines 生成] 行 \"{full_line_text}\" 的起始和结束时间均为0");
        }
        // 检查2：结束时间小于或等于开始时间
        if end_ms <= start_ms {
            log::warn!(
                "[Lyricify Lines 生成] 行 \"{full_line_text}\" 的结束时间 {end_ms} 小于或等于开始时间 {start_ms}",
            );
            // 注意：即使时间戳有问题，当前实现仍然会按原样输出它们。
        }

        // 格式化并写入 LYL 行到输出字符串
        // 格式为：[开始时间,结束时间]文本
        if let Err(e) = writeln!(output, "[{start_ms},{end_ms}]{full_line_text}") {
            return Err(ConvertError::Format(e)); // 如果写入失败，返回错误
        }
    }

    Ok(output) // 返回生成的完整 LYL 字符串
}
