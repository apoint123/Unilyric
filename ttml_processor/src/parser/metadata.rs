//! # TTML 解析器 - Metadata 处理模块
//!
//! 该模块包含了所有用于解析 TTML 文件中 `<metadata>` 块的函数。

use std::collections::HashMap;

use crate::parser::state::PendingItem;

use super::{
    state::{AuxTrackType, MetadataContext, SpanContext, SpanRole, TtmlParserState},
    utils::{get_attribute_with_aliases, get_string_attribute, get_time_attribute},
};
use lyrics_helper_core::{
    Agent, AgentType, ContentType, ConvertError, LyricSyllable, LyricTrack, TrackMetadataKey, Word,
};
use quick_xml::{
    Reader,
    events::{BytesStart, BytesText, Event},
};

use super::constants::{
    ATTR_BEGIN, ATTR_END, ATTR_FOR, ATTR_KEY, ATTR_ROLE, ATTR_ROLE_ALIAS, ATTR_VALUE, ATTR_XML_ID,
    ATTR_XML_LANG, ROLE_BACKGROUND, TAG_AGENT, TAG_AGENT_TTM, TAG_ITUNES_METADATA, TAG_META,
    TAG_META_AMLL, TAG_METADATA, TAG_NAME, TAG_NAME_TTM, TAG_SONGWRITER, TAG_SPAN, TAG_TEXT,
    TAG_TRANSLATION, TAG_TRANSLATIONS, TAG_TRANSLITERATION, TAG_TRANSLITERATIONS,
};

/// 处理 `<metadata>` 块内部的事件。
pub(super) fn handle_metadata_event(
    event: &Event,
    reader: &mut Reader<&[u8]>,
    state: &mut TtmlParserState,
    raw_metadata: &mut HashMap<String, Vec<String>>,
    warnings: &mut Vec<String>,
) -> Result<(), ConvertError> {
    match event {
        Event::Start(e) => handle_metadata_start_tag(e, reader, state, raw_metadata, warnings),
        Event::Text(e) => handle_metadata_text(e, state, raw_metadata),
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

            if decoded_char != '\0' {
                let meta_state = &mut state.metadata_state;
                if !meta_state.span_stack.is_empty() {
                    meta_state.text_buffer.push(decoded_char);
                } else if matches!(meta_state.context, MetadataContext::InAuxiliaryText { .. }) {
                    let s = decoded_char.to_string();
                    meta_state.current_main_plain_text.push(decoded_char);
                    meta_state.pending_items.push(PendingItem::FreeText(s));
                } else if matches!(meta_state.context, MetadataContext::InSongwriter) {
                    raw_metadata
                        .entry("songwriters".to_string())
                        .or_default()
                        .push(decoded_char.to_string());
                }
            }
            Ok(())
        }
        Event::End(e) => {
            handle_metadata_end_tag(e, state);
            Ok(())
        }
        _ => Ok(()),
    }
}

/// 处理 `<metadata>` 块内部的开始标签事件。
fn handle_metadata_start_tag(
    e: &BytesStart,
    reader: &mut Reader<&[u8]>,
    state: &mut TtmlParserState,
    raw_metadata: &mut HashMap<String, Vec<String>>,
    warnings: &mut Vec<String>,
) -> Result<(), ConvertError> {
    let meta_state = &mut state.metadata_state;

    match e.name().as_ref() {
        TAG_AGENT | TAG_AGENT_TTM => process_agent_start_in_metadata(e, reader, state, warnings)?,
        TAG_NAME | TAG_NAME_TTM => {
            if let MetadataContext::InAgent { id: Some(agent_id) } = &meta_state.context {
                let name = reader
                    .read_text(e.name())
                    .map_err(ConvertError::new_parse)?
                    .into_owned();
                if !name.trim().is_empty()
                    && let Some(agent) = state.agent_store.agents_by_id.get_mut(agent_id)
                {
                    agent.name = Some(name.trim().to_string());
                }
            }
        }
        TAG_META | TAG_META_AMLL => process_meta_start_in_metadata(e, reader, raw_metadata)?,
        TAG_ITUNES_METADATA => meta_state.context = MetadataContext::InITunesMetadata,
        TAG_SONGWRITER => {
            if matches!(meta_state.context, MetadataContext::InITunesMetadata) {
                meta_state.context = MetadataContext::InSongwriter;
            }
        }
        TAG_TRANSLATIONS => {
            if matches!(meta_state.context, MetadataContext::InITunesMetadata) {
                meta_state.context = MetadataContext::InAuxiliaryContainer {
                    aux_type: AuxTrackType::Translation,
                };
            }
        }
        TAG_TRANSLITERATIONS => {
            if matches!(meta_state.context, MetadataContext::InITunesMetadata) {
                meta_state.context = MetadataContext::InAuxiliaryContainer {
                    aux_type: AuxTrackType::Romanization,
                };
            }
        }
        TAG_TRANSLATION | TAG_TRANSLITERATION => {
            if let MetadataContext::InAuxiliaryContainer { aux_type } = meta_state.context {
                let lang = get_string_attribute(e, reader, &[ATTR_XML_LANG])?;
                meta_state.context = MetadataContext::InAuxiliaryEntry { aux_type, lang };
            }
        }
        TAG_TEXT => process_text_start_in_metadata(e, reader, state)?,
        TAG_SPAN => process_span_start_in_metadata(e, reader, state, warnings)?,
        _ => {}
    }
    Ok(())
}

/// 处理 `metadata` 中的 `<agent>` 或 `<ttm:agent>` 开始标签
fn process_agent_start_in_metadata(
    e: &BytesStart,
    reader: &Reader<&[u8]>,
    state: &mut TtmlParserState,
    warnings: &mut Vec<String>,
) -> Result<(), ConvertError> {
    let id_opt = get_string_attribute(e, reader, &[ATTR_XML_ID])?;
    if let Some(id) = id_opt {
        let type_str = get_string_attribute(e, reader, &[b"type"])?.unwrap_or_default();
        let agent_type = match type_str.as_str() {
            "person" => AgentType::Person,
            "group" => AgentType::Group,
            _ => AgentType::Other,
        };

        let agent = Agent {
            id: id.clone(),
            name: None,
            agent_type,
        };
        state.agent_store.agents_by_id.insert(id.clone(), agent);
        state.metadata_state.context = MetadataContext::InAgent { id: Some(id) };
    } else {
        warnings.push("发现一个没有 xml:id 的 <ttm:agent> 标签，已忽略。".to_string());
    }
    Ok(())
}

/// 处理 `metadata` 中的 `<meta>` 或 `<amll:meta>` 开始标签
fn process_meta_start_in_metadata(
    e: &BytesStart,
    reader: &mut Reader<&[u8]>,
    raw_metadata: &mut HashMap<String, Vec<String>>,
) -> Result<(), ConvertError> {
    let key_attr = get_string_attribute(e, reader, &[ATTR_KEY])?;
    let value_attr = get_string_attribute(e, reader, &[ATTR_VALUE])?;
    let text_content = reader
        .read_text(e.name())
        .map_err(ConvertError::new_parse)?;

    if let Some(key) = key_attr {
        let value = value_attr.unwrap_or_else(|| text_content.into_owned());
        if !key.is_empty() {
            raw_metadata.entry(key).or_default().push(value);
        }
    }
    Ok(())
}

/// 处理 `metadata` 中的 `<text>` 开始标签
fn process_text_start_in_metadata(
    e: &BytesStart,
    reader: &Reader<&[u8]>,
    state: &mut TtmlParserState,
) -> Result<(), ConvertError> {
    let meta_state = &mut state.metadata_state;
    if let MetadataContext::InAuxiliaryEntry { aux_type, lang } = &meta_state.context {
        let key = get_string_attribute(e, reader, &[ATTR_FOR])?;
        meta_state.context = MetadataContext::InAuxiliaryText {
            aux_type: *aux_type,
            lang: lang.clone(),
            key,
        };
        meta_state.current_main_plain_text.clear();
        meta_state.current_bg_plain_text.clear();
        meta_state.pending_items.clear();
        meta_state.span_stack.clear();
        meta_state.text_buffer.clear();
    }
    Ok(())
}

/// 处理 `metadata` 中的 `<span>` 开始标签
fn process_span_start_in_metadata(
    e: &BytesStart,
    reader: &Reader<&[u8]>,
    state: &mut TtmlParserState,
    warnings: &mut Vec<String>,
) -> Result<(), ConvertError> {
    let meta_state = &mut state.metadata_state;
    if matches!(meta_state.context, MetadataContext::InAuxiliaryText { .. }) {
        if !meta_state.span_stack.is_empty() && !meta_state.text_buffer.is_empty() {
            let existing_text = std::mem::take(&mut meta_state.text_buffer);
            meta_state
                .pending_items
                .push(PendingItem::FreeText(existing_text));
        }
        meta_state.text_buffer.clear();

        let role = get_attribute_with_aliases(e, reader, &[ATTR_ROLE, ATTR_ROLE_ALIAS], |s| {
            Ok(match s.as_bytes() {
                ROLE_BACKGROUND => SpanRole::Background,
                _ => SpanRole::Generic,
            })
        })?
        .unwrap_or(SpanRole::Generic);

        let start_ms = get_time_attribute(e, reader, &[ATTR_BEGIN], warnings)?;
        let end_ms = get_time_attribute(e, reader, &[ATTR_END], warnings)?;

        meta_state.span_stack.push(SpanContext {
            role,
            start_ms,
            end_ms,
            lang: None,
            scheme: None,
        });
    }
    Ok(())
}

/// 处理 `<metadata>` 块内部的文本事件。
fn handle_metadata_text(
    e: &BytesText,
    state: &mut TtmlParserState,
    raw_metadata: &mut HashMap<String, Vec<String>>,
) -> Result<(), ConvertError> {
    let meta_state = &mut state.metadata_state;
    let text_slice = e.xml_content().map_err(ConvertError::new_parse)?;

    if !meta_state.span_stack.is_empty() {
        meta_state.text_buffer.push_str(&text_slice);
    } else if matches!(meta_state.context, MetadataContext::InAuxiliaryText { .. }) {
        meta_state.current_main_plain_text.push_str(&text_slice);
        meta_state
            .pending_items
            .push(PendingItem::FreeText(text_slice.into_owned()));
    } else if matches!(meta_state.context, MetadataContext::InSongwriter) {
        raw_metadata
            .entry("songwriters".to_string())
            .or_default()
            .push(text_slice.into_owned());
    }
    Ok(())
}

/// 处理 `<metadata>` 块内部的结束标签事件。
fn handle_metadata_end_tag(e: &quick_xml::events::BytesEnd, state: &mut TtmlParserState) {
    let meta_state = &mut state.metadata_state;

    match e.name().as_ref() {
        TAG_METADATA => state.in_metadata = false,
        TAG_ITUNES_METADATA => meta_state.context = MetadataContext::None,
        TAG_SONGWRITER => meta_state.context = MetadataContext::InITunesMetadata,
        TAG_AGENT | TAG_AGENT_TTM => {
            meta_state.context = MetadataContext::None;
        }
        TAG_TRANSLATIONS | TAG_TRANSLITERATIONS => {
            meta_state.context = MetadataContext::InITunesMetadata;
        }
        TAG_TRANSLATION | TAG_TRANSLITERATION => {
            if let MetadataContext::InAuxiliaryEntry { aux_type, .. } = &meta_state.context {
                meta_state.context = MetadataContext::InAuxiliaryContainer {
                    aux_type: *aux_type,
                };
            }
        }
        TAG_SPAN => process_span_end_in_metadata(state),
        TAG_TEXT => process_text_end_in_metadata(state),
        _ => {}
    }
}

fn process_span_end_in_metadata(state: &mut TtmlParserState) {
    let meta_state = &mut state.metadata_state;
    if matches!(meta_state.context, MetadataContext::InAuxiliaryText { .. })
        && let Some(ended_span_ctx) = meta_state.span_stack.pop()
    {
        let raw_text = std::mem::take(&mut meta_state.text_buffer);

        if let (Some(start_ms), Some(end_ms)) = (ended_span_ctx.start_ms, ended_span_ctx.end_ms) {
            let is_within_background_container = meta_state
                .span_stack
                .iter()
                .any(|s| s.role == SpanRole::Background);

            let is_background_syllable =
                ended_span_ctx.role == SpanRole::Background || is_within_background_container;

            let target_content_type = if is_background_syllable {
                ContentType::Background
            } else {
                ContentType::Main
            };

            let trimmed_text = raw_text.trim();
            if trimmed_text.is_empty() && !raw_text.is_empty() {
                meta_state
                    .pending_items
                    .push(PendingItem::FreeText(raw_text));
            } else if !trimmed_text.is_empty() {
                meta_state.pending_items.push(PendingItem::Syllable {
                    text: raw_text,
                    start_ms,
                    end_ms: end_ms.max(start_ms),
                    content_type: target_content_type,
                });
            }
        } else if !raw_text.trim().is_empty() {
            let is_within_background_container = meta_state
                .span_stack
                .iter()
                .any(|s| s.role == SpanRole::Background);

            let is_background_span =
                ended_span_ctx.role == SpanRole::Background || is_within_background_container;

            if is_background_span {
                meta_state.current_bg_plain_text.push_str(&raw_text);
            } else {
                meta_state.current_main_plain_text.push_str(&raw_text);
            }
            meta_state
                .pending_items
                .push(PendingItem::FreeText(raw_text));
        }
    }
}

#[allow(clippy::too_many_lines)]
fn process_text_end_in_metadata(state: &mut TtmlParserState) {
    let meta_state = &mut state.metadata_state;
    if let MetadataContext::InAuxiliaryText {
        aux_type,
        lang,
        key: Some(text_key),
    } = &meta_state.context
    {
        let main_plain_text = meta_state.current_main_plain_text.trim();
        let bg_plain_text = meta_state.current_bg_plain_text.trim();
        let has_plain_text = !main_plain_text.is_empty() || !bg_plain_text.is_empty();

        let has_syllables = meta_state
            .pending_items
            .iter()
            .any(|item| matches!(item, PendingItem::Syllable { .. }));

        if !has_syllables && has_plain_text && matches!(aux_type, AuxTrackType::Translation) {
            let line_translation = super::state::LineTranslation {
                main: if main_plain_text.is_empty() {
                    None
                } else {
                    Some(main_plain_text.to_string())
                },
                background: if bg_plain_text.is_empty() {
                    None
                } else {
                    Some(bg_plain_text.to_string())
                },
            };

            meta_state
                .line_translation_map
                .entry(text_key.clone())
                .or_default()
                .push((line_translation, lang.clone()));
        } else if has_syllables {
            let mut main_syllables: Vec<LyricSyllable> = Vec::new();
            let mut bg_syllables: Vec<LyricSyllable> = Vec::new();

            let mut iter = meta_state.pending_items.iter().peekable();
            while let Some(item) = iter.next() {
                if let PendingItem::Syllable {
                    text,
                    start_ms,
                    end_ms,
                    content_type,
                } = item
                {
                    let mut external_space = false;
                    while let Some(PendingItem::FreeText(next_text)) = iter.peek() {
                        if next_text.chars().all(char::is_whitespace) {
                            iter.next();

                            let has_space = next_text.chars().any(char::is_whitespace);
                            let has_newline = next_text.chars().any(|c| c == '\n' || c == '\r');

                            if has_space && !has_newline {
                                external_space = true;
                            }
                        } else {
                            break;
                        }
                    }

                    let target_syllables = match content_type {
                        ContentType::Main => &mut main_syllables,
                        ContentType::Background => &mut bg_syllables,
                    };

                    super::body::process_syllable(
                        *start_ms,
                        *end_ms,
                        text,
                        *content_type == ContentType::Background,
                        &mut state.text_processing_buffer,
                        target_syllables,
                    );

                    if let Some(syl) = target_syllables.last_mut() {
                        syl.ends_with_space = syl.ends_with_space || external_space;
                    }
                }
            }

            let entry = meta_state
                .timed_track_map
                .entry(text_key.clone())
                .or_default();
            let mut metadata = HashMap::new();
            if let Some(language) = lang {
                metadata.insert(TrackMetadataKey::Language, language.clone());
            }

            if !main_syllables.is_empty() {
                let track = LyricTrack {
                    words: vec![Word {
                        syllables: main_syllables,
                        ..Default::default()
                    }],
                    metadata: metadata.clone(),
                };
                let target_set = &mut entry.main_tracks;
                match aux_type {
                    AuxTrackType::Translation => target_set.translations.push(track),
                    AuxTrackType::Romanization => target_set.romanizations.push(track),
                }
            }
            if !bg_syllables.is_empty() {
                let track = LyricTrack {
                    words: vec![Word {
                        syllables: bg_syllables,
                        ..Default::default()
                    }],
                    metadata: metadata.clone(),
                };
                let target_set = &mut entry.background_tracks;
                match aux_type {
                    AuxTrackType::Translation => target_set.translations.push(track),
                    AuxTrackType::Romanization => target_set.romanizations.push(track),
                }
            }
        }

        meta_state.current_main_plain_text.clear();
        meta_state.current_bg_plain_text.clear();
        meta_state.pending_items.clear();
    }
    if let MetadataContext::InAuxiliaryText { aux_type, lang, .. } = &meta_state.context {
        meta_state.context = MetadataContext::InAuxiliaryEntry {
            aux_type: *aux_type,
            lang: lang.clone(),
        };
    }
}
