use dashmap::DashMap;
use ferrous_opencc::OpenCC;
use ferrous_opencc::config::BuiltinConfig as OpenccConfig;
use lyrics_helper_core::ChineseConversionConfig;
use std::sync::{Arc, LazyLock};

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

pub fn convert(text: &str, config: ChineseConversionConfig) -> String {
    if text.is_empty() {
        return String::new();
    }

    let opencc_config = to_opencc_config(config);
    let cache_key = opencc_config.to_filename();

    if let Some(converter) = CONVERTER_CACHE.get(cache_key) {
        return converter.convert(text);
    }

    CONVERTER_CACHE
        .entry(cache_key.to_string())
        .or_try_insert_with(|| OpenCC::from_config(opencc_config).map(Arc::new))
        .map_or_else(
            |_| text.to_string(),
            |converter_ref| converter_ref.value().convert(text),
        )
}
