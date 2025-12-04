//! # TTML 解析器 - 事件处理器与分发器
//!
//! 该模块负责顶层的事件分发、全局状态管理和错误恢复。

use std::collections::HashMap;

use super::{
    body,
    constants::{
        ATTR_AGENT, ATTR_AGENT_ALIAS, ATTR_BEGIN, ATTR_END, ATTR_ITUNES_KEY, ATTR_ITUNES_SONG_PART,
        ATTR_ITUNES_SONG_PART_NEW, ATTR_ITUNES_TIMING, ATTR_XML_LANG, TAG_BODY, TAG_DIV,
        TAG_METADATA, TAG_P, TAG_TT,
    },
    state::{BodyParseState, CurrentPElementData, MetadataParseState, TtmlParserState},
    utils::{get_string_attribute, get_time_attribute},
};
use lyrics_helper_core::{ConvertError, LyricLine, TtmlParsingOptions, TtmlTimingMode};
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};

/// 处理全局事件（在 `<p>` 或 `<metadata>` 之外的事件）。
/// 主要负责识别文档的根元素、body、div 和 p 的开始，并相应地更新状态。
pub(super) fn handle_global_event(
    event: &Event<'_>,
    state: &mut TtmlParserState,
    reader: &Reader<&[u8]>,
    raw_metadata: &mut HashMap<String, Vec<String>>,
    warnings: &mut Vec<String>,
    has_timed_span_tags: bool,
    options: &TtmlParsingOptions,
) -> Result<(), ConvertError> {
    match event {
        Event::Start(e) => match e.local_name().as_ref() {
            TAG_TT => process_tt_start(
                e,
                state,
                raw_metadata,
                reader,
                has_timed_span_tags,
                warnings,
                options,
            )?,
            TAG_METADATA => state.in_metadata = true,

            TAG_BODY => state.body_state.in_body = true,
            TAG_DIV if state.body_state.in_body => {
                state.body_state.in_div = true;
                state.body_state.current_div_song_part = get_string_attribute(
                    e,
                    reader,
                    &[ATTR_ITUNES_SONG_PART_NEW, ATTR_ITUNES_SONG_PART],
                )?;
            }
            TAG_P if state.body_state.in_body => {
                state.body_state.in_p = true;

                let start_ms = get_time_attribute(e, reader, &[ATTR_BEGIN], warnings)?.unwrap_or(0);
                let end_ms = get_time_attribute(e, reader, &[ATTR_END], warnings)?.unwrap_or(0);

                let agent_attr_val =
                    get_string_attribute(e, reader, &[ATTR_AGENT, ATTR_AGENT_ALIAS])?;
                let final_agent_id = state.resolve_agent_id(agent_attr_val);

                let song_part = get_string_attribute(
                    e,
                    reader,
                    &[ATTR_ITUNES_SONG_PART_NEW, ATTR_ITUNES_SONG_PART],
                )?
                .or_else(|| state.body_state.current_div_song_part.clone());
                let itunes_key = get_string_attribute(e, reader, &[ATTR_ITUNES_KEY])?;

                state.body_state.current_p_element_data = Some(CurrentPElementData {
                    start_ms,
                    end_ms,
                    agent: final_agent_id,
                    song_part,
                    itunes_key,
                    ..Default::default()
                });

                state.text_buffer.clear();
                state.body_state.span_stack.clear();
            }
            _ => {}
        },
        Event::End(e) => match e.local_name().as_ref() {
            TAG_DIV if state.body_state.in_div => {
                state.body_state.in_div = false;
                state.body_state.current_div_song_part = None; // 离开 div 时清除
            }
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

/// 处理 `<tt>` 标签的开始事件，这是文档的根元素。
/// 主要任务是确定计时模式（逐行 vs 逐字）和文档的默认语言。
fn process_tt_start(
    e: &BytesStart,
    state: &mut TtmlParserState,
    raw_metadata: &mut HashMap<String, Vec<String>>,
    reader: &Reader<&[u8]>,
    has_timed_span_tags: bool,
    warnings: &mut Vec<String>,
    options: &TtmlParsingOptions,
) -> Result<(), ConvertError> {
    if let Some(forced_mode) = options.force_timing_mode {
        state.is_line_timing_mode = forced_mode == TtmlTimingMode::Line;
    } else {
        let timing_attr = e
            .try_get_attribute(ATTR_ITUNES_TIMING)
            .map_err(ConvertError::new_parse)?;
        if let Some(attr) = timing_attr {
            if attr.value.as_ref() == b"line" {
                state.is_line_timing_mode = true;
            }
        } else if !has_timed_span_tags {
            state.is_line_timing_mode = true;
            state.detected_line_mode = true;
            warnings.push(
                "未找到带时间戳的 <span> 标签且未指定 itunes:timing 模式，切换到逐行歌词模式。"
                    .to_string(),
            );
        }
    }

    // 获取 xml:lang 属性
    if let Some(attr) = e
        .try_get_attribute(ATTR_XML_LANG)
        .map_err(ConvertError::new_parse)?
    {
        let lang_val = attr
            .decode_and_unescape_value(reader.decoder())
            .map_err(ConvertError::new_parse)?;
        if !lang_val.is_empty() {
            let lang_val_owned = lang_val.into_owned();
            raw_metadata
                .entry("Language".to_string())
                .or_default()
                .push(lang_val_owned.clone());
            if state.default_main_lang.is_none() {
                state.default_main_lang = Some(lang_val_owned);
            }
        }
    }

    Ok(())
}

/// 尝试从一个XML格式错误中恢复。
pub(super) fn attempt_recovery_from_error(
    state: &mut TtmlParserState,
    reader: &Reader<&[u8]>,
    lines: &mut Vec<LyricLine>,
    warnings: &mut Vec<String>,
    error: &quick_xml::errors::Error,
) {
    let position = reader.error_position();
    warnings.push(format!("TTML 格式错误，位置 {position}: {error}。"));

    if state.body_state.in_p {
        // 错误发生在 <p> 标签内部
        // 尝试抢救当前行的数据，然后跳出这个<p>
        warnings.push(format!(
            "错误发生在 <p> 元素内部 (开始于 {}ms)。尝试恢复已经解析的数据。",
            state
                .body_state
                .current_p_element_data
                .as_ref()
                .map_or(0, |d| d.start_ms)
        ));

        // 处理和保存当前 <p> 中已经累积的数据
        // 把current_p_element_data中的内容（即使不完整）转换成一个 LyricLine
        body::handle_p_end(state, lines);

        // handle_p_end 已经将 in_p 设为 false，并清理了 span 栈，
        // 我们现在回到了“p之外，body之内”的安全状态
    } else if state.in_metadata {
        // 错误发生在 <metadata> 内部
        // 元数据太复杂了，简单地放弃所有数据好了
        warnings.push("错误发生在 <metadata> 块内部。放弃所有元数据。".to_string());
        state.in_metadata = false;
        state.metadata_state = MetadataParseState::default();
    } else {
        // 错误发生在全局作用域
        // 可能是 <body> 或 <div> 标签损坏。恢复的把握较小。
        // 我们重置所有 body 相关的状态，期望能找到下一个有效的 <p>。
        warnings
            .push("错误发生在全局作用域。将重置解析器状态，尝试寻找下一个有效元素。".to_string());
        state.body_state = BodyParseState::default();
    }
}
