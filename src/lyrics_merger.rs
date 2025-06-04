use crate::lrc_parser;
use crate::metadata_processor::MetadataStore;
use crate::qrc_parser;
use crate::types::DisplayLrcLine;
use crate::types::LrcLine;
use crate::types::LyricFormat;
use crate::types::{ConvertError, LrcContentType, TtmlParagraph};
use log::{debug, error, info, trace, warn};
use std::collections::HashMap;

pub struct PendingSecondaryLyrics {
    pub translation_lrc: Option<String>,
    pub romanization_qrc: Option<String>,
    pub romanization_lrc: Option<String>,
    pub krc_translation_lines: Option<Vec<String>>,
}

/// 辅助方法：将LRC格式的次要歌词内容逐行合并到主歌词段落中。
/// 适用于主歌词是YRC而次要歌词是LRC的情况。
///
/// # Arguments
/// * `primary_paragraphs` - 可变的主歌词段落列表 (`Vec<TtmlParagraph>`)。
/// * `lrc_content_str` - 包含次要歌词的完整LRC文本字符串。
/// * `content_type` - 指示LRC内容是翻译还是罗马音。
/// * `language_code` - 可选的语言代码 (主要用于翻译)。
///
/// # Returns
/// `Result<(), ConvertError>` - 如果解析LRC内容时发生错误，则返回Err。
pub(crate) fn merge_lrc_content_line_by_line_with_primary_paragraphs(
    primary_paragraphs: &mut [TtmlParagraph],
    lrc_content_str: &str, // 假设这是原始LRC文本
    content_type: LrcContentType,
    language_code: Option<String>,
) -> Result<(), ConvertError> {
    if lrc_content_str.is_empty() || primary_paragraphs.is_empty() {
        trace!(
            "[LyricsMerger] 逐行合并：LRC内容或主段落为空，跳过。类型: {:?}",
            content_type
        );
        return Ok(());
    }
    debug!(
        "[LyricsMerger] 开始逐行合并LRC内容，类型: {:?}, 主段落数: {}",
        content_type,
        primary_paragraphs.len()
    );

    // 解析原始LRC文本，现在返回 Vec<DisplayLrcLine>
    let (display_lrc_lines, _bilingual_translations, _parsed_lrc_meta) =
        match lrc_parser::parse_lrc_text_to_lines(lrc_content_str) {
            Ok(result) => result,
            Err(e) => {
                error!("[LyricsMerger] 逐行合并时解析LRC内容失败: {}", e);
                return Err(e);
            }
        };

    // 过滤出可用的 LrcLine
    let mut lrc_lines_for_merge: Vec<LrcLine> = display_lrc_lines
        .into_iter()
        .filter_map(|display_line| match display_line {
            DisplayLrcLine::Parsed(lrc_line) => Some(lrc_line),
            DisplayLrcLine::Raw { .. } => None,
        })
        .collect();

    // 确保按时间戳排序，因为逐行合并依赖于顺序
    lrc_lines_for_merge.sort_by_key(|line| line.timestamp_ms);

    if lrc_lines_for_merge.is_empty() {
        info!(
            "[LyricsMerger] 逐行合并：LRC解析后有效行数为0，不执行合并。类型: {:?}",
            content_type
        );
        return Ok(());
    }

    for (para_idx, primary_para) in primary_paragraphs.iter_mut().enumerate() {
        if let Some(lrc_line) = lrc_lines_for_merge.get(para_idx) {
            // 使用 lrc_lines_for_merge
            let text_to_set = lrc_line.text.clone();
            match content_type {
                LrcContentType::Romanization => {
                    primary_para.romanization = Some(text_to_set);
                }
                LrcContentType::Translation => {
                    primary_para.translation = Some((text_to_set, language_code.clone()));
                }
            }
        } else {
            warn!(
                "[LyricsMerger] 逐行合并：有效LRC行数 ({}) 少于主歌词段落数 ({})，段落 #{} 及之后无匹配。类型: {:?}",
                lrc_lines_for_merge.len(),
                primary_paragraphs.len(),
                para_idx,
                content_type
            );
            break;
        }
    }
    debug!(
        "[LyricsMerger] 逐行合并LRC内容完成，类型: {:?}",
        content_type
    );
    Ok(())
}

/// 辅助方法：将QRC格式的次要歌词内容（主要是罗马音）合并到主歌词段落中。
///
/// # Arguments
/// * `primary_paragraphs` - 可变的主歌词段落列表 (`Vec<TtmlParagraph>`)。
/// * `qrc_content` - 包含次要歌词的完整QRC文本字符串。
/// * `content_type` - 指示QRC内容是翻译还是罗马音。
///
/// # Returns
/// `Result<(), ConvertError>` - 如果解析QRC内容时发生错误，则返回Err。
pub(crate) fn merge_secondary_qrc_into_paragraphs_internal(
    primary_paragraphs: &mut [TtmlParagraph],
    qrc_content: &str,
    content_type: LrcContentType,
) -> Result<(), ConvertError> {
    if qrc_content.is_empty() || primary_paragraphs.is_empty() {
        trace!(
            "[LyricsMerger] 合并QRC：QRC内容或主段落为空，跳过。类型: {:?}",
            content_type
        );
        return Ok(());
    }
    debug!(
        "[LyricsMerger] 开始合并QRC内容，类型: {:?}, 主段落数: {}",
        content_type,
        primary_paragraphs.len()
    );

    let (secondary_qrc_lines, _secondary_qrc_meta) =
        match qrc_parser::load_qrc_from_string(qrc_content) {
            Ok(result) => result,
            Err(e) => {
                error!("[LyricsMerger] 合并QRC时解析QRC内容失败: {}", e);
                return Err(e);
            }
        };

    if secondary_qrc_lines.is_empty() {
        info!(
            "[LyricsMerger] 合并QRC：QRC解析后行数为0，不执行合并。类型: {:?}",
            content_type
        );
        return Ok(());
    }

    const QRC_MATCH_TOLERANCE_MS: u64 = 15;
    let mut qrc_search_start_idx = 0;

    for primary_paragraph in primary_paragraphs.iter_mut() {
        let para_start_ms = primary_paragraph.p_start_ms;
        let mut best_match_qrc_line_idx: Option<usize> = None;
        let mut smallest_diff_ms = u64::MAX;

        for (current_qrc_idx, sec_qrc_line) in secondary_qrc_lines
            .iter()
            .enumerate()
            .skip(qrc_search_start_idx)
        {
            let sec_qrc_line_start_ms = sec_qrc_line.line_start_ms;
            let diff_ms = (sec_qrc_line_start_ms as i64 - para_start_ms as i64).unsigned_abs();

            if diff_ms <= QRC_MATCH_TOLERANCE_MS {
                if diff_ms < smallest_diff_ms {
                    smallest_diff_ms = diff_ms;
                    best_match_qrc_line_idx = Some(current_qrc_idx);
                }
            } else if sec_qrc_line_start_ms > para_start_ms + QRC_MATCH_TOLERANCE_MS {
                break;
            }
        }

        if let Some(matched_idx) = best_match_qrc_line_idx {
            let sec_qrc_line = &secondary_qrc_lines[matched_idx];
            let mut combined_text_for_line = String::new();
            if !sec_qrc_line.syllables.is_empty() {
                combined_text_for_line = sec_qrc_line
                    .syllables
                    .iter()
                    .map(|syl| syl.text.clone())
                    .collect::<String>()
                    .trim()
                    .to_string();
            }

            if !combined_text_for_line.is_empty() || sec_qrc_line.line_duration_ms > 0
            // Also merge if it's a timed empty line
            {
                match content_type {
                    LrcContentType::Romanization => {
                        primary_paragraph.romanization = Some(combined_text_for_line.clone());
                        trace!(
                            "[LyricsMerger] 合并QRC: 段落 [{}ms] 匹配到罗马音QRC行 [{}ms]: '{}'",
                            para_start_ms, sec_qrc_line.line_start_ms, combined_text_for_line
                        );
                    }
                    LrcContentType::Translation => {
                        primary_paragraph.translation =
                            Some((combined_text_for_line.clone(), None)); // QRC usually doesn't specify lang for translation
                        trace!(
                            "[LyricsMerger] 合并QRC: 段落 [{}ms] 匹配到翻译QRC行 [{}ms]: '{}'",
                            para_start_ms, sec_qrc_line.line_start_ms, combined_text_for_line
                        );
                    }
                }
            }
            qrc_search_start_idx = matched_idx + 1;
        }
    }
    debug!("[LyricsMerger] 合并QRC内容完成，类型: {:?}", content_type);
    Ok(())
}

/// 辅助方法：将LRC格式的次要歌词内容（翻译或罗马音）按时间戳合并到主歌词段落中。
///
/// # Arguments
/// * `primary_paragraphs` - 可变的主歌词段落列表 (`Vec<TtmlParagraph>`)。
/// * `lrc_content` - 包含次要歌词的完整LRC文本字符串。
/// * `content_type` - 指示LRC内容是翻译还是罗马音。
/// * `language_code_from_lrc_meta` - 从LRC文件头部元数据中解析出的可选语言代码。
///
/// # Returns
/// `Result<(), ConvertError>` - 如果解析LRC内容时发生错误，则返回Err。
pub(crate) fn merge_lrc_lines_into_paragraphs_internal(
    primary_paragraphs: &mut [TtmlParagraph],
    lrc_content: &str, // 这是从 app.rs 传入的预处理后的翻译LRC字符串
    content_type: LrcContentType,
    language_code_from_lrc_meta: Option<String>,
) -> Result<(), ConvertError> {
    if lrc_content.is_empty() || primary_paragraphs.is_empty() {
        info!(
            "[LyricsMerger] 时间戳合并LRC：LRC内容或主段落为空，跳过。类型: {:?}",
            content_type
        );
        return Ok(());
    }
    debug!(
        "[LyricsMerger] 开始时间戳合并LRC内容，类型: {:?}, 主段落数: {}",
        content_type,
        primary_paragraphs.len()
    );

    let (display_lrc_lines, _bilingual_translations, parsed_lrc_meta) =
        match lrc_parser::parse_lrc_text_to_lines(lrc_content) {
            Ok(result) => result,
            Err(e) => {
                error!("[LyricsMerger] 时间戳合并LRC时解析LRC内容失败: {}", e);
                return Err(e);
            }
        };

    let mut lrc_lines_for_merge: Vec<LrcLine> = display_lrc_lines
        .into_iter()
        .filter_map(|display_line| match display_line {
            DisplayLrcLine::Parsed(lrc_line) => Some(lrc_line),
            DisplayLrcLine::Raw { .. } => None,
        })
        .collect();

    lrc_lines_for_merge.sort_by_key(|line| line.timestamp_ms);

    if lrc_lines_for_merge.is_empty() {
        info!(
            "[LyricsMerger] 时间戳合并LRC：LRC解析后有效行数为0，不执行合并。类型: {:?}",
            content_type
        );
        for primary_paragraph in primary_paragraphs.iter_mut() {
            match content_type {
                LrcContentType::Romanization => primary_paragraph.romanization = None,
                LrcContentType::Translation => primary_paragraph.translation = None,
            }
            if let Some(bg_section) = primary_paragraph.background_section.as_mut() {
                match content_type {
                    LrcContentType::Romanization => bg_section.romanization = None,
                    LrcContentType::Translation => bg_section.translation = None,
                }
            }
        }
        return Ok(());
    }

    let final_language_code_for_translation: Option<String> =
        if content_type == LrcContentType::Translation {
            language_code_from_lrc_meta.or_else(|| {
                parsed_lrc_meta
                    .iter()
                    .find(|m| {
                        m.key.eq_ignore_ascii_case("language") || m.key.eq_ignore_ascii_case("lang")
                    })
                    .map(|m| m.value.clone())
            })
        } else {
            None
        };

    const LRC_MATCH_TOLERANCE_MS: u64 = 15;

    for primary_paragraph in primary_paragraphs.iter_mut() {
        match content_type {
            LrcContentType::Romanization => primary_paragraph.romanization = None,
            LrcContentType::Translation => primary_paragraph.translation = None,
        }
        if let Some(bg_section) = primary_paragraph.background_section.as_mut() {
            match content_type {
                LrcContentType::Romanization => bg_section.romanization = None,
                LrcContentType::Translation => bg_section.translation = None,
            }
        }

        let para_start_ms = primary_paragraph.p_start_ms;
        let mut best_match_lrc_line_idx: Option<usize> = None;
        let mut smallest_diff_ms = u64::MAX;

        for (current_lrc_idx, lrc_line) in lrc_lines_for_merge.iter().enumerate() {
            //.skip(lrc_search_start_idx) 移除了skip
            let lrc_ts = lrc_line.timestamp_ms;
            let diff_ms = (lrc_ts as i64 - para_start_ms as i64).unsigned_abs();

            if diff_ms <= LRC_MATCH_TOLERANCE_MS {
                if diff_ms < smallest_diff_ms {
                    smallest_diff_ms = diff_ms;
                    best_match_lrc_line_idx = Some(current_lrc_idx);
                }
            } else if lrc_ts > para_start_ms + LRC_MATCH_TOLERANCE_MS
                && best_match_lrc_line_idx.is_some()
            {
                // 如果已经有最佳匹配，且当前行已超出容差，则停止
                break;
            }
        }

        if let Some(matched_idx) = best_match_lrc_line_idx {
            let lrc_line = &lrc_lines_for_merge[matched_idx];
            let text_to_set = lrc_line.text.clone();

            match content_type {
                LrcContentType::Romanization => {
                    primary_paragraph.romanization = Some(text_to_set);
                }
                LrcContentType::Translation => {
                    primary_paragraph.translation =
                        Some((text_to_set, final_language_code_for_translation.clone()));
                }
            }
        }

        if let Some(bg_section) = primary_paragraph.background_section.as_mut() {
            let bg_start_ms = bg_section.start_ms;
            let mut best_match_bg_lrc_line_idx: Option<usize> = None;
            let mut smallest_diff_bg_ms = u64::MAX;
            for (current_lrc_idx, lrc_line) in lrc_lines_for_merge.iter().enumerate() {
                let lrc_ts = lrc_line.timestamp_ms;
                let diff_ms = (lrc_ts as i64 - bg_start_ms as i64).unsigned_abs();
                if diff_ms <= LRC_MATCH_TOLERANCE_MS {
                    if diff_ms < smallest_diff_bg_ms {
                        smallest_diff_bg_ms = diff_ms;
                        best_match_bg_lrc_line_idx = Some(current_lrc_idx);
                    }
                } else if lrc_ts > bg_start_ms + LRC_MATCH_TOLERANCE_MS
                    && best_match_bg_lrc_line_idx.is_some()
                {
                    break;
                }
            }
            if let Some(matched_idx_bg) = best_match_bg_lrc_line_idx {
                let lrc_line_bg = &lrc_lines_for_merge[matched_idx_bg];
                let text_to_set_bg = lrc_line_bg.text.clone();
                match content_type {
                    LrcContentType::Romanization => {
                        bg_section.romanization = Some(text_to_set_bg);
                    }
                    LrcContentType::Translation => {
                        bg_section.translation =
                            Some((text_to_set_bg, final_language_code_for_translation.clone()));
                    }
                }
            }
        }
    }
    Ok(())
}

/// 合并从网络下载获取的次要歌词（如翻译LRC、罗马音QRC/LRC）到主歌词段落中。
pub fn merge_downloaded_secondary_lyrics(
    primary_paragraphs_opt: &mut Option<Vec<TtmlParagraph>>,
    pending_lyrics: PendingSecondaryLyrics,
    session_platform_metadata: &HashMap<String, String>,
    metadata_store: &MetadataStore,
    source_format: LyricFormat,
) -> (Option<Vec<DisplayLrcLine>>, Option<Vec<DisplayLrcLine>>) {
    let mut independently_loaded_translation_lrc: Option<Vec<DisplayLrcLine>> = None;
    let mut independently_loaded_romanization_lrc: Option<Vec<DisplayLrcLine>> = None;

    let primary_paragraphs_are_empty_or_none =
        primary_paragraphs_opt.as_ref().is_none_or(|p| p.is_empty());

    // --- 处理翻译 ---
    if let Some(trans_lrc_content_str) = pending_lyrics.translation_lrc {
        if primary_paragraphs_are_empty_or_none {
            match lrc_parser::parse_lrc_text_to_lines(&trans_lrc_content_str) {
                Ok((lines, _bilingual_translations, _meta)) => {
                    if !lines.is_empty() {
                        independently_loaded_translation_lrc = Some(lines);
                    }
                    info!(
                        "[LyricsMerger] 主段落为空，独立解析了翻译LRC ({}行)。",
                        independently_loaded_translation_lrc
                            .as_ref()
                            .map_or(0, |v| v.len())
                    );
                }
                Err(e) => error!("[LyricsMerger] 主段落为空时，解析独立翻译LRC失败: {}", e),
            }
        } else if let Some(primary_paragraphs) = primary_paragraphs_opt {
            let lang_code_for_merge: Option<String> = session_platform_metadata
                .get("language")
                .cloned()
                .or_else(|| {
                    metadata_store
                        .get_single_value(&crate::types::CanonicalMetadataKey::Language)
                        .cloned()
                });

            if source_format == LyricFormat::Yrc {
                info!("[LyricsMerger] 主歌词为YRC，正在逐行合并LRC格式的翻译...");
                if let Err(e) = merge_lrc_content_line_by_line_with_primary_paragraphs(
                    primary_paragraphs,
                    &trans_lrc_content_str,
                    LrcContentType::Translation,
                    lang_code_for_merge,
                ) {
                    error!("[LyricsMerger] 逐行合并LRC翻译到YRC主歌词失败: {}", e);
                }
            } else {
                info!(
                    "[LyricsMerger] 主歌词非YRC (当前为 {:?})，正在按时间戳合并下载的LRC翻译...",
                    source_format
                );
                if let Err(e) = merge_lrc_lines_into_paragraphs_internal(
                    primary_paragraphs,
                    &trans_lrc_content_str,
                    LrcContentType::Translation,
                    lang_code_for_merge,
                ) {
                    error!("[LyricsMerger] 按时间戳合并下载的LRC翻译失败: {}", e);
                }
            }
        }
    }

    // 处理罗马音
    if let Some(roma_qrc_content_str) = pending_lyrics.romanization_qrc {
        if primary_paragraphs_are_empty_or_none {
            warn!("[LyricsMerger] 主段落为空，无法合并QRC罗马音。");
        } else if let Some(primary_paragraphs) = primary_paragraphs_opt {
            info!("[LyricsMerger] 正在按时间戳合并下载的QRC罗马音...");
            if let Err(e) = merge_secondary_qrc_into_paragraphs_internal(
                primary_paragraphs,
                &roma_qrc_content_str,
                LrcContentType::Romanization,
            ) {
                error!("[LyricsMerger] 合并下载的QRC罗马音失败: {}", e);
            }
        }
    } else if let Some(roma_lrc_content_str) = pending_lyrics.romanization_lrc {
        if primary_paragraphs_are_empty_or_none {
            match lrc_parser::parse_lrc_text_to_lines(&roma_lrc_content_str) {
                Ok((lines, _bilingual_translations, _meta)) => {
                    if !lines.is_empty() {
                        independently_loaded_romanization_lrc = Some(lines);
                    }
                    info!(
                        "[LyricsMerger] 主段落为空，独立解析了罗马音LRC ({}行)。",
                        independently_loaded_romanization_lrc
                            .as_ref()
                            .map_or(0, |v| v.len())
                    );
                }
                Err(e) => error!("[LyricsMerger] 主段落为空时，解析独立罗马音LRC失败: {}", e),
            }
        } else if let Some(primary_paragraphs) = primary_paragraphs_opt {
            if source_format == LyricFormat::Yrc {
                info!("[LyricsMerger] 主歌词为YRC，正在逐行合并LRC格式的罗马音...");
                if let Err(e) = merge_lrc_content_line_by_line_with_primary_paragraphs(
                    primary_paragraphs,
                    &roma_lrc_content_str,
                    LrcContentType::Romanization,
                    None,
                ) {
                    error!("[LyricsMerger] 逐行合并LRC罗马音到YRC主歌词失败: {}", e);
                }
            } else {
                info!(
                    "[LyricsMerger] 主歌词非YRC (当前为 {:?})，正在按时间戳合并下载的LRC罗马音...",
                    source_format
                );
                if let Err(e) = merge_lrc_lines_into_paragraphs_internal(
                    primary_paragraphs,
                    &roma_lrc_content_str,
                    LrcContentType::Romanization,
                    None,
                ) {
                    error!("[LyricsMerger] 按时间戳合并下载的LRC罗马音失败: {}", e);
                }
            }
        }
    }

    // KRC内嵌翻译的处理逻辑
    if let Some(trans_lines) = pending_lyrics.krc_translation_lines {
        if let Some(paragraphs) = primary_paragraphs_opt {
            if !paragraphs.is_empty() && !trans_lines.is_empty() {
                info!(
                    "[LyricsMerger] 正在应用KRC内嵌翻译 (共 {} 行翻译到 {} 个段落)",
                    trans_lines.len(),
                    paragraphs.len()
                );
                for (i, para_line) in paragraphs.iter_mut().enumerate() {
                    if let Some(trans_text) = trans_lines.get(i) {
                        let text_to_use = if trans_text.trim().is_empty() || trans_text == "//" {
                            ""
                        } else {
                            trans_text.as_str()
                        };
                        if para_line.translation.is_none()
                            || para_line
                                .translation
                                .as_ref()
                                .is_some_and(|(t, _)| t.is_empty())
                        {
                            para_line.translation = Some((text_to_use.to_string(), None));
                        }
                    }
                }
            }
        }
    }
    (
        independently_loaded_translation_lrc,
        independently_loaded_romanization_lrc,
    )
}

/// 将通过“加载翻译/罗马音LRC”菜单加载的LRC行合并到当前的主歌词段落中。
pub fn merge_manually_loaded_lrc_into_paragraphs(
    primary_paragraphs_opt: &mut Option<Vec<TtmlParagraph>>,
    loaded_translation_lrc: Option<&Vec<DisplayLrcLine>>, // 只读访问
    loaded_romanization_lrc: Option<&Vec<DisplayLrcLine>>, // 只读访问
    metadata_store: &MetadataStore,                       // 只读访问
) {
    if primary_paragraphs_opt.is_none() {
        debug!("[LyricsMerger MergeManually] No main paragraphs, skipping merge.");
        return;
    }

    let paragraphs = primary_paragraphs_opt.as_mut().unwrap();
    if paragraphs.is_empty() {
        debug!("[LyricsMerger MergeManually] Main paragraphs list is empty, skipping merge.");
        return;
    }

    const LRC_MATCH_TOLERANCE_MS: u64 = 15; // 匹配容差

    let specific_translation_lang_for_para: Option<String>;
    {
        // metadata_store 已作为参数传入
        specific_translation_lang_for_para = metadata_store
            .get_single_value_by_str("translation_language")
            .cloned()
            .or_else(|| {
                metadata_store
                    .get_single_value(&crate::types::CanonicalMetadataKey::Language) // 确保路径正确
                    .cloned()
            });
    }

    // --- 合并翻译 LRC ---
    let mut translation_lines_for_merge: Vec<LrcLine> = Vec::new();
    if let Some(display_lines_vec) = loaded_translation_lrc {
        translation_lines_for_merge = display_lines_vec
            .iter()
            .filter_map(|entry| match entry {
                DisplayLrcLine::Parsed(lrc_line) => Some(lrc_line.clone()),
                DisplayLrcLine::Raw { .. } => None,
            })
            .collect();
        translation_lines_for_merge.sort_by_key(|line| line.timestamp_ms);
    }

    if !translation_lines_for_merge.is_empty() {
        debug!(
            "[LyricsMerger MergeManually] Merging {} parsed translation LRC lines into {} paragraphs.",
            translation_lines_for_merge.len(),
            paragraphs.len()
        );
        let mut available_lrc_lines: Vec<(&LrcLine, bool)> = translation_lines_for_merge
            .iter()
            .map(|line| (line, false))
            .collect();

        for paragraph in paragraphs.iter_mut() {
            paragraph.translation = None;
            let para_start_ms = paragraph.p_start_ms;
            let mut best_match_main_idx: Option<usize> = None;
            let mut smallest_diff_main = u64::MAX;

            for (current_lrc_idx, (lrc_line, used)) in available_lrc_lines.iter().enumerate() {
                if *used {
                    continue;
                }
                let diff = (lrc_line.timestamp_ms as i64 - para_start_ms as i64).unsigned_abs();
                if diff <= LRC_MATCH_TOLERANCE_MS {
                    if diff < smallest_diff_main {
                        smallest_diff_main = diff;
                        best_match_main_idx = Some(current_lrc_idx);
                    }
                } else if lrc_line.timestamp_ms > para_start_ms + LRC_MATCH_TOLERANCE_MS
                    && best_match_main_idx.is_some()
                {
                    break;
                }
            }

            if let Some(matched_idx) = best_match_main_idx {
                let (matched_lrc, used_flag_ref) = &mut available_lrc_lines[matched_idx];
                paragraph.translation = Some((
                    matched_lrc.text.clone(),
                    specific_translation_lang_for_para.clone(),
                ));
                *used_flag_ref = true;
            }

            if let Some(bg_section_mut) = paragraph.background_section.as_mut() {
                bg_section_mut.translation = None;
                let bg_start_ms = bg_section_mut.start_ms;
                let mut best_match_bg_idx: Option<usize> = None;
                let mut smallest_diff_bg = u64::MAX;
                for (current_lrc_idx, (lrc_line, used)) in available_lrc_lines.iter().enumerate() {
                    if *used {
                        continue;
                    }
                    let diff = (lrc_line.timestamp_ms as i64 - bg_start_ms as i64).unsigned_abs();
                    if diff <= LRC_MATCH_TOLERANCE_MS {
                        if diff < smallest_diff_bg {
                            smallest_diff_bg = diff;
                            best_match_bg_idx = Some(current_lrc_idx);
                        }
                    } else if lrc_line.timestamp_ms > bg_start_ms + LRC_MATCH_TOLERANCE_MS
                        && best_match_bg_idx.is_some()
                    {
                        break;
                    }
                }
                if let Some(matched_idx_bg) = best_match_bg_idx {
                    let (matched_lrc_bg, used_flag_ref_bg) =
                        &mut available_lrc_lines[matched_idx_bg];
                    bg_section_mut.translation = Some((
                        matched_lrc_bg.text.clone(),
                        specific_translation_lang_for_para.clone(),
                    ));
                    *used_flag_ref_bg = true;
                }
            }
        }
    } else {
        for paragraph in paragraphs.iter_mut() {
            paragraph.translation = None;
            if let Some(bg_section) = paragraph.background_section.as_mut() {
                bg_section.translation = None;
            }
        }
    }

    // --- 合并罗马音 LRC (逻辑与翻译LRC类似) ---
    let mut romanization_lines_for_merge: Vec<LrcLine> = Vec::new();
    if let Some(display_lines_vec) = loaded_romanization_lrc {
        romanization_lines_for_merge = display_lines_vec
            .iter()
            .filter_map(|entry| match entry {
                DisplayLrcLine::Parsed(lrc_line) => Some(lrc_line.clone()),
                DisplayLrcLine::Raw { .. } => None,
            })
            .collect();
        romanization_lines_for_merge.sort_by_key(|line| line.timestamp_ms);
    }

    if !romanization_lines_for_merge.is_empty() {
        debug!(
            "[LyricsMerger MergeManually] Merging {} parsed romanization LRC lines into {} paragraphs.",
            romanization_lines_for_merge.len(),
            paragraphs.len()
        );
        let mut available_lrc_lines: Vec<(&LrcLine, bool)> = romanization_lines_for_merge
            .iter()
            .map(|line| (line, false))
            .collect();

        for paragraph in paragraphs.iter_mut() {
            paragraph.romanization = None;
            let para_start_ms = paragraph.p_start_ms;
            let mut best_match_main_idx: Option<usize> = None;
            let mut smallest_diff_main = u64::MAX;

            for (current_lrc_idx, (lrc_line, used)) in available_lrc_lines.iter().enumerate() {
                if *used {
                    continue;
                }
                let diff = (lrc_line.timestamp_ms as i64 - para_start_ms as i64).unsigned_abs();
                if diff <= LRC_MATCH_TOLERANCE_MS {
                    if diff < smallest_diff_main {
                        smallest_diff_main = diff;
                        best_match_main_idx = Some(current_lrc_idx);
                    }
                } else if lrc_line.timestamp_ms > para_start_ms + LRC_MATCH_TOLERANCE_MS
                    && best_match_main_idx.is_some()
                {
                    break;
                }
            }

            if let Some(matched_idx) = best_match_main_idx {
                let (matched_lrc, used_flag_ref) = &mut available_lrc_lines[matched_idx];
                paragraph.romanization = Some(matched_lrc.text.clone());
                *used_flag_ref = true;
            }

            if let Some(bg_section_mut) = paragraph.background_section.as_mut() {
                bg_section_mut.romanization = None;
                let bg_start_ms = bg_section_mut.start_ms;
                let mut best_match_bg_idx: Option<usize> = None;
                let mut smallest_diff_bg = u64::MAX;
                for (current_lrc_idx, (lrc_line, used)) in available_lrc_lines.iter().enumerate() {
                    if *used {
                        continue;
                    }
                    let diff = (lrc_line.timestamp_ms as i64 - bg_start_ms as i64).unsigned_abs();
                    if diff <= LRC_MATCH_TOLERANCE_MS {
                        if diff < smallest_diff_bg {
                            smallest_diff_bg = diff;
                            best_match_bg_idx = Some(current_lrc_idx);
                        }
                    } else if lrc_line.timestamp_ms > bg_start_ms + LRC_MATCH_TOLERANCE_MS
                        && best_match_bg_idx.is_some()
                    {
                        break;
                    }
                }
                if let Some(matched_idx_bg) = best_match_bg_idx {
                    let (matched_lrc_bg, used_flag_ref_bg) =
                        &mut available_lrc_lines[matched_idx_bg];
                    bg_section_mut.romanization = Some(matched_lrc_bg.text.clone());
                    *used_flag_ref_bg = true;
                }
            }
        }
    } else {
        for paragraph in paragraphs.iter_mut() {
            paragraph.romanization = None;
            if let Some(bg_section) = paragraph.background_section.as_mut() {
                bg_section.romanization = None;
            }
        }
    }
}
