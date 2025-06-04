// 导入 kugou_lyrics_fetcher 模块中的错误和模型，用于处理 KRC 特有的 language 标签中的翻译
use crate::{
    kugou_lyrics_fetcher::{error::KugouError, kugoumodel::KugouTranslation},
    types::{AssMetadata, ConvertError, LysSyllable, QrcLine}, // LysSyllable 用于音节，QrcLine 用于行
};
// 导入 base64 引擎，用于解码 language 标签中的内容
use base64::Engine;
// 导入正则表达式库和 once_cell 用于静态初始化 Regex
use once_cell::sync::Lazy;
use regex::Regex;

// 正则表达式：匹配 KRC 的行级别时间戳，例如 "[12345,5000]"
// (?P<start>\d{1,}) 捕获行开始时间（毫秒）到名为 "start" 的组 (允许1位或多位数字)
// (?P<duration>\d{1,}) 捕获行持续时间（毫秒）到名为 "duration" 的组
static KRC_LINE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[(\d{1,}),(\d{1,})\](.*)").unwrap());

// 正则表达式：匹配 KRC 的音节级别时间戳和文本，例如 "<100,200,0>歌"
// <(?P<offset>\d{1,}),(?P<duration>\d{1,}),(?P<pitch_or_type>\d{1,})>
//   - offset: 音节相对于行开始时间的偏移量（毫秒）
//   - duration: 音节的持续时间（毫秒）
//   - pitch_or_type: 第三个参数，通常为0，在此解析器中主要用于匹配结构
// (?P<text>[^<>]*) 捕获音节文本（不包含 '<' 或 '>' 的任意字符）到名为 "text" 的组
static KRC_SYLLABLE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<(\d{1,}),(\d{1,}),(\d{1,})>([^<>]*)").unwrap());

// 正则表达式：专门匹配 KRC 特有的 [language:Base64编码的JSON] 标签
static KRC_LANGUAGE_TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\[language:(.*)\]$").unwrap());

// 正则表达式：匹配通用的元数据标签，例如 "[ti:歌曲标题]"
// 与 qrc_parser.rs 中的 METADATA_TAG_REGEX 类似
static GENERIC_METADATA_TAG_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\[([a-zA-Z0-9_]+):(.*?)\]$").unwrap());

/// 从字符串加载并解析 KRC 内容。
///
/// # Arguments
/// * `content` - 包含完整 KRC 文件内容的字符串。
///
/// # Returns
/// `Result<(Vec<QrcLine>, Vec<AssMetadata>), ConvertError>` -
/// 如果成功，返回一个元组，包含解析出的歌词行列表（使用 `QrcLine` 结构，因为其字段适用）
/// 和元数据列表；否则返回错误。
pub fn load_krc_from_string(
    content: &str,
) -> Result<(Vec<QrcLine>, Vec<AssMetadata>), ConvertError> {
    let mut lines_data = Vec::new(); // 存储解析后的歌词行
    let mut metadata = Vec::new(); // 存储解析后的元数据
    let mut line_number = 0; // 当前处理的行号，用于日志和错误报告

    // 逐行处理输入内容
    for line_str in content.lines() {
        line_number += 1;
        let trimmed_line = line_str.trim(); // 去除行首尾空格

        if trimmed_line.is_empty() {
            // 跳过空行
            continue;
        }

        // 1. 优先尝试匹配 KRC 特有的 [language:Base64...] 标签
        if let Some(lang_caps) = KRC_LANGUAGE_TAG_RE.captures(trimmed_line) {
            if let Some(base64_value) = lang_caps.get(1) {
                // 获取 Base64 内容
                metadata.push(AssMetadata {
                    // 将此特殊标签作为一条元数据存储，键名固定，方便后续提取翻译
                    key: "KrcInternalTranslation".to_string(),
                    value: base64_value.as_str().to_string(),
                });
                log::info!("[KRC 解析] 行 {}: 处理KRC特有language标签。", line_number);
            }
            continue; // 处理完 language 标签后跳到下一行
        }
        // 2. 尝试匹配所有其他 [key:value] 格式的元数据标签
        else if let Some(meta_caps) = GENERIC_METADATA_TAG_RE.captures(trimmed_line) {
            if let (Some(key_match), Some(value_match)) = (meta_caps.get(1), meta_caps.get(2)) {
                let key = key_match.as_str().trim().to_lowercase(); // 键名转小写，方便匹配
                let value = value_match.as_str().trim().to_string();

                match key.as_str() {
                    // 一些 KRC 文件中常见的但可能不需要在 UniLyric 中作为核心元数据处理的标签
                    "id" | "hash" | "total" | "by" | "offset" => {
                        // 这些通常是酷狗内部ID或文件信息，可以选择性记录或忽略
                        // metadata.push(AssMetadata { key, value }); // 如果需要存储
                    }
                    "ti" | "ar" | "al" => {
                        // 存储这些标准元数据
                        metadata.push(AssMetadata {
                            key: key.clone(),
                            value: value.clone(),
                        });
                        log::trace!(
                            "[KRC 解析] 行 {}: 处理标准元数据标签: [{}:{}]",
                            line_number,
                            key,
                            value
                        );
                    }
                    // "language" 标签如果不是 Base64 格式的，则按普通元数据处理或警告
                    // (已被上面的 KRC_LANGUAGE_TAG_RE 优先捕获，这里理论上不会匹配到 "language")
                    "language" => {}
                    _ => {
                        // 对于其他未知的 [key:value] 标签，可以发出警告
                        //log::warn!("[KRC 解析] 行 {}: 未知或当前未处理的元数据标签: [{}:{}]", line_number, key, value);
                        // metadata.push(AssMetadata { key, value }); // 如果希望存储所有未知标签
                    }
                }
            } else {
                // GENERIC_METADATA_TAG_RE 匹配了行，但无法提取 key/value (理论上不应发生，因为正则定义了捕获组)
                log::warn!(
                    "[KRC 解析] 行 {}: 疑似元数据标签但无法解析key/value: '{}'",
                    line_number,
                    trimmed_line
                );
            }
            continue; // 处理完元数据标签后跳到下一行
        }
        // 3. 尝试匹配歌词行 [行开始时间,行持续时间]<音节偏移,音节时长,类型>音节文本...
        else if let Some(caps) = KRC_LINE_RE.captures(trimmed_line) {
            let line_start_ms_str = caps.get(1).map_or("", |m| m.as_str());
            let line_duration_ms_str = caps.get(2).map_or("", |m| m.as_str());
            let syllables_part = caps.get(3).map_or("", |m| m.as_str()); // 行内所有音节的部分

            // 解析行开始时间和行持续时间
            let line_start_ms = line_start_ms_str.parse::<u64>().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "KRC 行 {} 开始时间无效 '{}': {}",
                    line_number, line_start_ms_str, e
                ))
            })?;
            let line_duration_ms = line_duration_ms_str.parse::<u64>().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "KRC 行 {} 持续时间无效 '{}': {}",
                    line_number, line_duration_ms_str, e
                ))
            })?;

            let mut krc_syllables = Vec::new(); // 存储当前行的音节
            let mut current_char_pos_in_syllables_part = 0; // 跟踪在 syllables_part 中的解析位置

            // 遍历所有匹配到的音节时间戳和文本
            for syl_cap in KRC_SYLLABLE_RE.captures_iter(syllables_part) {
                let syl_match_start = syl_cap.get(0).unwrap().start(); // 当前音节标签的开始位置
                let syl_match_end = syl_cap.get(0).unwrap().end(); // 当前音节标签的结束位置

                // 检查音节标签之间是否有未被捕获的文本（理论上KRC格式不应有）
                if syl_match_start > current_char_pos_in_syllables_part {
                    let unprocessed_text =
                        &syllables_part[current_char_pos_in_syllables_part..syl_match_start];
                    if !unprocessed_text.trim().is_empty() {
                        log::info!(
                            "[KRC 解析] 行 {}: 在音节时间戳 '{}' 前发现未处理文本: '{}'",
                            line_number,
                            syl_cap.get(0).unwrap().as_str(),
                            unprocessed_text
                        );
                    }
                }

                // 提取音节的偏移、时长和文本
                let syl_offset_ms_str = syl_cap.get(1).map_or("0", |m| m.as_str());
                let syl_duration_ms_str = syl_cap.get(2).map_or("0", |m| m.as_str());
                // 第三个参数 (pitch_or_type) 在这里被捕获但未使用，通常为0
                let syl_text = syl_cap.get(4).map_or("", |m| m.as_str()).to_string();

                // 解析音节的偏移和时长
                let syl_offset_ms = syl_offset_ms_str.parse::<u64>().map_err(|e| {
                    ConvertError::InvalidTime(format!(
                        "KRC 行 {} 音节偏移无效 '{}': {}",
                        line_number, syl_offset_ms_str, e
                    ))
                })?;
                let syl_duration_ms = syl_duration_ms_str.parse::<u64>().map_err(|e| {
                    ConvertError::InvalidTime(format!(
                        "KRC 行 {} 音节时长无效 '{}': {}",
                        line_number, syl_duration_ms_str, e
                    ))
                })?;

                // KRC 音节的开始时间是相对于行开始时间的偏移量，需要转换为绝对时间
                let absolute_syl_start_ms = line_start_ms + syl_offset_ms;

                krc_syllables.push(LysSyllable {
                    text: syl_text,
                    start_ms: absolute_syl_start_ms, // 存储绝对开始时间
                    duration_ms: syl_duration_ms,
                });
                current_char_pos_in_syllables_part = syl_match_end; // 更新解析位置
            }

            // 检查最后一个音节标签后是否还有剩余文本
            if current_char_pos_in_syllables_part < syllables_part.len() {
                let trailing_text = &syllables_part[current_char_pos_in_syllables_part..];
                if !trailing_text.trim().is_empty() {
                    log::warn!(
                        "[KRC 解析] 行 {}: 在最后一个音节后发现文本: '{}'",
                        line_number,
                        trailing_text
                    );
                    // 将尾随文本追加到最后一个音节，或创建一个新音节
                    if let Some(last_syl) = krc_syllables.last_mut() {
                        last_syl.text.push_str(trailing_text);
                    } else {
                        // 如果之前没有音节（例如，行内容只有文本而没有音节标签），则将整行视为一个音节
                        krc_syllables.push(LysSyllable {
                            text: trailing_text.to_string(),
                            start_ms: line_start_ms,       // 使用行开始时间
                            duration_ms: line_duration_ms, // 使用行持续时间
                        });
                    }
                }
            }

            // 如果解析出了音节，则创建 QrcLine（复用此结构）并添加到列表
            if !krc_syllables.is_empty() {
                lines_data.push(QrcLine {
                    line_start_ms,
                    line_duration_ms,
                    syllables: krc_syllables,
                });
            } else if !syllables_part.trim().is_empty() {
                // 如果音节部分非空，但没有解析出带时间戳的音节，
                // 可能意味着整行文本没有逐字时间信息，将其作为单个音节处理。
                log::warn!(
                    "[KRC 解析] 行 {}: 内容 '{}' 中未找到有效的KRC音节时间戳，但内容非空。将其作为单音节行处理。",
                    line_number,
                    syllables_part
                );
                lines_data.push(QrcLine {
                    line_start_ms,
                    line_duration_ms,
                    syllables: vec![LysSyllable {
                        text: syllables_part.to_string(),
                        start_ms: line_start_ms,
                        duration_ms: line_duration_ms,
                    }],
                });
            }
            // 如果音节列表为空且音节部分也为空（例如，只有行时间戳 `[123,456]`），则忽略此行。
        }
        // 4. 如果行不匹配任何已知格式 (且非空)
        else if !trimmed_line.is_empty() {
            // log::warn!(
            //     "[KRC 解析] 行 {}: 未能识别为元数据或KRC行: '{}'",
            //     line_number,
            //     trimmed_line
            // );
        }
    }
    Ok((lines_data, metadata)) // 返回解析的歌词行和元数据
}

// --- KRC 内嵌翻译提取逻辑 ---
// KRC 文件可能在 [language:Base64编码的JSON] 标签中包含翻译信息。

// language 标签的开始和结束标记
const LANGUAGE_TAG_START: &str = "[language:";
const LANGUAGE_TAG_END: char = ']';

/// 从 KRC 内容字符串中提取内嵌的翻译文本行。
/// KRC 的翻译信息通常存储在一个 Base64 编码的 JSON 对象中，位于 `[language:]` 标签内。
///
/// # Arguments
/// * `krc_content` - 完整的 KRC 文件内容字符串。
///
/// # Returns
/// `Result<Option<Vec<String>>, KugouError>` -
///   - `Ok(Some(Vec<String>))`：如果成功提取到翻译行。
///   - `Ok(None)`：如果没有找到翻译信息或翻译内容为空。
///   - `Err(KugouError)`：如果解析过程中发生错误（如Base64解码失败、JSON解析失败）。
pub fn extract_translation_from_krc(krc_content: &str) -> Result<Option<Vec<String>>, KugouError> {
    // 检查内容中是否包含 language 标签的起始部分
    if !krc_content.contains(LANGUAGE_TAG_START) {
        return Ok(None); // 没有 language 标签，直接返回 None
    }

    // 找到 Base64 内容的开始索引
    let start_index = if let Some(idx) = krc_content.find(LANGUAGE_TAG_START) {
        idx + LANGUAGE_TAG_START.len() // Base64 内容在 "[language:" 之后
    } else {
        return Ok(None); // 理论上不会执行到这里，因为上面已经检查过 contains
    };

    // 找到 Base64 内容的结束索引 (即 ']' 的位置)
    let end_index = match krc_content[start_index..].find(LANGUAGE_TAG_END) {
        Some(idx) => start_index + idx, // ']' 在 start_index 之后的相对位置
        None => {
            return Err(KugouError::InvalidKrcData(
                "language标签缺少结束符 ']'".to_string(),
            ));
        }
    };

    // 提取 Base64 编码的 JSON 字符串
    let base64_encoded_json = &krc_content[start_index..end_index];

    // 解码 Base64 字符串
    let json_bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_encoded_json)
        .map_err(KugouError::Base64)?; // 如果解码失败，映射到 KugouError::Base64

    // 将解码后的字节转换为 UTF-8 字符串
    let json_string = String::from_utf8(json_bytes)?; // 如果转换失败，返回 FromUtf8Error，会被 KugouError::Utf8 捕获

    // 解析 JSON 字符串为 KugouTranslation 结构体
    // KugouTranslation 结构定义在 kugou_lyrics_fetcher/kugoumodel.rs 中
    let translation_data: KugouTranslation =
        serde_json::from_str(&json_string).map_err(KugouError::Json)?;

    // 如果 JSON 中 content 数组为空，则没有翻译信息
    if translation_data.content.is_empty() {
        return Ok(None);
    }

    // 查找 item_type 为 1 的翻译内容项 (根据观察，type=1 通常代表翻译)
    let target_content_item = translation_data
        .content
        .iter()
        .find(|item| item.item_type == 1);

    match target_content_item {
        Some(item) => {
            // 如果找到了翻译项，但其 lyric_content (实际的翻译行) 为空
            if item.lyric_content.is_empty() {
                return Ok(None);
            }
            let mut translations = Vec::new();
            // KugouTranslation.lyric_content 是 Vec<Vec<String>>
            // 每一内部 Vec<String> 代表一行翻译的多个部分（通常我们只关心第一个部分）
            for line_parts in &item.lyric_content {
                if let Some(first_part) = line_parts.first() {
                    translations.push(first_part.clone()); // 取第一个部分作为该行的翻译文本
                } else {
                    translations.push(String::new()); // 如果内部 Vec 为空，则添加空字符串作为占位
                }
            }
            if translations.is_empty() {
                // 如果最终没有收集到任何翻译行
                Ok(None)
            } else {
                Ok(Some(translations)) // 返回包含所有翻译行的 Vec
            }
        }
        None => Ok(None), // 没有找到 item_type 为 1 的项
    }
}
