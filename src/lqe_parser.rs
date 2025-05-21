use crate::types::{AssMetadata, ConvertError, LqeSection, LyricFormat, ParsedLqeData};
use once_cell::sync::Lazy;
use regex::Regex;

static LQE_HEADER_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\[Lyricify Quick Export\]").expect("未能编译 LQE_HEADER_REGEX"));
static LQE_VERSION_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\[version:(.*?)\]").expect("未能编译 LQE_VERSION_REGEX"));
static LQE_METADATA_TAG_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\[(ti|ar|al|by|offset|re|ve|length):(.*?)\]")
        .expect("未能编译 LQE_METADATA_TAG_REGEX")
});
static LQE_SECTION_HEADER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\[(lyrics|translation|pronunciation):([^\]]*)\]")
        .expect("未能编译 LQE_SECTION_HEADER_REGEX")
});

fn parse_lqe_section_attributes(attrs_str: &str) -> (Option<LyricFormat>, Option<String>) {
    let mut format = None;
    let mut language = None;
    for attr in attrs_str.split(',') {
        let parts: Vec<&str> = attr.trim().splitn(2, '@').collect();
        if parts.len() == 2 {
            let key = parts[0].trim();
            let value = parts[1].trim();
            match key {
                "format" => format = LyricFormat::from_string(value),
                "language" => language = Some(value.to_string()),
                _ => log::warn!("[LQE 处理] 未知的属性: {}={}", key, value),
            }
        }
    }
    (format, language)
}

pub fn load_lqe_from_string(lqe_content: &str) -> Result<ParsedLqeData, ConvertError> {
    let mut lines = lqe_content.lines().peekable();
    let mut parsed_data = ParsedLqeData::default();

    if let Some(first_line) = lines.next() {
        if !LQE_HEADER_REGEX.is_match(first_line.trim()) {
            return Err(ConvertError::Internal(
                "无效的 LQE 格式: 缺失 '[Lyricify Quick Export]'".to_string(),
            ));
        }
    } else {
        return Err(ConvertError::Internal("空内容".to_string()));
    }

    while let Some(line_str) = lines.peek() {
        let trimmed_line = line_str.trim();
        if trimmed_line.is_empty() {
            lines.next();
            continue;
        }

        if let Some(caps) = LQE_VERSION_REGEX.captures(trimmed_line) {
            parsed_data.version = Some(caps.get(1).map_or("", |m| m.as_str()).trim().to_string());
            lines.next();
        } else if let Some(caps) = LQE_METADATA_TAG_REGEX.captures(trimmed_line) {
            let key = caps.get(1).map_or("", |m| m.as_str()).to_string();
            let value = caps.get(2).map_or("", |m| m.as_str()).trim().to_string();
            if !key.is_empty() {
                parsed_data.global_metadata.push(AssMetadata { key, value });
            }
            lines.next();
        } else if LQE_SECTION_HEADER_REGEX.is_match(trimmed_line) {
            break;
        } else {
            log::warn!("[LQE 处理] 无法识别的元数据: '{}'", trimmed_line);
            continue;
        }
    }

    let mut current_active_section_type: Option<String> = None;

    for line_str_raw in lines {
        let line_for_header_check = line_str_raw.trim();

        if let Some(caps) = LQE_SECTION_HEADER_REGEX.captures(line_for_header_check) {
            let section_name = caps.get(1).map_or("", |m| m.as_str()).to_string();
            let attrs_str = caps.get(2).map_or("", |m| m.as_str());
            let (fmt_attr, lang_attr) = parse_lqe_section_attributes(attrs_str);

            current_active_section_type = Some(section_name.clone());

            match section_name.as_str() {
                "lyrics" => {
                    if parsed_data.lyrics_section.is_some() {
                        log::warn!("[LQE 处理] 找到多个 [lyrics:...]，正在使用第一个");
                    } else {
                        parsed_data.lyrics_section = Some(LqeSection {
                            format: fmt_attr,
                            language: lang_attr,
                            content: String::new(),
                        });
                    }
                }
                "translation" => {
                    if parsed_data.translation_section.is_some() {
                        log::warn!("[LQE 处理] 找到多个 [translation:...]，正在使用第一个");
                    } else {
                        parsed_data.translation_section = Some(LqeSection {
                            format: fmt_attr,
                            language: lang_attr,
                            content: String::new(),
                        });
                    }
                }
                "pronunciation" => {
                    if parsed_data.pronunciation_section.is_some() {
                        log::warn!("[LQE 处理] 找到多个 [pronunciation:...]，正在使用第一个");
                    } else {
                        parsed_data.pronunciation_section = Some(LqeSection {
                            format: fmt_attr,
                            language: lang_attr,
                            content: String::new(),
                        });
                    }
                }
                _ => {
                    log::error!("[LQE 处理] 意外错误：未知的名称 '{}'", section_name);
                    current_active_section_type = None;
                }
            }
        } else if let Some(active_type) = &current_active_section_type {
            let line_content_to_append = line_str_raw.trim_end();

            match active_type.as_str() {
                "lyrics" => {
                    if let Some(section) = &mut parsed_data.lyrics_section {
                        if !section.content.is_empty() {
                            section.content.push('\n');
                        }
                        section.content.push_str(line_content_to_append);
                    }
                }
                "translation" => {
                    if let Some(section) = &mut parsed_data.translation_section {
                        if !section.content.is_empty() {
                            section.content.push('\n');
                        }
                        section.content.push_str(line_content_to_append);
                    }
                }
                "pronunciation" => {
                    if let Some(section) = &mut parsed_data.pronunciation_section {
                        if !section.content.is_empty() {
                            section.content.push('\n');
                        }
                        section.content.push_str(line_content_to_append);
                    }
                }
                _ => {}
            }
        } else if !line_str_raw.trim().is_empty() {
            log::warn!("[LQE 处理] 在歌词/翻译/音译部分之外的行 '{}'", line_str_raw);
        }
    }

    if let Some(s) = &mut parsed_data.lyrics_section {
        s.content = s.content.trim().to_string();
    }
    if let Some(s) = &mut parsed_data.translation_section {
        s.content = s.content.trim().to_string();
    }
    if let Some(s) = &mut parsed_data.pronunciation_section {
        s.content = s.content.trim().to_string();
    }

    if parsed_data.lyrics_section.is_none() {
        log::warn!("[LQE 处理] 未找到歌词");
    }

    Ok(parsed_data)
}
