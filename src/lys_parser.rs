// 导入 once_cell 用于静态初始化 Regex，以及 regex 本身
use once_cell::sync::Lazy;
use regex::Regex;
// 从项目中导入类型定义：AssMetadata (用于元数据), ConvertError (错误类型),
// LysLine (LYS行结构), LysSyllable (LYS音节结构)
use crate::types::{AssMetadata, ConvertError, LysLine, LysSyllable};

// 静态正则表达式：匹配 LYS 行首的属性标记，例如 "[1]"
// \[(\d+)\]: 匹配方括号内的1个或多个数字，并将数字捕获到第一个组
static LYS_PROPERTY_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\[(\d+)\]").expect("未能编译 LYS_PROPERTY_REGEX"));

// 静态正则表达式：匹配 LYS 音节的时间戳，例如 "(100,200)"
// \((?P<start>\d+),(?P<duration>\d+)\): 匹配圆括号内的两个数字，分别捕获到 "start" 和 "duration" 组
// "start" 是音节相对于行内第一个音节开始时间的偏移量（毫秒）
// "duration" 是音节的持续时间（毫秒）
static LYS_TIMESTAMP_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\((?P<start>\d+),(?P<duration>\d+)\)").expect("未能编译 LYS_TIMESTAMP_REGEX")
});

// 静态正则表达式：匹配 LYS 文件头部的元数据标签，例如 "[ti:歌曲名]"
// 与 QRC/KRC 解析器中的元数据正则类似，但捕获的键名范围可能略有不同（这里是 ti, ar, al, by）
static LYS_METADATA_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\[(ti|ar|al|by):(.*?)\]$").expect("未能编译 LYS_METADATA_REGEX"));

/// 解析单行 LYS 歌词文本。
///
/// LYS 行格式通常为：`[属性]音节文本1(ts1,时长1)音节文本2(ts2,时长2)...`
///
/// # Arguments
/// * `line_str` - 要解析的单行 LYS 文本。
/// * `line_num` - 当前行号，用于错误报告。
///
/// # Returns
/// `Result<LysLine, ConvertError>` - 如果解析成功，返回包含行属性和音节列表的 `LysLine`；否则返回错误。
pub fn parse_lys_line(line_str: &str, line_num: usize) -> Result<LysLine, ConvertError> {
    // 1. 解析行属性
    // 尝试匹配行首的属性标记，如 "[0]"
    let property_cap = LYS_PROPERTY_REGEX.captures(line_str).ok_or_else(|| {
        // 如果匹配失败，说明行首缺少属性标记，返回格式错误
        ConvertError::InvalidLysFormat {
            line_num,
            message: "行首缺少属性标记 [属性数字]".to_string(),
        }
    })?;
    // 提取属性数字字符串
    let property_str = property_cap.get(1).unwrap().as_str();
    // 将属性字符串解析为 u8 数字
    let property: u8 = property_str
        .parse()
        .map_err(|_| ConvertError::InvalidLysProperty {
            line_num,
            property_str: property_str.to_string(),
        })?;

    // 2. 解析音节
    // 获取属性标记之后的内容，这部分包含音节文本和音节时间戳
    let content_after_property = &line_str[property_cap.get(0).unwrap().end()..];
    let mut syllables: Vec<LysSyllable> = Vec::new(); // 用于存储解析出的音节
    let mut last_match_end = 0; // 跟踪上一个音节时间戳之后文本部分的结束位置在 content_after_property 中的索引

    // 遍历所有匹配到的音节时间戳
    for ts_cap in LYS_TIMESTAMP_REGEX.captures_iter(content_after_property) {
        let ts_match = ts_cap.get(0).unwrap(); // 获取整个时间戳匹配项，如 "(100,200)"
        let ts_start_in_content = ts_match.start(); // 时间戳在 content_after_property 中的开始位置

        // 提取位于上一个时间戳标记之后、当前时间戳标记之前的文本作为音节文本
        let text_slice = &content_after_property[last_match_end..ts_start_in_content];

        // 从捕获组中提取音节开始偏移和持续时间的字符串
        let start_ms_str = ts_cap.name("start").unwrap().as_str();
        let duration_ms_str = ts_cap.name("duration").unwrap().as_str();

        // 解析音节的开始和持续时间
        let start_ms: u64 = start_ms_str.parse().map_err(|e| {
            ConvertError::InvalidLysSyllable {
                line_num,
                text: text_slice.to_string(), // 关联的文本，用于错误报告
                message: format!("开始时间无效 '{}': {}", start_ms_str, e),
            }
        })?;
        let duration_ms: u64 =
            duration_ms_str
                .parse()
                .map_err(|e| ConvertError::InvalidLysSyllable {
                    line_num,
                    text: text_slice.to_string(),
                    message: format!("持续时长无效 '{}': {}", duration_ms_str, e),
                })?;

        // 如果音节文本非空，则创建 LysSyllable 结构并添加到列表中
        // 注意：LYS 格式中，文本在时间戳之前。如果文本为空，但时间戳有效，
        // 这可能表示一个没有文本的音节（例如，纯粹的停顿或空格的特殊表示）。
        // 当前实现：只有当文本非空时才添加。如果需要处理空文本音节，这里的逻辑可能需要调整。
        if !text_slice.is_empty() {
            // 只在 text_slice 非空时添加
            syllables.push(LysSyllable {
                text: text_slice.to_string(),
                start_ms, // LYS 的 start_ms 是相对于行内第一个音节的偏移
                duration_ms,
            });
        }
        // 更新下一个文本段的开始位置（即当前时间戳标记的结束位置）
        last_match_end = ts_match.end();
    }

    // 3. 处理最后一个音节时间戳之后的文本（如果有）
    // LYS 格式通常要求所有文本段都后跟一个时间戳。
    // 如果在最后一个时间戳之后还有文本，这通常被视为格式问题或文件末尾的非歌词内容。
    if last_match_end < content_after_property.len() {
        let remaining_text = &content_after_property[last_match_end..].trim();
        if !remaining_text.is_empty() {
            // 记录警告，说明发现了多余的文本
            log::warn!(
                "行 {}: 在最后一个时间戳后发现多余的文本: '{}'",
                line_num,
                remaining_text
            );
            // 根据需求，可以选择忽略这些文本，或者尝试将其作为最后一个音节的一部分（但这不符合典型LYS格式）
        }
    }

    // 4. 检查和错误处理
    // 如果在属性标记后有非空内容，但没有解析出任何音节，则认为格式无效
    if syllables.is_empty()
        && !content_after_property.trim().is_empty()
        && syllables.is_empty()
        && !content_after_property.trim().is_empty()
    {
        log::warn!(
            "行 {}: 在属性后发现无时间戳的文本: '{}'",
            line_num,
            content_after_property.trim()
        );
        return Err(ConvertError::InvalidLysFormat {
            line_num,
            message: format!(
                "属性标记后的内容 '{}' 未能解析为有效音节。",
                content_after_property
            ),
        });
    }

    // 返回解析成功的 LysLine
    Ok(LysLine {
        property,
        syllables,
    })
}

/// 从字符串加载并解析 LYS 内容。
///
/// # Arguments
/// * `lys_content` - 包含完整 LYS 文件内容的字符串。
///
/// # Returns
/// `Result<(Vec<LysLine>, Vec<AssMetadata>), ConvertError>` -
/// 如果成功，返回一个元组，包含解析出的 LYS 行列表和元数据列表；否则返回错误。
pub fn load_lys_from_string(
    lys_content: &str,
) -> Result<(Vec<LysLine>, Vec<AssMetadata>), ConvertError> {
    let mut lys_lines_vec: Vec<LysLine> = Vec::new(); // 存储解析后的歌词行
    let mut lys_metadata_vec: Vec<AssMetadata> = Vec::new(); // 存储解析后的元数据

    // 逐行处理输入内容
    for (i, line_str_raw) in lys_content.lines().enumerate() {
        let line_num = i + 1; // 行号从1开始
        let trimmed_line = line_str_raw.trim(); // 去除行首尾空格

        if trimmed_line.is_empty() {
            // 跳过空行
            continue;
        }

        // 尝试匹配元数据标签 (如 [ti:xxx], [ar:xxx], [al:xxx], [by:xxx])
        if let Some(meta_caps) = LYS_METADATA_REGEX.captures(trimmed_line) {
            let tag = meta_caps.get(1).map_or("", |m| m.as_str()); // 提取标签名 (ti, ar, al, by)
            let value = meta_caps
                .get(2)
                .map_or("", |m| m.as_str())
                .trim()
                .to_string(); // 提取标签值

            // 根据标签名进行映射并存储
            // 注意：这里将 LYS 的标签键映射到内部更通用的元数据键名
            match tag {
                "ti" => lys_metadata_vec.push(AssMetadata {
                    key: "musicName".to_string(),
                    value,
                }),
                "ar" => {
                    // 艺术家，可能包含多个，用 "/" 分隔
                    if value.contains('/') {
                        for artist_part in value.split('/') {
                            let trimmed_artist_part = artist_part.trim();
                            if !trimmed_artist_part.is_empty() {
                                lys_metadata_vec.push(AssMetadata {
                                    key: "artists".to_string(),
                                    value: trimmed_artist_part.to_string(),
                                });
                            }
                        }
                    } else if !value.is_empty() {
                        lys_metadata_vec.push(AssMetadata {
                            key: "artists".to_string(),
                            value,
                        });
                    }
                }
                "al" => lys_metadata_vec.push(AssMetadata {
                    key: "album".to_string(),
                    value,
                }),
                "by" => lys_metadata_vec.push(AssMetadata {
                    key: "ttmlAuthorGithubLogin".to_string(),
                    value,
                }), // LYS 'by' 通常指制作者
                _ => log::warn!("行 {}: 未知的元数据标签类型 '{}'", line_num, tag), // 理论上不会执行，因为正则已限定
            }
        }
        // 如果不是元数据标签，则尝试匹配行首的属性标记，判断是否为歌词行
        else if LYS_PROPERTY_REGEX.is_match(trimmed_line) {
            match parse_lys_line(trimmed_line, line_num) {
                // 调用单行解析函数
                Ok(parsed_line) => {
                    lys_lines_vec.push(parsed_line); // 添加到歌词行列表
                }
                Err(e) => {
                    // 解析单行失败，记录错误
                    log::error!("解析行 {} ('{}') 失败: {}", line_num, trimmed_line, e);
                }
            }
        }
        // 如果行既不是元数据也不是有效的歌词行格式
        else {
            log::warn!("行 {}: 无法识别的行: '{}'", line_num, trimmed_line);
        }
    }
    // 返回解析结果
    Ok((lys_lines_vec, lys_metadata_vec))
}
