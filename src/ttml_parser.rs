use crate::types::{AssMetadata, BackgroundSection, ConvertError, TtmlParagraph, TtmlSyllable};
use log::{error, info};
use quick_xml::Reader;
use quick_xml::events::Event;
use regex::Regex;
use std::collections::HashMap;

/// 定义一个类型别名，用于表示 TTML 解析函数的返回结果。
/// 这是一个包含元组的 Result，元组中包含：
/// 1. `Vec<TtmlParagraph>`: 解析出的歌词段落。
/// 2. `Vec<AssMetadata>`: 解析出的元数据。
/// 3. `bool`: 是否为逐行模式。
/// 4. `bool`: 是否检测到文件被格式化过（可能影响解析）。
/// 5. `Option<String>`: 检测到的第一个翻译的语言代码。
type ParseTtmlResult = Result<
    (
        Vec<TtmlParagraph>,
        Vec<AssMetadata>,
        bool,
        bool,
        Option<String>,
    ),
    ConvertError,
>;

/// 解析 TTML 中各种格式的时间字符串，并统一转换为毫秒。
///
/// 支持的格式包括：
/// - `HH:MM:SS.mmm` (小时:分钟:秒.毫秒)
/// - `MM:SS.mmm` (分钟:秒.毫秒)
/// - `SS.mmm` (秒.毫秒)
/// - `HH:MM:SS:fff` (小时:分钟:秒:帧，帧频在此处不适用，但可兼容解析)
/// - `HH:MM:SS` (小时:分钟:秒)
/// - `MM:SS` (分钟:秒)
/// - `SS` (秒)
///   其中，毫秒部分可以是1到3位数字。
///
/// # 参数
/// * `time_str` - TTML 格式的时间字符串。
///
/// # 返回
/// `Result<u64, ConvertError>` - 成功时返回总毫秒数，失败时返回相应的解析错误。
pub fn parse_any_ttml_time_ms(time_str: &str) -> Result<u64, ConvertError> {
    let colon_parts: Vec<&str> = time_str.split(':').collect(); // 按冒号分割时间字符串
    let hours: u64;
    let minutes: u64;
    let seconds: u64;
    let milliseconds: u64;

    // 辅助闭包，用于解析毫秒部分，并根据其长度进行适配（例如 "1" -> 100ms, "12" -> 120ms, "123" -> 123ms）
    let parse_ms_part = |ms_str: &str, original_time_str: &str| -> Result<u64, ConvertError> {
        // 校验毫秒部分的长度和内容是否合法
        if ms_str.is_empty() || ms_str.len() > 3 || ms_str.chars().any(|c| !c.is_ascii_digit()) {
            return Err(ConvertError::InvalidTime(format!(
                "时间戳 '{}' 中的毫秒部分 '{}' 格式无效",
                original_time_str, ms_str
            )));
        }
        Ok(match ms_str.len() {
            1 => ms_str.parse::<u64>().map_err(ConvertError::ParseInt)? * 100, // 1位数字，乘以100
            2 => ms_str.parse::<u64>().map_err(ConvertError::ParseInt)? * 10,  // 2位数字，乘以10
            3 => ms_str.parse::<u64>().map_err(ConvertError::ParseInt)?,       // 3位数字，直接解析
            _ => unreachable!(), // 此处不可达，因为前面已校验长度
        })
    };

    match colon_parts.len() {
        3 => {
            // 格式: HH:MM:SS.mmm
            hours = colon_parts[0].parse().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "在 '{}' 中解析小时 '{}' 失败: {}",
                    time_str, colon_parts[0], e
                ))
            })?;
            minutes = colon_parts[1].parse().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "在 '{}' 中解析分钟 '{}' 失败: {}",
                    time_str, colon_parts[1], e
                ))
            })?;
            let sec_ms_part = colon_parts[2]; // 秒和毫秒部分，例如 "SS.mmm"
            let dot_parts: Vec<&str> = sec_ms_part.split('.').collect(); // 按点分割秒和毫秒
            seconds = dot_parts[0].parse().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "在 '{}' 中解析秒 '{}' 失败: {}",
                    time_str, dot_parts[0], e
                ))
            })?;
            if dot_parts.len() == 2 {
                milliseconds = parse_ms_part(dot_parts[1], time_str)?;
            } else if dot_parts.len() == 1 {
                milliseconds = 0;
            } else {
                return Err(ConvertError::InvalidTime(format!(
                    "时间戳 '{}' 中的秒和毫秒部分格式无效: '{}'",
                    time_str, sec_ms_part
                )));
            }
        }
        2 => {
            // 格式: MM:SS.mmm
            hours = 0;
            minutes = colon_parts[0].parse().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "在 '{}' 中解析分钟 '{}' 失败: {}",
                    time_str, colon_parts[0], e
                ))
            })?;
            let sec_ms_part = colon_parts[1];
            let dot_parts: Vec<&str> = sec_ms_part.split('.').collect();
            seconds = dot_parts[0].parse().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "在 '{}' 中解析秒 '{}' 失败: {}",
                    time_str, dot_parts[0], e
                ))
            })?;
            if dot_parts.len() == 2 {
                milliseconds = parse_ms_part(dot_parts[1], time_str)?;
            } else if dot_parts.len() == 1 {
                milliseconds = 0;
            } else {
                return Err(ConvertError::InvalidTime(format!(
                    "时间戳 '{}' 中的秒和毫秒部分格式无效: '{}'",
                    time_str, sec_ms_part
                )));
            }
        }
        1 => {
            // 格式: SS.mmm 或 SS
            hours = 0;
            minutes = 0;
            let sec_ms_part = colon_parts[0];
            let dot_parts: Vec<&str> = sec_ms_part.split('.').collect();
            seconds = dot_parts[0].parse().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "在 '{}' 中解析秒 '{}' 失败: {}",
                    time_str, dot_parts[0], e
                ))
            })?;
            if dot_parts.len() == 2 {
                milliseconds = parse_ms_part(dot_parts[1], time_str)?;
            } else if dot_parts.len() == 1 {
                milliseconds = 0;
            } else {
                return Err(ConvertError::InvalidTime(format!(
                    "时间戳 '{}' 中的秒和毫秒部分格式无效: '{}'",
                    time_str, sec_ms_part
                )));
            }
        }
        _ => {
            return Err(ConvertError::InvalidTime(format!(
                "时间格式 '{}' 无效。",
                time_str
            )));
        }
    }

    // 校验分钟和秒的值是否超出正常范围
    if minutes >= 60 {
        return Err(ConvertError::InvalidTime(format!(
            "分钟值 '{}' (应小于60) 在时间戳 '{}' 中无效",
            minutes, time_str
        )));
    }
    if seconds >= 60 {
        return Err(ConvertError::InvalidTime(format!(
            "秒值 '{}' (应小于60) 在时间戳 '{}' 中无效",
            seconds, time_str
        )));
    }

    // 计算总毫秒数并返回
    Ok(hours * 3_600_000 + minutes * 60_000 + seconds * 1000 + milliseconds)
}

/// 枚举：表示 TTML 中 `<span>` 标签可能包含的内容类型。
#[derive(Debug, Clone, Copy, PartialEq)]
enum SpanContentType {
    None,                // 未指定类型或通用内容
    Syllable,            // 逐字歌词音节
    Translation,         // 翻译文本
    Romanization,        // 罗马音文本
    BackgroundContainer, // 背景歌词容器 (通常是 <span ttm:role="x-bg">)
}

/// 枚举：表示当前解析到的文本应附加到哪个部分（主歌词或背景歌词）。
#[derive(Debug, Clone, Copy, PartialEq)]
enum TextTargetContext {
    Main,       // 目标是主歌词部分
    Background, // 目标是背景歌词部分
}

/// 枚举：记录上一个结束的 `<span>` 标签是否是音节，以及其类型。
/// 这个状态用于正确处理音节后面的空格（当空格作为独立的文本节点存在时）。
#[derive(Debug, Clone, Copy, PartialEq)]
enum LastEndedSyllableSpanInfo {
    None,               // 上一个结束的 span 不是音节，或者是第一个音节
    MainSyllable,       // 上一个结束的 span 是主歌词音节
    BackgroundSyllable, // 上一个结束的 span 是背景歌词音节
}

/// 提取主翻译中括号里的部分，作为背景翻译。
///
/// # 参数
/// * `para` - 一个可变的 `TtmlParagraph` 引用。函数将直接修改这个段落。
fn extract_and_apply_parenthesized_translation(
    para: &mut TtmlParagraph,
    main_translation_text: &str,
    main_translation_lang: &Option<String>,
) {
    let re = Regex::new(r"[\s　]*[（(]([^（()）]+)[）)][\s　]*$").unwrap();

    let mut final_main_text = main_translation_text.to_string();

    if let Some(caps) = re.captures(main_translation_text) {
        if let Some(bg_trans_match) = caps.get(1) {
            let bg_trans_text = bg_trans_match.as_str().trim().to_string();

            if !bg_trans_text.is_empty() {
                let bg_sec = para
                    .background_section
                    .get_or_insert_with(|| BackgroundSection {
                        start_ms: para.p_start_ms,
                        end_ms: para.p_end_ms,
                        ..Default::default()
                    });

                if bg_sec.translation.is_none() {
                    bg_sec.translation = Some((bg_trans_text, main_translation_lang.clone()));
                    final_main_text = re.replace(main_translation_text, "").trim().to_string();
                }
            }
        }
    }

    para.translation = Some((final_main_text, main_translation_lang.clone()));
}

/// 从字符串解析 TTML 内容。
///
/// # 参数
/// * `ttml_content` - 包含 TTML 数据格式的字符串。
///
/// # 返回
/// 一个 `ParseTtmlResult`，其中包含：
///   - `Vec<TtmlParagraph>`: 解析出的歌词段落列表。
///   - `Vec<AssMetadata>`: 解析出的元数据列表。
///   - `bool`: 指示 TTML 是否为逐行模式 (`itunes:timing="Line"`)。
///   - `bool`: 指示 TTML 是否可能经过了格式化（例如，标签间有不必要的换行和空格）。
///   - `Option<String>`: 检测到的第一个翻译的语言代码。
pub fn parse_ttml_from_string(ttml_content: &str) -> ParseTtmlResult {
    let mut reader = Reader::from_str(ttml_content);
    reader.config_mut().trim_text(false); // 配置解析器不自动去除文本节点两端的空白

    // 用于存储最终解析结果的变量
    let mut paragraphs: Vec<TtmlParagraph> = Vec::new();
    let mut metadata: Vec<AssMetadata> = Vec::new();
    let mut is_line_timing_mode = false;
    let mut detected_formatted_ttml_or_normalized_text = false;
    let mut first_translation_lang_code: Option<String> = None;

    // 用于解析和存储 Apple Music 的翻译。
    let mut am_translations: HashMap<String, (String, Option<String>)> = HashMap::new();
    let mut in_translations_tag = false;
    let mut in_translation_tag = false;
    let mut current_translation_lang: Option<String> = None;
    let mut current_text_for_key: Option<String> = None;
    let mut current_am_translation_text = String::new();

    // 解析过程中的状态变量
    let mut in_metadata_section = false;
    let mut in_itunes_metadata = false;
    let mut in_songwriters_tag = false;
    let mut in_songwriter_tag = false;
    let mut current_songwriter_name = String::new();
    let mut in_agent_tag = false;
    let mut in_agent_name_tag = false;
    let mut current_agent_id_for_name: Option<String> = None;
    let mut current_agent_name_text = String::new();

    // 逐字模式 (Word timing) 的状态变量
    let mut current_paragraph_word_mode: Option<TtmlParagraph> = None;
    let mut current_span_text_accumulator = String::new();
    let mut last_ended_syllable_span_info = LastEndedSyllableSpanInfo::None;
    type SpanStackItem = (
        SpanContentType,
        TextTargetContext,
        Option<u64>,
        Option<u64>,
        Option<String>,
    );
    let mut span_type_stack_word_mode: Vec<SpanStackItem> = Vec::new();
    let mut current_div_song_part_word_mode: Option<String> = None;

    // 逐行模式 (Line timing) 的状态变量
    let mut current_p_data_for_line_mode: Option<(u64, u64, Option<String>, Option<String>)> = None;
    let mut current_p_text_for_line_mode = String::new();
    let mut current_p_translation_line_mode: Option<(String, Option<String>)> = None;
    let mut current_p_romanization_line_mode: Option<String> = None;
    let mut current_div_song_part_line_mode: Option<String> = None;
    let mut in_p_element = false;

    // 主循环，通过读取 XML 事件来驱动解析过程
    loop {
        let event = reader.read_event();

        match event {
            Ok(Event::Start(e)) => {
                // --- 处理开始标签 <...> ---
                last_ended_syllable_span_info = LastEndedSyllableSpanInfo::None;
                let full_name_bytes = e.name();
                let local_name_bytes = e.local_name();
                let local_name_str =
                    String::from_utf8_lossy(local_name_bytes.as_ref()).into_owned();

                match local_name_str.as_str() {
                    "tt" => {
                        // <tt> 根元素
                        for attr_res in e.attributes() {
                            let attr = attr_res?;
                            match attr.key.as_ref() {
                                b"itunes:timing" => {
                                    if attr.decode_and_unescape_value(reader.decoder())?.as_ref()
                                        == "Line"
                                    {
                                        is_line_timing_mode = true;
                                        info!(target: "unilyric::ttml_parser", "[TTML 解析] 检测到逐行模式 (itunes:timing=\"Line\")。");
                                    }
                                }
                                b"xml:lang" => {
                                    let lang_val = attr
                                        .decode_and_unescape_value(reader.decoder())?
                                        .into_owned();
                                    metadata.push(AssMetadata {
                                        key: "lang".to_string(),
                                        value: lang_val,
                                    });
                                }
                                _ => {}
                            }
                        }
                    }
                    "metadata" if !in_p_element => in_metadata_section = true,
                    "iTunesMetadata" if in_metadata_section => in_itunes_metadata = true,
                    "translations" if in_itunes_metadata => {
                        in_translations_tag = true;
                    }
                    "translation" if in_translations_tag => {
                        in_translation_tag = true;
                        current_translation_lang = e
                            .attributes()
                            .flatten()
                            .find(|attr| attr.key.as_ref() == b"xml:lang")
                            .and_then(|attr| attr.decode_and_unescape_value(reader.decoder()).ok())
                            .map(|cow| cow.into_owned());
                    }
                    "text" if in_translation_tag => {
                        current_text_for_key = e
                            .attributes()
                            .flatten()
                            .find(|attr| attr.key.as_ref() == b"for")
                            .and_then(|attr| attr.decode_and_unescape_value(reader.decoder()).ok())
                            .map(|cow| cow.into_owned());
                        current_am_translation_text.clear();
                    }
                    "songwriters" if in_itunes_metadata => in_songwriters_tag = true,
                    "songwriter" if in_songwriters_tag => {
                        in_songwriter_tag = true;
                        current_songwriter_name.clear();
                    }
                    "agent" if full_name_bytes.as_ref() == b"ttm:agent" && in_metadata_section => {
                        in_agent_tag = true;
                        current_agent_id_for_name = e
                            .attributes()
                            .flatten()
                            .find(|attr| attr.key.as_ref() == b"xml:id")
                            .and_then(|attr| attr.decode_and_unescape_value(reader.decoder()).ok())
                            .map(|cow| cow.into_owned());
                    }
                    "name" if in_agent_tag && full_name_bytes.as_ref() == b"ttm:name" => {
                        in_agent_name_tag = true;
                        current_agent_name_text.clear();
                    }
                    "meta" if in_metadata_section => {
                        let mut key_attr = None;
                        let mut value_attr = None;
                        for attr_res in e.attributes() {
                            let attr = attr_res?;
                            let attr_value_str = attr
                                .decode_and_unescape_value(reader.decoder())?
                                .into_owned();
                            if attr.key.local_name().as_ref() == b"key" {
                                key_attr = Some(attr_value_str);
                            } else if attr.key.local_name().as_ref() == b"value" {
                                value_attr = Some(attr_value_str);
                            }
                        }
                        if let (Some(k), Some(v)) = (key_attr.filter(|s| !s.is_empty()), value_attr)
                        {
                            metadata.push(AssMetadata { key: k, value: v });
                        }
                    }
                    "div" => {
                        let song_part_attr_val = e
                            .attributes()
                            .filter_map(Result::ok)
                            .find(|attr| attr.key.as_ref() == b"itunes:song-part")
                            .and_then(|attr| {
                                attr.decode_and_unescape_value(reader.decoder())
                                    .ok()
                                    .map(|cow| cow.into_owned())
                            });
                        if is_line_timing_mode {
                            current_div_song_part_line_mode = song_part_attr_val;
                        } else {
                            current_div_song_part_word_mode = song_part_attr_val;
                        }
                    }
                    "p" => {
                        // <p> 歌词段落
                        in_p_element = true;
                        if is_line_timing_mode {
                            let mut p_start_ms_val = 0;
                            let mut p_end_ms_val = 0;
                            let mut p_agent_opt_val: Option<String> = None;
                            let mut p_song_part_val = current_div_song_part_line_mode.clone();
                            let mut p_key_val: Option<String> = None;

                            for attr_res in e.attributes() {
                                let attr = attr_res?;
                                let value_cow = attr.decode_and_unescape_value(reader.decoder())?;
                                match attr.key.as_ref() {
                                    b"begin" => {
                                        p_start_ms_val = parse_any_ttml_time_ms(&value_cow)?
                                    }
                                    b"end" => p_end_ms_val = parse_any_ttml_time_ms(&value_cow)?,
                                    b"agent" | b"ttm:agent" => {
                                        p_agent_opt_val = Some(value_cow.into_owned())
                                    }
                                    b"itunes:song-part" => {
                                        p_song_part_val = Some(value_cow.into_owned())
                                    }
                                    b"itunes:key" => {
                                        p_key_val = Some(value_cow.into_owned());
                                    }
                                    _ => {}
                                }
                            }
                            current_p_data_for_line_mode =
                                Some((p_start_ms_val, p_end_ms_val, p_agent_opt_val, p_key_val));
                            current_div_song_part_line_mode = p_song_part_val;
                            current_p_text_for_line_mode.clear();
                            current_p_translation_line_mode = None;
                            current_p_romanization_line_mode = None;
                        } else {
                            // 逐字模式
                            span_type_stack_word_mode.clear();
                            let mut p_data = TtmlParagraph {
                                song_part: current_div_song_part_word_mode.clone(),
                                ..Default::default()
                            };
                            for attr_res in e.attributes() {
                                let attr = attr_res?;
                                let value_cow = attr.decode_and_unescape_value(reader.decoder())?;
                                match attr.key.as_ref() {
                                    b"ttm:agent" => p_data.agent = value_cow.into_owned(),
                                    b"begin" => {
                                        p_data.p_start_ms = parse_any_ttml_time_ms(&value_cow)?
                                    }
                                    b"end" => p_data.p_end_ms = parse_any_ttml_time_ms(&value_cow)?,
                                    b"itunes:song-part" => {
                                        p_data.song_part = Some(value_cow.into_owned())
                                    }
                                    b"itunes:key" => {
                                        p_data.itunes_key = Some(value_cow.into_owned());
                                    }
                                    _ => {}
                                }
                            }
                            current_paragraph_word_mode = Some(p_data);
                        }
                    }
                    "span" if in_p_element => {
                        // <span> 标签
                        current_span_text_accumulator.clear();
                        let mut current_span_begin_ms: Option<u64> = None;
                        let mut current_span_end_ms: Option<u64> = None;
                        let mut role_attr: Option<String> = None;
                        let mut lang_attr: Option<String> = None;
                        for attr_res in e.attributes() {
                            let attr = attr_res?;
                            let value_cow = attr.decode_and_unescape_value(reader.decoder())?;
                            match attr.key.as_ref() {
                                b"role" | b"ttm:role" => role_attr = Some(value_cow.into_owned()),
                                b"xml:lang" | b"lang" => {
                                    let lang_value = value_cow.into_owned();
                                    if !lang_value.is_empty() {
                                        lang_attr = Some(lang_value);
                                    }
                                }
                                b"begin" => {
                                    current_span_begin_ms =
                                        Some(parse_any_ttml_time_ms(&value_cow)?)
                                }
                                b"end" => {
                                    current_span_end_ms = Some(parse_any_ttml_time_ms(&value_cow)?)
                                }
                                _ => {}
                            }
                        }

                        if role_attr.as_deref() == Some("x-translation")
                            && first_translation_lang_code.is_none()
                        {
                            if let Some(lang) = &lang_attr {
                                if !lang.is_empty() {
                                    first_translation_lang_code = Some(lang.clone());
                                }
                            }
                        }

                        if is_line_timing_mode {
                            let content_type = match role_attr.as_deref() {
                                Some("x-translation") => SpanContentType::Translation,
                                Some("x-roman") => SpanContentType::Romanization,
                                _ => SpanContentType::None,
                            };
                            span_type_stack_word_mode.push((
                                content_type,
                                TextTargetContext::Main,
                                None,
                                None,
                                lang_attr,
                            ));
                        } else {
                            let content_type = match role_attr.as_deref() {
                                Some("x-translation") => SpanContentType::Translation,
                                Some("x-roman") => SpanContentType::Romanization,
                                Some("x-bg") => SpanContentType::BackgroundContainer,
                                None if current_span_begin_ms.is_some()
                                    && current_span_end_ms.is_some() =>
                                {
                                    SpanContentType::Syllable
                                }
                                _ => SpanContentType::None,
                            };
                            let parent_context = span_type_stack_word_mode
                                .last()
                                .map_or(TextTargetContext::Main, |(_, ctx, ..)| *ctx);
                            let current_target_context =
                                if content_type == SpanContentType::BackgroundContainer {
                                    TextTargetContext::Background
                                } else {
                                    parent_context
                                };
                            span_type_stack_word_mode.push((
                                content_type,
                                current_target_context,
                                current_span_begin_ms,
                                current_span_end_ms,
                                lang_attr,
                            ));

                            if content_type == SpanContentType::BackgroundContainer {
                                if let Some(para) = current_paragraph_word_mode.as_mut() {
                                    if para.background_section.is_none() {
                                        para.background_section = Some(BackgroundSection {
                                            start_ms: current_span_begin_ms
                                                .unwrap_or(para.p_start_ms),
                                            end_ms: current_span_end_ms.unwrap_or(para.p_end_ms),
                                            ..Default::default()
                                        });
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                // --- 处理自闭合标签 <.../> ---
                last_ended_syllable_span_info = LastEndedSyllableSpanInfo::None;
                if e.local_name().as_ref() == b"meta" && in_metadata_section {
                    // 主要处理自闭合的 <meta ... /> 标签
                    let mut key_attr: Option<String> = None;
                    let mut value_attr: Option<String> = None;
                    for attr_res in e.attributes() {
                        let attr = attr_res?;
                        let attr_value_cow = attr.decode_and_unescape_value(reader.decoder())?;
                        if attr.key.as_ref() == b"key" {
                            key_attr = Some(attr_value_cow.into_owned());
                        } else if attr.key.as_ref() == b"value" {
                            value_attr = Some(attr_value_cow.into_owned());
                        }
                    }
                    if let (Some(k), Some(v)) = (key_attr.filter(|s| !s.is_empty()), value_attr) {
                        metadata.push(AssMetadata { key: k, value: v });
                    }
                }
            }
            Ok(Event::Text(e_text)) => {
                // --- 处理文本节点 ---
                let text_cow = e_text.unescape()?;
                let text_str = text_cow.as_ref();

                if current_text_for_key.is_some() {
                    current_am_translation_text.push_str(text_str);
                } else if in_p_element {
                    if is_line_timing_mode {
                        if let Some((span_type, _, _, _, lang_code_opt)) =
                            span_type_stack_word_mode.last()
                        {
                            match span_type {
                                SpanContentType::Translation => {
                                    let (text, _lang) = current_p_translation_line_mode
                                        .get_or_insert_with(|| {
                                            (String::new(), lang_code_opt.clone())
                                        });
                                    text.push_str(text_str);
                                }
                                SpanContentType::Romanization => {
                                    current_p_romanization_line_mode
                                        .get_or_insert_with(String::new)
                                        .push_str(text_str);
                                }
                                _ => {
                                    current_p_text_for_line_mode.push_str(text_str);
                                }
                            }
                        } else {
                            current_p_text_for_line_mode.push_str(text_str);
                        }
                    } else {
                        // 逐字模式
                        if last_ended_syllable_span_info != LastEndedSyllableSpanInfo::None {
                            // 处理音节之间的文本（通常是空格）
                            if text_str.contains('\n')
                                && !detected_formatted_ttml_or_normalized_text
                            {
                                detected_formatted_ttml_or_normalized_text = true;
                            }
                            if !text_str.is_empty() && text_str.chars().all(char::is_whitespace) {
                                if let Some(para) = current_paragraph_word_mode.as_mut() {
                                    let target_syllables = if last_ended_syllable_span_info
                                        == LastEndedSyllableSpanInfo::MainSyllable
                                    {
                                        &mut para.main_syllables
                                    } else if let Some(bg) = para.background_section.as_mut() {
                                        &mut bg.syllables
                                    } else {
                                        // 如果背景部分不存在，则忽略
                                        continue;
                                    };
                                    if let Some(last_syl) = target_syllables.last_mut() {
                                        last_syl.ends_with_space = true;
                                    }
                                }
                            }
                            last_ended_syllable_span_info = LastEndedSyllableSpanInfo::None;
                        } else if !span_type_stack_word_mode.is_empty() {
                            // 文本在 <span> 内部
                            current_span_text_accumulator.push_str(text_str);
                        } else {
                            // 【兼容性处理】文本直接在 <p> 内部（不规范情况，如单个字成行）
                            let trimmed_text = text_str.trim();
                            if !trimmed_text.is_empty() {
                                if let Some(para) = current_paragraph_word_mode.as_mut() {
                                    // 将这个文本视为一个覆盖整个 <p> 时长的音节
                                    let syllable = TtmlSyllable {
                                        text: trimmed_text.to_string(),
                                        start_ms: para.p_start_ms,
                                        end_ms: para.p_end_ms,
                                        ends_with_space: false,
                                    };
                                    para.main_syllables.push(syllable);
                                    last_ended_syllable_span_info =
                                        LastEndedSyllableSpanInfo::MainSyllable;
                                }
                            }
                        }
                    }
                } else if in_songwriter_tag {
                    current_songwriter_name.push_str(text_str);
                } else if in_agent_name_tag {
                    current_agent_name_text.push_str(text_str);
                }
            }
            Ok(Event::End(e)) => {
                // --- 处理结束标签 </...> ---
                let local_name_str = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                match local_name_str.as_str() {
                    "div" => {
                        if is_line_timing_mode {
                            current_div_song_part_line_mode = None;
                        } else {
                            current_div_song_part_word_mode = None;
                        }
                    }
                    "p" => {
                        if is_line_timing_mode {
                            if let Some((p_start, p_end, p_agent_opt, p_key_opt)) =
                                current_p_data_for_line_mode.take()
                            {
                                let main_text = current_p_text_for_line_mode.trim().to_string();
                                if !main_text.is_empty()
                                    || current_p_translation_line_mode.is_some()
                                    || current_p_romanization_line_mode.is_some()
                                    || (p_end > p_start)
                                {
                                    paragraphs.push(TtmlParagraph {
                                        p_start_ms: p_start,
                                        p_end_ms: p_end,
                                        agent: p_agent_opt.unwrap_or_else(|| "v1".to_string()),
                                        main_syllables: vec![TtmlSyllable {
                                            text: main_text,
                                            start_ms: p_start,
                                            end_ms: p_end,
                                            ..Default::default()
                                        }],
                                        song_part: current_div_song_part_line_mode.clone(),
                                        translation: current_p_translation_line_mode.take(),
                                        romanization: current_p_romanization_line_mode.take(),
                                        itunes_key: p_key_opt,
                                        ..Default::default()
                                    });
                                }
                            }
                        } else {
                            if let Some(para) = current_paragraph_word_mode.take() {
                                if !para.main_syllables.is_empty()
                                    || para.background_section.as_ref().is_some_and(|bs| {
                                        !bs.syllables.is_empty()
                                            || bs.translation.is_some()
                                            || bs.romanization.is_some()
                                    })
                                    || para.translation.is_some()
                                    || para.romanization.is_some()
                                    || (para.p_end_ms > para.p_start_ms)
                                {
                                    paragraphs.push(para);
                                }
                            }
                            span_type_stack_word_mode.clear();
                        }
                        in_p_element = false;
                        last_ended_syllable_span_info = LastEndedSyllableSpanInfo::None;
                    }
                    "span" if in_p_element => {
                        if let Some((span_type, context, begin_ms_opt, end_ms_opt, lang_attr_opt)) =
                            span_type_stack_word_mode.pop()
                        {
                            let raw_accumulated_text = current_span_text_accumulator.clone();
                            current_span_text_accumulator.clear();

                            if !is_line_timing_mode {
                                if let Some(para) = current_paragraph_word_mode.as_mut() {
                                    match span_type {
                                        SpanContentType::Syllable => {
                                            if let (Some(start_ms), Some(end_ms)) =
                                                (begin_ms_opt, end_ms_opt)
                                            {
                                                let (core_text_str, ends_with_space) =
                                                    if !raw_accumulated_text.is_empty()
                                                        && raw_accumulated_text
                                                            .chars()
                                                            .all(char::is_whitespace)
                                                    {
                                                        (" ".to_string(), false)
                                                    } else {
                                                        let trimmed =
                                                            raw_accumulated_text.trim_end();
                                                        (
                                                            trimmed.to_string(),
                                                            raw_accumulated_text.len()
                                                                > trimmed.len(),
                                                        )
                                                    };

                                                if !core_text_str.is_empty()
                                                    || ((core_text_str.is_empty()
                                                        || core_text_str == " ")
                                                        && end_ms > start_ms)
                                                {
                                                    let syllable = TtmlSyllable {
                                                        text: core_text_str,
                                                        start_ms,
                                                        end_ms,
                                                        ends_with_space,
                                                    };
                                                    match context {
                                                        TextTargetContext::Main => {
                                                            para.main_syllables.push(syllable);
                                                            last_ended_syllable_span_info = LastEndedSyllableSpanInfo::MainSyllable;
                                                        }
                                                        TextTargetContext::Background => {
                                                            let bg_sec = para
                                                                .background_section
                                                                .get_or_insert_with(
                                                                    Default::default,
                                                                );
                                                            bg_sec.syllables.push(syllable);
                                                            last_ended_syllable_span_info = LastEndedSyllableSpanInfo::BackgroundSyllable;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        SpanContentType::Translation
                                        | SpanContentType::Romanization => {
                                            let (normalized_text, was_changed) =
                                                normalize_whitespace_and_check_changes(
                                                    &raw_accumulated_text,
                                                );
                                            if was_changed
                                                && !detected_formatted_ttml_or_normalized_text
                                            {
                                                detected_formatted_ttml_or_normalized_text = true;
                                            }
                                            if !normalized_text.is_empty() {
                                                // 分别处理主歌词和背景歌词的上下文
                                                match context {
                                                    TextTargetContext::Main => {
                                                        if span_type == SpanContentType::Translation
                                                        {
                                                            para.translation = Some((
                                                                normalized_text,
                                                                lang_attr_opt,
                                                            ));
                                                        } else {
                                                            // Romanization
                                                            para.romanization =
                                                                Some(normalized_text);
                                                        }
                                                    }
                                                    TextTargetContext::Background => {
                                                        if let Some(bg_sec) =
                                                            para.background_section.as_mut()
                                                        {
                                                            if span_type
                                                                == SpanContentType::Translation
                                                            {
                                                                bg_sec.translation = Some((
                                                                    normalized_text,
                                                                    lang_attr_opt,
                                                                ));
                                                            } else {
                                                                // Romanization
                                                                bg_sec.romanization =
                                                                    Some(normalized_text);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        SpanContentType::BackgroundContainer => {
                                            let trimmed_direct_text = raw_accumulated_text.trim();

                                            if !trimmed_direct_text.is_empty() {
                                                if let Some(para_mut) =
                                                    current_paragraph_word_mode.as_mut()
                                                {
                                                    let bg_section_has_no_syllables = para_mut
                                                        .background_section
                                                        .as_ref()
                                                        .is_none_or(|bs| bs.syllables.is_empty());

                                                    if bg_section_has_no_syllables {
                                                        if let (
                                                            Some(bg_start_ms),
                                                            Some(bg_end_ms),
                                                        ) = (begin_ms_opt, end_ms_opt)
                                                        {
                                                            if bg_end_ms > bg_start_ms
                                                                || (!trimmed_direct_text.is_empty()
                                                                    && bg_end_ms == bg_start_ms)
                                                            {
                                                                let syllable_text_content =
                                                                    raw_accumulated_text.clone();

                                                                let ends_with_space_flag =
                                                                    syllable_text_content
                                                                        .ends_with(' ')
                                                                        && syllable_text_content
                                                                            .len()
                                                                            > 1
                                                                        && !syllable_text_content
                                                                            .trim_end()
                                                                            .is_empty();

                                                                let syllable = TtmlSyllable {
                                                                    text: syllable_text_content,
                                                                    start_ms: bg_start_ms,
                                                                    end_ms: bg_end_ms,
                                                                    ends_with_space:
                                                                        ends_with_space_flag,
                                                                };

                                                                let bg_sec = para_mut
                                                                    .background_section
                                                                    .get_or_insert_with(|| {
                                                                        BackgroundSection {
                                                                            start_ms: bg_start_ms,
                                                                            end_ms: bg_end_ms,
                                                                            ..Default::default()
                                                                        }
                                                                    });
                                                                bg_sec.start_ms = bg_sec
                                                                    .start_ms
                                                                    .min(bg_start_ms);
                                                                bg_sec.end_ms =
                                                                    bg_sec.end_ms.max(bg_end_ms);

                                                                bg_sec.syllables.push(syllable);
                                                                last_ended_syllable_span_info = LastEndedSyllableSpanInfo::BackgroundSyllable;
                                                            } else if !trimmed_direct_text
                                                                .is_empty()
                                                            {
                                                                log::warn!(
                                                                    target: "unilyric::ttml_parser",
                                                                    "[TTML 处理] 背景区块 <span ttm:role=\"x-bg\"> 包含直接文本 \"{}\"，但时间戳无效 ({}ms - {}ms)。该文本未作为音节处理。",
                                                                    raw_accumulated_text, bg_start_ms, bg_end_ms
                                                                );
                                                                last_ended_syllable_span_info =
                                                                    LastEndedSyllableSpanInfo::None;
                                                            } else {
                                                                last_ended_syllable_span_info =
                                                                    LastEndedSyllableSpanInfo::None;
                                                            }
                                                        } else {
                                                            log::warn!(
                                                                target: "unilyric::ttml_parser",
                                                                "[TTML 处理] 背景区块 <span ttm:role=\"x-bg\"> 包含直接文本 \"{}\"，但该 x-bg span 缺少时间信息。该文本未作为音节处理。",
                                                                raw_accumulated_text
                                                            );
                                                            last_ended_syllable_span_info =
                                                                LastEndedSyllableSpanInfo::None;
                                                        }
                                                    } else if !trimmed_direct_text.is_empty() {
                                                        log::warn!(
                                                            target: "unilyric::ttml_parser",
                                                            "[TTML 处理] 背景区块 <span ttm:role=\"x-bg\"> 包含直接文本 \"{}\"，但也包含嵌套音节。该直接文本被忽略。",
                                                            raw_accumulated_text
                                                        );
                                                    }
                                                } else {
                                                    last_ended_syllable_span_info =
                                                        LastEndedSyllableSpanInfo::None;
                                                }
                                            }
                                        }
                                        _ => {
                                            last_ended_syllable_span_info =
                                                LastEndedSyllableSpanInfo::None;
                                        }
                                    }
                                }
                            }
                        } else {
                            last_ended_syllable_span_info = LastEndedSyllableSpanInfo::None;
                        }
                    }
                    "translations" if in_translations_tag => {
                        in_translations_tag = false;
                    }
                    "translation" if in_translation_tag => {
                        in_translation_tag = false;
                        current_translation_lang = None;
                    }
                    "text" if in_translation_tag => {
                        if let Some(key) = current_text_for_key.take() {
                            if !current_am_translation_text.is_empty() {
                                am_translations.entry(key).or_insert_with(|| {
                                    (
                                        current_am_translation_text.clone(),
                                        current_translation_lang.clone(),
                                    )
                                });
                            }
                        }
                    }
                    "metadata" if in_metadata_section => in_metadata_section = false,
                    "songwriter" if in_songwriter_tag => {
                        if !current_songwriter_name.trim().is_empty() {
                            metadata.push(AssMetadata {
                                key: "songwriter".to_string(),
                                value: current_songwriter_name.trim().to_string(),
                            });
                        }
                        in_songwriter_tag = false;
                    }
                    "songwriters" if in_songwriters_tag => in_songwriters_tag = false,
                    "iTunesMetadata" if in_itunes_metadata => in_itunes_metadata = false,
                    "name" if in_agent_name_tag && e.name().as_ref() == b"ttm:name" => {
                        if let Some(agent_id) = &current_agent_id_for_name {
                            if !current_agent_name_text.trim().is_empty() {
                                metadata.push(AssMetadata {
                                    key: agent_id.clone(),
                                    value: current_agent_name_text.trim().to_string(),
                                });
                            }
                        }
                        in_agent_name_tag = false;
                    }
                    "agent" if in_agent_tag && e.name().as_ref() == b"ttm:agent" => {
                        in_agent_tag = false;
                        current_agent_id_for_name = None;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break, // 文件结束
            Err(e) => {
                let error_msg = format!(
                    "[TTML 解析] XML解析失败，位置 {}: {}",
                    reader.buffer_position(),
                    e
                );
                error!(target: "unilyric::ttml_parser", "{}", error_msg);
                return Err(ConvertError::Xml(e));
            }
            _ => {}
        }
    }
    if !am_translations.is_empty() {
        for para in paragraphs.iter_mut() {
            if para.translation.is_none() {
                if let Some(key) = &para.itunes_key {
                    if let Some((trans_text, trans_lang)) = am_translations.get(key) {
                        extract_and_apply_parenthesized_translation(para, trans_text, trans_lang);
                    }
                }
            } else if let Some((trans_text, trans_lang)) = para.translation.clone() {
                extract_and_apply_parenthesized_translation(para, &trans_text, &trans_lang);
            }
        }
    }
    info!(target: "unilyric::ttml_parser", "[TTML 解析] 解析完成。共 {} 个段落, {} 条元数据。逐行模式: {}, 检测到格式化: {}", paragraphs.len(), metadata.len(), is_line_timing_mode, detected_formatted_ttml_or_normalized_text);
    Ok((
        paragraphs,
        metadata,
        is_line_timing_mode,
        detected_formatted_ttml_or_normalized_text,
        first_translation_lang_code,
    ))
}

/// 规范化字符串中的空白字符（多个连续空白变为单个空格，并移除首尾空白），并检查字符串是否因此发生改变。
fn normalize_whitespace_and_check_changes(text: &str) -> (String, bool) {
    let normalized: String = text.split_whitespace().collect::<Vec<&str>>().join(" ");
    let changed = normalized != text;
    (normalized, changed)
}
