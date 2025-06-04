use crate::types::TtmlParagraph;
use log::{debug, trace, warn};
use regex::{Regex, RegexBuilder};
use std::borrow::Cow;

/// 辅助函数：从 TtmlParagraph 获取用于匹配的纯文本内容
fn get_plain_text_from_paragraph(paragraph: &TtmlParagraph) -> String {
    // 直接将所有音节的文本（作为 &str）收集到一个 String 中
    let joined_text: String = paragraph
        .main_syllables
        .iter()
        .map(|syllable| syllable.text.as_str())
        .collect();

    // 对连接后的字符串进行 trim 操作
    let trimmed_slice = joined_text.trim();

    // 仅当 trim 确实改变了字符串（移除了首尾空格）时，才创建新的 String
    // 否则，如果 trim 后长度不变，可以直接返回原始的 joined_text (如果它是 String)
    // 或者，如果 joined_text 本身就是通过 collect 创建的，它已经是拥有所有权的 String
    // 这里的逻辑是：如果 trim 后的 slice 与原始 String 长度不同，说明发生了裁剪，需要 to_string()
    // 如果长度相同，说明没有首尾空格，或者 trim 行为不符合预期（例如，内部空格）
    // 对于 .trim().to_string() 的常见模式，如果确定需要 String，这是标准做法。
    // 优化点在于避免了中间的 Vec<String>
    if trimmed_slice.len() < joined_text.len() {
        trimmed_slice.to_string()
    } else {
        // 如果长度相等，说明没有裁剪掉任何字符，或者 trim 行为是针对整个字符串的引用。
        // 由于 joined_text 是新构建的 String，如果 trimmed_slice 指向它的全部内容，
        // 理论上可以直接返回 joined_text。但是，为了确保返回的是 trim 后的结果，
        // 并且与原先 .trim().to_string() 语义一致，这里也进行 to_string()。
        // 更精细的优化可以检查指针是否相同，但通常 .to_string() 在这里是安全的。
        // 考虑到 trim() 返回的是 &str，如果它与原始 String 内容完全一致，
        // to_string() 仍然会创建一个新的 String。
        // 一个更直接的优化是如果 joined_text 本身就不需要 trim，则直接返回。
        // 但函数语义是返回 trim 后的结果。
        // 此处保持与原逻辑相似的 .trim().to_string() 行为，但已优化了 join 过程。
        trimmed_slice.to_string() // 或者直接返回 joined_text 如果确定 trim 不会改变它
        // 但为了安全和明确，to_string() 确保返回的是 trim 后的独立 String
    }
    // 更简洁且通常足够高效的方式：
    // paragraph
    //     .main_syllables
    //     .iter()
    //     .map(|syllable| syllable.text.as_str())
    //     .collect::<String>() // 收集为 String
    //     .trim()              // 获取 &str
    //     .to_string()         // 转换为拥有的 String
}

/// 根据关键词和正则表达式移除 TtmlParagraph 列表中的描述性元数据块和匹配行。
///
/// # Arguments
/// * `paragraphs`: 原始的 TtmlParagraph 列表 (传入所有权)。
/// * `keywords`: 用于阶段1的关键词列表。
/// * `keyword_case_sensitive`: 阶段1关键词匹配是否区分大小写。
/// * `header_scan_limit`: 阶段1从歌词开头向前检查多少行以识别头部元数据。
/// * `end_lookback_count`: 阶段1从歌词末尾向前检查多少行以识别尾部元数据。
/// * `enable_regex_stripping`: 是否启用阶段2的正则表达式移除。
/// * `regex_patterns`: 用户定义的正则表达式字符串列表。
/// * `regex_case_sensitive`: 阶段2正则表达式匹配是否区分大小写。
pub fn strip_descriptive_metadata_blocks(
    paragraphs: Vec<TtmlParagraph>,
    keywords: &[String],
    keyword_case_sensitive: bool,
    enable_regex_stripping: bool,
    regex_patterns: &[String],
    regex_case_sensitive: bool,
) -> Vec<TtmlParagraph> {
    const HEADER_SCAN_LIMIT: usize = 20;
    const END_LOOKBACK_COUNT: usize = 10;

    // 如果输入段落为空，或没有关键词且未启用正则移除（或正则列表为空），则直接返回原始段落
    if paragraphs.is_empty() {
        return paragraphs;
    }
    if keywords.is_empty() && (!enable_regex_stripping || regex_patterns.is_empty()) {
        return paragraphs;
    }

    let original_count = paragraphs.len();
    let mut processed_paragraphs = paragraphs; // 获取所有权，后续将就地修改或替换

    // --- 阶段 1: 基于关键词移除头部和尾部块 ---
    if !keywords.is_empty() {
        // 定义一个闭包（lambda函数）来判断单行是否匹配关键词规则
        // 优化了字符串处理，减少不必要的分配
        let line_matches_keyword_rule = |line_to_check: &str, // 要检查的行文本
                                         kw_list: &[String], // 关键词列表
                                         case_sens: bool|    // 是否区分大小写
         -> bool {
            // 移除行首可能存在的时间戳标记，如 "[00:12.345]" 或 "(singer)"
            let mut text_after_prefix = line_to_check.trim_start();
            if text_after_prefix.starts_with('[') {
                if let Some(end_bracket_idx) = text_after_prefix.find(']') {
                    text_after_prefix = text_after_prefix[end_bracket_idx + 1..].trim_start();
                }
            } else if text_after_prefix.starts_with('(') {
                if let Some(end_paren_idx) = text_after_prefix.find(')') {
                    text_after_prefix = text_after_prefix[end_paren_idx + 1..].trim_start();
                }
            }

            // 根据是否区分大小写，预处理待检查的文本
            // Cow<str> (Clone-on-Write) 用于在不区分大小写时持有小写版本的字符串，区分大小写时则借用原字符串切片
            let prepared_line_cow: Cow<str> = if case_sens {
                Cow::Borrowed(text_after_prefix)
            } else {
                Cow::Owned(text_after_prefix.to_lowercase())
            };
            let line_to_compare = prepared_line_cow.as_ref(); // 获取处理后的行文本引用

            for keyword_base in kw_list {
                if keyword_base.is_empty() { // 跳过空关键词
                    continue;
                }

                let prepared_keyword_cow: Cow<str> = if case_sens {
                    Cow::Borrowed(keyword_base)
                } else {
                    Cow::Owned(keyword_base.to_lowercase())
                };
                let keyword_to_compare = prepared_keyword_cow.as_ref(); // 获取处理后的关键词引用

                // 检查处理后的行文本是否以处理后的关键词开头
                if let Some(stripped_segment) = line_to_compare.strip_prefix(keyword_to_compare) {
                    // 如果匹配，则检查关键词后的部分是否以 ':' 或 '：' 开头
                    let after_keyword_segment = stripped_segment.trim_start();
                    if after_keyword_segment.starts_with(':') || after_keyword_segment.starts_with('：') {
                        return true;
                    }
                }
            }
            false // 未匹配任何关键词规则
        };

        // 扫描头部元数据
        let mut last_matching_header_index: Option<usize> = None;
        // 确定头部扫描的实际行数上限，不超过总段落数
        let current_header_scan_limit = HEADER_SCAN_LIMIT.min(processed_paragraphs.len());

        for (i, paragraph_item) in processed_paragraphs
            .iter()
            .enumerate()
            .take(current_header_scan_limit)
        {
            let paragraph_text = get_plain_text_from_paragraph(paragraph_item);
            if line_matches_keyword_rule(&paragraph_text, keywords, keyword_case_sensitive) {
                last_matching_header_index = Some(i);
                debug!(
                    "[行处理器] 头部行 {} ('{}') 是元数据。last_matching_header_index 目前: {:?}",
                    i,
                    paragraph_text.chars().take(30).collect::<String>(),
                    last_matching_header_index
                );
            }
        }

        // 根据头部扫描结果，确定歌词实际开始的段落索引
        let first_lyric_paragraph_index = if let Some(last_match_idx) = last_matching_header_index {
            debug!(
                "[行处理器] 最后匹配的头部元数据位于索引 {}。将移除此行及之前所有行。",
                last_match_idx
            );
            last_match_idx + 1 // 歌词从匹配到的元数据行的下一行开始
        } else {
            debug!(
                "[行处理器] 在前 {} 行中未找到头部元数据。",
                current_header_scan_limit
            );
            0 // 没有头部元数据，歌词从第一行开始
        };
        debug!(
            "[行处理器] --- 完成头部关键词扫描。歌词起始段落索引: {} ---",
            first_lyric_paragraph_index
        );

        // 扫描尾部元数据
        let mut last_lyric_paragraph_exclusive_index = processed_paragraphs.len(); // 默认为总段落数（不包含）
        // 仅当头部扫描后仍有段落，且这些段落可能包含尾部元数据时，才进行尾部扫描
        if first_lyric_paragraph_index < processed_paragraphs.len() {
            // 计算尾部扫描的起始索引（从后向前）
            // .saturating_sub 确保不会下溢到负数
            // .max 确保不会扫描到已确定的歌词内容之前
            let footer_scan_absolute_start_index = processed_paragraphs
                .len()
                .saturating_sub(END_LOOKBACK_COUNT)
                .max(first_lyric_paragraph_index);

            debug!(
                "[行处理器] --- 开始尾部关键词扫描 (绝对范围: [{}..{}], 回看行数: {}) ---",
                footer_scan_absolute_start_index,
                processed_paragraphs.len().saturating_sub(1), // 确保不为负
                END_LOOKBACK_COUNT
            );
            // 从后向前扫描尾部行
            for i in (footer_scan_absolute_start_index..processed_paragraphs.len()).rev() {
                let paragraph_text = get_plain_text_from_paragraph(&processed_paragraphs[i]);
                if line_matches_keyword_rule(&paragraph_text, keywords, keyword_case_sensitive) {
                    last_lyric_paragraph_exclusive_index = i; // 该行为元数据，歌词内容应在此行之前结束
                    debug!(
                        "[行处理器] 尾部行 {} ('{}') 是元数据。last_lyric_paragraph_exclusive_index 目前: {}",
                        i,
                        paragraph_text.chars().take(30).collect::<String>(),
                        last_lyric_paragraph_exclusive_index
                    );
                } else {
                    // 一旦遇到非元数据行，说明之前的行都是歌词内容，停止尾部扫描
                    debug!(
                        "[行处理器] 尾部行 {} ('{}') 不是元数据。停止尾部扫描。",
                        i,
                        paragraph_text.chars().take(30).collect::<String>()
                    );
                    break;
                }
            }
            debug!(
                "[行处理器] --- 完成尾部关键词扫描。歌词结束段落索引(不含): {} ---",
                last_lyric_paragraph_exclusive_index
            );
        } else {
            debug!("[行处理器] 跳过尾部扫描，因为头部扫描已移除所有段落或开始时就没有段落。");
            // 如果头部扫描移除了所有内容，确保尾部索引也相应调整
            last_lyric_paragraph_exclusive_index = first_lyric_paragraph_index;
        }

        // 只有当确定的歌词范围与原始范围不同时，才进行操作
        if first_lyric_paragraph_index > 0 || last_lyric_paragraph_exclusive_index < original_count
        {
            if first_lyric_paragraph_index < last_lyric_paragraph_exclusive_index {
                // 使用 retain_mut 和索引计数器就地过滤段落
                // 保留索引在 [first_lyric_paragraph_index, last_lyric_paragraph_exclusive_index) 范围内的段落
                let mut current_idx = 0;
                processed_paragraphs.retain(|_| {
                    // _ 表示我们不直接使用段落本身进行判断，而是用索引
                    let retain_this_paragraph = current_idx >= first_lyric_paragraph_index
                        && current_idx < last_lyric_paragraph_exclusive_index;
                    current_idx += 1;
                    retain_this_paragraph
                });
            } else {
                // 如果有效歌词范围为空（例如，头部元数据覆盖了尾部元数据），则清空所有段落
                processed_paragraphs.clear();
            }
            debug!(
                "[行处理器] 关键词移除后，剩余 {} 段落。",
                processed_paragraphs.len()
            );
        } else {
            debug!("[行处理器] 头部或尾部没有发生基于关键词的移除。");
        }
    } else {
        debug!("[行处理器] 关键词列表为空。跳过关键词移除阶段。");
    }
    // --- 关键词移除结束 ---

    // --- 阶段 2: 基于正则表达式移除任意匹配的行 ---
    if enable_regex_stripping && !regex_patterns.is_empty() && !processed_paragraphs.is_empty() {
        debug!(
            "[行处理器] --- 开始正则表达式扫描。表达式: {:?}, 区分大小写: {} ---",
            regex_patterns, regex_case_sensitive
        );
        // 编译用户提供的所有正则表达式
        let compiled_regexes: Vec<Regex> = regex_patterns
            .iter()
            .filter_map(|pattern_str| {
                if pattern_str.trim().is_empty() {
                    // 跳过空的表达式字符串
                    trace!("[行处理器] 跳过空的正则表达式。");
                    return None;
                }
                // 构建正则表达式，设置是否区分大小写和多行模式（此处设为false）
                RegexBuilder::new(pattern_str)
                    .case_insensitive(!regex_case_sensitive) // 注意：case_insensitive 与 regex_case_sensitive 是反相关的
                    .multi_line(false) // 通常歌词行按单行匹配
                    .build()
                    .map_err(|e| {
                        // 如果编译失败，记录警告
                        warn!("[行处理器] 编译正则表达式 '{}' 失败: {}", pattern_str, e);
                        e
                    })
                    .ok() // 转换 Result 为 Option，编译失败的表达式将被忽略
            })
            .collect();

        if !compiled_regexes.is_empty() {
            let mut regex_removed_count = 0;
            // 使用 retain 方法就地过滤段落，移除匹配任何一个已编译正则表达式的行
            processed_paragraphs.retain(|paragraph| {
                let paragraph_text = get_plain_text_from_paragraph(paragraph);
                for compiled_regex in &compiled_regexes {
                    if compiled_regex.is_match(&paragraph_text) {
                        debug!(
                            "[行处理器] 正则表达式 '{}' 匹配并移除了段落: '{}'",
                            compiled_regex.as_str(),
                            paragraph_text.chars().take(50).collect::<String>() // 日志显示部分被移除的行内容
                        );
                        regex_removed_count += 1;
                        return false; // 匹配则不保留 (retain 返回 false)
                    }
                }
                true // 未匹配任何正则，保留该段落 (retain 返回 true)
            });
            if regex_removed_count > 0 {
                debug!(
                    "[行处理器] 正则表达式移除操作移除了 {} 段落。",
                    regex_removed_count
                );
            } else {
                debug!("[行处理器] 正则表达式移除操作没有移除任何段落。");
            }
        } else {
            debug!("[行处理器] 没有编译有效的正则表达式。跳过正则表达式移除阶段。");
        }
    } else if enable_regex_stripping {
        // 虽然启用了正则移除，但可能因为表达式列表为空或已无段落可处理而跳过
        debug!("[行处理器] 正则表达式移除已启用，但未提供表达式或没有段落可处理。跳过正则阶段。");
    }
    // --- 正则表达式移除结束 ---

    // 记录最终处理结果
    if processed_paragraphs.len() < original_count {
        debug!(
            "[行处理器] 总段落数从 {} 减少到 {}。",
            original_count,
            processed_paragraphs.len()
        );
    } else {
        debug!(
            "[行处理器] 总计没有段落被移除。数量仍为 {}。",
            original_count
        );
    }

    processed_paragraphs // 返回处理后的段落列表
}
