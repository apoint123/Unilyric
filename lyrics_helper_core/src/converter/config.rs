use bitflags::bitflags;
use derive_builder::Builder;
use serde::{Deserialize, Serialize};

use crate::LyricFormat;

/// TTML 生成时的计时模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TtmlTimingMode {
    #[default]
    /// 逐字计时
    Word,
    /// 逐行计时
    Line,
}

/// TTML 解析选项
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TtmlParsingOptions {
    /// 当TTML本身未指定语言时，解析器可以使用的默认语言。
    #[serde(default)]
    pub default_languages: DefaultLanguageOptions,

    /// 强制指定计时模式，忽略文件内的 `itunes:timing` 属性和自动检测逻辑。
    #[serde(default)]
    pub force_timing_mode: Option<TtmlTimingMode>,
}

/// TTML 生成选项
#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
#[builder(setter(into), default)]
pub struct TtmlGenerationOptions {
    /// 生成的计时模式（逐字或逐行）。
    pub timing_mode: TtmlTimingMode,
    /// 指定输出 TTML 的主语言 (xml:lang)。如果为 None，则尝试从元数据推断。
    pub main_language: Option<String>,
    /// 为内联的翻译 `<span>` 指定默认语言代码。
    pub translation_language: Option<String>,
    /// 为内联的罗马音 `<span>` 指定默认语言代码。
    pub romanization_language: Option<String>,
    /// 是否遵循 Apple Music 的特定格式规则（例如，将翻译写入`<head>`而不是内联）。
    pub use_apple_format_rules: bool,
    /// 是否输出格式化的 TTML 文件。
    pub format: bool,
    /// 是否启用自动分词功能。
    pub auto_word_splitting: bool,
    /// 自动分词时，一个标点符号所占的权重（一个字符的权重为1.0）。
    pub punctuation_weight: f64,
}

impl Default for TtmlGenerationOptions {
    fn default() -> Self {
        Self {
            timing_mode: TtmlTimingMode::Word,
            main_language: None,
            translation_language: None,
            romanization_language: None,
            use_apple_format_rules: false,
            format: false,
            auto_word_splitting: false,
            punctuation_weight: 0.3,
        }
    }
}

/// TTML 解析时使用的默认语言选项
/// 当TTML本身未指定语言时，解析器可以使用这些值。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DefaultLanguageOptions {
    /// 默认主语言代码
    pub main: Option<String>,
    /// 默认翻译语言代码
    pub translation: Option<String>,
    /// 默认罗马音语言代码
    pub romanization: Option<String>,
}

/// LQE 生成选项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LqeGenerationOptions {
    /// 用于 [lyrics] 区块的格式
    pub main_lyric_format: LyricFormat,
    /// 用于 [translation] 和 [pronunciation] 区块的格式
    pub auxiliary_format: LyricFormat,
}

impl Default for LqeGenerationOptions {
    fn default() -> Self {
        Self {
            main_lyric_format: LyricFormat::Lys,
            auxiliary_format: LyricFormat::Lrc,
        }
    }
}

/// 定义辅助歌词（翻译、音译）与主歌词的匹配策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuxiliaryLineMatchingStrategy {
    /// 精确匹配：要求时间戳完全相同。对时间差异敏感。
    Exact,
    /// 容差匹配：在预设的时间窗口内寻找匹配。
    Tolerance {
        /// 匹配时允许的最大时间差（毫秒）。
        tolerance_ms: u64,
    },
    /// 同步匹配：假定主歌词和辅助歌词都按时间排序，使用双指针算法在时间窗口内匹配。
    SortedSync {
        /// 匹配时允许的最大时间差（毫秒）。
        tolerance_ms: u64,
    },
}

impl Default for AuxiliaryLineMatchingStrategy {
    fn default() -> Self {
        Self::SortedSync { tolerance_ms: 20 }
    }
}

/// 指定LRC中具有相同时间戳的行的角色
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LrcLineRole {
    /// 主歌词
    Main,
    /// 翻译
    Translation,
    /// 罗马音
    Romanization,
}

/// 定义如何处理LRC中具有相同时间戳的多行歌词的策略
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LrcSameTimestampStrategy {
    /// [默认] 将文件顺序中的第一行视为主歌词，其余的都视为翻译。
    #[default]
    FirstIsMain,
    /// 将每一行都视为一个独立的、并列的主歌词轨道。
    AllAreMain,
    /// 根据用户提供的角色列表，按顺序为每一行分配角色。
    /// 列表的长度应与具有相同时间戳的行数相匹配。
    UseRoleOrder(Vec<LrcLineRole>),
}

/// LRC 解析选项
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LrcParsingOptions {
    /// 定义如何处理具有相同时间戳的多行歌词的策略。
    #[serde(default)]
    pub same_timestamp_strategy: LrcSameTimestampStrategy,
}

/// 统一管理所有格式的转换选项
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConversionOptions {
    /// TTML 生成选项
    pub ttml: TtmlGenerationOptions,
    /// TTML 解析选项
    #[serde(default)]
    pub ttml_parsing: TtmlParsingOptions,
    /// LQE 转换选项
    #[serde(default)]
    pub lqe: LqeGenerationOptions,
    /// ASS 转换选项
    pub ass: AssGenerationOptions,
    /// LRC 转换选项
    #[serde(default)]
    pub lrc: LrcGenerationOptions,
    /// LRC 解析选项
    #[serde(default)]
    pub lrc_parsing: LrcParsingOptions,
    /// 元数据移除选项
    pub metadata_stripper: MetadataStripperOptions,
    /// 简繁转换选项
    #[serde(default)]
    pub chinese_conversion: ChineseConversionOptions,
    /// 辅助歌词（如翻译）的匹配策略
    #[serde(default)]
    pub matching_strategy: AuxiliaryLineMatchingStrategy,
}

/// ASS 生成转换选项
#[derive(Debug, Clone, Default, Deserialize, Serialize, Builder)]
#[builder(setter(into), default)]
pub struct AssGenerationOptions {
    /// 自定义的 [Script Info] 部分内容。如果为 `None`，则使用默认值。
    /// 用户提供的内容应包含 `[Script Info]` 头部。
    pub script_info: Option<String>,
    /// 自定义的 [V4+ Styles] 部分内容。如果为 `None`，则使用默认值。
    /// 用户提供的内容应包含 `[V4+ Styles]` 头部和 `Format:` 行。
    pub styles: Option<String>,
}

bitflags! {
    /// 元数据清理器的配置标志
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct MetadataStripperFlags: u8 {
        /// 启用元数据清理功能
        const ENABLED                 = 1 << 0;
        /// 启用基于正则表达式的行移除
        const ENABLE_REGEX_STRIPPING  = 1 << 1;
    }
}

impl Default for MetadataStripperFlags {
    fn default() -> Self {
        Self::ENABLED | Self::ENABLE_REGEX_STRIPPING
    }
}

/// 元数据扫描行数的限制
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanLimitConfig {
    /// 扫描行数的比例 (例如, 0.1 表示 10%)。
    pub ratio: f32,
    /// 扫描的最小行数
    pub min_lines: usize,
    /// 扫描的最大行数
    pub max_lines: usize,
}

impl ScanLimitConfig {
    pub fn calculate(&self, total_lines: usize) -> usize {
        let proportional_lines = (total_lines as f32 * self.ratio).ceil() as usize;

        proportional_lines
            .max(self.min_lines)
            .min(self.max_lines)
            .min(total_lines)
    }
}

fn default_header_scan_limit() -> ScanLimitConfig {
    ScanLimitConfig {
        ratio: 0.2,
        min_lines: 20,
        max_lines: 70,
    }
}

fn default_footer_scan_limit() -> ScanLimitConfig {
    ScanLimitConfig {
        ratio: 0.2,
        min_lines: 20,
        max_lines: 50,
    }
}

/// 配置元数据行清理器的选项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataStripperOptions {
    /// 用于控制清理器行为的位标志。
    #[serde(default)]
    pub flags: MetadataStripperFlags,

    /// 用于匹配头部/尾部块的关键词列表。
    #[serde(default)]
    pub keywords: Vec<String>,

    /// 正则表达式列表。
    ///
    /// 匹配后，会移除开头或结尾到该行的所有内容。
    #[serde(default)]
    pub regex_patterns: Vec<String>,

    /// 头部扫描的行数限制。
    #[serde(default = "default_header_scan_limit")]
    pub header_scan_limit: ScanLimitConfig,

    /// 尾部扫描的行数限制。
    #[serde(default = "default_footer_scan_limit")]
    pub footer_scan_limit: ScanLimitConfig,
}

impl Default for MetadataStripperOptions {
    fn default() -> Self {
        Self {
            flags: Default::default(),
            keywords: Vec::new(),
            regex_patterns: Vec::new(),
            header_scan_limit: default_header_scan_limit(),
            footer_scan_limit: default_footer_scan_limit(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChineseConversionConfig {
    /// 简体到繁体
    S2t = 0,
    /// 繁体到简体
    T2s = 1,
    /// 简体到台湾正体
    S2tw = 2,
    /// 台湾正体到简体
    Tw2s = 3,
    /// 简体到香港繁体
    S2hk = 4,
    /// 香港繁体到简体
    Hk2s = 5,
    /// 简体到台湾正体（包含词汇转换）
    S2twp = 6,
    /// 台湾正体（包含词汇转换）到简体
    Tw2sp = 7,
    /// 繁体到台湾正体
    T2tw = 8,
    /// 台湾正体到繁体
    Tw2t = 9,
    /// 繁体到香港繁体
    T2hk = 10,
    /// 香港繁体到繁体
    Hk2t = 11,
    /// 日语新字体到繁体
    Jp2t = 12,
    /// 繁体到日语新字体
    T2jp = 13,
}

impl ChineseConversionConfig {
    /// 推断配置对应的目标语言标签
    pub fn deduce_lang_tag(self) -> Option<&'static str> {
        use ChineseConversionConfig::*;
        match self {
            S2t | Jp2t | Hk2t | T2tw | Tw2t => Some("zh-Hant"),
            S2tw | S2twp => Some("zh-Hant-TW"),
            S2hk | T2hk => Some("zh-Hant-HK"),
            T2s | Tw2s | Tw2sp | Hk2s => Some("zh-Hans"),
            T2jp => Some("ja"),
        }
    }
}

/// 简繁转换的配置选项
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChineseConversionOptions {
    /// 指定要使用的 `OpenCC` 配置。
    /// 当值为 `Some(config)` 时，功能启用。
    pub config: Option<ChineseConversionConfig>,

    /// 为翻译指定 BCP 47 语言标签，例如 "zh-Hant" 或 "zh-Hant-HK"。
    /// 如果未指定，将根据配置自动推断。
    pub target_lang_tag: Option<String>,

    /// 指定转换模式，默认为直接替换
    #[serde(default)]
    pub mode: ChineseConversionMode,
}

/// 简繁转换的模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ChineseConversionMode {
    /// 直接替换原文
    #[default]
    Replace,
    /// 作为翻译条目添加
    AddAsTranslation,
}

/// LRC 生成时，背景人声的输出方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LrcSubLinesOutputMode {
    /// 默认忽略所有背景人声，只输出主歌词
    #[default]
    Ignore,
    /// 将子行用括号合并到主行中，如 "主歌词 (背景人声)"
    MergeWithParentheses,
    /// 将背景人声行作为独立的、带时间戳的歌词行输出
    SeparateLines,
}

/// LRC 生成时，行结束时间标记 `[mm:ss.xx]` 的输出方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LrcEndTimeOutputMode {
    /// [默认] 不输出任何结束时间标记
    #[default]
    Never,
    /// 为每一行歌词都输出一个结束时间标记
    Always,
    /// 仅在当前行与下一行的时间间隔超过阈值时，才输出结束标记
    OnLongPause {
        /// 触发输出的最小暂停时长（毫秒）
        threshold_ms: u64,
    },
}

/// LRC 生成选项
#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
#[builder(setter(into), default)]
pub struct LrcGenerationOptions {
    /// 控制背景人声的输出方式
    pub sub_lines_output_mode: LrcSubLinesOutputMode,
    /// 控制行结束时间标记的输出方式
    pub end_time_output_mode: LrcEndTimeOutputMode,
}

impl Default for LrcGenerationOptions {
    fn default() -> Self {
        Self {
            sub_lines_output_mode: LrcSubLinesOutputMode::Ignore,
            end_time_output_mode: LrcEndTimeOutputMode::Never,
        }
    }
}

// =============================================================================
// 9. 平滑优化选项
// =============================================================================

/// 控制平滑优化的选项。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Builder)]
#[builder(setter(into), default)]
pub struct SyllableSmoothingOptions {
    /// 用于平滑的因子 (0.0 ~ 0.5)。
    pub factor: f64,
    /// 用于分组的时长差异阈值（毫秒）。
    pub duration_threshold_ms: u64,
    /// 用于分组的间隔阈值（毫秒）。
    pub gap_threshold_ms: u64,
    /// 组内平滑的次数。
    pub smoothing_iterations: u32,
}

impl Default for SyllableSmoothingOptions {
    fn default() -> Self {
        Self {
            factor: 0.15,
            duration_threshold_ms: 50,
            gap_threshold_ms: 100,
            smoothing_iterations: 5,
        }
    }
}
