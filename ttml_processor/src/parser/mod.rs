//! # TTML (Timed Text Markup Language) 解析器
//!
//! 该解析器设计上仅用于解析 Apple Music 和 AMLL 使用的 TTML 歌词文件，
//! 不建议用于解析通用的 TTML 字幕文件。

mod body;
mod constants;
mod handlers;
mod metadata;
mod state;
mod utils;

use std::collections::HashMap;

use quick_xml::{Reader, errors::Error as QuickXmlError, events::Event};
use tracing::error;

use self::state::{FormatDetection, TtmlParserState};
use lyrics_helper_core::{
    ConvertError, LyricFormat, LyricLine, ParsedSourceData, TtmlParsingOptions,
};

/// 解析 TTML 格式的歌词文件。
///
/// # 参数
///
/// * `content` - TTML 格式的歌词文件内容字符串。
/// * `options` - TTML 解析选项，包含默认语言配置和时间模式设置。
///
/// # 返回
///
/// * `Ok(ParsedSourceData)` - 成功解析后，返回包含歌词行、元数据等信息的统一数据结构。
/// * `Err(ConvertError)` - 解析失败时，返回具体的错误信息。
///
/// # Errors
///
/// 此函数在以下情况下会返回错误：
///
/// * `ConvertError::Xml` - 当输入的 TTML 内容不是有效的 XML 格式时
/// * `ConvertError::InvalidTime` - 当 TTML 中的时间戳格式无效或无法解析时
/// * `ConvertError::Internal` - 当内部处理过程中出现意外错误时（如上下文丢失）
pub fn parse_ttml(
    content: &str,
    options: &TtmlParsingOptions,
) -> Result<ParsedSourceData, ConvertError> {
    // 预扫描以确定是否存在带时间的span，辅助判断计时模式
    let has_timed_span_tags = content.contains("<span") && content.contains("begin=");

    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(false);
    reader.config_mut().expand_empty_elements = true;

    let mut lines: Vec<LyricLine> = Vec::with_capacity(content.matches("<p").count());
    let mut raw_metadata: HashMap<String, Vec<String>> = HashMap::new();
    let mut warnings: Vec<String> = Vec::new();

    // 初始化解析状态机
    let mut state = TtmlParserState {
        default_main_lang: options.default_languages.main.clone(),
        default_translation_lang: options.default_languages.translation.clone(),
        default_romanization_lang: options.default_languages.romanization.clone(),
        ..Default::default()
    };
    let mut buf = Vec::new();

    loop {
        if state.format_detection == FormatDetection::Undetermined {
            state.total_nodes_processed += 1;
            if state.whitespace_nodes_with_newline > 5 {
                state.format_detection = FormatDetection::IsFormatted;
            } else if state.total_nodes_processed > 5000 {
                state.format_detection = FormatDetection::NotFormatted;
            }
        }

        let event = match reader.read_event_into(&mut buf) {
            Ok(event) => event,
            Err(e) => {
                // 尝试抢救数据
                if let QuickXmlError::IllFormed(_) = e {
                    handlers::attempt_recovery_from_error(
                        &mut state,
                        &reader,
                        &mut lines,
                        &mut warnings,
                        &e,
                    );
                    buf.clear();
                    continue;
                }

                // 无法恢复的 IO 错误等
                error!(
                    "TTML 解析错误，位置 {}: {}。无法继续解析",
                    reader.error_position(),
                    e
                );
                return Err(ConvertError::Xml(e));
            }
        };

        if let Event::Text(e) = &event
            && state.format_detection == FormatDetection::Undetermined
        {
            let bytes = e.as_ref();
            if bytes.contains(&b'\n') && bytes.iter().all(|&b| b.is_ascii_whitespace()) {
                state.whitespace_nodes_with_newline += 1;
            }
        }

        if event == Event::Eof {
            break;
        }

        if state.in_metadata {
            metadata::handle_metadata_event(
                &event,
                &mut reader,
                &mut state,
                &mut raw_metadata,
                &mut warnings,
            )?;
        } else if state.body_state.in_p {
            body::handle_p_event(&event, &mut state, &reader, &mut lines, &mut warnings)?;
        } else {
            if event == Event::Eof {
                break;
            }
            handlers::handle_global_event(
                &event,
                &mut state,
                &reader,
                &mut raw_metadata,
                &mut warnings,
                has_timed_span_tags,
                options,
            )?;
        }

        buf.clear();
    }

    Ok(ParsedSourceData {
        lines,
        raw_metadata,
        agents: state.agent_store,
        source_format: LyricFormat::Ttml,
        source_filename: None,
        is_line_timed_source: state.is_line_timing_mode,
        warnings,
        raw_ttml_from_input: Some(content.to_string()),
        detected_formatted_ttml_input: Some(state.format_detection == FormatDetection::IsFormatted),
        ..Default::default()
    })
}
