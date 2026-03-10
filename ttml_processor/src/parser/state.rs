use lyrics_helper_core::{Agent, AgentStore, AgentType, AnnotatedTrack, ContentType, LyricTrack};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormatDetection {
    #[default]
    Undetermined,
    IsFormatted,
    NotFormatted,
}

/// 主解析器状态机，聚合了所有子状态和全局配置。
#[derive(Debug, Default)]
pub struct TtmlParserState {
    // --- 全局配置与状态 ---
    /// 是否为逐行计时模式。由 `<tt itunes:timing="line">` 或自动检测确定。
    pub is_line_timing_mode: bool,
    /// 标记是否是通过启发式规则（没有找到带时间的span）自动检测为逐行模式。
    pub detected_line_mode: bool,
    /// 标记是否被检测为格式化的 TTML（包含大量换行和缩进）。
    pub format_detection: FormatDetection,
    /// 用于格式化检测的计数器。
    pub whitespace_nodes_with_newline: u32,
    /// 已处理的节点总数，用于格式化检测。
    pub total_nodes_processed: u32,
    /// 默认的主要语言。
    pub default_main_lang: Option<String>,
    /// 默认的翻译语言。
    pub default_translation_lang: Option<String>,
    /// 默认的罗马音语言。
    pub default_romanization_lang: Option<String>,
    /// 通用文本缓冲区，用于临时存储标签内的文本内容。
    pub text_buffer: String,
    /// 文本处理缓冲区，用于优化字符串处理。
    pub text_processing_buffer: String,

    pub in_metadata: bool,
    /// 存储 `<metadata>` 区域解析状态的结构体。
    pub metadata_state: MetadataParseState,
    /// 存储 `<body>` 和 `<p>` 区域解析状态的结构体。
    pub body_state: BodyParseState,

    /// 用于存储正在构建的 `AgentStore`
    pub agent_store: AgentStore,
    /// 用于为在 `<p>` 标签中直接发现的 `agent` 名称生成新ID的计数器
    pub agent_counter: u32,
    /// 用于存储已为直接名称生成的 `ID` 映射 (`name` -> `id`)
    pub agent_name_to_id_map: HashMap<String, String>,
}

impl TtmlParserState {
    /// 根据 `<p>` 标签中的 `agent` 属性值，查找或创建一个 Agent ID。
    ///
    /// 这个方法会：
    /// 1. 检查值是否为已知的 ID。
    /// 2. 如果不是 ID，则检查是否为已知的名称。
    /// 3. 如果两者都不是，则创建一个新的 Agent 记录和 ID。
    pub fn resolve_agent_id(&mut self, agent_attr_val: Option<String>) -> Option<String> {
        let val = agent_attr_val?;

        if self.agent_store.agents_by_id.contains_key(&val) {
            return Some(val);
        }

        if let Some(existing_id) = self.agent_name_to_id_map.get(&val) {
            return Some(existing_id.clone());
        }

        self.agent_counter += 1;
        let new_id = format!("v{}", self.agent_counter);

        self.agent_name_to_id_map
            .insert(val.clone(), new_id.clone());

        let new_agent = Agent {
            id: new_id.clone(),
            name: Some(val),
            agent_type: AgentType::Person,
        };
        self.agent_store
            .agents_by_id
            .insert(new_id.clone(), new_agent);

        Some(new_id)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AuxTrackType {
    Translation,
    Romanization,
}

#[derive(Debug, Default)]
pub enum MetadataContext {
    #[default]
    None, // 不在任何特殊元数据标签内
    InAgent {
        id: Option<String>,
    },
    InITunesMetadata,
    InSongwriter,
    InAuxiliaryContainer {
        // 代表 <translations> 或 <transliterations>
        aux_type: AuxTrackType,
    },
    InAuxiliaryEntry {
        // 代表 <translation> 或 <transliteration>
        aux_type: AuxTrackType,
        lang: Option<String>,
    },
    InAuxiliaryText {
        // 代表 <text>
        aux_type: AuxTrackType,
        lang: Option<String>,
        key: Option<String>,
    },
}

#[derive(Debug, Default, Clone)]
pub struct AuxiliaryTrackSet {
    pub translations: Vec<LyricTrack>,
    pub romanizations: Vec<LyricTrack>,
}

#[derive(Debug, Default, Clone)]
pub struct DetailedAuxiliaryTracks {
    pub main_tracks: AuxiliaryTrackSet,
    pub background_tracks: AuxiliaryTrackSet,
}

/// 在 `<p>` 或 `<text>` 标签内解析到的内容
#[derive(Debug, Clone)]
pub enum PendingItem {
    Syllable {
        text: String,
        start_ms: u64,
        end_ms: u64,
        content_type: ContentType,
    },
    FreeText(String),
}

/// 存储 `<metadata>` 区域解析状态的结构体。
#[derive(Debug, Default)]
pub struct MetadataParseState {
    pub line_translation_map: HashMap<String, Vec<(LineTranslation, Option<String>)>>,
    pub timed_track_map: HashMap<String, DetailedAuxiliaryTracks>,

    pub context: MetadataContext,
    pub pending_items: Vec<PendingItem>,

    pub current_main_plain_text: String,
    pub current_bg_plain_text: String,

    pub span_stack: Vec<SpanContext>,
    pub text_buffer: String,
}

/// 存储 `<body>` 和 `<p>` 区域解析状态的结构体。
#[derive(Debug, Default)]
pub struct BodyParseState {
    pub in_body: bool,
    pub in_div: bool,
    pub in_p: bool,
    /// 当前 `<div>` 的 `itunes:song-part` 属性，会被子 `<p>` 继承。
    pub current_div_song_part: Option<String>,
    /// 存储当前正在处理的 `<p>` 元素的临时数据。
    pub current_p_element_data: Option<CurrentPElementData>,
    /// `<span>` 标签的上下文堆栈，用于处理嵌套的 span。
    pub span_stack: Vec<SpanContext>,
}

/// 存储当前处理的 `<p>` 元素解析过程中的临时数据。
#[derive(Debug, Default)]
pub struct CurrentPElementData {
    pub start_ms: u64,
    pub end_ms: u64,
    pub agent: Option<String>,
    pub song_part: Option<String>,
    pub itunes_key: Option<String>,
    pub tracks_accumulator: Vec<AnnotatedTrack>,
    pub pending_items: Vec<PendingItem>,
}

/// 代表当前 `<span>` 的上下文信息，用于处理嵌套和内容分类。
#[derive(Debug, Clone)]
pub struct SpanContext {
    pub role: SpanRole,
    pub lang: Option<String>,   // xml:lang 属性
    pub scheme: Option<String>, // xml:scheme 属性
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
}

/// 定义 `<span>` 标签可能扮演的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanRole {
    /// 普通音节
    Generic,
    /// 翻译
    Translation,
    /// 罗马音
    Romanization,
    /// 背景人声容器
    Background,
}

/// 用于存储从 `<head>` 中解析的逐行翻译。
#[derive(Debug, Default, Clone)]
pub struct LineTranslation {
    pub main: Option<String>,
    pub background: Option<String>,
}
