// 导入项目中定义的类型：
// AssMetadata: 用于存储元数据（虽然QRC到TTML的转换通常直接传递元数据，不在此模块修改）
// ConvertError: 错误处理枚举
// QrcLine: QRC解析器输出的单行歌词数据结构
// TtmlParagraph: 目标中间数据结构，表示一个歌词段落（通常对应一行歌词）
use crate::types::{AssMetadata, ConvertError, QrcLine, TtmlParagraph};
// 导入工具函数：
// process_parsed_syllables_to_ttml: 将原始音节列表（如LysSyllable）转换为TTML音节列表（TtmlSyllable），处理文本规范化等。
// post_process_ttml_syllable_line_spacing: 对整行TTML音节进行后处理，如移除行首尾多余空格音节。
use crate::utils::{post_process_ttml_syllable_line_spacing, process_parsed_syllables_to_ttml};

/// 将解析后的 QRC 数据转换为 TTML 段落数据。
///
/// QRC 格式的音节时间戳是绝对时间，`LysSyllable` 中的 `start_ms` 已经是绝对时间。
/// `process_parsed_syllables_to_ttml` 会将这些 `LysSyllable` 转换为 `TtmlSyllable`，
/// 其中 `TtmlSyllable` 的 `start_ms` 和 `end_ms` 也是绝对时间。
///
/// # Arguments
/// * `qrc_lines` - 一个包含 `QrcLine` 结构体的切片，代表从QRC文件解析出的所有歌词行。
/// * `qrc_metadata` - 从QRC文件解析出的元数据列表。此函数通常直接传递这些元数据，不作修改。
///
/// # Returns
/// `Result<(Vec<TtmlParagraph>, Vec<AssMetadata>), ConvertError>` -
/// 如果成功，返回一个元组，包含转换后的 TTML 段落列表和原始元数据列表；否则返回错误。
pub fn convert_qrc_to_ttml_data(
    qrc_lines: &[QrcLine],
    qrc_metadata: Vec<AssMetadata>,
) -> Result<(Vec<TtmlParagraph>, Vec<AssMetadata>), ConvertError> {
    let mut ttml_paragraphs: Vec<TtmlParagraph> = Vec::new(); // 初始化用于存储结果的 TTML 段落列表

    // 遍历从 QRC 解析器得到的每一行 QrcLine
    for qrc_line in qrc_lines.iter() {
        // 将当前 QrcLine 中的原始音节列表 (qrc_line.syllables，类型为 Vec<LysSyllable>)
        // 转换为 TTML 音节列表 (Vec<TtmlSyllable>)。
        // "QRC" 作为源格式提示传递给工具函数。
        // process_parsed_syllables_to_ttml 会处理音节文本的初步规范化，
        // 例如，将 QRC 中的 `(0,0)` 空格音节转换为包含单个空格字符的 TtmlSyllable。
        let mut current_ttml_syllables =
            process_parsed_syllables_to_ttml(&qrc_line.syllables, "QRC");

        // 对转换后的 TTML 音节列表进行行级别的后处理。
        // post_process_ttml_syllable_line_spacing 会移除行首的空文本音节，
        // 并确保行尾最后一个有意义的音节其 ends_with_space 标志为 false，
        // 同时移除最后一个有意义音节之后的所有空文本音节。
        post_process_ttml_syllable_line_spacing(&mut current_ttml_syllables);

        // 只有当处理后的音节列表不为空，或者原始 QRC 行本身有明确的持续时间时，
        // 才创建并添加 TtmlParagraph。
        // qrc_line.line_duration_ms > 0 这个条件确保即使一行歌词没有逐字音节，
        // 也能在TTML中表示为一个有时间的段落。
        if !current_ttml_syllables.is_empty() || qrc_line.line_duration_ms > 0 {
            let paragraph = TtmlParagraph {
                // 段落的开始时间直接使用 QRC 行的开始时间
                p_start_ms: qrc_line.line_start_ms,
                // 段落的结束时间是 QRC 行的开始时间加上其持续时间
                p_end_ms: qrc_line
                    .line_start_ms
                    .saturating_add(qrc_line.line_duration_ms),
                // 默认的演唱者信息，通常用于TTML中的 ttm:agent 属性
                agent: "v1".to_string(),
                // 存储处理和后处理过的音节列表
                main_syllables: current_ttml_syllables,
                // QRC格式本身不直接包含背景、翻译、罗马音或歌曲部分信息，
                // 这些字段在转换为TTML时默认为 None。
                // 它们可能会在后续处理步骤中（例如，如果加载了外部的翻译LRC文件）被填充。
                background_section: None,
                translation: None,
                romanization: None,
                song_part: None,
                itunes_key: None,
            };
            ttml_paragraphs.push(paragraph); // 将创建的段落添加到结果列表中
        } else {
            // 如果一行QRC既没有解析出有效音节，也没有行持续时间，则记录警告并跳过
            log::warn!(
                "[QRC -> TTML] QRC 行 (开始时间: {}) 没有音节且无持续时间，已跳过",
                qrc_line.line_start_ms
            );
        }
    }
    // 返回转换后的 TTML 段落列表和未经修改的原始元数据
    Ok((ttml_paragraphs, qrc_metadata))
}
