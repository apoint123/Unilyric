use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{LyricFormat, ParsedSourceData};

/// 批量加载文件的唯一标识符。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BatchFileId(pub u64);
impl Default for BatchFileId {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchFileId {
    /// 生成一个新的唯一 `BatchFileId`。
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        // 使用静态原子计数器确保ID的唯一性。
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        BatchFileId(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

/// 表示在批量转换模式下加载的单个文件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchLoadedFile {
    /// 文件的唯一ID。
    pub id: BatchFileId,
    /// 文件的完整路径。
    pub path: PathBuf,
    /// 从路径中提取的文件名。
    pub filename: String,
}
impl BatchLoadedFile {
    /// 根据文件路径创建一个新的 `BatchLoadedFile` 实例。
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        Self {
            id: BatchFileId::new(),
            path,
            filename,
        }
    }
}

/// 表示批量转换中单个条目的状态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BatchEntryStatus {
    /// 等待转换。
    Pending,
    /// 准备好进行转换。
    ReadyToConvert,
    /// 正在转换中。
    Converting,
    /// 转换完成。
    Completed {
        /// 输出文件的路径。
        output_path: PathBuf,
        /// 转换过程中产生的警告信息。
        warnings: Vec<String>,
    },
    /// 转换失败。
    Failed(String),
    /// 跳过转换，通常因为在配对逻辑中未能找到匹配的主歌词文件（针对辅助歌词文件）。
    SkippedNoMatch,
}

/// 批量转换配置的唯一标识符。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BatchConfigId(pub u64);
impl Default for BatchConfigId {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchConfigId {
    /// 生成一个新的唯一 `BatchConfigId`。
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        BatchConfigId(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

/// 表示单个批量转换任务的配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchConversionConfig {
    /// 配置的唯一ID。
    pub id: BatchConfigId,
    /// 主歌词文件的ID。
    pub main_lyric_id: BatchFileId,
    /// 关联的翻译歌词文件的ID列表。
    pub translation_lyric_ids: Vec<BatchFileId>,
    /// 关联的罗马音文件的ID列表。
    pub romanization_lyric_ids: Vec<BatchFileId>,
    /// 目标输出格式。
    pub target_format: LyricFormat,
    /// 用于UI预览的输出文件名（实际输出路径在任务执行时结合输出目录确定）。
    pub output_filename_preview: String,
    /// 当前转换任务的状态。
    pub status: BatchEntryStatus,
    /// 如果任务失败，存储相关的错误信息。
    pub last_error: Option<String>,
}

impl BatchConversionConfig {
    /// 创建一个新的 `BatchConversionConfig` 实例。
    #[must_use]
    pub fn new(
        main_lyric_id: BatchFileId,
        target_format: LyricFormat,
        output_filename: String,
    ) -> Self {
        Self {
            id: BatchConfigId::new(),
            main_lyric_id,
            translation_lyric_ids: Vec::new(),
            romanization_lyric_ids: Vec::new(),
            target_format,
            output_filename_preview: output_filename,
            status: BatchEntryStatus::Pending,
            last_error: None,
        }
    }
}

/// 用于在Rust后端内部传递批量转换任务状态更新的消息。
#[derive(Debug, Clone)]
pub struct BatchTaskUpdate {
    /// 关联的批量转换配置ID。
    pub entry_config_id: BatchConfigId,
    /// 更新后的任务状态。
    pub new_status: BatchEntryStatus,
}

/// 用于表示传递给核心转换函数的单个输入文件的信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputFile {
    /// 文件内容字符串。
    pub content: String,
    /// 该文件内容的歌词格式。
    pub format: LyricFormat,
    /// 可选的语言代码。
    /// 对于翻译或罗马音文件，指示其语言或具体方案。
    pub language: Option<String>,
    /// 可选的原始文件名。
    /// 可用于日志记录、元数据提取或某些特定转换逻辑。
    pub filename: Option<String>,
}

impl InputFile {
    /// 创建一个新的 `InputFile` 实例。
    ///
    /// 这是一个便利的构造函数，用于简化 `InputFile` 对象的创建，
    /// 使其在库的顶层 API (如 `lib.rs` 的示例) 中更易于使用。
    ///
    /// # 参数
    /// * `content` - 歌词文件的原始文本内容。
    /// * `format` - 歌词的格式 (`LyricFormat` 枚举)。
    /// * `language` - 可选的语言代码 (BCP-47 格式，例如 "zh-Hans")。
    /// * `filename` - 可选的原始文件名，用于提供上下文。
    #[must_use]
    pub fn new(
        content: String,
        format: LyricFormat,
        language: Option<String>,
        filename: Option<String>,
    ) -> Self {
        Self {
            content,
            format,
            language,
            filename,
        }
    }
}

impl Default for InputFile {
    ///
    /// 创建一个默认的 `InputFile` 实例。
    ///
    /// 这对于某些场景下需要一个“占位符”或空的 `InputFile` 实例非常有用。
    /// 默认值包括：
    /// - `content`: 空字符串
    /// - `format`: `LyricFormat` 的默认值，即 TTML
    /// - `language`: None
    /// - `filename`: None
    ///
    fn default() -> Self {
        Self {
            content: String::new(),
            format: LyricFormat::default(),
            language: None,
            filename: None,
        }
    }
}

/// 封装了调用核心歌词转换函数所需的所有输入参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionInput {
    /// 主歌词文件信息。
    pub main_lyric: InputFile,
    /// 翻译文件信息列表。每个 `InputFile` 包含内容、格式（通常是LRC）和语言。
    pub translations: Vec<InputFile>,
    /// 罗马音/音译文件信息列表。每个 `InputFile` 包含内容、格式（通常是LRC）和语言/方案。
    pub romanizations: Vec<InputFile>,
    /// 目标歌词格式。
    pub target_format: LyricFormat,
    /// 可选的用户指定的元数据覆盖（原始键值对）。
    pub user_metadata_overrides: Option<HashMap<String, Vec<String>>>,
    // /// 可选的应用级别的固定元数据规则（原始键值对）。
    // pub fixed_metadata_rules: Option<HashMap<String, Vec<String>>>,
}

// =============================================================================
// 8. 转换任务入口结构体
// =============================================================================

/// 用于批量转换的输入参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchInput {
    /// 包含源歌词文件的输入目录。
    pub input_dir: PathBuf,
    /// 用于保存转换后文件的输出目录。
    pub output_dir: PathBuf,
    /// 所有任务的目标输出格式。
    pub target_format: LyricFormat,
}

/// 表示一个转换任务，可以是单个文件或批量处理。
#[derive(Debug, Clone)]
pub enum ConversionTask {
    /// 单个转换任务，输入为内存中的内容。
    Single(ConversionInput),
    /// 批量转换任务，输入为文件目录。
    Batch(BatchInput),
}

/// 表示转换操作的输出结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConversionResult {
    /// 单个文件转换的结果，为一个字符串。
    Single(String),
    /// 批量转换的结果，为所有任务的最终状态列表。
    Batch(Vec<BatchConversionConfig>),
}

/// 包含完整转换结果的结构体。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullConversionResult {
    /// 最终生成的歌词字符串。
    pub output_lyrics: String,
    /// 在转换开始时从输入解析出的源数据。
    pub source_data: ParsedSourceData,
}
