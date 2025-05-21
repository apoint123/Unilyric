// 导入标准库的 Write trait，用于向字符串写入格式化文本
use std::fmt::Write as FmtWrite;
// 导入 lys_generator 模块的函数，用于从 TTML 数据生成 LYS 格式的歌词
use crate::lys_generator::generate_lys_from_ttml_data;
// 导入项目中定义的类型：
// ConvertError: 错误处理枚举
// ParsedSourceData: 包含解析后的源数据，包括 TTML 段落、提取的LRC翻译/罗马音等
// LrcContentType: 枚举，用于指示LRC歌词是翻译还是罗马音 (虽然在此生成器中不直接使用此枚举，但相关的 lrc_generator 会用)
// CanonicalMetadataKey: 元数据键的规范化枚举
use crate::types::{CanonicalMetadataKey, ConvertError, ParsedSourceData};
// 导入元数据处理器
use crate::metadata_processor::MetadataStore;
// 导入 lrc_generator 模块，用于从 TTML 数据生成 LRC 格式的歌词
use crate::lrc_generator;

/// 从中间数据结构 (`ParsedSourceData`) 和元数据存储 (`MetadataStore`) 生成 LQE 格式的字符串。
///
/// LQE 文件结构通常如下：
/// ```
/// [Lyricify Quick Export]
/// [version:1.0]
/// [ti:歌曲名]
/// [ar:歌手]
/// ... (其他全局元数据) ...
///
/// [lyrics: format@Lyricify Syllable, language@zh]
/// ... (LYS格式的主歌词内容) ...
///
/// [translation: format@LRC, language@en]
/// ... (LRC格式的翻译内容) ...
///
/// [pronunciation: format@LRC, language@ja-ro]
/// ... (LRC格式的罗马音/发音内容) ...
/// ```
///
/// # Arguments
/// * `data` - `ParsedSourceData` 结构，包含主歌词段落 (`paragraphs`) 以及
///   可能已提取的翻译LRC (`lqe_extracted_translation_lrc_content`) 和
///   罗马音LRC (`lqe_extracted_romanization_lrc_content`)。
///   `lqe_main_lyrics_as_lrc` 字段指示主歌词部分是否应输出为LRC格式。
///   `lqe_direct_main_lrc_content` 字段允许直接使用预存的LRC作为主歌词。
/// * `metadata_store` - `MetadataStore` 的引用，包含要写入LQE文件头部的全局元数据。
///
/// # Returns
/// `Result<String, ConvertError>` - 如果成功，返回生成的 LQE 格式字符串；否则返回错误。
pub fn generate_lqe_from_intermediate_data(
    data: &ParsedSourceData,
    metadata_store: &MetadataStore,
) -> Result<String, ConvertError> {
    let mut lqe_output = String::new(); // 初始化输出字符串

    // 1. 写入 LQE 文件头和版本信息
    writeln!(lqe_output, "[Lyricify Quick Export]")?;
    // 获取版本号，如果元数据存储中没有，则默认为 "1.0"
    let version_str = metadata_store
        .get_single_value(&CanonicalMetadataKey::Version)
        .filter(|s| !s.trim().is_empty()) // 确保值非空
        .map_or_else(|| "1.0".to_string(), |v| v.trim().to_string());
    writeln!(lqe_output, "[version:{}]", version_str)?;

    let lqe_header_tags_map = [
        (CanonicalMetadataKey::Title, "ti"),
        (CanonicalMetadataKey::Artist, "ar"),
        (CanonicalMetadataKey::Album, "al"),
        (CanonicalMetadataKey::Author, "by"),
        (CanonicalMetadataKey::Editor, "re"),
        (CanonicalMetadataKey::Offset, "offset"),
    ];
    let multi_value_keys_for_lqe = [
        CanonicalMetadataKey::Artist,
        CanonicalMetadataKey::Songwriter,
        CanonicalMetadataKey::Author,
    ];
    for (ckey, lqe_tag_name) in lqe_header_tags_map.iter() {
        if multi_value_keys_for_lqe.contains(ckey) {
            if let Some(values) = metadata_store.get_multiple_values(ckey) {
                let combined_value = values
                    .iter()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<&str>>()
                    .join("/");
                if !combined_value.is_empty() {
                    writeln!(lqe_output, "[{}:{}]", lqe_tag_name, combined_value)?;
                }
            }
        } else if let Some(value) = metadata_store.get_single_value(ckey) {
            let trimmed_value = value.trim();
            if !trimmed_value.is_empty() {
                writeln!(lqe_output, "[{}:{}]", lqe_tag_name, trimmed_value)?;
            }
        }
    }

    // 3. 写入主歌词区段 `[lyrics: ...]`
    // 确定主歌词的语言属性字符串，例如 ", language@zh"
    let main_lyrics_lang_attr = data
        .language_code // 优先使用 ParsedSourceData 中解析出的主语言
        .as_ref()
        .or_else(|| metadata_store.get_single_value(&CanonicalMetadataKey::Language)) // 其次尝试元数据存储中的全局语言
        .filter(|s| !s.is_empty()) // 确保语言代码非空
        .map_or_else(String::new, |lang| format!(", language@{}", lang.trim()));

    if data.lqe_main_lyrics_as_lrc {
        // 如果标记指示主歌词应为 LRC 格式 (通常当源文件是 LRC 时)
        writeln!(
            lqe_output,
            "\n[lyrics: format@LRC{}]",
            main_lyrics_lang_attr
        )?;
        let main_lrc_content = if let Some(direct_lrc) = &data.lqe_direct_main_lrc_content {
            // 如果 ParsedSourceData 中有直接提供的主LRC歌词 (例如，从网易云下载的LRC主歌词)
            log::info!("[LQE 生成] 使用直接提供的主LRC歌词。");
            direct_lrc.clone()
        } else {
            // 否则，从 TTML 段落 (`data.paragraphs`) 生成主LRC歌词
            log::info!("[LQE 生成] 从TTML中间层生成主LRC歌词。");
            // 调用 lrc_generator 生成主LRC，但不包含其自身的元数据头部，
            // 因为 LQE 有自己的全局元数据和区段头。
            lrc_generator::generate_main_lrc_from_paragraphs(&data.paragraphs, metadata_store)?
                .lines() // 按行分割
                .filter(|line| !line.trim().is_empty() && line.starts_with('[')) // 只保留有效的LRC时间标签行
                .collect::<Vec<&str>>()
                .join("\n") // 重新组合
                + if data.paragraphs.is_empty() && data.lqe_direct_main_lrc_content.is_none() { "" } else { "\n" } // 如果有内容，确保末尾有换行
        };
        if !main_lrc_content.trim().is_empty() {
            writeln!(lqe_output, "{}", main_lrc_content.trim())?; // 写入LRC歌词，并去除可能的首尾多余空白
        } else {
            log::warn!("[LQE 生成] 主歌词LRC歌词为空，lyrics区段可能不含歌词行。");
        }
    } else {
        // 主歌词默认为 LYS (Lyricify Syllable) 格式
        writeln!(
            lqe_output,
            "\n[lyrics: format@Lyricify Syllable{}]",
            main_lyrics_lang_attr
        )?;
        // 从 TTML 段落生成 LYS 内容，不包含 LYS 的元数据头部 (include_metadata: false)
        let lys_content = generate_lys_from_ttml_data(&data.paragraphs, metadata_store, false)?;
        if !lys_content.trim().is_empty() {
            writeln!(lqe_output, "{}", lys_content.trim())?;
        } else {
            log::warn!("[LQE 生成] 主歌词LYS内容为空，lyrics区段可能不含歌词行。");
        }
    }

    // 4. 写入翻译区段 `[translation: ...]` (如果存在)
    // `lqe_extracted_translation_lrc_content` 字段存储了从源文件（如LQE本身或下载）中提取的LRC格式翻译
    if let Some(trans_content) = &data.lqe_extracted_translation_lrc_content {
        if !trans_content.trim().is_empty() {
            // 确保翻译内容非空
            // 确定翻译的语言属性字符串
            let trans_lang_attr = data
                .lqe_translation_language
                .as_ref()
                .filter(|s| !s.is_empty())
                .map_or_else(String::new, |lang| format!(", language@{}", lang.trim()));
            // 写入翻译区段头部，格式固定为 LRC
            writeln!(lqe_output, "\n[translation: format@LRC{}]", trans_lang_attr)?;
            writeln!(lqe_output, "{}", trans_content.trim())?; // 写入翻译LRC歌词
        }
    }

    // 5. 写入发音/罗马音区段 `[pronunciation: ...]` (如果存在)
    // `lqe_extracted_romanization_lrc_content` 存储了LRC格式的罗马音
    if let Some(pron_content) = &data.lqe_extracted_romanization_lrc_content {
        if !pron_content.trim().is_empty() {
            // 确保罗马音内容非空
            // 确定罗马音的语言属性字符串，如果未指定，则默认为 "romaji"
            let pron_lang_attr = data
                .lqe_romanization_language
                .as_ref()
                .filter(|s| !s.is_empty())
                .map_or_else(
                    || ", language@romaji".to_string(), // 默认语言为 romaji
                    |lang| format!(", language@{}", lang.trim()),
                );
            // 写入发音区段头部，格式固定为 LRC
            writeln!(
                lqe_output,
                "\n[pronunciation: format@LRC{}]",
                pron_lang_attr
            )?;
            writeln!(lqe_output, "{}", pron_content.trim())?; // 写入罗马音LRC歌词
        }
    }

    // 确保最终输出以单个换行符结束，并移除可能的多余前导/尾随空白
    Ok(lqe_output.trim().to_string() + "\n")
}
