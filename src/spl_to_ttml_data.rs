// 导入项目中定义的各种类型：
// AssMetadata: 用于存储元数据（尽管SPL格式本身不直接包含元数据，但为保持类型兼容性可能保留）。
// ConvertError: 错误处理枚举，用于表示转换过程中可能发生的错误。
// TtmlParagraph: TTML段落的数据结构，是此转换器的主要输出单元。
// LysSyllable: 解析器输出的原始音节结构，包含文本和计时信息。
// TtmlSyllable: 转换后用于TtmlParagraph的音节结构。
use crate::types::{AssMetadata, ConvertError, LysSyllable, TtmlParagraph, TtmlSyllable};
// 从 spl_parser 模块导入 SplLineBlock 结构体，这是SPL解析器的输出。
use crate::spl_parser::SplLineBlock;
// 导入 utils 模块，可能包含一些辅助函数，例如音节处理。
use crate::utils;

/// 将解析后的 SPL 行块 (`SplLineBlock`) 转换为 TTML 数据结构 (`TtmlParagraph`)。
///
/// # Arguments
/// * `spl_blocks` - `&[SplLineBlock]` 类型，一个包含已解析 SPL 块的切片。
/// * `_spl_metadata` - `Vec<AssMetadata>` 类型，来自SPL的元数据。
///   由于SPL格式通常不包含独立的元数据部分，此参数当前保留但未使用
///
/// # Returns
/// `Result<(Vec<TtmlParagraph>, Vec<AssMetadata>), ConvertError>` - 如果转换成功，返回：
///   - `Ok((ttml_paragraphs, ttml_generated_metadata))`：
///     - `ttml_paragraphs`: 一个 `Vec<TtmlParagraph>`，包含所有转换生成的TTML段落。
///     - `ttml_generated_metadata`: 一个 `Vec<AssMetadata>`，在当前实现中，SPL不生成新的元数据，故此向量为空。
///   - 如果发生错误，则返回 `Err(ConvertError)`。
pub fn convert_spl_to_ttml_data(
    spl_blocks: &[SplLineBlock],
    _spl_metadata: Vec<AssMetadata>, // SPL不支持元数据，此参数保留但未使用
) -> Result<(Vec<TtmlParagraph>, Vec<AssMetadata>), ConvertError> {
    // 初始化一个可变的向量，用于存储转换后的 TtmlParagraph 对象
    let mut ttml_paragraphs: Vec<TtmlParagraph> = Vec::new();
    // 初始化一个空的元数据向量。当前SPL到TTML的转换不生成新的元数据。
    let ttml_generated_metadata: Vec<AssMetadata> = Vec::new();

    // 遍历输入的每一个 SplLineBlock，并获取其索引
    for (idx, spl_block) in spl_blocks.iter().enumerate() {
        // 获取当前 SPL 块的第一个起始时间。如果没有，则默认为0。
        // spl_block.start_times_ms 是一个 Vec<u64>，因为SPL支持重复行。
        let block_main_start_ms = spl_block.start_times_ms.first().cloned().unwrap_or(0);

        // 确定此 SPL 块用于音节解析的整体结束时间 (block_end_ms_for_syllable_parsing)。
        // 这个时间主要用作后续 `parse_spl_main_text_to_syllables` 函数中音节时间计算的上限。
        let block_end_ms_for_syllable_parsing = if let Some(explicit_end) =
            spl_block.explicit_block_end_ms
        {
            // 优先使用 SPL 块中显式指定的结束时间。
            explicit_end
        } else {
            // 如果没有显式结束时间，则尝试从下一个块的开始时间推断，或进行估算。
            if idx < spl_blocks.len() - 1 {
                // 如果这不是最后一个块，则查看下一个块。
                let next_block_first_start_ms_opt = spl_blocks[idx + 1].start_times_ms.first();
                if let Some(&next_block_start_ms) = next_block_first_start_ms_opt {
                    if next_block_start_ms > block_main_start_ms {
                        // 下一个块的开始时间有效，用作当前块的隐式结束时间。
                        next_block_start_ms
                    } else {
                        // 下一个块的开始时间不合理（例如早于或等于当前块的开始时间）。
                        // 这种情况不应作为隐式结束时间，回退到估算。
                        log::warn!(
                            "[SPL 处理] 块 {} (开始于 {}ms): 下一个块的开始时间 {}ms 不适用于作为隐式结束时间。将使用开始时间加5秒作为结束时间。",
                            idx,
                            block_main_start_ms,
                            next_block_start_ms
                        );
                        block_main_start_ms + 5000 // 默认时长5秒
                    }
                } else {
                    // 下一个块存在，但没有有效的开始时间（理论上不太可能发生，因为解析器应确保）。
                    log::warn!(
                        "[SPL 处理] 块 {} (开始于 {}ms): 下一个块没有有效的开始时间。将使用开始时间加5秒作为结束时间。",
                        idx,
                        block_main_start_ms
                    );
                    block_main_start_ms + 5000 // 默认时长5秒
                }
            } else {
                // 这是列表中的最后一个块，没有后续块来确定隐式结束时间。
                block_main_start_ms + 5000 // 默认时长5秒
            }
        };

        // 解析 SPL 块中的主歌词文本 (main_text_with_inline_ts)，将其转换为 LysSyllable 音节列表。
        let main_syllables_lys: Vec<LysSyllable> =
            match crate::spl_parser::parse_spl_main_text_to_syllables(
                &spl_block.main_text_with_inline_ts, // 包含内联时间戳的主歌词文本
                block_main_start_ms,                 // 歌词行起始时间 (使用块的第一个开始时间)
                block_end_ms_for_syllable_parsing, // 歌词行用于音节解析的（可能更准确的）整体结束时间
                idx,                               // 日志用的行号/块索引
            ) {
                Ok(syls) => syls, // 解析成功，得到音节列表
                Err(e) => {
                    // 解析失败，记录警告信息，并返回一个空音节列表，避免程序中断。
                    log::warn!(
                        "[SPL 处理] 解析 SPL 块 {} \"{}\" 的主文本音节失败: {}. 将跳过此块的主歌词部分。",
                        idx,
                        spl_block.main_text_with_inline_ts,
                        e
                    );
                    Vec::new()
                }
            };

        // 如果一个 SPL 块既没有有效的主歌词音节（解析后为空列表），
        // 其原始主歌词文本去除空白后也为空，并且没有任何翻译行，
        // 则认为这是一个空的或无效的块（除非它有明确的独立时长，这种情况由解析器生成空文本块处理），跳过它。
        if main_syllables_lys.is_empty()
            && spl_block.main_text_with_inline_ts.trim().is_empty()
            && spl_block.all_translation_lines.is_empty()
        {
            // 进一步检查：如果块本身有显式结束时间且大于开始时间，它可能是一个有效的静默块。
            // 但 spl_parser 应该已经将这类情况（如 [t1][t2]）处理为 main_text_with_inline_ts 为空但有 explicit_block_end_ms。
            // 此处的 continue 主要是为了过滤掉那些完全没有内容和有效时长的块。
            if spl_block
                .explicit_block_end_ms
                .is_none_or(|end_ms| end_ms <= block_main_start_ms)
            {
                log::trace!("[SPL 转 TTML] 跳过空的 SPL 块索引 {}", idx);
                continue; // 处理下一个 spl_block
            }
        }

        // 将解析得到的 LysSyllable 列表转换为 TtmlSyllable 列表。
        let processed_main_syllables: Vec<TtmlSyllable> =
            utils::process_parsed_syllables_to_ttml(&main_syllables_lys, "SPL");

        // --- 处理翻译行 ---
        let translation_string: Option<String> = if !spl_block.all_translation_lines.is_empty() {
            Some(spl_block.all_translation_lines.join("/")) // 使用 "/" 连接多行翻译
        } else {
            None // 没有翻译行
        };
        let translation_tuple = translation_string.map(|t| (t, None));

        // --- 为 SPL 块的每个起始时间创建 TtmlParagraph ---
        // SPL 的重复行特性 ([t1][t2]歌词) 会导致 spl_block.start_times_ms 包含多个时间戳。
        // 对于每个这样的起始时间，我们都生成一个独立的 TtmlParagraph。
        for &line_start_time_ms_for_para in &spl_block.start_times_ms {
            // 确定当前这个 TtmlParagraph 的结束时间 (p_end_ms_for_para)。
            // 优先使用 SPL 块中显式指定的结束时间。
            // 如果没有，则根据已处理的主歌词音节推断 (最后一个音节的 end_ms)。
            // 如果连音节都没有（例如，主歌词行为空但有翻译，或者纯静默块），
            // 则结束时间默认为段落的开始时间（可能导致0时长，除非 explicit_block_end_ms 提供了时长）。
            let mut p_end_ms_for_para = spl_block.explicit_block_end_ms.unwrap_or_else(|| {
                processed_main_syllables
                    .last()
                    .map_or(line_start_time_ms_for_para, |syl| syl.end_ms)
            });

            // 确保结束时间不早于开始时间。
            if p_end_ms_for_para < line_start_time_ms_for_para {
                // 如果推断的结束时间早于开始时间（可能由于音节处理或默认值问题），
                // 尝试使用最后一个音节的持续时间来修正，或者如果无音节，则至少等于开始时间。
                log::warn!(
                    "[SPL 处理] 块 {} (行开始于 {}ms): 推断的段落结束时间 {}ms 早于开始时间。将进行校正。",
                    idx,
                    line_start_time_ms_for_para,
                    p_end_ms_for_para
                );
                p_end_ms_for_para = line_start_time_ms_for_para
                    + processed_main_syllables
                        .last()
                        .map_or(0, |s| s.end_ms.saturating_sub(s.start_ms));
                // 再次确保至少是开始时间
                if p_end_ms_for_para < line_start_time_ms_for_para {
                    p_end_ms_for_para = line_start_time_ms_for_para;
                }
            }

            ttml_paragraphs.push(TtmlParagraph {
                p_start_ms: line_start_time_ms_for_para, // 段落开始时间
                p_end_ms: p_end_ms_for_para,             // 段落结束时间
                main_syllables: processed_main_syllables.clone(), // 主歌词音节列表 (克隆)
                translation: translation_tuple.clone(),  // 翻译内容 (克隆)
                agent: "v1".to_string(),                 // SPL不支持对唱，硬编码
                romanization: None,                      // SPL不直接支持
                background_section: None,                // SPL不直接支持
                song_part: None,                         // SPL不直接支持
                itunes_key: None,
            });
        } // 结束对 spl_block.start_times_ms 的循环
    } // 结束对所有 spl_blocks 的循环

    // 返回转换得到的 TtmlParagraph 列表和空的元数据列表
    Ok((ttml_paragraphs, ttml_generated_metadata))
}
