//! YRC 歌词格式生成器。

use std::fmt::Write;

use serde_json::{Value, json};

use lyrics_helper_core::{
    CanonicalMetadataKey, ContentType, ConvertError, LyricLine, MetadataStore,
};

/// YRC 生成的主入口函数。
pub fn generate_yrc(
    lines: &[LyricLine],
    metadata_store: &MetadataStore,
) -> Result<String, ConvertError> {
    let mut yrc_output = String::new();

    let mut metadata_to_generate: Vec<(String, &[String])> = Vec::new();
    for (key, values) in metadata_store.get_all_data() {
        if values.is_empty() {
            continue;
        }
        let label = match key {
            CanonicalMetadataKey::Title => "曲名",
            CanonicalMetadataKey::Artist => "歌手",
            CanonicalMetadataKey::Album => "专辑",
            CanonicalMetadataKey::Songwriter => "作词",
            CanonicalMetadataKey::Custom(custom_key) if custom_key == "composer" => "作曲",
            // 对于其他不适合显示的键（如 Offset, Language），返回空字符串来忽略它们
            _ => "",
        };

        if !label.is_empty() {
            metadata_to_generate.push((label.to_string(), values));
        }
    }

    let metadata_count = metadata_to_generate.len();
    if metadata_count > 0 {
        // 如果没有歌词行，则默认在 5 秒内显示完毕
        const DEFAULT_METADATA_TIMESPAN: u64 = 5000;
        let available_time = lines
            .first()
            .map_or(DEFAULT_METADATA_TIMESPAN, |l| l.start_ms);

        let interval = if available_time > 0 {
            available_time / (metadata_count as u64)
        } else {
            0
        };

        for (i, (label, values)) in metadata_to_generate.iter().enumerate() {
            let metadata_time = (i as u64) * interval;
            let json_line = build_yrc_metadata_json(label, values, metadata_time);
            writeln!(yrc_output, "{json_line}")?;
        }
    }

    for line in lines {
        if let Some(main_track) = line
            .tracks
            .iter()
            .find(|t| t.content_type == ContentType::Main)
        {
            let syllables: Vec<_> = main_track
                .content
                .words
                .iter()
                .flat_map(|w| &w.syllables)
                .collect();
            if syllables.is_empty() {
                continue;
            }

            let line_duration = line.end_ms.saturating_sub(line.start_ms);
            write!(yrc_output, "[{},{}]", line.start_ms, line_duration)?;

            for syl in syllables {
                let syl_duration = syl.end_ms.saturating_sub(syl.start_ms);
                write!(
                    yrc_output,
                    "({},{},0){}",
                    syl.start_ms, syl_duration, syl.text
                )?;
            }
            writeln!(yrc_output)?;
        }
    }

    Ok(yrc_output)
}

/// 辅助函数，构建单行 YRC 元数据 JSON 字符串。
///
/// # 参数
/// * `label` - 元数据的标签，例如 "作词" 或 "作曲"。
/// * `values` - 与标签关联的值列表，例如多个作者的名字。
/// * `time` - 该行元数据的时间戳。
///
/// # 返回
/// 一个格式化好的、代表单行元数据的 JSON 字符串。
fn build_yrc_metadata_json(label: &str, values: &[String], time: u64) -> String {
    let mut c_array: Vec<Value> = vec![json!({ "tx": format!("{}: ", label) })];

    for (i, value) in values.iter().enumerate() {
        c_array.push(json!({ "tx": value }));
        if i < values.len() - 1 {
            c_array.push(json!({ "tx": "/" }));
        }
    }

    let json_data = json!({
        "t": time,
        "c": c_array
    });

    json_data.to_string()
}
