use crate::ttml_parser;
use crate::types::{AppleMusicRoot, AssMetadata, ConvertError, TtmlParagraph};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct ParsedJsonDataBundle {
    pub paragraphs: Vec<TtmlParagraph>,
    pub apple_music_id: String,
    pub language_code: Option<String>,
    pub songwriters: Vec<String>,
    pub agent_names: HashMap<String, String>,
    pub general_metadata: Vec<AssMetadata>,
    pub is_line_timed: bool,
    pub raw_ttml_string: String,
    pub detected_formatted_ttml: bool,
    pub _detected_source_translation_language: Option<String>,
}

pub fn load_from_string(json_content: &str) -> Result<ParsedJsonDataBundle, ConvertError> {
    let root: AppleMusicRoot = serde_json::from_str(json_content)?;

    let data_object = root
        .data
        .first()
        .ok_or_else(|| ConvertError::InvalidJsonStructure("JSON 'data' 为空。".to_string()))?;

    if data_object.data_type != "syllable-lyrics" {
        return Err(ConvertError::InvalidJsonStructure(format!(
            "期望的 data_type 是 'syllable-lyrics', 但找到的是 '{}'",
            data_object.data_type
        )));
    }

    let ttml_string_from_json_attributes = &data_object.attributes.ttml;

    let mut parsed_apple_music_id = data_object.id.clone();

    if parsed_apple_music_id.is_empty() {
        let catalog_id_val = &data_object.attributes.play_params.catalog_id;
        if !catalog_id_val.is_empty() {
            parsed_apple_music_id = catalog_id_val.clone();
        }
    }

    if parsed_apple_music_id.is_empty() {
        let id_val = &data_object.attributes.play_params.id;
        if !id_val.is_empty() {
            parsed_apple_music_id = id_val.clone();
        }
    }

    let (
        parsed_paragraphs,
        ttml_internal_metadata,
        is_line_timed_val,
        detected_formatted,
        detected_ttml_trans_lang,
    ) = match ttml_parser::parse_ttml_from_string(ttml_string_from_json_attributes) {
        Ok(result_tuple) => result_tuple,
        Err(e) => {
            eprintln!("[JSON Parser] Failed to parse TTML content from JSON: {e}");
            return Err(ConvertError::Internal(format!(
                "无法解析JSON中的TTML内容: {e}"
            )));
        }
    };

    let mut parsed_language_code: Option<String> = None;
    let mut parsed_songwriters: Vec<String> = Vec::new();
    let mut parsed_agent_names: HashMap<String, String> = HashMap::new();
    let mut remaining_general_metadata: Vec<AssMetadata> = Vec::new();

    for meta_item in ttml_internal_metadata {
        let key_lower = meta_item.key.to_lowercase();
        if key_lower == "lang" && parsed_language_code.is_none() {
            parsed_language_code = Some(meta_item.value);
        } else if key_lower == "songwriter" {
            parsed_songwriters.push(meta_item.value);
        } else if key_lower == "v1" || key_lower == "v2" || key_lower == "v1000" {
            parsed_agent_names.insert(meta_item.key.clone(), meta_item.value);
        } else if key_lower == "applemusicid" {
            if parsed_apple_music_id.is_empty() {
                parsed_apple_music_id = meta_item.value;
            }
        } else {
            remaining_general_metadata.push(meta_item);
        }
    }

    Ok(ParsedJsonDataBundle {
        paragraphs: parsed_paragraphs,
        apple_music_id: parsed_apple_music_id,
        language_code: parsed_language_code,
        songwriters: parsed_songwriters,
        agent_names: parsed_agent_names,
        general_metadata: remaining_general_metadata,
        is_line_timed: is_line_timed_val,
        raw_ttml_string: ttml_string_from_json_attributes.clone(),
        detected_formatted_ttml: detected_formatted,
        _detected_source_translation_language: detected_ttml_trans_lang,
    })
}
