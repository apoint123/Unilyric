// 导入项目中定义的类型：
// ConvertError: 错误处理枚举
// TtmlParagraph: 目标中间数据结构，表示一个歌词段落
// TtmlSyllable: TTML音节结构，用于存储行文本
// AssMetadata: 用于存储元数据 (LYL格式本身不包含元数据标签，因此这里返回空列表)
use crate::types::{AssMetadata, ConvertError, TtmlParagraph, TtmlSyllable};
// 导入 LYL 解析器输出的行结构
use crate::types::ParsedLyricifyLine;

/// 将解析后的 Lyricify Lines (LYL) 数据 (`Vec<ParsedLyricifyLine>`)
/// 转换为 TTML 段落数据 (`Vec<TtmlParagraph>`)。
///
/// LYL 是一种逐行歌词，每行对应一个时间段和一段文本。
/// 在转换为 TTML 时，每个 LYL 行通常会映射为一个 `TtmlParagraph`，
/// 该段落的 `main_syllables` 列表将只包含一个 `TtmlSyllable`，这个音节的文本就是 LYL 行的完整文本。
///
/// # Arguments
/// * `lines` - 一个包含 `ParsedLyricifyLine` 结构体的切片，代表从LYL文件解析出的所有歌词行。
///
/// # Returns
/// `Result<(Vec<TtmlParagraph>, Vec<AssMetadata>), ConvertError>` -
/// 如果成功，返回一个元组，包含转换后的 TTML 段落列表和空的元数据列表
/// (因为 LYL 格式本身不定义文件级元数据标签)。
/// 失败时返回错误 (尽管在此简单转换中，出错的可能性较低，除非发生意外的内部错误)。
pub fn convert_lyricify_to_ttml_data(
    lines: &[ParsedLyricifyLine],
) -> Result<(Vec<TtmlParagraph>, Vec<AssMetadata>), ConvertError> {
    let mut ttml_paragraphs: Vec<TtmlParagraph> = Vec::new(); // 初始化用于存储结果的 TTML 段落列表

    // 遍历从 LYL 解析器得到的每一行 ParsedLyricifyLine
    for line in lines.iter() {
        // 将 LYL 行的完整文本作为一个单独的 TTML 音节
        let line_syllable = TtmlSyllable {
            text: line.text.clone(), // 直接使用 LYL 行的文本
            start_ms: line.start_ms, // 音节的开始时间即为 LYL 行的开始时间
            end_ms: line.end_ms,     // 音节的结束时间即为 LYL 行的结束时间
            ends_with_space: false,  // 对于整行文本，通常不认为其末尾有需要特殊处理的逻辑空格
        };

        // 为每个 LYL 行创建一个 TtmlParagraph
        let paragraph = TtmlParagraph {
            p_start_ms: line.start_ms,           // 段落开始时间
            p_end_ms: line.end_ms,               // 段落结束时间
            agent: "v1".to_string(),             // 默认演唱者为 "v1"
            main_syllables: vec![line_syllable], // 主音节列表只包含上面创建的代表整行文本的音节
            background_section: None,            // LYL 格式不直接支持背景人声
            translation: None,                   // LYL 格式不直接支持翻译
            romanization: None,                  // LYL 格式不直接支持罗马音
            song_part: None,                     // LYL 格式不直接支持歌曲部分标记
            itunes_key: None,
        };
        ttml_paragraphs.push(paragraph); // 将创建的段落添加到结果列表中
    }

    // LYL 格式本身没有文件级元数据标签，因此返回一个空的元数据列表
    let metadata: Vec<AssMetadata> = Vec::new();

    Ok((ttml_paragraphs, metadata)) // 返回转换后的 TTML 段落和空元数据
}
