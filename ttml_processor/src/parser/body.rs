//! # TTML 解析器 - Body 处理模块
//!
//! 该模块包含了所有用于解析 TTML 文件中 `<body>` 块的函数，
//! 包括处理 `<p>`, `<span>` 标签和文本内容。

use std::{collections::HashMap, str};

use super::{
    state::{
        CurrentPElementData, LastSyllableInfo, MetadataParseState, SpanContext, SpanRole,
        TtmlParserState,
    },
    utils::{
        clean_parentheses_from_bg_text_into, get_attribute_with_aliases, get_string_attribute,
        get_time_attribute, normalize_text_whitespace_into,
    },
};
use lyrics_helper_core::{
    AnnotatedTrack, ContentType, ConvertError, LyricLine, LyricSyllable, LyricTrack,
    TrackMetadataKey, Word,
};
use quick_xml::{
    Reader,
    events::{BytesStart, BytesText, Event},
};

use super::constants::{
    ATTR_BEGIN, ATTR_END, ATTR_ROLE, ATTR_ROLE_ALIAS, ATTR_XML_LANG, ATTR_XML_SCHEME,
    ROLE_BACKGROUND, ROLE_ROMANIZATION, ROLE_TRANSLATION, TAG_BR, TAG_P, TAG_SPAN,
};

/// 处理在 `<p>` 标签内部的事件。
pub(super) fn handle_p_event(
    event: &Event<'_>,
    state: &mut TtmlParserState,
    reader: &Reader<&[u8]>,
    lines: &mut Vec<LyricLine>,
    warnings: &mut Vec<String>,
) -> Result<(), ConvertError> {
    match event {
        Event::Start(e) if e.local_name().as_ref() == TAG_SPAN => {
            process_span_start(e, state, reader, warnings)?;
        }
        Event::Text(e) => process_text_event(e, state)?,
        Event::GeneralRef(e) => {
            let entity_name = str::from_utf8(e.as_ref())
                .map_err(|err| ConvertError::Internal(format!("无法将实体名解码为UTF-8: {err}")))?;

            let decoded_char = if let Some(num_str) = entity_name.strip_prefix('#') {
                let (radix, code_point_str) = num_str
                    .strip_prefix('x')
                    .map_or((10, num_str), |stripped| (16, stripped));

                u32::from_str_radix(code_point_str, radix).map_or_else(
                    |_| {
                        warnings.push(format!("无法解析无效的XML数字实体 '&{entity_name};'"));
                        '\0'
                    },
                    |code_point| char::from_u32(code_point).unwrap_or('\0'),
                )
            } else {
                match entity_name {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    _ => {
                        warnings.push(format!("忽略了未知的XML实体 '&{entity_name};'"));
                        '\0'
                    }
                }
            };

            if decoded_char != '\0'
                && let Some(p_data) = state.body_state.current_p_element_data.as_mut()
            {
                if state.body_state.span_stack.is_empty() {
                    p_data.line_text_accumulator.push(decoded_char);
                } else {
                    state.text_buffer.push(decoded_char);
                }
            }
        }
        Event::End(e) => match e.local_name().as_ref() {
            TAG_BR => {
                warnings.push(format!(
                    "在 <p> ({}ms-{}ms) 中发现并忽略了一个 <br/> 标签。",
                    state
                        .body_state
                        .current_p_element_data
                        .as_ref()
                        .map_or(0, |d| d.start_ms),
                    state
                        .body_state
                        .current_p_element_data
                        .as_ref()
                        .map_or(0, |d| d.end_ms)
                ));
            }
            TAG_P => {
                handle_p_end(state, lines, warnings);
            }
            TAG_SPAN => {
                process_span_end(state, warnings)?;
            }
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

/// 处理 `</p>` 结束事件。
/// 在此事件中，会回填来自 <iTunesMetadata> 的逐行翻译
pub(super) fn handle_p_end(
    state: &mut TtmlParserState,
    lines: &mut Vec<LyricLine>,
    warnings: &mut [String],
) {
    if let Some(mut p_data) = state.body_state.current_p_element_data.take() {
        if let Some(key) = &p_data.itunes_key {
            // 回填逐行翻译
            if let Some(translations_for_line) = state.metadata_state.line_translation_map.get(key)
            {
                for (line_translation, lang) in translations_for_line {
                    // 处理主音轨翻译
                    if let Some(main_text) = &line_translation.main
                        && let Some(main_annotated_track) = p_data
                            .tracks_accumulator
                            .iter_mut()
                            .find(|at| at.content_type == ContentType::Main)
                    {
                        // 检查是否已存在具有相同文本的翻译轨道
                        let translation_exists =
                            main_annotated_track.translations.iter().any(|track| {
                                track
                                    .words
                                    .iter()
                                    .flat_map(|w| &w.syllables)
                                    .any(|s| s.text == *main_text)
                            });

                        if !translation_exists {
                            let translation_track =
                                create_simple_translation_track(main_text, lang.as_ref());
                            main_annotated_track.translations.push(translation_track);
                        }
                    }

                    // 处理背景人声音轨的翻译
                    if let Some(bg_text) = &line_translation.background {
                        let bg_annotated_track = get_or_create_target_annotated_track(
                            &mut p_data,
                            ContentType::Background,
                        );

                        let translation_exists =
                            bg_annotated_track.translations.iter().any(|track| {
                                track
                                    .words
                                    .iter()
                                    .flat_map(|w| &w.syllables)
                                    .any(|s| s.text == *bg_text)
                            });

                        if !translation_exists {
                            let translation_track =
                                create_simple_translation_track(bg_text, lang.as_ref());
                            bg_annotated_track.translations.push(translation_track);
                        }
                    }
                }
            }
        }
        finalize_p_element(p_data, lines, state, warnings);
    }
    // 重置 p 内部的状态
    state.body_state.in_p = false;
    state.body_state.span_stack.clear();
    state.body_state.last_syllable_info = LastSyllableInfo::None;
}

/// 处理 `<span>` 标签的开始事件。
/// 这是解析器中最复杂的部分之一，需要确定 span 的角色、语言和时间信息。
fn process_span_start(
    e: &BytesStart,
    state: &mut TtmlParserState,
    reader: &Reader<&[u8]>,
    warnings: &mut Vec<String>,
) -> Result<(), ConvertError> {
    // 进入新的 span 前，清空文本缓冲区
    state.text_buffer.clear();

    // 获取 span 的各个属性
    let role = get_attribute_with_aliases(e, reader, &[ATTR_ROLE, ATTR_ROLE_ALIAS], |s| {
        Ok(match s.as_bytes() {
            ROLE_TRANSLATION => SpanRole::Translation,
            ROLE_ROMANIZATION => SpanRole::Romanization,
            ROLE_BACKGROUND => SpanRole::Background,
            _ => SpanRole::Generic,
        })
    })?
    .unwrap_or(SpanRole::Generic);

    let lang = get_string_attribute(e, reader, &[ATTR_XML_LANG])?;
    let scheme = get_string_attribute(e, reader, &[ATTR_XML_SCHEME])?;
    let start_ms = get_time_attribute(e, reader, &[ATTR_BEGIN], warnings)?;
    let end_ms = get_time_attribute(e, reader, &[ATTR_END], warnings)?;

    // 将解析出的上下文压入堆栈，以支持嵌套 span
    state.body_state.span_stack.push(SpanContext {
        role,
        lang,
        scheme,
        start_ms,
        end_ms,
    });

    // 如果是背景人声容器的开始，则初始化背景数据累加器
    if role == SpanRole::Background
        && let Some(p_data) = state.body_state.current_p_element_data.as_mut()
        && !p_data
            .tracks_accumulator
            .iter()
            .any(|t| t.content_type == ContentType::Background)
    {
        p_data.tracks_accumulator.push(AnnotatedTrack {
            content_type: ContentType::Background,
            content: LyricTrack::default(),
            translations: vec![],
            romanizations: vec![],
        });
    }

    Ok(())
}

/// 处理文本事件。
/// 这个函数的核心逻辑是区分 "音节间的空格" 和 "音节内的文本"。
fn process_text_event(e_text: &BytesText, state: &mut TtmlParserState) -> Result<(), ConvertError> {
    let text_slice = e_text.xml_content().map_err(ConvertError::new_parse)?;

    if !state.body_state.in_p {
        return Ok(()); // 不在 <p> 标签内，忽略任何文本
    }

    // 如果上一个事件是一个结束的音节 (</span>)，并且当前文本是纯空格，
    // 那么这个空格应该附加到上一个音节上。
    let LastSyllableInfo::EndedSyllable { was_background } = state.body_state.last_syllable_info
    else {
        handle_general_text(&text_slice, state);
        return Ok(());
    };

    if text_slice.is_empty() || !text_slice.chars().all(char::is_whitespace) {
        handle_general_text(&text_slice, state);
        return Ok(());
    }

    let has_space = state.format_detection == super::state::FormatDetection::NotFormatted
        || (!text_slice.contains('\n') && !text_slice.contains('\r'));

    if has_space && let Some(p_data) = state.body_state.current_p_element_data.as_mut() {
        let target_content_type = if was_background {
            ContentType::Background
        } else {
            ContentType::Main
        };

        if let Some(last_syl) = p_data
            .tracks_accumulator
            .iter_mut()
            .find(|t| t.content_type == target_content_type)
            .and_then(|at| at.content.words.last_mut())
            .and_then(|w| w.syllables.last_mut())
            && !last_syl.ends_with_space
        {
            last_syl.ends_with_space = true;
        }
    }
    // 消费掉这个空格，并重置状态，然后直接返回
    state.body_state.last_syllable_info = LastSyllableInfo::None;
    Ok(())
}

/// 处理普通文本的逻辑
fn handle_general_text(text_slice: &str, state: &mut TtmlParserState) {
    state.body_state.last_syllable_info = LastSyllableInfo::None;

    let trimmed_text = text_slice.trim();
    if trimmed_text.is_empty() && state.body_state.span_stack.is_empty() {
        // 如果trim后为空（意味着它不是音节间空格，只是普通的空白节点），则忽略
        return;
    }

    // 累加到缓冲区
    if !state.body_state.span_stack.is_empty() {
        // 如果在 span 内，文本属于这个 span
        state.text_buffer.push_str(text_slice);
    } else if let Some(p_data) = state.body_state.current_p_element_data.as_mut() {
        // 如果在 p 内但在任何 span 外，文本直接属于 p
        p_data.line_text_accumulator.push_str(text_slice);
    }
}

/// 处理 `</span>` 结束事件的分发器。
fn process_span_end(
    state: &mut TtmlParserState,
    warnings: &mut Vec<String>,
) -> Result<(), ConvertError> {
    // 从堆栈中弹出刚刚结束的 span 的上下文
    if let Some(ended_span_ctx) = state.body_state.span_stack.pop() {
        // 获取并清空缓冲区中的文本
        let raw_text_from_buffer = std::mem::take(&mut state.text_buffer);

        // 根据 span 的角色分发给不同的处理器
        match ended_span_ctx.role {
            SpanRole::Generic => {
                handle_generic_span_end(state, &ended_span_ctx, &raw_text_from_buffer, warnings)?;
            }
            SpanRole::Translation | SpanRole::Romanization => {
                handle_auxiliary_span_end(state, &ended_span_ctx, &raw_text_from_buffer)?;
            }
            SpanRole::Background => {
                handle_background_span_end(
                    state,
                    &ended_span_ctx,
                    &raw_text_from_buffer,
                    warnings,
                )?;
            }
        }
    }
    Ok(())
}

/// 处理普通音节 `<span>` 结束的逻辑。
fn handle_generic_span_end(
    state: &mut TtmlParserState,
    ctx: &SpanContext,
    text: &str,
    warnings: &mut Vec<String>,
) -> Result<(), ConvertError> {
    if let (Some(start_ms), Some(end_ms)) = (ctx.start_ms, ctx.end_ms) {
        if text.is_empty() {
            return Ok(());
        }

        let trimmed_text = text.trim();

        let p_data = state
            .body_state
            .current_p_element_data
            .as_mut()
            .ok_or_else(|| {
                ConvertError::Internal("在处理 span 时丢失了 p_data 上下文".to_string())
            })?;

        let was_within_bg = state
            .body_state
            .span_stack
            .iter()
            .any(|s| s.role == SpanRole::Background);

        let target_content_type = if was_within_bg {
            ContentType::Background
        } else {
            ContentType::Main
        };

        // 如果 span 只包含空白字符，则将其视为空格
        if trimmed_text.is_empty() {
            // 这是一个空格 span，标记前一个音节
            if let Some(last_syl) = p_data
                .tracks_accumulator
                .iter_mut()
                .find(|t| t.content_type == target_content_type)
                .and_then(|at| at.content.words.last_mut())
                .and_then(|w| w.syllables.last_mut())
            {
                last_syl.ends_with_space = true;
            }
            // 空格不应该产生音节，所以直接返回
            return Ok(());
        }

        if start_ms > end_ms {
            warnings.push(format!(
                "音节 '{}' 的时间戳无效 (start_ms {} > end_ms {}), 但仍会创建音节。",
                text.escape_debug(),
                start_ms,
                end_ms
            ));
        }

        let target_annotated_track =
            get_or_create_target_annotated_track(p_data, target_content_type);
        let target_content_track = &mut target_annotated_track.content;

        if target_content_track.words.is_empty() {
            target_content_track.words.push(Word::default());
        }
        let target_word = target_content_track.words.first_mut().unwrap();

        process_syllable(
            start_ms,
            end_ms.max(start_ms),
            text,
            was_within_bg,
            &mut state.text_processing_buffer,
            &mut target_word.syllables,
        );

        if !target_word.syllables.is_empty() {
            state.body_state.last_syllable_info = LastSyllableInfo::EndedSyllable {
                was_background: was_within_bg,
            };
        }
    } else if !text.trim().is_empty() {
        if state.is_line_timing_mode {
            if let Some(p_data) = state.body_state.current_p_element_data.as_mut() {
                if !p_data.line_text_accumulator.is_empty()
                    && !p_data.line_text_accumulator.ends_with(char::is_whitespace)
                {
                    p_data.line_text_accumulator.push(' ');
                }
                p_data.line_text_accumulator.push_str(text.trim());
            }
        } else {
            warnings.push(format!(
                "逐字模式下，span缺少时间信息，文本 '{}' 被忽略。",
                text.trim().escape_debug()
            ));
        }
    }
    Ok(())
}

pub(super) fn process_syllable(
    start_ms: u64,
    end_ms: u64,
    raw_text: &str,
    is_background: bool,
    text_processing_buffer: &mut String,
    syllable_accumulator: &mut Vec<LyricSyllable>,
) {
    // 处理前导空格
    if raw_text.starts_with(char::is_whitespace)
        && let Some(prev_syllable) = syllable_accumulator.last_mut()
        && !prev_syllable.ends_with_space
    {
        prev_syllable.ends_with_space = true;
    }

    let trimmed_text = raw_text.trim();
    if trimmed_text.is_empty() {
        return;
    }

    // 根据是否为背景人声，对文本进行清理
    text_processing_buffer.clear();
    if is_background {
        clean_parentheses_from_bg_text_into(trimmed_text, text_processing_buffer);
    } else {
        normalize_text_whitespace_into(trimmed_text, text_processing_buffer);
    }

    if text_processing_buffer.is_empty() {
        return;
    }

    // 创建新的音节
    let new_syllable = LyricSyllable {
        text: std::mem::take(text_processing_buffer),
        start_ms,
        end_ms,
        duration_ms: Some(end_ms.saturating_sub(start_ms)),
        ends_with_space: raw_text.ends_with(char::is_whitespace),
    };

    syllable_accumulator.push(new_syllable);
}

/// 处理翻译和罗马音 `<span>` 结束的逻辑。
fn handle_auxiliary_span_end(
    state: &mut TtmlParserState,
    ctx: &SpanContext,
    text: &str,
) -> Result<(), ConvertError> {
    normalize_text_whitespace_into(text, &mut state.text_processing_buffer);
    if state.text_processing_buffer.is_empty() {
        return Ok(());
    }

    let p_data = state
        .body_state
        .current_p_element_data
        .as_mut()
        .ok_or_else(|| {
            ConvertError::Internal("在处理辅助 span 时丢失了 p_data 上下文".to_string())
        })?;

    let was_within_bg = state
        .body_state
        .span_stack
        .iter()
        .any(|s| s.role == SpanRole::Background);

    let target_content_type = if was_within_bg {
        ContentType::Background
    } else {
        ContentType::Main
    };

    let target_annotated_track = get_or_create_target_annotated_track(p_data, target_content_type);

    let syllable = LyricSyllable {
        text: std::mem::take(&mut state.text_processing_buffer),
        ..Default::default()
    };
    let word = Word {
        syllables: vec![syllable],
        ..Default::default()
    };
    let mut metadata = HashMap::new();

    let mut aux_track = LyricTrack {
        words: vec![word],
        metadata: HashMap::default(),
    };

    match ctx.role {
        SpanRole::Translation => {
            if let Some(lang) = ctx
                .lang
                .clone()
                .or_else(|| state.default_translation_lang.clone())
            {
                metadata.insert(TrackMetadataKey::Language, lang);
            }
            aux_track.metadata = metadata;
            target_annotated_track.translations.push(aux_track);
        }
        SpanRole::Romanization => {
            if let Some(lang) = ctx
                .lang
                .clone()
                .or_else(|| state.default_romanization_lang.clone())
            {
                metadata.insert(TrackMetadataKey::Language, lang);
            }
            if let Some(scheme) = ctx.scheme.clone() {
                metadata.insert(TrackMetadataKey::Scheme, scheme);
            }
            aux_track.metadata = metadata;
            target_annotated_track.romanizations.push(aux_track);
        }
        _ => {} // 不应该发生
    }

    Ok(())
}

/// 处理背景人声容器 `<span>` 结束的逻辑。
fn handle_background_span_end(
    state: &mut TtmlParserState,
    ctx: &SpanContext,
    text: &str, // 背景容器直接包含的文本
    warnings: &mut Vec<String>,
) -> Result<(), ConvertError> {
    let p_data = state
        .body_state
        .current_p_element_data
        .as_mut()
        .ok_or_else(|| {
            ConvertError::Internal("在处理背景 span 时丢失了 p_data 上下文".to_string())
        })?;

    // 处理不规范的情况：背景容器直接包含文本，而不是通过嵌套的 span。
    let trimmed_text = text.trim();
    if !trimmed_text.is_empty() {
        if let (Some(start_ms), Some(end_ms)) = (ctx.start_ms, ctx.end_ms) {
            if let Some(bg_annotated_track) = p_data
                .tracks_accumulator
                .iter_mut()
                .find(|t| t.content_type == ContentType::Background)
            {
                let bg_content_track = &mut bg_annotated_track.content;
                // 只有在背景容器内部没有其他音节时，才将此直接文本视为一个音节
                if bg_content_track.words.is_empty()
                    || bg_content_track
                        .words
                        .iter()
                        .all(|w| w.syllables.is_empty())
                {
                    clean_parentheses_from_bg_text_into(
                        trimmed_text,
                        &mut state.text_processing_buffer,
                    );

                    let syllable = LyricSyllable {
                        text: std::mem::take(&mut state.text_processing_buffer),
                        start_ms,
                        end_ms: end_ms.max(start_ms),
                        duration_ms: Some(end_ms.saturating_sub(start_ms)),
                        ends_with_space: !text.is_empty() && text.ends_with(char::is_whitespace),
                    };

                    if bg_content_track.words.is_empty() {
                        bg_content_track.words.push(Word::default());
                    }
                    bg_content_track
                        .words
                        .first_mut()
                        .unwrap()
                        .syllables
                        .push(syllable);

                    state.body_state.last_syllable_info = LastSyllableInfo::EndedSyllable {
                        was_background: true,
                    };
                } else {
                    warnings.push(format!("<span ttm:role='x-bg'> 直接包含文本 '{}'，但其内部已有音节，此直接文本被忽略。", trimmed_text.escape_debug()));
                }
            }
        } else {
            warnings.push(format!(
                "<span ttm:role='x-bg'> 直接包含文本 '{}'，但缺少时间信息，忽略。",
                trimmed_text.escape_debug()
            ));
        }
    }
    Ok(())
}

// =================================================================================
// 6. 数据终结逻辑
// =================================================================================

/// 在 `</p>` 结束时，终结并处理一个 `LyricLine`。
/// 这个函数负责将 `CurrentPElementData` 中的所有累积数据，
/// 组合成一个完整的 `LyricLine` 对象，并添加到最终结果中。
fn finalize_p_element(
    mut p_data: CurrentPElementData,
    lines: &mut Vec<LyricLine>,
    state: &mut TtmlParserState,
    _warnings: &mut [String],
) {
    // 步骤 1: 如果是逐行模式且没有音节，则根据累积的文本创建主轨道
    create_main_track_from_accumulator_if_needed(&mut p_data, state);

    // 步骤 2: 合并来自 <metadata> 的带时间戳的辅助轨道
    merge_metadata_tracks_into_p_data(&mut p_data, &state.metadata_state);

    let mut new_line = LyricLine {
        start_ms: p_data.start_ms,
        end_ms: p_data.end_ms,
        agent: p_data.agent,
        song_part: p_data.song_part,
        tracks: p_data.tracks_accumulator,
        itunes_key: p_data.itunes_key.clone(),
    };

    // 步骤 3: 根据所有音节的实际结束时间，重新计算行的最终结束时间
    let max_track_end_ms = recalculate_line_end_ms(&new_line);
    new_line.end_ms = new_line.end_ms.max(max_track_end_ms);

    // 步骤 4: 确保行不为空，然后添加到结果列表中
    let is_empty = new_line.tracks.iter().all(|at| {
        at.content.words.iter().all(|w| w.syllables.is_empty())
            && at.translations.is_empty()
            && at.romanizations.is_empty()
    });

    if !is_empty {
        lines.push(new_line);
    }
}

/// 如果主轨道没有音节且累积器中有文本（通常在逐行模式下），则创建主轨道内容。
fn create_main_track_from_accumulator_if_needed(
    p_data: &mut CurrentPElementData,
    state: &mut TtmlParserState,
) {
    let main_track_has_syllables = p_data
        .tracks_accumulator
        .iter()
        .find(|at| at.content_type == ContentType::Main)
        .is_some_and(|at| !at.content.words.iter().all(|w| w.syllables.is_empty()));

    if !main_track_has_syllables && !p_data.line_text_accumulator.trim().is_empty() {
        // 如果尚不存在 Main 类型的轨道，则创建一个
        if !p_data
            .tracks_accumulator
            .iter()
            .any(|at| at.content_type == ContentType::Main)
        {
            p_data.tracks_accumulator.insert(
                0,
                AnnotatedTrack {
                    content_type: ContentType::Main,
                    ..Default::default()
                },
            );
        }

        let main_annotated_track = p_data
            .tracks_accumulator
            .iter_mut()
            .find(|at| at.content_type == ContentType::Main)
            .unwrap(); // 已经在上面保证了它一定存在

        normalize_text_whitespace_into(
            &p_data.line_text_accumulator,
            &mut state.text_processing_buffer,
        );
        if !state.text_processing_buffer.is_empty() {
            let syllable = LyricSyllable {
                text: std::mem::take(&mut state.text_processing_buffer),
                start_ms: p_data.start_ms,
                end_ms: p_data.end_ms,
                ..Default::default()
            };
            main_annotated_track.content.words = vec![Word {
                syllables: vec![syllable],
                ..Default::default()
            }];
        }
    }
}

/// 将从 <metadata> 中解析出的带时间戳的辅助轨道合并到当前行的 `p_data` 中。
fn merge_metadata_tracks_into_p_data(
    p_data: &mut CurrentPElementData,
    metadata_state: &MetadataParseState,
) {
    if let Some(key) = &p_data.itunes_key
        && let Some(detailed_tracks) = metadata_state.timed_track_map.get(key)
    {
        if let Some(main_annotated_track) = p_data
            .tracks_accumulator
            .iter_mut()
            .find(|at| at.content_type == ContentType::Main)
        {
            main_annotated_track
                .translations
                .extend(detailed_tracks.main_tracks.translations.clone());
            main_annotated_track
                .romanizations
                .extend(detailed_tracks.main_tracks.romanizations.clone());
        }
        if let Some(bg_annotated_track) = p_data
            .tracks_accumulator
            .iter_mut()
            .find(|at| at.content_type == ContentType::Background)
        {
            bg_annotated_track
                .translations
                .extend(detailed_tracks.background_tracks.translations.clone());
            bg_annotated_track
                .romanizations
                .extend(detailed_tracks.background_tracks.romanizations.clone());
        }
    }
}

/// 遍历一行中的所有轨道和音节，计算最晚的结束时间戳。
fn recalculate_line_end_ms(line: &LyricLine) -> u64 {
    line.tracks
        .iter()
        .flat_map(|at| {
            let content_words = at.content.words.iter();
            let translation_words = at.translations.iter().flat_map(|t| t.words.iter());
            let romanization_words = at.romanizations.iter().flat_map(|r| r.words.iter());

            content_words
                .chain(translation_words)
                .chain(romanization_words)
        })
        .flat_map(|word| &word.syllables)
        .map(|syllable| syllable.end_ms)
        .max()
        .unwrap_or(0)
}

pub(super) fn get_or_create_target_annotated_track(
    p_data: &mut CurrentPElementData,
    content_type: ContentType,
) -> &mut AnnotatedTrack {
    if let Some(index) = p_data
        .tracks_accumulator
        .iter()
        .position(|t| t.content_type == content_type)
    {
        &mut p_data.tracks_accumulator[index]
    } else {
        p_data.tracks_accumulator.push(AnnotatedTrack {
            content_type,
            ..Default::default()
        });
        p_data.tracks_accumulator.last_mut().unwrap() // 刚插入，所以 unwrap 是安全的
    }
}

pub(super) fn create_simple_translation_track(text: &str, lang: Option<&String>) -> LyricTrack {
    let syllable = LyricSyllable {
        text: text.to_string(),
        ..Default::default()
    };
    let word = Word {
        syllables: vec![syllable],
        ..Default::default()
    };
    let mut metadata = HashMap::new();
    if let Some(lang_code) = lang {
        metadata.insert(TrackMetadataKey::Language, lang_code.clone());
    }
    LyricTrack {
        words: vec![word],
        metadata,
    }
}
