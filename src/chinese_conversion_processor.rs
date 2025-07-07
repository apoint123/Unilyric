use std::sync::Arc;

use dashmap::DashMap;
use ferrous_opencc::OpenCC;
use log::error;
use std::sync::LazyLock;

use crate::types::{ChineseConversionOptions, TtmlParagraph};

static CONVERTER_CACHE: LazyLock<DashMap<String, Arc<OpenCC>>> = LazyLock::new(DashMap::new);

pub fn convert(text: &str, config_name: &str) -> String {
    if let Some(converter) = CONVERTER_CACHE.get(config_name) {
        return converter.convert(text);
    }

    match CONVERTER_CACHE
        .entry(config_name.to_string())
        .or_try_insert_with(|| {
            OpenCC::from_config_name(config_name)
                .map(Arc::new)
                .map_err(|e| {
                    error!("使用配置 '{}' 初始化 Opencc 转换器失败: {}", config_name, e);
                    e
                })
        }) {
        Ok(converter_ref) => converter_ref.value().convert(text),
        Err(_) => text.to_string(),
    }
}

#[derive(Debug, Default)]
pub struct ChineseConversionProcessor;

impl ChineseConversionProcessor {
    pub fn new() -> Self {
        Self
    }

    pub fn process(&self, paragraphs: &mut [TtmlParagraph], options: &ChineseConversionOptions) {
        let Some(config_name) = options.config_name.as_ref().filter(|s| !s.is_empty()) else {
            return;
        };

        for paragraph in paragraphs.iter_mut() {
            if !paragraph.main_syllables.is_empty() {
                for syllable in &mut paragraph.main_syllables {
                    syllable.text = convert(&syllable.text, config_name);
                }
            }

            if let Some((text, _lang)) = &mut paragraph.translation {
                *text = convert(text, config_name);
            }

            if let Some(bg_section) = &mut paragraph.background_section {
                for syllable in &mut bg_section.syllables {
                    syllable.text = convert(&syllable.text, config_name);
                }
                if let Some((text, _lang)) = &mut bg_section.translation {
                    *text = convert(text, config_name);
                }
            }
        }
    }
}
