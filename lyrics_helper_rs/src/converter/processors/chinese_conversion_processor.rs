//! 简繁中文转换器。

use std::sync::Arc;

use dashmap::DashMap;
use ferrous_opencc::OpenCC;
use ferrous_opencc::config::BuiltinConfig as OpenccConfig;
use lyrics_helper_core::{
    ChineseConversionConfig, ChineseConversionMode, ChineseConversionOptions, ContentType,
    LyricLine,
};
use pinyin::ToPinyin;
use std::sync::LazyLock;
use tracing::{error, warn};

/// 使用 `DashMap` 来创建一个 `OpenCC` 实例缓存。
/// 键是配置文件名 (e.g., "s2t.json")，值是对应的 `OpenCC` 实例。
static CONVERTER_CACHE: LazyLock<DashMap<String, Arc<OpenCC>>> = LazyLock::new(DashMap::new);

const fn to_opencc_config(config: ChineseConversionConfig) -> OpenccConfig {
    match config {
        ChineseConversionConfig::S2t => OpenccConfig::S2t,
        ChineseConversionConfig::T2s => OpenccConfig::T2s,
        ChineseConversionConfig::S2tw => OpenccConfig::S2tw,
        ChineseConversionConfig::Tw2s => OpenccConfig::Tw2s,
        ChineseConversionConfig::S2hk => OpenccConfig::S2hk,
        ChineseConversionConfig::Hk2s => OpenccConfig::Hk2s,
        ChineseConversionConfig::S2twp => OpenccConfig::S2twp,
        ChineseConversionConfig::Tw2sp => OpenccConfig::Tw2sp,
        ChineseConversionConfig::T2tw => OpenccConfig::T2tw,
        ChineseConversionConfig::Tw2t => OpenccConfig::Tw2t,
        ChineseConversionConfig::T2hk => OpenccConfig::T2hk,
        ChineseConversionConfig::Hk2t => OpenccConfig::Hk2t,
        ChineseConversionConfig::Jp2t => OpenccConfig::Jp2t,
        ChineseConversionConfig::T2jp => OpenccConfig::T2jp,
    }
}

/// 根据指定的 `OpenCC` 配置转换文本。
///
/// # 参数
/// * `text` - 需要转换的文本。
/// * `config` - `OpenCC` 配置枚举。
///
/// # 返回
/// 转换后的字符串。如果指定的配置加载失败，将打印错误日志并返回原始文本。
pub fn convert(text: &str, config: ChineseConversionConfig) -> String {
    let opencc_config = to_opencc_config(config);
    let cache_key = opencc_config.to_filename();

    // 检查缓存中是否已存在该转换器
    if let Some(converter) = CONVERTER_CACHE.get(cache_key) {
        return converter.convert(text);
    }

    // 如果缓存中没有，则尝试创建并插入
    CONVERTER_CACHE
        .entry(cache_key.to_string())
        .or_try_insert_with(|| {
            OpenCC::from_config(opencc_config)
                .map(Arc::new)
                .map_err(|e| {
                    error!("使用配置 '{:?}' 初始化 Opencc 时失败: {}", config, e);
                    e // 将错误传递出去，or_try_insert_with 需要
                })
        })
        .map_or_else(
            |_| text.to_string(),
            |converter_ref| converter_ref.value().convert(text),
        )
}

/// 比较两个字符串的拼音是否相同。
///
/// 因为多音字的音调非常难以确定，所以忽略声调。
fn pinyin_is_same(original: &str, converted: &str) -> bool {
    if original.chars().count() != converted.chars().count() {
        return false;
    }

    // 获取无声调的拼音
    let original_pinyins: Vec<_> = original
        .to_pinyin()
        .map(|p| p.map_or("", |p_val| p_val.plain()))
        .collect();

    let converted_pinyins: Vec<_> = converted
        .to_pinyin()
        .map(|p| p.map_or("", |p_val| p_val.plain()))
        .collect();

    original_pinyins == converted_pinyins
}

/// 一个用于执行简繁中文转换的处理器。
#[derive(Debug, Default)]
pub struct ChineseConversionProcessor;

impl ChineseConversionProcessor {
    /// 创建一个新的处理器实例。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// 对一组歌词行应用简繁转换。
    ///
    /// # 参数
    /// * `lines` - 一个可变的歌词行切片，转换结果将直接写入其中。
    /// * `options` - 简繁转换的配置选项，决定是否执行以及执行何种模式的转换。
    pub fn process(lines: &mut [LyricLine], options: &ChineseConversionOptions) {
        let Some(config) = options.config else {
            return;
        };

        match options.mode {
            ChineseConversionMode::AddAsTranslation => {
                Self::add_as_translation(lines, config, options);
            }
            ChineseConversionMode::Replace => {
                Self::replace(lines, config);
            }
        }
    }

    fn add_as_translation(
        lines: &mut [LyricLine],
        config: ChineseConversionConfig,
        options: &ChineseConversionOptions,
    ) {
        let lang_tag = options
            .target_lang_tag
            .as_deref()
            .or_else(|| config.deduce_lang_tag());

        let Some(target_lang_tag) = lang_tag else {
            warn!(
                "无法确定 target_lang_tag (未提供且无法从 '{:?}' 推断)。跳过简繁转换。",
                config
            );
            return;
        };

        for line in lines.iter_mut() {
            for at in &mut line.tracks {
                if !matches!(at.content_type, ContentType::Main | ContentType::Background) {
                    continue;
                }

                if at.has_translation(target_lang_tag) {
                    continue;
                }

                let original_text = at.content.text();

                if !original_text.is_empty() {
                    let converted_text = convert(&original_text, config);

                    at.add_translation(&converted_text, target_lang_tag);
                }
            }
        }
    }

    fn replace(lines: &mut [LyricLine], config: ChineseConversionConfig) {
        for line in lines.iter_mut() {
            for at in &mut line.tracks {
                if at.content_type == ContentType::Main {
                    let main_track = &mut at.content;
                    for word in &mut main_track.words {
                        let original_syllable_texts: Vec<String> =
                            word.syllables.iter().map(|s| s.text.clone()).collect();
                        let full_word_text = original_syllable_texts.join("");

                        if full_word_text.is_empty() {
                            continue;
                        }

                        let converted_full_text = convert(&full_word_text, config);

                        if pinyin_is_same(&full_word_text, &converted_full_text) {
                            let mut converted_chars = converted_full_text.chars();
                            for (i, original_text) in original_syllable_texts.iter().enumerate() {
                                let char_count = original_text.chars().count();
                                let new_syllable_text: String =
                                    converted_chars.by_ref().take(char_count).collect();
                                if let Some(syllable) = word.syllables.get_mut(i) {
                                    syllable.text = new_syllable_text;
                                }
                            }
                        } else {
                            warn!(
                                "词组 '{}' 转换后读音或长度改变 ('{}')，回退到逐音节转换。",
                                full_word_text, converted_full_text,
                            );

                            for syllable in &mut word.syllables {
                                if syllable.text.is_empty() {
                                    continue;
                                }

                                let original_text = &syllable.text;
                                let converted_text_syllable = convert(original_text, config);

                                if pinyin_is_same(original_text, &converted_text_syllable) {
                                    syllable.text = converted_text_syllable;
                                } else {
                                    let char_by_char_converted: String = original_text
                                        .chars()
                                        .map(|c| {
                                            let mut char_str = [0u8; 4];
                                            convert(c.encode_utf8(&mut char_str), config)
                                        })
                                        .collect();

                                    if pinyin_is_same(original_text, &char_by_char_converted) {
                                        syllable.text = char_by_char_converted;
                                    } else {
                                        warn!(
                                            "音节 '{}' 转换后读音改变，逐字转换也无效。保留原文。",
                                            original_text
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyrics_helper_core::{
        AnnotatedTrack, ChineseConversionConfig, ChineseConversionMode, ContentType, LyricLine,
        LyricSyllable, LyricTrack, Word,
    };

    fn new_track_line(text: &str) -> LyricLine {
        let mut line = LyricLine::default();
        line.add_content_track(ContentType::Main, text);
        line
    }

    fn new_syllable_track_line(syllables: Vec<&str>) -> LyricLine {
        let content_track = LyricTrack {
            words: vec![Word {
                syllables: syllables
                    .into_iter()
                    .map(|s| LyricSyllable {
                        text: s.to_string(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        };
        LyricLine {
            tracks: vec![AnnotatedTrack {
                content_type: ContentType::Main,
                content: content_track,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn test_convert_function_simple() {
        let text = "简体中文";
        let config = ChineseConversionConfig::S2t;
        let converted_text = convert(text, config);
        assert_eq!(converted_text, "簡體中文");
    }

    #[test]
    fn test_replace_mode_for_simple_line() {
        let mut lines = vec![new_track_line("我是简体字。")];
        let options = ChineseConversionOptions {
            config: Some(ChineseConversionConfig::S2t),
            mode: ChineseConversionMode::Replace,
            ..Default::default()
        };
        ChineseConversionProcessor::process(&mut lines, &options);
        assert_eq!(lines[0].main_text().unwrap(), "我是簡體字。");
    }

    #[test]
    fn test_replace_mode_syllables_count_unchanged() {
        let mut lines = vec![new_syllable_track_line(vec!["简体", "中文"])];
        let options = ChineseConversionOptions {
            config: Some(ChineseConversionConfig::S2t),
            mode: ChineseConversionMode::Replace,
            ..Default::default()
        };
        ChineseConversionProcessor::process(&mut lines, &options);
        let syllables: Vec<String> = lines[0].tracks[0].content.words[0]
            .syllables
            .iter()
            .map(|s| s.text.clone())
            .collect();
        assert_eq!(syllables, vec!["簡體".to_string(), "中文".to_string()]);
    }

    #[test]
    fn test_replace_mode_syllables_count_changed_fallback() {
        let mut lines = vec![new_syllable_track_line(vec!["我的", "内存"])];
        let options = ChineseConversionOptions {
            config: Some(ChineseConversionConfig::S2twp), // "内存" -> "記憶體"
            mode: ChineseConversionMode::Replace,
            ..Default::default()
        };

        ChineseConversionProcessor::process(&mut lines, &options);

        let syllables: Vec<String> = lines[0].tracks[0].content.words[0]
            .syllables
            .iter()
            .map(|s| s.text.clone())
            .collect();

        // 验证不会被错误地转换为 “記憶體”
        assert_eq!(syllables, vec!["我的".to_string(), "內存".to_string()]);
    }

    #[test]
    fn test_add_translation_mode_success() {
        let mut lines = vec![new_track_line("鼠标和键盘")];
        let options = ChineseConversionOptions {
            config: Some(ChineseConversionConfig::S2twp),
            mode: ChineseConversionMode::AddAsTranslation,
            target_lang_tag: None,
        };

        ChineseConversionProcessor::process(&mut lines, &options);

        let line = &lines[0];
        let translation = line
            .get_translation_by_lang("zh-Hant-TW")
            .expect("应该找到 zh-Hant-TW 翻译");

        assert_eq!(translation.text(), "滑鼠和鍵盤");
    }

    #[test]
    fn test_add_translation_mode_skip_if_exists() {
        let mut line = new_track_line("简体");

        line.add_translation(ContentType::Main, "預設繁體", Some("zh-Hant"));

        let mut lines = vec![line];

        let options = ChineseConversionOptions {
            config: Some(ChineseConversionConfig::S2t),
            mode: ChineseConversionMode::AddAsTranslation,
            target_lang_tag: Some("zh-Hant".to_string()),
        };

        ChineseConversionProcessor::process(&mut lines, &options);

        let processed_line = &lines[0];

        assert_eq!(
            processed_line.main_track().unwrap().translations.len(),
            1,
            "不应添加新的翻译"
        );

        let translation = processed_line
            .get_translation_by_lang("zh-Hant")
            .expect("应该能找到预设的 zh-Hant 翻译");

        assert_eq!(translation.text(), "預設繁體", "已存在的翻译内容不应被改变");
    }

    #[test]
    fn test_add_translation_mode_skip_if_config_is_none() {
        let mut lines = vec![new_track_line("一些文字")];
        let options = ChineseConversionOptions {
            config: None,
            mode: ChineseConversionMode::AddAsTranslation,
            target_lang_tag: None,
        };
        ChineseConversionProcessor::process(&mut lines, &options);
        assert_eq!(lines[0].tracks[0].translations.len(), 0);
    }
}
