//! # Timed Text Markup Language 歌词格式生成器
//!
//! 注意：该模块设计上仅用于生成 Apple Music 和 AMLL 使用的 TTML 歌词文件，
//! 无法用于生成通用的 TTML 字幕文件。

mod body;
mod head;
mod track;
mod utils;

use std::io::Cursor;

use lyrics_helper_core::{
    AgentStore, CanonicalMetadataKey, ConvertError, LyricLine, MetadataStore,
    TtmlGenerationOptions, TtmlTimingMode,
};
use quick_xml::Writer;

/// TTML 生成的主入口函数。
///
/// # 参数
/// * `lines` - 歌词行数据切片。
/// * `metadata_store` - 规范化后的元数据存储。
/// * `agent_store` - 代理信息存储，用于生成歌手标识。
/// * `options` - TTML 生成选项，控制输出格式和规则。
///
/// # 返回
///
/// * `Ok(String)` - 成功生成的 TTML 字符串。
///
/// # Errors
///
/// 如果在生成 XML 或将结果转换为字符串时发生错误（例如 I/O 错误或 UTF-8 编码问题），
/// 则会返回 `ConvertError`。
pub fn generate_ttml(
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
    agent_store: &AgentStore,
    options: &TtmlGenerationOptions,
) -> Result<String, ConvertError> {
    let mut buffer = Vec::new();
    let indent_char = b' ';
    let indent_size = 2;

    // 决定是否输出格式化的 TTML
    let result = if options.format {
        let mut writer =
            Writer::new_with_indent(Cursor::new(&mut buffer), indent_char, indent_size);
        generate_ttml_inner(&mut writer, lines, metadata_store, agent_store, options)
    } else {
        let mut writer = Writer::new(Cursor::new(&mut buffer));
        generate_ttml_inner(&mut writer, lines, metadata_store, agent_store, options)
    };

    result?;

    String::from_utf8(buffer).map_err(ConvertError::FromUtf8)
}

/// TTML 生成的核心内部逻辑。
fn generate_ttml_inner<W: std::io::Write>(
    writer: &mut Writer<W>,
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
    agent_store: &AgentStore,
    options: &TtmlGenerationOptions,
) -> Result<(), ConvertError> {
    // 准备根元素的属性
    let mut namespace_attrs: Vec<(&str, String)> = Vec::new();
    namespace_attrs.push(("xmlns", "http://www.w3.org/ns/ttml".to_string()));
    namespace_attrs.push((
        "xmlns:ttm",
        "http://www.w3.org/ns/ttml#metadata".to_string(),
    ));
    namespace_attrs.push((
        "xmlns:itunes",
        "http://music.apple.com/lyric-ttml-internal".to_string(),
    ));

    let amll_keys_to_check_for_namespace = [
        CanonicalMetadataKey::Title,
        CanonicalMetadataKey::Artist,
        CanonicalMetadataKey::Album,
        CanonicalMetadataKey::Isrc,
        CanonicalMetadataKey::AppleMusicId,
        CanonicalMetadataKey::NcmMusicId,
        CanonicalMetadataKey::QqMusicId,
        CanonicalMetadataKey::SpotifyId,
        CanonicalMetadataKey::TtmlAuthorGithub,
        CanonicalMetadataKey::TtmlAuthorGithubLogin,
    ];
    if amll_keys_to_check_for_namespace
        .iter()
        .any(|key| metadata_store.get_multiple_values(key).is_some())
    {
        namespace_attrs.push(("xmlns:amll", "http://www.example.com/ns/amll".to_string()));
    }

    // 设置主语言属性
    let lang_attr = options
        .main_language
        .as_ref()
        .or_else(|| metadata_store.get_single_value(&CanonicalMetadataKey::Language))
        .filter(|s| !s.is_empty())
        .map(|lang| ("xml:lang", lang.clone()));

    // 设置 itunes:timing 属性
    let timing_mode_str = match options.timing_mode {
        TtmlTimingMode::Word => "Word",
        TtmlTimingMode::Line => "Line",
    };
    let timing_attr = ("itunes:timing", timing_mode_str.to_string());

    // 属性排序以保证输出稳定
    namespace_attrs.sort_by_key(|&(key, _)| key);

    // 写入 <tt> 根元素
    let mut element_writer = writer.create_element("tt");

    for (i, (key, value)) in namespace_attrs.iter().enumerate() {
        if i > 0 {
            element_writer = element_writer.new_line();
        }
        element_writer = element_writer.with_attribute((*key, value.as_str()));
    }

    element_writer = element_writer
        .new_line()
        .with_attribute((timing_attr.0, timing_attr.1.as_str()));

    if let Some((key, value)) = &lang_attr {
        element_writer = element_writer
            .new_line()
            .with_attribute((*key, value.as_str()));
    }

    element_writer.write_inner_content(|writer| {
        head::write_ttml_head(writer, metadata_store, lines, agent_store, options)?;
        body::write_ttml_body(writer, lines, options)?;
        Ok(())
    })?;

    Ok(())
}
