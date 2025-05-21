// 导入 quick_xml 库用于高效的 XML 解析
use quick_xml::Reader;
use quick_xml::events::Event;
// 导入标准库的 Cow (Clone-on-Write) 用于智能字符串切片或所有权，以及 str 模块
use std::{borrow::Cow, str};
// 导入 log 宏，用于记录信息和错误
use log::{error, info};
// 从项目类型模块导入所需的数据结构和错误类型
use crate::types::{AssMetadata, BackgroundSection, ConvertError, TtmlParagraph, TtmlSyllable};

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

/// 解析 TTML 时间字符串（多种格式）并转换为毫秒。
/// 支持的格式包括：
/// - HH:MM:SS.mmm (小时:分钟:秒.毫秒)
/// - MM:SS.mmm (分钟:秒.毫秒)
/// - SS.mmm (秒.毫秒)
/// - HH:MM:SS (小时:分钟:秒)
/// - MM:SS (分钟:秒)
/// - SS (秒)
///   毫秒部分可以是1到3位数字。
///
/// # Arguments
/// * `time_str` - TTML 时间字符串。
///
/// # Returns
/// `Result<u64, ConvertError>` - 成功时返回总毫秒数，失败时返回错误。
pub fn parse_any_ttml_time_ms(time_str: &str) -> Result<u64, ConvertError> {
    let colon_parts: Vec<&str> = time_str.split(':').collect(); // 按冒号分割时间字符串
    let hours: u64;
    let minutes: u64;
    let seconds: u64;
    let milliseconds: u64;

    // 辅助闭包，用于解析毫秒部分，并根据其长度调整（例如 "1" -> 100ms, "12" -> 120ms, "123" -> 123ms）
    let parse_ms_part = |ms_str: &str, original_time_str: &str| -> Result<u64, ConvertError> {
        // 校验毫秒部分的长度和内容
        if ms_str.is_empty() || ms_str.len() > 3 || ms_str.chars().any(|c| !c.is_ascii_digit()) {
            return Err(ConvertError::InvalidTime(format!(
                "毫秒部分 '{}' 在时间戳 '{}' 中无效",
                ms_str, original_time_str
            )));
        }
        Ok(match ms_str.len() {
            1 => ms_str.parse::<u64>().map_err(ConvertError::ParseInt)? * 100, // 1位 -> 乘以100
            2 => ms_str.parse::<u64>().map_err(ConvertError::ParseInt)? * 10,  // 2位 -> 乘以10
            3 => ms_str.parse::<u64>().map_err(ConvertError::ParseInt)?,       // 3位 -> 直接解析
            _ => unreachable!(), // 因为前面已经校验了长度
        })
    };

    match colon_parts.len() {
        3 => {
            // HH:MM:SS.mmm 格式
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
                // 如果有毫秒部分
                milliseconds = parse_ms_part(dot_parts[1], time_str)?;
            } else if dot_parts.len() == 1 {
                // 如果只有秒部分
                milliseconds = 0;
            } else {
                // 非法格式
                return Err(ConvertError::InvalidTime(format!(
                    "在 '{}' 中秒和毫秒部分格式无效: '{}'",
                    time_str, sec_ms_part
                )));
            }
        }
        2 => {
            // MM:SS.mmm 格式
            hours = 0; // 小时为0
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
                    "在 '{}' 中秒和毫秒部分格式无效: '{}'",
                    time_str, sec_ms_part
                )));
            }
        }
        1 => {
            // SS.mmm 或 SS 格式
            hours = 0;
            minutes = 0; // 小时和分钟都为0
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
                    "在 '{}' 中秒和毫秒部分格式无效: '{}'",
                    time_str, sec_ms_part
                )));
            }
        }
        _ => {
            // 其他非法格式
            return Err(ConvertError::InvalidTime(format!(
                "时间格式 '{}' 无效。",
                time_str
            )));
        }
    }
    // 校验分钟和秒是否超出范围
    if minutes >= 60 {
        return Err(ConvertError::InvalidTime(format!(
            "分钟值 '{}' (应 < 60) 在时间戳 '{}' 中无效",
            minutes, time_str
        )));
    }
    if seconds >= 60 {
        return Err(ConvertError::InvalidTime(format!(
            "秒值 '{}' (应 < 60) 在时间戳 '{}' 中无效",
            seconds, time_str
        )));
    }

    // 计算总毫秒数
    Ok(hours * 3_600_000 + minutes * 60_000 + seconds * 1000 + milliseconds)
}

// 枚举：表示 TTML 中 <span> 标签可能的内容类型
#[derive(Debug, Clone, Copy, PartialEq)]
enum SpanContentType {
    None,                // 未指定或通用内容
    Syllable,            // 逐字歌词音节
    Translation,         // 翻译文本
    Romanization,        // 罗马音文本
    BackgroundContainer, // 背景歌词容器 (通常是 <span ttm:role="x-bg">)
}

// 枚举：表示当前解析的文本应附加到主歌词部分还是背景歌词部分
#[derive(Debug, Clone, Copy, PartialEq)]
enum TextTargetContext {
    Main,       // 目标是主歌词
    Background, // 目标是背景歌词
}

// 枚举：记录上一个结束的 <span> 标签是否是音节，以及是主音节还是背景音节
// 用于处理音节后的空格（如果空格在 <span> 标签之外作为纯文本节点存在）
#[derive(Debug, Clone, Copy, PartialEq)]
enum LastEndedSyllableSpanInfo {
    None,               // 上一个结束的 span 不是音节，或者是第一个音节
    MainSyllable,       // 上一个结束的 span 是主歌词音节
    BackgroundSyllable, // 上一个结束的 span 是背景歌词音节
}

/// 从字符串解析 TTML 内容。
///
/// # Arguments
/// * `ttml_content` - 包含 TTML 数据的字符串。
///
/// # Returns
/// `Result<(Vec<TtmlParagraph>, Vec<AssMetadata>, bool, bool), ConvertError>` -
///   - `Vec<TtmlParagraph>`: 解析出的歌词段落列表。
///   - `Vec<AssMetadata>`: 解析出的元数据列表。
///   - `bool`: 指示 TTML 是否为逐行歌词 (itunes:timing="Line")。
///   - `bool`: 指示 TTML 是否可能被格式化过（例如，标签间有不必要的换行和空格）。
pub fn parse_ttml_from_string(ttml_content: &str) -> ParseTtmlResult {
    let mut reader = Reader::from_str(ttml_content);
    reader.config_mut().trim_text(false);

    // 初始化用于存储解析结果的变量
    let mut paragraphs: Vec<TtmlParagraph> = Vec::new(); // 存储歌词段落
    let mut metadata: Vec<AssMetadata> = Vec::new(); // 存储元数据
    let mut is_line_timing_mode = false; // 标记是否为逐行歌词
    // 标记是否检测到TTML源文件可能经过了格式化（例如，IDE自动格式化引入了标签间的换行和缩进）
    // 这种情况可能导致音节间的空格解析不准确。
    let mut detected_formatted_ttml_or_normalized_text = false;
    let mut first_translation_lang_code: Option<String> = None;

    // 解析状态变量
    let mut in_metadata_section = false; // 是否在 <metadata> 标签内
    let mut in_itunes_metadata = false; // 是否在 <iTunesMetadata> 标签内 (Apple特定元数据)
    let mut in_songwriters_tag = false; // 是否在 <songwriters> 标签内
    let mut in_songwriter_tag = false; // 是否在 <songwriter> 标签内
    let mut current_songwriter_name = String::new(); // 当前解析的作曲者名称
    let mut in_agent_tag = false; // 是否在 <ttm:agent> 标签内
    let mut in_agent_name_tag = false; // 是否在 <ttm:name> 标签内 (agent的名称)
    let mut current_agent_id_for_name: Option<String> = None; // 当前 agent 的 xml:id
    let mut current_agent_name_text = String::new(); // 当前 agent 的名称文本

    // 状态变量，用于逐字歌词 (Word timing)
    let mut current_paragraph_word_mode: Option<TtmlParagraph> = None; // 当前正在构建的 TtmlParagraph
    let mut current_span_text_accumulator = String::new(); // 累积当前 <span> 内的文本
    let mut last_ended_syllable_span_info = LastEndedSyllableSpanInfo::None; // 记录上一个结束的音节span类型
    // Span 栈，用于处理嵌套的 <span> 标签及其类型和时间信息
    // 元组结构: (内容类型, 目标上下文, 可选开始时间ms, 可选结束时间ms, 可选xml:lang属性值)
    type SpanStackItem = (
        SpanContentType,
        TextTargetContext,
        Option<u64>,
        Option<u64>,
        Option<String>,
    );
    let mut span_type_stack_word_mode: Vec<SpanStackItem> = Vec::new();
    let mut current_div_song_part_word_mode: Option<String> = None; // 当前 <div> 的 itunes:song-part 属性值

    // 状态变量，用于逐行歌词 (Line timing)
    let mut current_p_data_for_line_mode: Option<(u64, u64, Option<String>)> = None; // (开始时间ms, 结束时间ms, 可选agent)
    let mut current_p_text_for_line_mode = String::new(); // 当前 <p> 内的主歌词文本
    let mut current_p_translation_line_mode: Option<(String, Option<String>)> = None; // (翻译文本, 可选语言代码)
    let mut current_p_romanization_line_mode: Option<String> = None; // 罗马音文本
    let mut current_div_song_part_line_mode: Option<String> = None; // 当前 <div> 的 itunes:song-part (行模式)
    let mut in_p_element = false; // 标记是否在 <p> 标签内部

    // 主循环，读取 XML 事件
    loop {
        let event = reader.read_event(); // 读取下一个 XML 事件

        match event {
            Ok(Event::Start(e)) => {
                // 处理开始标签 <...>
                last_ended_syllable_span_info = LastEndedSyllableSpanInfo::None; // 重置上一个音节信息
                let full_name_bytes = e.name(); // 标签的完整名称 (包括命名空间前缀)
                let local_name_bytes = e.local_name(); // 标签的本地名称 (不包括命名空间前缀)
                let local_name_str =
                    String::from_utf8_lossy(local_name_bytes.as_ref()).into_owned(); // 转换为字符串

                match local_name_str.as_str() {
                    "tt" => {
                        // TTML根元素 <tt>
                        for attr_res in e.attributes() {
                            // 遍历所有属性
                            let attr = attr_res?;
                            match attr.key.as_ref() {
                                // 检查 Apple iTunes 的计时模式属性
                                b"itunes:timing" => {
                                    if attr.decode_and_unescape_value(reader.decoder())?.as_ref()
                                        == "Line"
                                    {
                                        is_line_timing_mode = true; // 设置为逐行歌词
                                        info!(target: "unilyric::ttml_parser", "[TTML 处理] 检测到逐行TTML (itunes:timing=Line)。");
                                    }
                                }
                                // 检查 xml:lang 属性，作为全局语言代码
                                b"xml:lang" => {
                                    let lang_val = attr
                                        .decode_and_unescape_value(reader.decoder())?
                                        .into_owned();
                                    metadata.push(AssMetadata {
                                        key: "lang".to_string(),
                                        value: lang_val,
                                    });
                                }
                                _ => {} // 其他属性忽略
                            }
                        }
                    }
                    "metadata" if !in_p_element => {
                        // <metadata> 标签 (且不在 <p> 内部)
                        in_metadata_section = true;
                    }
                    "iTunesMetadata" if in_metadata_section => {
                        // <iTunesMetadata> (Apple特定)
                        in_itunes_metadata = true;
                    }
                    "songwriters" if in_itunes_metadata => {
                        // <songwriters>
                        in_songwriters_tag = true;
                    }
                    "songwriter" if in_songwriters_tag => {
                        // <songwriter>
                        in_songwriter_tag = true;
                        current_songwriter_name.clear(); // 清空当前的作曲者名称累加器
                    }
                    // <ttm:agent> 标签
                    "agent" if full_name_bytes.as_ref() == b"ttm:agent" && in_metadata_section => {
                        in_agent_tag = true;
                        current_agent_id_for_name = None; // 重置当前 agent ID
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"xml:id" {
                                current_agent_id_for_name = Some(
                                    attr.decode_and_unescape_value(reader.decoder())?
                                        .into_owned(),
                                );
                                break;
                            }
                        }
                    }
                    // <ttm:name> 标签 (agent的名称)
                    "name" if in_agent_tag && full_name_bytes.as_ref() == b"ttm:name" => {
                        in_agent_name_tag = true;
                        current_agent_name_text.clear(); // 清空当前 agent 名称累加器
                    }
                    // 通用 <meta> 标签 (在 <metadata> 内部)
                    "meta" if in_metadata_section => {
                        let mut key_attr = None;
                        let mut value_attr = None;
                        for attr_res in e.attributes() {
                            // 提取 key 和 value 属性
                            let attr = attr_res?;
                            let attr_key_local_name_owned =
                                String::from_utf8_lossy(attr.key.local_name().as_ref())
                                    .into_owned();
                            let attr_value_str = attr
                                .decode_and_unescape_value(reader.decoder())?
                                .into_owned();
                            match attr_key_local_name_owned.as_str() {
                                "key" => key_attr = Some(attr_value_str),
                                "value" => value_attr = Some(attr_value_str),
                                _ => {}
                            }
                        }
                        if let (Some(k), Some(v)) = (key_attr.filter(|s| !s.is_empty()), value_attr)
                        {
                            metadata.push(AssMetadata { key: k, value: v }); // 添加到元数据列表
                        }
                    }
                    "div" => {
                        // <div> 标签，可能包含 itunes:song-part
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
                        // <p> 标签 (歌词段落)
                        in_p_element = true; // 标记进入 <p> 元素
                        if is_line_timing_mode {
                            // 逐行歌词处理
                            let mut p_start_ms_val = 0;
                            let mut p_end_ms_val = 0;
                            let mut p_agent_opt_val: Option<String> = None;
                            let mut p_song_part_val = current_div_song_part_line_mode.clone(); // 继承自父 <div>
                            for attr_res in e.attributes() {
                                // 解析 <p> 标签的属性
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
                                    } // <p> 上的 song-part 优先
                                    _ => {}
                                }
                            }
                            current_p_data_for_line_mode =
                                Some((p_start_ms_val, p_end_ms_val, p_agent_opt_val));
                            // 如果 <p> 上有 song-part，更新当前 div 的 song-part 状态 (虽然通常 song-part 在 div 上)
                            if e.attributes().any(|a| {
                                a.is_ok_and(|attr| attr.key.as_ref() == b"itunes:song-part")
                            }) {
                                current_div_song_part_line_mode = p_song_part_val;
                            }
                            current_p_text_for_line_mode.clear(); // 清空行文本累加器
                            current_p_translation_line_mode = None;
                            current_p_romanization_line_mode = None;
                        } else {
                            // 逐字歌词处理
                            span_type_stack_word_mode.clear(); // 清空 span 栈
                            let mut p_data = TtmlParagraph {
                                song_part: current_div_song_part_word_mode.clone(),
                                ..Default::default()
                            }; // 创建新的 TtmlParagraph
                            for attr_res in e.attributes() {
                                // 解析 <p> 标签的属性
                                let attr = attr_res?;
                                let value_cow = attr.decode_and_unescape_value(reader.decoder())?;
                                match attr.key.as_ref() {
                                    b"agent" | b"ttm:agent" => {
                                        p_data.agent = value_cow.into_owned()
                                    }
                                    b"begin" => {
                                        p_data.p_start_ms = parse_any_ttml_time_ms(&value_cow)?
                                    }
                                    b"end" => p_data.p_end_ms = parse_any_ttml_time_ms(&value_cow)?,
                                    b"itunes:song-part" => {
                                        p_data.song_part = Some(value_cow.into_owned())
                                    }
                                    _ => {}
                                }
                            }
                            current_paragraph_word_mode = Some(p_data); // 设置当前正在构建的段落
                        }
                    }
                    "span" if in_p_element => {
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
                    _ => {} // 其他开始标签忽略
                }
            }
            Ok(Event::Empty(e)) => {
                // 处理自闭合标签 <.../>
                last_ended_syllable_span_info = LastEndedSyllableSpanInfo::None;
                let local_name_bytes = e.local_name();
                let local_name_str = String::from_utf8_lossy(local_name_bytes.as_ref());

                // 主要处理自闭合的 <meta ... /> 标签
                if local_name_str == "meta" && in_metadata_section {
                    let mut key_attr: Option<String> = None;
                    let mut value_attr: Option<String> = None;
                    for attr_res in e.attributes() {
                        let attr = attr_res?;
                        let attr_key_bytes = attr.key.as_ref();
                        let attr_value_cow = attr.decode_and_unescape_value(reader.decoder())?;
                        if attr_key_bytes == b"key" {
                            key_attr = Some(attr_value_cow.into_owned());
                        } else if attr_key_bytes == b"value" {
                            value_attr = Some(attr_value_cow.into_owned());
                        }
                    }
                    if let (Some(k), Some(v)) = (key_attr.filter(|s| !s.is_empty()), value_attr) {
                        metadata.push(AssMetadata { key: k, value: v });
                    }
                }
            }
            Ok(Event::Text(e_text)) => {
                // 处理文本节点
                // 解码文本内容，处理 XML 转义字符
                let text_cow = match e_text.unescape() {
                    Ok(cow_str) => cow_str,
                    Err(_err) => Cow::Owned(String::from_utf8_lossy(e_text.as_ref()).into_owned()),
                };
                let text_str = text_cow.as_ref();

                if in_p_element {
                    // 如果在 <p> 标签内
                    if is_line_timing_mode {
                        // 逐行歌词处理
                        // 根据 span 栈顶的类型，将文本追加到相应的累加器
                        if let Some((span_type, _context, _b, _e, lang_code_for_current_span_opt)) =
                            span_type_stack_word_mode.last()
                        {
                            match span_type {
                                SpanContentType::Translation => {
                                    current_p_translation_line_mode =
                                        match current_p_translation_line_mode.take() {
                                            Some((mut existing_text, existing_lang_opt)) => {
                                                existing_text.push_str(text_str);
                                                Some((existing_text, existing_lang_opt))
                                            }
                                            None => Some((
                                                text_str.to_string(),
                                                lang_code_for_current_span_opt.clone(),
                                            )),
                                        };
                                }
                                SpanContentType::Romanization => {
                                    if let Some(ref mut text) = current_p_romanization_line_mode {
                                        text.push_str(text_str);
                                    } else {
                                        current_p_romanization_line_mode =
                                            Some(text_str.to_string());
                                    }
                                }
                                _ => {
                                    // 其他情况，追加到主歌词文本
                                    current_p_text_for_line_mode.push_str(text_str);
                                }
                            }
                        } else {
                            // 如果 span 栈为空 (不太可能在行模式的 <p> 内直接有文本，但作为保险)
                            current_p_text_for_line_mode.push_str(text_str);
                        }
                    } else {
                        // 逐字歌词处理
                        // 检查是否是音节后的空格文本
                        if last_ended_syllable_span_info != LastEndedSyllableSpanInfo::None {
                            // 如果文本包含换行符，或者不仅仅是单个空格，则可能意味着TTML源文件被格式化过
                            if !detected_formatted_ttml_or_normalized_text
                                && text_str.contains('\n')
                            {
                                detected_formatted_ttml_or_normalized_text = true;
                            }
                            if !detected_formatted_ttml_or_normalized_text && text_str == " " {
                                // 精确匹配单个空格
                                if let Some(para) = current_paragraph_word_mode.as_mut() {
                                    match last_ended_syllable_span_info {
                                        // 为上一个音节设置 ends_with_space
                                        LastEndedSyllableSpanInfo::MainSyllable => {
                                            if let Some(last_syl) = para.main_syllables.last_mut() {
                                                last_syl.ends_with_space = true;
                                            }
                                        }
                                        LastEndedSyllableSpanInfo::BackgroundSyllable => {
                                            if let Some(bg_sec) = para.background_section.as_mut() {
                                                if let Some(last_syl) = bg_sec.syllables.last_mut()
                                                {
                                                    last_syl.ends_with_space = true;
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            last_ended_syllable_span_info = LastEndedSyllableSpanInfo::None; // 重置
                        } else {
                            // 否则，文本属于当前活动的 <span>
                            if !span_type_stack_word_mode.is_empty() {
                                current_span_text_accumulator.push_str(text_str);
                            }
                        }
                    }
                } else if in_songwriter_tag {
                    current_songwriter_name.push_str(text_str);
                }
                // 作曲者名称
                else if in_agent_name_tag {
                    current_agent_name_text.push_str(text_str);
                } // Agent 名称
            }
            Ok(Event::End(e)) => {
                // 处理结束标签 </...>
                let local_name_str = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                match local_name_str.as_str() {
                    "div" => {
                        // 结束 </div>
                        if is_line_timing_mode {
                            current_div_song_part_line_mode = None;
                        } else {
                            current_div_song_part_word_mode = None;
                        }
                    }
                    "p" => {
                        // 结束 </p>
                        if is_line_timing_mode {
                            // 行模式下，处理累积的行数据
                            if let Some((p_start, p_end, p_agent_opt)) =
                                current_p_data_for_line_mode.take()
                            {
                                let main_text = current_p_text_for_line_mode.trim().to_string();
                                // 只有当主文本、翻译、罗马音之一非空，或段落有明确时长时，才创建段落
                                if !main_text.is_empty()
                                    || current_p_translation_line_mode.is_some()
                                    || current_p_romanization_line_mode.is_some()
                                    || (p_end > p_start)
                                {
                                    paragraphs.push(TtmlParagraph {
                                        p_start_ms: p_start,
                                        p_end_ms: p_end,
                                        agent: p_agent_opt.unwrap_or_else(|| "v1".to_string()), // 默认 agent
                                        // 行模式下，主歌词作为一个整体音节
                                        main_syllables: vec![TtmlSyllable {
                                            text: main_text,
                                            start_ms: p_start,
                                            end_ms: p_end,
                                            ends_with_space: false,
                                        }],
                                        song_part: current_div_song_part_line_mode.clone(), // 使用 div 的 song-part
                                        translation: current_p_translation_line_mode.take(),
                                        romanization: current_p_romanization_line_mode.take(),
                                        ..Default::default()
                                    });
                                }
                            }
                            current_p_text_for_line_mode.clear(); // 清理累加器
                            current_p_translation_line_mode = None;
                            current_p_romanization_line_mode = None;
                        } else {
                            // 逐字模式下，将当前构建的段落添加到列表
                            if let Some(para) = current_paragraph_word_mode.take() {
                                // 只有当段落包含有效内容或有明确时长时才添加
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
                            span_type_stack_word_mode.clear(); // 清空 span 栈
                        }
                        in_p_element = false;
                        last_ended_syllable_span_info = LastEndedSyllableSpanInfo::None; // 重置状态
                    }
                    "span" if in_p_element => {
                        // 结束 </span>
                        if let Some((span_type, context, begin_ms_opt, end_ms_opt, lang_attr_opt)) =
                            span_type_stack_word_mode.pop()
                        {
                            let raw_accumulated_text = current_span_text_accumulator.clone();
                            current_span_text_accumulator.clear();

                            if is_line_timing_mode {
                                // 行模式下，文本和语言已在 Event::Text 中处理，这里仅维护栈状态
                            } else {
                                // 逐字模式
                                if let Some(para) = current_paragraph_word_mode.as_mut() {
                                    match span_type {
                                        SpanContentType::Syllable => {
                                            // 处理音节
                                            if let (Some(start_ms), Some(end_ms)) =
                                                (begin_ms_opt, end_ms_opt)
                                            {
                                                let syllable = TtmlSyllable {
                                                    text: raw_accumulated_text,
                                                    start_ms,
                                                    end_ms,
                                                    ends_with_space: false,
                                                };
                                                match context {
                                                    // 根据上下文添加到主音节或背景音节
                                                    TextTargetContext::Main => {
                                                        if !syllable.text.is_empty()
                                                            || (syllable.end_ms > syllable.start_ms)
                                                        {
                                                            para.main_syllables.push(syllable);
                                                            last_ended_syllable_span_info = LastEndedSyllableSpanInfo::MainSyllable;
                                                        } else {
                                                            last_ended_syllable_span_info =
                                                                LastEndedSyllableSpanInfo::None;
                                                        }
                                                    }
                                                    TextTargetContext::Background => {
                                                        if !syllable.text.is_empty()
                                                            || (syllable.end_ms > syllable.start_ms)
                                                        {
                                                            let bg_sec = para
                                                                .background_section
                                                                .get_or_insert_with(
                                                                    Default::default,
                                                                );
                                                            if bg_sec.syllables.is_empty()
                                                                && bg_sec.start_ms == 0
                                                                && bg_sec.end_ms == 0
                                                            {
                                                                if let Some(container_span_info) = span_type_stack_word_mode.iter().find(|(st,_,_,_,_)| *st == SpanContentType::BackgroundContainer) { bg_sec.start_ms = container_span_info.2.unwrap_or(para.p_start_ms); bg_sec.end_ms = container_span_info.3.unwrap_or(para.p_end_ms); }
                                                            }
                                                            bg_sec.syllables.push(syllable);
                                                            last_ended_syllable_span_info = LastEndedSyllableSpanInfo::BackgroundSyllable;
                                                        } else {
                                                            last_ended_syllable_span_info =
                                                                LastEndedSyllableSpanInfo::None;
                                                        }
                                                    }
                                                }
                                            } else {
                                                last_ended_syllable_span_info =
                                                    LastEndedSyllableSpanInfo::None;
                                            }
                                        }
                                        SpanContentType::Translation
                                        | SpanContentType::Romanization => {
                                            // 处理翻译或罗马音
                                            // 对累积的文本进行空白符规范化处理
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
                                                if span_type == SpanContentType::Translation {
                                                    match context {
                                                        TextTargetContext::Main => {
                                                            para.translation = Some((
                                                                normalized_text,
                                                                lang_attr_opt.clone(),
                                                            ))
                                                        }
                                                        TextTargetContext::Background => {
                                                            if let Some(bg_sec) =
                                                                para.background_section.as_mut()
                                                            {
                                                                bg_sec.translation = Some((
                                                                    normalized_text,
                                                                    lang_attr_opt.clone(),
                                                                ));
                                                            }
                                                        }
                                                    }
                                                } else {
                                                    // Romanization
                                                    match context {
                                                        TextTargetContext::Main => {
                                                            para.romanization =
                                                                Some(normalized_text)
                                                        }
                                                        TextTargetContext::Background => {
                                                            if let Some(bg_sec) =
                                                                para.background_section.as_mut()
                                                            {
                                                                bg_sec.romanization =
                                                                    Some(normalized_text);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            last_ended_syllable_span_info =
                                                LastEndedSyllableSpanInfo::None;
                                        }
                                        _ => {
                                            last_ended_syllable_span_info =
                                                LastEndedSyllableSpanInfo::None;
                                        } // 其他 span 类型
                                    }
                                }
                            }
                        } else {
                            last_ended_syllable_span_info = LastEndedSyllableSpanInfo::None;
                        } // span 栈为空
                    }
                    // 处理元数据相关的结束标签
                    "metadata" if in_metadata_section => {
                        in_metadata_section = false;
                    }
                    "songwriter" if in_songwriter_tag => {
                        if !current_songwriter_name.trim().is_empty() {
                            metadata.push(AssMetadata {
                                key: "songwriter".to_string(),
                                value: current_songwriter_name.trim().to_string(),
                            });
                        }
                        current_songwriter_name.clear();
                        in_songwriter_tag = false;
                    }
                    "songwriters" if in_songwriters_tag => {
                        in_songwriters_tag = false;
                    }
                    "iTunesMetadata" if in_itunes_metadata => {
                        in_itunes_metadata = false;
                    }
                    "name" if in_agent_name_tag && e.name().as_ref() == b"ttm:name" => {
                        if let Some(agent_id) = &current_agent_id_for_name {
                            if !current_agent_name_text.trim().is_empty() {
                                metadata.push(AssMetadata {
                                    key: agent_id.clone(),
                                    value: current_agent_name_text.trim().to_string(),
                                });
                            }
                        }
                        current_agent_name_text.clear();
                        in_agent_name_tag = false;
                    }
                    "agent" if in_agent_tag && e.name().as_ref() == b"ttm:agent" => {
                        in_agent_tag = false;
                        current_agent_id_for_name = None;
                    }
                    _ => {} // 其他结束标签
                }
            }
            Ok(Event::Eof) => break, // 文件结束
            Err(quick_xml_error) => {
                // XML 解析错误
                let error_msg = format!(
                    "[TTML 处理] XML解析错误于位置 {}: {}",
                    reader.buffer_position(),
                    quick_xml_error
                );
                error!(target: "unilyric::ttml_parser", "{}", error_msg);
                return Err(ConvertError::Xml(quick_xml_error));
            }
            _ => {} // 其他事件类型忽略
        }
    }
    info!(target: "unilyric::ttml_parser", "[TTML 处理] 解析完成. 共 {} 段落, {} 条元数据. 行模式: {}, 格式化: {}", paragraphs.len(), metadata.len(), is_line_timing_mode, detected_formatted_ttml_or_normalized_text);
    Ok((
        paragraphs,
        metadata,
        is_line_timing_mode,
        detected_formatted_ttml_or_normalized_text,
        first_translation_lang_code,
    ))
}

/// 规范化字符串中的空白（多个连续空白变单个空格，移除首尾空白）并检查是否发生改变。
fn normalize_whitespace_and_check_changes(text: &str) -> (String, bool) {
    let mut normalized = String::with_capacity(text.len());
    let mut last_char_was_whitespace = false;
    let mut effective_char_count = 0; // 用于跟踪非空白字符，以判断是否在单词间添加空格
    let original_trimmed_len = text.trim().len(); // 原始文本去除首尾空格后的长度

    for c in text.chars() {
        if c.is_whitespace() {
            last_char_was_whitespace = true; // 标记遇到空白符
        } else {
            // 如果上一个是空白符，并且当前已处理过非空白字符（避免在字符串开头加空格）
            if last_char_was_whitespace && effective_char_count > 0 {
                normalized.push(' '); // 添加一个空格作为分隔
            }
            normalized.push(c); // 添加当前非空白字符
            last_char_was_whitespace = false; // 重置空白标记
            effective_char_count += 1;
        }
    }

    let final_normalized = normalized.trim().to_string(); // 最后再 trim 一次，确保结果的首尾无空格
    // 比较原始文本（去除首尾空格后）与规范化后的文本，以及它们的长度，来判断是否发生了改变
    let changed = text.trim() != final_normalized
        || text.len() != original_trimmed_len
        || final_normalized.len() != original_trimmed_len;
    (final_normalized, changed)
}
