//! Apple Music JSON 格式生成器。
//!
//! 这个 JSON 内嵌有 Apple Music 样式的 TTML 文件。

use lyrics_helper_core::{
    AgentStore, CanonicalMetadataKey, ConversionOptions, ConvertError, LyricLine, MetadataStore,
    TtmlGenerationOptions, TtmlTimingMode,
};
use serde::Serialize;
use ttml_processor::generate_ttml;

// 用于序列化为 JSON 的结构体
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Root {
    data: Vec<DataItem>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DataItem {
    id: String,
    #[serde(rename = "type")]
    type_field: String,
    attributes: Attributes,
    play_params: PlayParams,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Attributes {
    ttml: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlayParams {
    id: String,
    kind: String,
    catalog_id: String,
    display_type: i32,
}

/// Apple Music JSON 生成的主入口函数。
pub fn generate_apple_music_json(
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
    agent_store: &AgentStore,
    options: &ConversionOptions,
) -> Result<String, ConvertError> {
    let apple_ttml_options = TtmlGenerationOptions {
        use_apple_format_rules: true,
        format: false,
        ..options.ttml.clone()
    };

    let ttml_content = generate_ttml(lines, metadata_store, agent_store, &apple_ttml_options)?;

    let apple_music_id = metadata_store
        .get_single_value(&CanonicalMetadataKey::AppleMusicId)
        .cloned()
        .unwrap_or_else(|| {
            tracing::warn!("[AppleMusicJson 生成器] 未找到 AppleMusicId，将使用空字符串。");
            String::new()
        });

    let display_type = match options.ttml.timing_mode {
        TtmlTimingMode::Word => 3,
        TtmlTimingMode::Line => 2,
    };

    let json_output = Root {
        data: vec![DataItem {
            id: apple_music_id.clone(),
            type_field: "syllable-lyrics".to_string(),
            attributes: Attributes { ttml: ttml_content },
            play_params: PlayParams {
                id: format!("AP_{apple_music_id}"),
                kind: "lyric".to_string(),
                catalog_id: apple_music_id,
                display_type,
            },
        }],
    };

    serde_json::to_string(&json_output)
        .map_err(|e| ConvertError::json_parse(e, "序列化为 Apple Music JSON 失败".to_string()))
}
