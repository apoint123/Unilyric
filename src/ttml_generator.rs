use crate::metadata_processor::MetadataStore;
use crate::types::{
    ActorRole, AssLineContent, AssLineInfo, CanonicalMetadataKey, ConvertError, ProcessedAssData,
    TtmlParagraph,
};
use crate::utils::clean_parentheses_from_bg_text;
use quick_xml::escape::escape;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::writer::Writer;
use std::collections::{HashMap, HashSet};
use std::io::{self, Cursor, Error as IoError};

// --- 辅助函数区 ---

/// 辅助函数: 将 quick_xml 库的错误转换为标准的 IO 错误。
/// 这在 quick_xml 的写入器（Writer）的闭包中非常有用，因为这些闭包要求返回 `io::Result`。
fn map_xml_error_to_io(e: quick_xml::Error) -> IoError {
    IoError::other(e)
}

/// 辅助函数: 格式化毫秒时间为 TTML 标准的时间字符串。
///
/// # 示例
/// - `3661001` -> `"1:01:01.001"`
/// - `61001`   -> `"1:01.001"`
/// - `1001`    -> `"1.001"`
fn format_ttml_time(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1000;
    let fr = ms % 1000;

    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{fr:03}")
    } else if m > 0 {
        format!("{m}:{s:02}.{fr:03}")
    } else {
        format!("{s}.{fr:03}")
    }
}

/// 辅助函数，用于写入带角色和可选语言的 span 标签。
///
/// # 参数
/// * `writer` - XML 写入器。
/// * `role` - `ttm:role` 属性的值 (例如, "x-translation", "x-roman")。
/// * `lang` - 可选的 `xml:lang` 属性值。
/// * `text` - 要写入的文本内容。
fn write_optional_span(
    writer: &mut Writer<Cursor<&mut Vec<u8>>>,
    role: &str,
    lang: Option<&str>,
    text: &str,
) -> io::Result<()> {
    // 只有当文本内容非空时才写入 <span> 标签
    if !text.is_empty() {
        let mut span_builder = writer
            .create_element("span")
            .with_attribute(("ttm:role", role));

        // 如果提供了语言代码，则添加 xml:lang 属性
        if let Some(lang_code) = lang
            && !lang_code.is_empty()
        {
            span_builder = span_builder.with_attribute(("xml:lang", lang_code));
        }

        span_builder
            .write_text_content(BytesText::from_escaped(escape(text).as_ref()))
            .map_err(std::io::Error::other)?;
    }

    Ok(())
}

// --- 从 ASS 生成 TTML 的相关函数 ---

/// 内部辅助函数：根据收集到的 ASS 行信息写入一个 TTML `<p>` 元素及其内容。
/// 此函数处理主歌词、关联的背景歌词（包括其翻译和罗马音）、以及主翻译和主罗马音。
fn write_ttml_p_from_ass_lines(
    writer: &mut Writer<Cursor<&mut Vec<u8>>>,    // XML写入器
    key_counter: &mut usize,                      // 用于生成唯一 itunes:key 的计数器
    main_line_ass: &AssLineInfo,                  // 当前处理的“主”ASS行（必须是主唱或合唱）
    associated_bg_lines: &[&AssLineInfo],         // 明确关联到 main_line_ass 的背景行
    associated_main_trans_lines: &[&AssLineInfo], // 关联到 main_line_ass 的主翻译行
    associated_main_roma_lines: &[&AssLineInfo],  // 关联到 main_line_ass 的主罗马音行
) -> io::Result<()> {
    if let Some(AssLineContent::LyricLine {
        role: main_role,
        syllables: main_syllables,
        ..
    }) = &main_line_ass.content
    {
        // 确保传递给此函数作为 main_line_ass 的行不是背景角色。
        // 独立的背景行应该在 write_ttml_div_from_ass_lines 中被过滤掉或特殊处理。
        if main_role == &ActorRole::Background {
            // 背景行不应独立形成 <p>，除非它是孤立的且有特殊处理逻辑
            return Ok(());
        }

        // 判断是否存在任何需要渲染的内容。
        let has_any_content = !main_syllables.is_empty()
            || associated_bg_lines.iter().any(|line| {
                matches!(&line.content, Some(AssLineContent::LyricLine { syllables, bg_translation, bg_romanization, .. }) if !syllables.is_empty() || bg_translation.is_some() || bg_romanization.is_some())
            })
            || !associated_main_trans_lines.is_empty()
            || !associated_main_roma_lines.is_empty();

        if !has_any_content {
            return Ok(()); // 如果没有任何内容，则不生成此段落
        }

        *key_counter += 1;
        let key_str = format!("L{}", *key_counter);
        // 根据角色映射 agent 字符串
        let agent_str = match main_role {
            ActorRole::Vocal1 => "v1",
            ActorRole::Vocal2 => "v2",
            ActorRole::Chorus => "v1000",
            ActorRole::Background => unreachable!(), // 已在上一个if中处理
        };

        // 计算 <p> 标签的实际开始和结束时间
        let p_begin_str = format_ttml_time(main_line_ass.start_ms);
        let p_actual_end_ms = associated_bg_lines
            .iter()
            .fold(main_line_ass.end_ms, |max_end, line| {
                max_end.max(line.end_ms)
            });
        let p_end_str = format_ttml_time(p_actual_end_ms);

        let mut p_attributes = vec![
            ("begin", p_begin_str.as_str()),
            ("end", p_end_str.as_str()),
            ("itunes:key", key_str.as_str()),
        ];
        if !agent_str.is_empty() && agent_str != "v0" {
            p_attributes.push(("ttm:agent", agent_str));
        }

        writer
            .create_element("p")
            .with_attributes(p_attributes)
            .write_inner_content(|p_content_writer| -> io::Result<()> {
                // 写入主歌词音节
                for syl in main_syllables {
                    p_content_writer
                        .create_element("span")
                        .with_attribute(("begin", format_ttml_time(syl.start_ms).as_str()))
                        .with_attribute(("end", format_ttml_time(syl.end_ms).as_str()))
                        .write_text_content(BytesText::from_escaped(escape(&syl.text).as_ref()))?;
                    if syl.ends_with_space {
                        p_content_writer.write_event(Event::Text(BytesText::from_escaped(" ")))?;
                    }
                }

                // 写入关联的背景声部内容
                for bg_line_data in associated_bg_lines {
                    if let Some(AssLineContent::LyricLine {
                        role: current_bg_role,
                        syllables: bg_syllables,
                        bg_translation,
                        bg_romanization,
                    }) = &bg_line_data.content
                    {
                        if current_bg_role != &ActorRole::Background {
                            continue;
                        }

                        let has_syls = !bg_syllables.is_empty();
                        let has_trans = bg_translation
                            .as_ref()
                            .is_some_and(|(_, text)| !text.is_empty());
                        let has_roma = bg_romanization
                            .as_ref()
                            .is_some_and(|text| !text.is_empty());

                        if !has_syls && !has_trans && !has_roma {
                            continue;
                        }

                        p_content_writer
                            .create_element("span")
                            .with_attribute(("ttm:role", "x-bg"))
                            .with_attribute((
                                "begin",
                                format_ttml_time(bg_line_data.start_ms).as_str(),
                            ))
                            .with_attribute(("end", format_ttml_time(bg_line_data.end_ms).as_str()))
                            .write_inner_content(|bg_content_writer| -> io::Result<()> {
                                if !bg_syllables.is_empty() {
                                    let num_s = bg_syllables.len();
                                    for (idx, syl) in bg_syllables.iter().enumerate() {
                                        let mut text_bg = clean_parentheses_from_bg_text(&syl.text);
                                        if !text_bg.is_empty() {
                                            if num_s == 1 {
                                                text_bg = format!("({text_bg})");
                                            } else if idx == 0 {
                                                text_bg = format!("({text_bg}");
                                            } else if idx == num_s - 1 {
                                                text_bg = format!("{text_bg})");
                                            }
                                        }
                                        bg_content_writer
                                            .create_element("span")
                                            .with_attribute((
                                                "begin",
                                                format_ttml_time(syl.start_ms).as_str(),
                                            ))
                                            .with_attribute((
                                                "end",
                                                format_ttml_time(syl.end_ms).as_str(),
                                            ))
                                            .write_text_content(BytesText::from_escaped(
                                                escape(&text_bg).as_ref(),
                                            ))?;
                                        if syl.ends_with_space {
                                            bg_content_writer.write_event(Event::Text(
                                                BytesText::from_escaped(" "),
                                            ))?;
                                        }
                                    }
                                }
                                if let Some((lang_opt, text_val)) = bg_translation
                                    && !text_val.is_empty()
                                {
                                    let mut trans_span = bg_content_writer
                                        .create_element("span")
                                        .with_attribute(("ttm:role", "x-translation"));
                                    if let Some(lang) = lang_opt
                                        && !lang.is_empty()
                                    {
                                        trans_span =
                                            trans_span.with_attribute(("xml:lang", lang.as_str()));
                                    }
                                    trans_span.write_text_content(BytesText::from_escaped(
                                        escape(text_val).as_ref(),
                                    ))?;
                                }
                                if let Some(roma_text) = bg_romanization
                                    && !roma_text.is_empty()
                                {
                                    bg_content_writer
                                        .create_element("span")
                                        .with_attribute(("ttm:role", "x-roman"))
                                        .write_text_content(BytesText::from_escaped(
                                            escape(roma_text).as_ref(),
                                        ))?;
                                }
                                Ok(())
                            })?;
                    }
                }

                // 写入主翻译和主罗马音
                for trans_line in associated_main_trans_lines {
                    if let Some(AssLineContent::MainTranslation { lang_code, text }) =
                        &trans_line.content
                    {
                        write_optional_span(
                            p_content_writer,
                            "x-translation",
                            lang_code.as_deref(),
                            text,
                        )?;
                    }
                }
                for roma_line in associated_main_roma_lines {
                    if let Some(AssLineContent::MainRomanization { text }) = &roma_line.content {
                        write_optional_span(p_content_writer, "x-roman", None, text)?;
                    }
                }
                Ok(())
            })?;
    }
    Ok(())
}

/// 辅助函数: 将一组 ASS 行写入 TTML 的一个 `<div>` 块中。
/// 内部会调用 `write_ttml_p_from_ass_lines` 来处理每个 `<p>`。
fn write_ttml_div_from_ass_lines(
    writer: &mut Writer<Cursor<&mut Vec<u8>>>,
    lines_in_div: &[&AssLineInfo],
    song_part: &Option<String>,
    line_key_counter: &mut usize,
) -> Result<(), quick_xml::Error> {
    if lines_in_div.is_empty() {
        return Ok(());
    }

    // 计算 div 的开始和结束时间
    let div_begin_ms = lines_in_div.iter().map(|l| l.start_ms).min().unwrap_or(0);
    let div_end_ms = lines_in_div.iter().map(|l| l.end_ms).max().unwrap_or(0);

    let div_begin_str = format_ttml_time(div_begin_ms);
    let div_end_str = format_ttml_time(div_end_ms);

    let mut div_attributes = vec![
        ("begin", div_begin_str.as_str()),
        ("end", div_end_str.as_str()),
    ];
    if let Some(part) = song_part
        && !part.is_empty()
    {
        div_attributes.push(("itunes:song-part", part.as_str()));
    }

    writer
        .create_element("div")
        .with_attributes(div_attributes)
        .write_inner_content(|div_content_writer| -> io::Result<()> {
            let mut current_p_main_line: Option<&AssLineInfo> = None;
            let mut associated_bg_lines: Vec<&AssLineInfo> = Vec::new();
            let mut associated_trans_lines: Vec<&AssLineInfo> = Vec::new();
            let mut associated_roma_lines: Vec<&AssLineInfo> = Vec::new();

            for current_line in lines_in_div.iter() {
                match &current_line.content {
                    Some(AssLineContent::LyricLine { role, .. }) => {
                        match role {
                            ActorRole::Vocal1 | ActorRole::Vocal2 | ActorRole::Chorus => {
                                // 遇到主唱或合唱行：结束上一个 <p>（如果存在）
                                if let Some(main_line_to_write) = current_p_main_line {
                                    write_ttml_p_from_ass_lines(
                                        div_content_writer,
                                        line_key_counter,
                                        main_line_to_write,
                                        &associated_bg_lines,
                                        &associated_trans_lines,
                                        &associated_roma_lines,
                                    )?;
                                }
                                // 开始新的 <p>，以此行为主行
                                current_p_main_line = Some(current_line);
                                associated_bg_lines.clear();
                                associated_trans_lines.clear();
                                associated_roma_lines.clear();
                            }
                            ActorRole::Background => {
                                // 遇到背景行：如果当前有正在构建的主唱 <p>，则将其关联
                                if let Some(main_line_being_built) = current_p_main_line {
                                    // 确保不将背景行关联到另一个已经是背景的“主行”上
                                    if let Some(AssLineContent::LyricLine {
                                        role: main_p_role,
                                        ..
                                    }) = &main_line_being_built.content
                                    {
                                        if main_p_role != &ActorRole::Background {
                                            associated_bg_lines.push(current_line);
                                        } else {
                                            // 如果 current_p_main_line 本身就是背景行，这意味着前一个独立的背景 <p> 应该结束了。
                                            // 然后这个新的背景行将开始自己的（独立的）<p>。
                                            write_ttml_p_from_ass_lines(
                                                div_content_writer,
                                                line_key_counter,
                                                main_line_being_built,
                                                &associated_bg_lines, // 此时应为空
                                                &associated_trans_lines, // 此时应为空
                                                &associated_roma_lines, // 此时应为空
                                            )?;
                                            current_p_main_line = Some(current_line); // 新背景行成为新的“主行”
                                            associated_bg_lines.clear(); // 清空，因为它自己是这个新<p>的主体（背景内容）
                                            associated_trans_lines.clear();
                                            associated_roma_lines.clear();
                                        }
                                    } else {
                                        // current_p_main_line 不是 LyricLine (不太可能)
                                        associated_bg_lines.push(current_line); // 尝试关联
                                    }
                                } else {
                                    // 没有主唱行可以依附，此背景行将成为新的“主行”并开始自己的<p>
                                    // （这会导致背景行独立成段，如果ASS文件这样的话）
                                    current_p_main_line = Some(current_line);
                                    associated_bg_lines.clear();
                                    associated_trans_lines.clear();
                                    associated_roma_lines.clear();
                                }
                            }
                        }
                    }
                    Some(AssLineContent::MainTranslation { .. }) => {
                        if current_p_main_line.is_some() {
                            associated_trans_lines.push(current_line);
                        }
                        // 孤立的翻译行将被忽略，因为它们没有主歌词行可以附加
                    }
                    Some(AssLineContent::MainRomanization { .. }) => {
                        if current_p_main_line.is_some() {
                            associated_roma_lines.push(current_line);
                        }
                        // 孤立的罗马音行将被忽略
                    }
                    None => {} // 无内容行，已被 relevant_ass_lines 过滤
                }
            }

            // 处理循环结束后最后一个累积的 <p>
            if let Some(main_line_to_write) = current_p_main_line {
                write_ttml_p_from_ass_lines(
                    div_content_writer,
                    line_key_counter,
                    main_line_to_write,
                    &associated_bg_lines,
                    &associated_trans_lines,
                    &associated_roma_lines,
                )?;
            }
            Ok(())
        })?;
    Ok(())
}

/// 从 ProcessedAssData 生成中间 TTML 字符串。
/// 此函数主要用于当源文件是 ASS 格式时，先将其转换为一种“标准化的”TTML，
/// 然后再从这种 TTML 生成其他目标格式，或者直接输出这种 TTML。
///
/// # Arguments
/// * `ass_data` - 包含从 ASS 文件解析出的所有信息的 ProcessedAssData 结构。
/// * `include_general_metadata_as_amll` - 是否将 ASS 中的一般元数据（如 Title, Original Script）
///   也转换为 TTML head 中的 amll:meta 标签。
///
/// # Returns
/// `Result<String, ConvertError>` - 成功时返回生成的 TTML 字符串，失败时返回错误。
pub fn generate_intermediate_ttml_from_ass(
    ass_data: &ProcessedAssData,
    include_general_metadata_as_amll: bool,
) -> Result<String, ConvertError> {
    let mut buffer = Vec::new();
    let mut writer = Writer::new(Cursor::new(&mut buffer));

    // 临时元数据存储，用于收集和格式化将写入 <head> 的信息
    let mut temp_store = MetadataStore::new();

    if let Some(lang) = &ass_data.language_code {
        temp_store.add("language", lang.clone()).ok();
    }
    for sw in &ass_data.songwriters {
        temp_store.add("songwriter", sw.clone()).ok();
    }
    let mut ass_agent_ids = HashSet::<String>::new();
    for agent_id in ass_data.agent_names.keys() {
        if (agent_id.starts_with('v')
            && agent_id.len() > 1
            && agent_id[1..].chars().all(char::is_numeric))
            || agent_id == "v1"
            || agent_id == "v2"
            || agent_id == "v1000"
        {
            ass_agent_ids.insert(agent_id.clone());
        }
    }

    for (agent_id, agent_name) in &ass_data.agent_names {
        temp_store.add(agent_id, agent_name.clone()).ok();
    }
    if !ass_data.apple_music_id.is_empty() {
        temp_store
            .add("appleMusicId", ass_data.apple_music_id.clone())
            .ok();
    }

    if include_general_metadata_as_amll {
        for meta_item in &ass_data.metadata {
            let key_lower = meta_item.key.to_lowercase();
            let is_agent_key = matches!(key_lower.as_str(), "v1" | "v2" | "v1000");
            let is_handled_specifically = key_lower == "lang"
                || key_lower == "language"
                || key_lower == "songwriter"
                || key_lower == "songwriters"
                || key_lower == "applemusicid"
                || is_agent_key;

            if !is_handled_specifically {
                temp_store.add(&meta_item.key, meta_item.value.clone()).ok();
            }
        }
    }

    temp_store.deduplicate_values();

    let mut tt_attributes_map: HashMap<&str, String> = HashMap::new();
    tt_attributes_map.insert("xmlns", "http://www.w3.org/ns/ttml".to_string());
    tt_attributes_map.insert(
        "xmlns:itunes",
        "http://music.apple.com/lyric-ttml-internal".to_string(),
    );
    tt_attributes_map.insert(
        "xmlns:ttm",
        "http://www.w3.org/ns/ttml#metadata".to_string(),
    );
    tt_attributes_map.insert("itunes:timing", "Word".to_string());

    if let Some(lang_code_val) = temp_store.get_single_value(&CanonicalMetadataKey::Language)
        && !lang_code_val.is_empty()
    {
        tt_attributes_map.insert("xml:lang", lang_code_val.clone());
    }
    let amll_keys = [
        CanonicalMetadataKey::Album,
        CanonicalMetadataKey::AppleMusicId,
        CanonicalMetadataKey::Artist,
        CanonicalMetadataKey::Custom("isrc".to_string()),
        CanonicalMetadataKey::Title,
        CanonicalMetadataKey::Custom("ncmMusicId".to_string()),
        CanonicalMetadataKey::Custom("qqMusicId".to_string()),
        CanonicalMetadataKey::Custom("spotifyId".to_string()),
        CanonicalMetadataKey::Custom("ttmlAuthorGithub".to_string()),
        CanonicalMetadataKey::Author,
    ];
    if amll_keys.iter().any(|key| {
        temp_store
            .get_multiple_values(key)
            .is_some_and(|vals| vals.iter().any(|v| !v.is_empty()))
    }) {
        tt_attributes_map.insert("xmlns:amll", "http://www.example.com/ns/amll".to_string());
    }

    // 属性排序，确保输出一致性
    let mut sorted_tt_attributes: Vec<(&str, String)> = tt_attributes_map.into_iter().collect();
    sorted_tt_attributes.sort_by_key(|&(key, _)| key);

    // 写入 <tt> 开始标签
    let mut tt_start_event = BytesStart::new("tt");
    for (key, value) in &sorted_tt_attributes {
        tt_start_event.push_attribute((*key, value.as_str()));
    }
    writer.write_event(Event::Start(tt_start_event))?;

    MetadataStore::write_ttml_head_metadata(&temp_store, &mut writer, &ass_agent_ids)?;

    let mut min_body_start_ms = u64::MAX;
    let mut max_body_end_ms = 0;
    let mut body_has_any_content = false;

    let relevant_ass_lines: Vec<&AssLineInfo> = ass_data
        .lines
        .iter()
        .filter(|line| {
            line.content.as_ref().is_some_and(|c| match c {
                AssLineContent::LyricLine {
                    syllables,
                    bg_translation,
                    bg_romanization,
                    ..
                } => {
                    !syllables.is_empty()
                        || bg_translation
                            .as_ref()
                            .is_some_and(|(_, text_val)| !text_val.is_empty())
                        || bg_romanization
                            .as_ref()
                            .is_some_and(|text_val| !text_val.is_empty())
                }
                AssLineContent::MainTranslation { text, .. } => !text.is_empty(),
                AssLineContent::MainRomanization { text, .. } => !text.is_empty(),
            })
        })
        .collect();

    if !relevant_ass_lines.is_empty() {
        body_has_any_content = true;
        for line in &relevant_ass_lines {
            min_body_start_ms = min_body_start_ms.min(line.start_ms);
            max_body_end_ms = max_body_end_ms.max(line.end_ms);
        }
    }
    let body_dur_str = if body_has_any_content {
        format_ttml_time(max_body_end_ms)
    } else {
        "0s".to_string()
    };

    writer
        .create_element("body")
        .with_attribute(("dur", body_dur_str.as_str()))
        .write_inner_content(|body_writer| -> io::Result<()> {
            if body_has_any_content {
                let mut line_key_counter = 0;
                let mut current_div_lines_buffer: Vec<&AssLineInfo> = Vec::new();
                let mut current_div_song_part: Option<String> = None;

                for (idx, ass_line_info) in relevant_ass_lines.iter().enumerate() {
                    let line_song_part = &ass_line_info.song_part;
                    let is_primary_lyric_line_for_div_decision = matches!(
                        &ass_line_info.content,
                        Some(AssLineContent::LyricLine { .. })
                    ) && line_song_part.is_some();

                    let mut start_new_div = false;

                    if idx == 0 {
                        current_div_song_part = line_song_part.clone();
                    } else if is_primary_lyric_line_for_div_decision
                        && (current_div_song_part.is_none()
                            || line_song_part != &current_div_song_part)
                    {
                        start_new_div = true;
                    }

                    if start_new_div {
                        if !current_div_lines_buffer.is_empty() {
                            write_ttml_div_from_ass_lines(
                                body_writer,
                                &current_div_lines_buffer,
                                &current_div_song_part,
                                &mut line_key_counter,
                            )
                            .map_err(map_xml_error_to_io)?;
                        }
                        current_div_lines_buffer.clear();
                        current_div_song_part = line_song_part.clone();
                    }
                    current_div_lines_buffer.push(ass_line_info);
                }

                if !current_div_lines_buffer.is_empty() {
                    write_ttml_div_from_ass_lines(
                        body_writer,
                        &current_div_lines_buffer,
                        &current_div_song_part,
                        &mut line_key_counter,
                    )
                    .map_err(map_xml_error_to_io)?;
                }
            }
            Ok(())
        })?;

    writer.write_event(Event::End(BytesEnd::new("tt")))?; // 结束 <tt> 标签
    // 将字节缓冲区转换为 UTF-8 字符串
    String::from_utf8(buffer)
        .map_err(|e| ConvertError::Internal(format!("TTML 缓冲区转 UTF-8 失败: {e}")))
}

// --- 从 TtmlParagraph (内部标准格式) 生成 TTML 的相关函数 ---

/// 从 TtmlParagraph 列表生成最终的 TTML 输出字符串。
/// 这是项目中最通用的 TTML 生成函数，用于从内部标准格式生成输出。
///
/// # Arguments
/// * `paragraphs` - 包含 TtmlParagraph 结构体的切片。
/// * `metadata_store` - 包含元数据的 MetadataStore。
/// * `output_timing_mode` - 输出 TTML 的计时模式 ("Word" 或 "Line")。
/// * `_source_was_formatted_ttml` - (当前未使用) 指示源 TTML 是否被格式化过。
/// * `is_for_lyricify_json` - 如果为true，则不包含翻译和罗马音。
///
/// # Returns
/// `Result<String, ConvertError>` - 成功时返回生成的 TTML 字符串，失败时返回错误。
pub fn generate_ttml_from_paragraphs(
    paragraphs: &[TtmlParagraph],
    metadata_store: &MetadataStore,
    output_timing_mode: &str,
    _source_was_formatted_ttml: Option<bool>, // 参数保留，但当前未使用
    is_for_lyricify_json: bool,
) -> Result<String, ConvertError> {
    let mut buffer = Vec::new();
    let mut writer = Writer::new(Cursor::new(&mut buffer));

    // 构建 <tt> 根元素的属性
    let mut tt_attributes_map: HashMap<&str, String> = HashMap::new();
    tt_attributes_map.insert("xmlns", "http://www.w3.org/ns/ttml".to_string());
    tt_attributes_map.insert(
        "xmlns:itunes",
        "http://music.apple.com/lyric-ttml-internal".to_string(),
    );
    tt_attributes_map.insert(
        "xmlns:ttm",
        "http://www.w3.org/ns/ttml#metadata".to_string(),
    );
    tt_attributes_map.insert("itunes:timing", output_timing_mode.to_string());

    if let Some(lang_code_val) = metadata_store.get_single_value(&CanonicalMetadataKey::Language)
        && !lang_code_val.is_empty()
    {
        tt_attributes_map.insert("xml:lang", lang_code_val.clone());
    }
    let amll_keys = [
        CanonicalMetadataKey::Album,
        CanonicalMetadataKey::AppleMusicId,
        CanonicalMetadataKey::Artist,
        CanonicalMetadataKey::Custom("isrc".to_string()),
        CanonicalMetadataKey::Title,
        CanonicalMetadataKey::Custom("ncmMusicId".to_string()),
        CanonicalMetadataKey::Custom("qqMusicId".to_string()),
        CanonicalMetadataKey::Custom("spotifyId".to_string()),
        CanonicalMetadataKey::Custom("ttmlAuthorGithub".to_string()),
        CanonicalMetadataKey::Author,
    ];
    if amll_keys.iter().any(|key| {
        metadata_store
            .get_multiple_values(key)
            .is_some_and(|vals| vals.iter().any(|v| !v.is_empty()))
    }) {
        tt_attributes_map.insert("xmlns:amll", "http://www.example.com/ns/amll".to_string());
    }

    // 属性排序，确保输出一致性
    let mut sorted_tt_attributes: Vec<(&str, String)> = tt_attributes_map.into_iter().collect();
    sorted_tt_attributes.sort_by_key(|&(key, _)| key);

    // 写入 <tt> 开始标签
    let mut tt_start_event = BytesStart::new("tt");
    for (key, value) in &sorted_tt_attributes {
        tt_start_event.push_attribute((*key, value.as_str()));
    }
    writer.write_event(Event::Start(tt_start_event))?;

    let mut paragraph_agent_ids = HashSet::<String>::new();
    let relevant_paragraphs_for_agents: Vec<&TtmlParagraph> = paragraphs
        .iter()
        .filter(|p| {
            let has_main_syls = !p.main_syllables.is_empty();
            let has_bg_content =
                p.background_section.as_ref().is_some_and(|bs| {
                    !bs.syllables.is_empty()
                        || (!is_for_lyricify_json
                            && (bs.translation.as_ref().is_some_and(|(_, text)| {
                                text.as_ref().is_some_and(|t| !t.is_empty())
                            }) || bs
                                .romanization
                                .as_ref()
                                .is_some_and(|text| !text.is_empty())))
                });
            let has_main_trans = !is_for_lyricify_json
                && p.translation
                    .as_ref()
                    .is_some_and(|(text, _)| !text.is_empty());
            let has_main_roma = !is_for_lyricify_json
                && p.romanization.as_ref().is_some_and(|text| !text.is_empty());
            has_main_syls
                || has_bg_content
                || has_main_trans
                || has_main_roma
                || (p.p_end_ms > p.p_start_ms)
        })
        .collect();

    for para in &relevant_paragraphs_for_agents {
        if !para.agent.is_empty() {
            // 检查 agent 是否符合 "v" + 数字的模式
            if para.agent.starts_with('v')
                && para.agent.len() > 1
                && para.agent[1..].chars().all(char::is_numeric)
            {
                paragraph_agent_ids.insert(para.agent.clone());
            } else if para.agent == "v1" || para.agent == "v2" || para.agent == "v1000" {
                // 兼容旧的硬编码判断
                paragraph_agent_ids.insert(para.agent.clone());
            }
        }
    }

    MetadataStore::write_ttml_head_metadata(metadata_store, &mut writer, &paragraph_agent_ids)?;

    let mut overall_max_body_end_ms = 0;
    let mut body_has_content = false;

    // 使用之前为收集 agent ID 而过滤的 relevant_paragraphs_for_agents，避免重复过滤
    let relevant_paragraphs: Vec<&TtmlParagraph> = relevant_paragraphs_for_agents;

    // 计算 overall_max_body_end_ms 和 body_has_content
    for p in &relevant_paragraphs {
        let has_main_syls = !p.main_syllables.is_empty();
        let has_bg_content = p.background_section.as_ref().is_some_and(|bs| {
            !bs.syllables.is_empty()
                || (!is_for_lyricify_json
                    && (bs
                        .translation
                        .as_ref()
                        .is_some_and(|(_, text)| text.as_ref().is_some_and(|t| !t.is_empty()))
                        || bs
                            .romanization
                            .as_ref()
                            .is_some_and(|text| !text.is_empty())))
        });
        let has_main_trans = !is_for_lyricify_json
            && p.translation
                .as_ref()
                .is_some_and(|(text, _)| !text.is_empty());
        let has_main_roma =
            !is_for_lyricify_json && p.romanization.as_ref().is_some_and(|text| !text.is_empty());

        let has_any_renderable_content =
            has_main_syls || has_bg_content || has_main_trans || has_main_roma;
        if has_any_renderable_content {
            body_has_content = true;
        }

        let mut p_effective_end = p.p_start_ms;
        if output_timing_mode == "Word" {
            if let Some(last_syl) = p.main_syllables.last() {
                p_effective_end = p_effective_end.max(last_syl.end_ms);
            }
            if let Some(bg_sec) = &p.background_section {
                if let Some(last_bg_syl) = bg_sec.syllables.last() {
                    p_effective_end = p_effective_end.max(last_bg_syl.end_ms);
                }
                p_effective_end = p_effective_end.max(bg_sec.end_ms);
            }
        }
        p_effective_end = p_effective_end.max(p.p_end_ms);
        overall_max_body_end_ms = overall_max_body_end_ms.max(p_effective_end);
    }

    let body_dur_str = if body_has_content {
        format_ttml_time(overall_max_body_end_ms)
    } else {
        "0".to_string()
    };

    // 写入 <body> 标签
    writer
        .create_element("body")
        .with_attribute(("dur", body_dur_str.as_str()))
        .write_inner_content(|body_writer| -> io::Result<()> {
            if !relevant_paragraphs.is_empty() {
                let mut line_key_counter = 0;
                let mut current_div_song_part: Option<String> = None;
                let mut paragraphs_for_current_div_buffer: Vec<&TtmlParagraph> = Vec::new();

                for (idx, para_ref) in relevant_paragraphs.iter().enumerate() {
                    let para_song_part = &para_ref.song_part;
                    if idx == 0 {
                        current_div_song_part = para_song_part.clone();
                    } else if para_song_part != &current_div_song_part {
                        if !paragraphs_for_current_div_buffer.is_empty() {
                            write_ttml_div_from_paragraphs_internal(
                                body_writer,
                                &paragraphs_for_current_div_buffer,
                                &current_div_song_part,
                                &mut line_key_counter,
                                output_timing_mode,
                                is_for_lyricify_json,
                            )
                            .map_err(map_xml_error_to_io)?;
                        }
                        paragraphs_for_current_div_buffer.clear();
                        current_div_song_part = para_song_part.clone();
                    }
                    paragraphs_for_current_div_buffer.push(para_ref);
                }
                if !paragraphs_for_current_div_buffer.is_empty() {
                    write_ttml_div_from_paragraphs_internal(
                        body_writer,
                        &paragraphs_for_current_div_buffer,
                        &current_div_song_part,
                        &mut line_key_counter,
                        output_timing_mode,
                        is_for_lyricify_json,
                    )
                    .map_err(map_xml_error_to_io)?;
                }
            }
            Ok(())
        })?;

    writer.write_event(Event::End(BytesEnd::new("tt")))?;
    String::from_utf8(buffer)
        .map_err(|e| ConvertError::Internal(format!("TTML 缓冲区转 UTF-8 失败: {e}")))
}

/// 内部辅助函数: 将 TtmlParagraph 列表写入为 TTML 的 div 和 p 结构。
fn write_ttml_div_from_paragraphs_internal(
    writer: &mut Writer<Cursor<&mut Vec<u8>>>,
    paragraphs_in_div: &[&TtmlParagraph], // 当前 div 内的段落
    song_part: &Option<String>,           // div 的 song-part 属性
    line_key_counter: &mut usize,         // 行 key 计数器
    output_timing_mode: &str,             // 输出计时模式 ("Word" 或 "Line")
    is_for_lyricify_json: bool,           // 是否包含翻译和罗马音
) -> Result<(), quick_xml::Error> {
    if paragraphs_in_div.is_empty() {
        return Ok(());
    }

    // 计算 div 的开始和结束时间
    let div_begin_ms = paragraphs_in_div
        .iter()
        .map(|p| p.p_start_ms)
        .min()
        .unwrap_or(0);
    let mut div_max_end_ms = paragraphs_in_div
        .iter()
        .map(|p| p.p_end_ms)
        .max()
        .unwrap_or(0);
    if output_timing_mode == "Word" {
        for p in paragraphs_in_div {
            if let Some(last_syl) = p.main_syllables.last() {
                div_max_end_ms = div_max_end_ms.max(last_syl.end_ms);
            }
            if let Some(bg_sec) = &p.background_section {
                if let Some(last_bg_syl) = bg_sec.syllables.last() {
                    div_max_end_ms = div_max_end_ms.max(last_bg_syl.end_ms);
                }
                div_max_end_ms = div_max_end_ms.max(bg_sec.end_ms);
            }
        }
    }

    let div_begin_str = format_ttml_time(div_begin_ms);
    let div_end_str = format_ttml_time(div_max_end_ms);
    let mut div_attributes = vec![
        ("begin", div_begin_str.as_str()),
        ("end", div_end_str.as_str()),
    ];
    if let Some(part_str) = song_part
        && !part_str.is_empty()
    {
        div_attributes.push(("itunes:song-part", part_str.as_str()));
    }

    // 写入 <div> 标签
    writer
        .create_element("div")
        .with_attributes(div_attributes)
        .write_inner_content(|div_content_writer| -> io::Result<()> {
            for para in paragraphs_in_div {
                *line_key_counter += 1;
                let p_start_str = format_ttml_time(para.p_start_ms);
                let p_end_str = format_ttml_time(para.p_end_ms);
                let agent_val = if para.agent.is_empty() {
                    "v1"
                } else {
                    &para.agent
                };
                let key_val = format!("L{}", *line_key_counter);

                let mut p_attrs_vec = vec![
                    ("begin", p_start_str.as_str()),
                    ("end", p_end_str.as_str()),
                    ("itunes:key", key_val.as_str()),
                ];
                if !agent_val.is_empty() && agent_val != "v0" {
                    p_attrs_vec.push(("ttm:agent", agent_val));
                }

                div_content_writer
                    .create_element("p")
                    .with_attributes(p_attrs_vec)
                    .write_inner_content(|p_content_writer_inner| -> io::Result<()> {
                        if output_timing_mode == "Line" {
                            let line_text: String = para
                                .main_syllables
                                .iter()
                                .map(|s| {
                                    if s.ends_with_space {
                                        format!("{} ", s.text)
                                    } else {
                                        s.text.clone()
                                    }
                                })
                                .collect::<String>()
                                .trim_end()
                                .to_string();
                            if !line_text.is_empty() {
                                p_content_writer_inner.write_event(Event::Text(
                                    BytesText::from_escaped(escape(&line_text).as_ref()),
                                ))?;
                            }
                        } else {
                            for syl in &para.main_syllables {
                                if !syl.text.is_empty() || (syl.end_ms > syl.start_ms) {
                                    p_content_writer_inner
                                        .create_element("span")
                                        .with_attribute((
                                            "begin",
                                            format_ttml_time(syl.start_ms).as_str(),
                                        ))
                                        .with_attribute((
                                            "end",
                                            format_ttml_time(syl.end_ms).as_str(),
                                        ))
                                        .write_text_content(BytesText::from_escaped(
                                            escape(&syl.text).as_ref(),
                                        ))?;
                                    if syl.ends_with_space {
                                        p_content_writer_inner.write_event(Event::Text(
                                            BytesText::from_escaped(" "),
                                        ))?;
                                    }
                                }
                            }
                        }

                        if !is_for_lyricify_json {
                            if let Some(bg_sec) = &para.background_section {
                                let has_syls = !bg_sec.syllables.is_empty();
                                let has_trans = bg_sec
                                    .translation
                                    .as_ref()
                                    .is_some_and(|(t, _)| !t.is_empty());
                                let has_roma =
                                    bg_sec.romanization.as_ref().is_some_and(|r| !r.is_empty());

                                if has_syls
                                    || has_trans
                                    || has_roma
                                    || (bg_sec.end_ms > bg_sec.start_ms)
                                {
                                    p_content_writer_inner
                                        .create_element("span")
                                        .with_attribute(("ttm:role", "x-bg"))
                                        .with_attribute((
                                            "begin",
                                            format_ttml_time(bg_sec.start_ms).as_str(),
                                        ))
                                        .with_attribute((
                                            "end",
                                            format_ttml_time(bg_sec.end_ms).as_str(),
                                        ))
                                        .write_inner_content(
                                            |bg_syls_writer| -> io::Result<()> {
                                                let num_s = bg_sec.syllables.len();
                                                for (idx, syl_bg) in
                                                    bg_sec.syllables.iter().enumerate()
                                                {
                                                    if !syl_bg.text.is_empty()
                                                        || (syl_bg.end_ms > syl_bg.start_ms)
                                                    {
                                                        let mut text_bg_val = syl_bg.text.clone();
                                                        if text_bg_val != " " {
                                                            text_bg_val =
                                                                clean_parentheses_from_bg_text(
                                                                    &text_bg_val,
                                                                );
                                                            if !text_bg_val.is_empty() {
                                                                if num_s == 1 {
                                                                    text_bg_val =
                                                                        format!("({text_bg_val})");
                                                                } else if idx == 0 {
                                                                    text_bg_val =
                                                                        format!("({text_bg_val}");
                                                                } else if idx == num_s - 1 {
                                                                    text_bg_val =
                                                                        format!("{text_bg_val})");
                                                                }
                                                            }
                                                        }
                                                        if !text_bg_val.is_empty()
                                                            || (syl_bg.end_ms > syl_bg.start_ms)
                                                        {
                                                            bg_syls_writer
                                                                .create_element("span")
                                                                .with_attribute((
                                                                    "begin",
                                                                    format_ttml_time(
                                                                        syl_bg.start_ms,
                                                                    )
                                                                    .as_str(),
                                                                ))
                                                                .with_attribute((
                                                                    "end",
                                                                    format_ttml_time(syl_bg.end_ms)
                                                                        .as_str(),
                                                                ))
                                                                .write_text_content(
                                                                    BytesText::from_escaped(
                                                                        escape(&text_bg_val)
                                                                            .as_ref(),
                                                                    ),
                                                                )?;
                                                        }
                                                        if syl_bg.ends_with_space
                                                            && syl_bg.text != " "
                                                        {
                                                            bg_syls_writer.write_event(
                                                                Event::Text(
                                                                    BytesText::from_escaped(" "),
                                                                ),
                                                            )?;
                                                        }
                                                    }
                                                }
                                                if let Some((text_val, lang_opt)) =
                                                    &bg_sec.translation
                                                    && !text_val.is_empty()
                                                {
                                                    let mut ts_builder = bg_syls_writer
                                                        .create_element("span")
                                                        .with_attribute((
                                                            "ttm:role",
                                                            "x-translation",
                                                        ));
                                                    if let Some(lang) = lang_opt
                                                        && !lang.is_empty()
                                                    {
                                                        ts_builder = ts_builder.with_attribute((
                                                            "xml:lang",
                                                            lang.as_str(),
                                                        ));
                                                    }
                                                    ts_builder.write_text_content(
                                                        BytesText::from_escaped(
                                                            escape(text_val).as_ref(),
                                                        ),
                                                    )?;
                                                }
                                                if let Some(text_val) = &bg_sec.romanization
                                                    && !text_val.is_empty()
                                                {
                                                    bg_syls_writer
                                                        .create_element("span")
                                                        .with_attribute(("ttm:role", "x-roman"))
                                                        .write_text_content(
                                                            BytesText::from_escaped(
                                                                escape(text_val).as_ref(),
                                                            ),
                                                        )?;
                                                }
                                                Ok(())
                                            },
                                        )?;
                                }
                            }
                            if let Some((text_val, lang_opt)) = &para.translation
                                && !text_val.is_empty()
                            {
                                let mut ts_builder = p_content_writer_inner
                                    .create_element("span")
                                    .with_attribute(("ttm:role", "x-translation"));
                                if let Some(lang) = lang_opt
                                    && !lang.is_empty()
                                {
                                    ts_builder =
                                        ts_builder.with_attribute(("xml:lang", lang.as_str()));
                                }
                                ts_builder.write_text_content(BytesText::from_escaped(
                                    escape(text_val).as_ref(),
                                ))?;
                            }
                            if let Some(text_val) = &para.romanization
                                && !text_val.is_empty()
                            {
                                p_content_writer_inner
                                    .create_element("span")
                                    .with_attribute(("ttm:role", "x-roman"))
                                    .write_text_content(BytesText::from_escaped(
                                        escape(text_val).as_ref(),
                                    ))?;
                            }
                        }
                        Ok(())
                    })?;
            }
            Ok(())
        })?;
    Ok(())
}

/// 从解析后的 LRC 数据生成逐行TTML。
///
/// # Arguments
/// * `original_parsed_lrc_lines` - 从 LRC 解析器获得的 LrcLine 列表。
/// * `lrc_metadata` - 从 LRC 文件头部解析的元数据。
///
/// # Returns
/// `Result<String, ConvertError>` - 成功时返回生成的 TTML 字符串，失败时返回错误。
pub fn generate_line_timed_ttml_from_paragraphs(
    paragraphs: &[TtmlParagraph],
    metadata_store: &MetadataStore,
) -> Result<String, ConvertError> {
    let mut buffer = Vec::new();
    let mut writer = Writer::new(Cursor::new(&mut buffer));

    // 1. 准备 <tt> 标签的属性
    let mut tt_attributes_map: HashMap<&str, String> = HashMap::new();
    tt_attributes_map.insert("xmlns", "http://www.w3.org/ns/ttml".to_string());
    tt_attributes_map.insert(
        "xmlns:itunes",
        "http://music.apple.com/lyric-ttml-internal".to_string(),
    );
    tt_attributes_map.insert(
        "xmlns:ttm",
        "http://www.w3.org/ns/ttml#metadata".to_string(),
    );
    tt_attributes_map.insert("itunes:timing", "Line".to_string());

    if let Some(lang_val) = metadata_store.get_single_value(&CanonicalMetadataKey::Language)
        && !lang_val.is_empty()
    {
        let lang_to_use = lang_val.clone();
        tt_attributes_map.insert("xml:lang", lang_to_use);
    }

    let amll_keys = [
        CanonicalMetadataKey::Album,
        CanonicalMetadataKey::AppleMusicId,
        CanonicalMetadataKey::Artist,
        CanonicalMetadataKey::Custom("isrc".to_string()),
        CanonicalMetadataKey::Title,
        CanonicalMetadataKey::Custom("ncmMusicId".to_string()),
        CanonicalMetadataKey::Custom("qqMusicId".to_string()),
        CanonicalMetadataKey::Custom("spotifyId".to_string()),
        CanonicalMetadataKey::Custom("ttmlAuthorGithub".to_string()),
        CanonicalMetadataKey::Author,
    ];
    if amll_keys.iter().any(|key| {
        metadata_store
            .get_multiple_values(key)
            .is_some_and(|vals| vals.iter().any(|v| !v.is_empty()))
    }) {
        tt_attributes_map.insert("xmlns:amll", "http://www.example.com/ns/amll".to_string());
    }

    let mut sorted_tt_attributes: Vec<(&str, String)> = tt_attributes_map.into_iter().collect();
    sorted_tt_attributes.sort_by_key(|&(key, _)| key);
    let mut tt_start_event = BytesStart::new("tt");
    for (key, value) in &sorted_tt_attributes {
        tt_start_event.push_attribute((*key, value.as_str()));
    }
    writer.write_event(Event::Start(tt_start_event))?;

    // 2. 写入 <head> 和 <metadata>
    writer.write_event(Event::Start(BytesStart::new("head")))?;
    writer.write_event(Event::Start(BytesStart::new("metadata")))?;

    let mut agent_tag = BytesStart::new("ttm:agent");
    agent_tag.push_attribute(("xml:id", "v1"));
    agent_tag.push_attribute(("type", "person"));
    writer.write_event(Event::Empty(agent_tag))?;

    // (可选) 写入其他从元数据转换来的标准TTML元数据
    if let Some(title) = metadata_store.get_single_value(&CanonicalMetadataKey::Title)
        && !title.is_empty()
    {
        writer
            .create_element("ttm:title")
            .write_text_content(BytesText::from_escaped(escape(title).as_ref()))?;
    }

    writer.write_event(Event::End(BytesEnd::new("metadata")))?;
    writer.write_event(Event::End(BytesEnd::new("head")))?;

    // 3. 准备 <p> 元素的数据
    let body_overall_start_ms;
    let mut body_overall_end_ms = 0;

    // 过滤掉完全没有主歌词文本的段落 (这些段落可能只包含空的音节或仅用于时间标记)
    let valid_paragraphs: Vec<&TtmlParagraph> = paragraphs
        .iter()
        .filter(|p| {
            p.main_syllables
                .iter()
                .any(|syl| !syl.text.trim().is_empty())
        })
        .collect();

    if !valid_paragraphs.is_empty() {
        body_overall_start_ms = valid_paragraphs.first().map_or(0, |p| p.p_start_ms);
        body_overall_end_ms = valid_paragraphs.last().map_or(0, |p| p.p_end_ms); // p_end_ms 应该已经被正确计算
    } else {
        body_overall_start_ms = 0;
    }

    // 4. 写入 <body> 标签
    let body_dur_str = format_ttml_time(body_overall_end_ms);

    writer
        .create_element("body")
        .with_attribute(("dur", body_dur_str.as_str()))
        .write_inner_content(|body_writer: &mut Writer<Cursor<&mut Vec<u8>>>| -> Result<(), std::io::Error> {
            if !valid_paragraphs.is_empty() {
                let div_begin_str = format_ttml_time(body_overall_start_ms);
                let div_end_str = format_ttml_time(body_overall_end_ms);
                let mut div_attributes = vec![
                    ("begin", div_begin_str.as_str()),
                    ("end", div_end_str.as_str()),
                ];
                if let Some(song_part_val) = metadata_store.get_single_value(&CanonicalMetadataKey::Custom("songPart".to_string()))
                     && !song_part_val.is_empty() {
                        div_attributes.push(("itunes:songPart", song_part_val.as_str()));
                     }

                body_writer
                    .create_element("div")
                    .with_attributes(div_attributes)
                    .write_inner_content(|div_content_writer: &mut Writer<Cursor<&mut Vec<u8>>>| -> Result<(), std::io::Error> {
                        let mut line_key_counter = 0;
                        for para_to_render in valid_paragraphs { // 使用过滤后的段落
                            line_key_counter += 1;
                            let p_start_str = format_ttml_time(para_to_render.p_start_ms);
                            let p_end_str = format_ttml_time(para_to_render.p_end_ms); // 使用段落自身的结束时间
                            let key_val = format!("L{line_key_counter}");

                            let p_attributes = vec![
                                ("begin", p_start_str.as_str()),
                                ("end", p_end_str.as_str()),
                                ("itunes:key", key_val.as_str()),
                                ("ttm:agent", "v1"), // 固定agent
                            ];

                            div_content_writer
                                .create_element("p")
                                .with_attributes(p_attributes)
                                .write_inner_content(|p_writer: &mut Writer<Cursor<&mut Vec<u8>>>| -> Result<(), std::io::Error> {
                                    // 主文本 (假设LRC行只有一个音节代表整行)
                                    if let Some(main_syl) = para_to_render.main_syllables.first()
                                        && !main_syl.text.trim().is_empty() {
                                             p_writer.write_event(Event::Text(BytesText::from_escaped(escape(&main_syl.text).as_ref())))?;
                                        }

                                    // 写入翻译
                                    if let Some((trans_text, trans_lang_opt)) = &para_to_render.translation
                                        && !trans_text.is_empty() {
                                            let mut trans_span = p_writer.create_element("span")
                                                .with_attribute(("ttm:role", "x-translation"));
                                            if let Some(lang) = trans_lang_opt
                                                && !lang.is_empty() {
                                                    trans_span = trans_span.with_attribute(("xml:lang", lang.as_str()));
                                                }
                                            trans_span.write_text_content(BytesText::from_escaped(escape(trans_text).as_ref()))?;
                                        }

                                    // 写入罗马音
                                    if let Some(roma_text) = &para_to_render.romanization
                                        && !roma_text.is_empty() {
                                            p_writer.create_element("span")
                                                .with_attribute(("ttm:role", "x-roman"))
                                                .write_text_content(BytesText::from_escaped(escape(roma_text).as_ref()))?;
                                        }
                                    Ok(())
                                })?;
                        }
                        Ok(())
                    })?;
            }
            Ok(())
        })?;

    writer.write_event(Event::End(BytesEnd::new("tt")))?;

    String::from_utf8(buffer).map_err(ConvertError::FromUtf8)
}
