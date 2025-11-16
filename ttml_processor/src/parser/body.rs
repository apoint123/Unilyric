//! # TTML 解析器 - Body 处理模块
//!
//! 该模块包含了所有用于解析 TTML 文件中 `<body>` 块的函数，
//! 包括处理 `<p>`, `<span>` 标签和文本内容。

use std::{collections::HashMap, str};

use super::{
    state::{
        CurrentPElementData, MetadataParseState, PendingItem, SpanContext, SpanRole,
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
            let (decoded_char, warning) = match e.resolve_char_ref() {
                Ok(Some(ch)) => (ch, None),
                Ok(None) => match &**e {
                    b"lt" => ('<', None),
                    b"gt" => ('>', None),
                    b"amp" => ('&', None),
                    b"apos" => ('\'', None),
                    b"quot" => ('"', None),
                    _ => {
                        let warn_msg =
                            format!("忽略了未知的XML实体 '&{};'", String::from_utf8_lossy(e));
                        ('\0', Some(warn_msg))
                    }
                },
                Err(err) => {
                    let warn_msg = format!("无效的XML数字实体: {err}");
                    ('\0', Some(warn_msg))
                }
            };

            if let Some(warn_msg) = warning {
                warnings.push(warn_msg);
            }

            if decoded_char != '\0'
                && let Some(p_data) = state.body_state.current_p_element_data.as_mut()
            {
                if state.body_state.span_stack.is_empty() {
                    p_data
                        .pending_items
                        .push(PendingItem::FreeText(decoded_char.to_string()));
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
    warnings: &mut Vec<String>,
) {
    if let Some(p_data) = state.body_state.current_p_element_data.take() {
        let start_ms = p_data.start_ms;
        let end_ms = p_data.end_ms;
        let agent = p_data.agent.clone();
        let song_part = p_data.song_part.clone();
        let itunes_key = p_data.itunes_key.clone();

        let mut tracks = finalize_p_element(p_data, state, warnings);

        if let Some(key) = &itunes_key
            && let Some(translations_for_line) = state.metadata_state.line_translation_map.get(key)
        {
            for (line_translation, lang) in translations_for_line {
                // 处理主音轨翻译
                if let Some(main_text) = &line_translation.main {
                    let main_annotated_track =
                        get_or_create_track_in_vec(&mut tracks, ContentType::Main);

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
                    let bg_annotated_track =
                        get_or_create_track_in_vec(&mut tracks, ContentType::Background);

                    let translation_exists = bg_annotated_track.translations.iter().any(|track| {
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

        merge_metadata_tracks_into_tracks(&mut tracks, itunes_key.as_ref(), &state.metadata_state);

        let mut new_line = LyricLine {
            start_ms,
            end_ms,
            agent,
            song_part,
            tracks,
            itunes_key,
        };

        let max_track_end_ms = recalculate_line_end_ms(&new_line);
        new_line.end_ms = new_line.end_ms.max(max_track_end_ms);

        let is_empty = new_line.tracks.iter().all(|at| {
            at.content.words.iter().all(|w| w.syllables.is_empty())
                && at.translations.is_empty()
                && at.romanizations.is_empty()
        });

        if !is_empty {
            lines.push(new_line);
        }
    }

    state.body_state.in_p = false;
    state.body_state.span_stack.clear();
}

fn process_span_start(
    e: &BytesStart,
    state: &mut TtmlParserState,
    reader: &Reader<&[u8]>,
    warnings: &mut Vec<String>,
) -> Result<(), ConvertError> {
    state.text_buffer.clear();
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

    state.body_state.span_stack.push(SpanContext {
        role,
        lang,
        scheme,
        start_ms,
        end_ms,
    });

    Ok(())
}

fn process_text_event(e_text: &BytesText, state: &mut TtmlParserState) -> Result<(), ConvertError> {
    let text_slice = e_text.xml_content().map_err(ConvertError::new_parse)?;

    if !state.body_state.in_p {
        return Ok(());
    }

    if !state.body_state.span_stack.is_empty() {
        state.text_buffer.push_str(&text_slice);
    } else if let Some(p_data) = state.body_state.current_p_element_data.as_mut() {
        p_data
            .pending_items
            .push(PendingItem::FreeText(text_slice.to_string()));
    }

    Ok(())
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

        if start_ms > end_ms {
            warnings.push(format!(
                "音节 '{}' 的时间戳无效 (start_ms {} > end_ms {}), 但仍会创建音节。",
                text.escape_debug(),
                start_ms,
                end_ms
            ));
        }

        let trimmed_text = text.trim();
        if trimmed_text.is_empty() && !text.is_empty() {
            p_data
                .pending_items
                .push(PendingItem::FreeText(text.to_string()));
        } else if !trimmed_text.is_empty() {
            p_data.pending_items.push(PendingItem::Syllable {
                text: text.to_string(),
                start_ms,
                end_ms: end_ms.max(start_ms),
                content_type: target_content_type,
            });
        }
    } else if !text.trim().is_empty() {
        if state.is_line_timing_mode {
            if let Some(p_data) = state.body_state.current_p_element_data.as_mut() {
                p_data
                    .pending_items
                    .push(PendingItem::FreeText(text.to_string()));
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
    let normalized_text = std::mem::take(&mut state.text_processing_buffer);

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

    let target_annotated_track =
        get_or_create_track_in_vec(&mut p_data.tracks_accumulator, target_content_type);

    let syllable = LyricSyllable {
        text: normalized_text,
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
        _ => {}
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
            p_data.pending_items.push(PendingItem::Syllable {
                text: text.to_string(),
                start_ms,
                end_ms: end_ms.max(start_ms),
                content_type: ContentType::Background,
            });
        } else {
            warnings.push(format!(
                "<span ttm:role='x-bg'> 直接包含文本 '{}'，但缺少时间信息，忽略。",
                trimmed_text.escape_debug()
            ));
        }
    }
    Ok(())
}

fn finalize_p_element(
    mut p_data: CurrentPElementData,
    state: &mut TtmlParserState,
    warnings: &mut Vec<String>,
) -> Vec<AnnotatedTrack> {
    if state.is_line_timing_mode {
        let mut line_text = String::new();
        for item in &p_data.pending_items {
            match item {
                PendingItem::Syllable { text, .. } | PendingItem::FreeText(text) => {
                    line_text.push_str(text);
                }
            }
        }

        normalize_text_whitespace_into(&line_text, &mut state.text_processing_buffer);
        if !state.text_processing_buffer.is_empty() {
            let syllable = LyricSyllable {
                text: std::mem::take(&mut state.text_processing_buffer),
                start_ms: p_data.start_ms,
                end_ms: p_data.end_ms,
                ..Default::default()
            };
            let main_track =
                get_or_create_track_in_vec(&mut p_data.tracks_accumulator, ContentType::Main);
            main_track.content.words = vec![Word {
                syllables: vec![syllable],
                ..Default::default()
            }];
        }
    }

    let mut iter = p_data.pending_items.iter().peekable();
    while let Some(item) = iter.next() {
        match item {
            PendingItem::Syllable {
                text,
                start_ms,
                end_ms,
                content_type,
            } => {
                if state.is_line_timing_mode {
                    continue;
                }

                let mut external_space = false;
                while let Some(PendingItem::FreeText(next_text)) = iter.peek() {
                    if next_text.chars().all(char::is_whitespace) {
                        iter.next();

                        let has_space = next_text.chars().any(|c| c == ' ');
                        let has_newline = next_text.chars().any(|c| c == '\n' || c == '\r');

                        if has_space && !has_newline {
                            external_space = true;
                        }
                    } else {
                        break;
                    }
                }

                let target_track =
                    get_or_create_track_in_vec(&mut p_data.tracks_accumulator, *content_type);
                if target_track.content.words.is_empty() {
                    target_track.content.words.push(Word::default());
                }
                let target_word = target_track.content.words.first_mut().unwrap();

                process_syllable(
                    *start_ms,
                    *end_ms,
                    text,
                    *content_type == ContentType::Background,
                    &mut state.text_processing_buffer,
                    &mut target_word.syllables,
                );

                if let Some(syl) = target_word.syllables.last_mut() {
                    syl.ends_with_space = syl.ends_with_space || external_space;
                }
            }
            PendingItem::FreeText(text) => {
                if !state.is_line_timing_mode && !text.trim().is_empty() {
                    warnings.push(format!(
                        "逐字模式下, 在 <p> ({}ms) 中发现无时间戳的文本, 已忽略: '{}'",
                        p_data.start_ms,
                        text.trim().escape_debug()
                    ));
                }
            }
        }
    }

    p_data.tracks_accumulator
}

fn merge_metadata_tracks_into_tracks(
    tracks: &mut Vec<AnnotatedTrack>,
    itunes_key: Option<&String>,
    metadata_state: &MetadataParseState,
) {
    if let Some(key) = itunes_key
        && let Some(detailed_tracks) = metadata_state.timed_track_map.get(key)
    {
        if !detailed_tracks.main_tracks.translations.is_empty()
            || !detailed_tracks.main_tracks.romanizations.is_empty()
        {
            let main_annotated_track = get_or_create_track_in_vec(tracks, ContentType::Main);
            main_annotated_track
                .translations
                .extend(detailed_tracks.main_tracks.translations.clone());
            main_annotated_track
                .romanizations
                .extend(detailed_tracks.main_tracks.romanizations.clone());
        }
        if !detailed_tracks.background_tracks.translations.is_empty()
            || !detailed_tracks.background_tracks.romanizations.is_empty()
        {
            let bg_annotated_track = get_or_create_track_in_vec(tracks, ContentType::Background);
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

pub(super) fn get_or_create_track_in_vec(
    tracks: &mut Vec<AnnotatedTrack>,
    content_type: ContentType,
) -> &mut AnnotatedTrack {
    if let Some(index) = tracks.iter().position(|t| t.content_type == content_type) {
        &mut tracks[index]
    } else {
        tracks.push(AnnotatedTrack {
            content_type,
            ..Default::default()
        });
        tracks.last_mut().unwrap()
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
