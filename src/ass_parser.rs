// 导入 once_cell::sync::Lazy 用于延迟初始化静态正则表达式。
use once_cell::sync::Lazy;
// 导入 regex::Regex 用于正则表达式操作。
use regex::Regex;
// 导入标准库的 HashMap 用于存储键值对数据，例如演唱者名称映射。
use std::collections::HashMap;

// 导入项目中定义的各种类型：
// ActorRole: 定义演唱者角色（如主唱1, 主唱2, 背景, 合唱）。
// AssLineContent: 枚举，表示解析后的一行ASS内容（歌词行, 翻译, 罗马音等）。
// AssLineInfo: 结构体，存储解析后的一行ASS的完整信息（行号, 时间, 内容等）。
// AssMetadata: 结构体，存储从Comment行解析的元数据。
// AssSyllable: 结构体，表示ASS卡拉OK标签 `\k` 分割的音节及其计时。
// ConvertError: 错误处理枚举。
// MarkerInfo: 存储标记信息 (行号, 文本)。
// ParsedActor: 结构体，存储从Actor字段解析出的角色、语言代码等信息。
// ProcessedAssData: 结构体，包含解析整个ASS文件后的所有数据。
use crate::types::{
    ActorRole, AssLineContent, AssLineInfo, AssMetadata, AssSyllable, ConvertError, MarkerInfo,
    ParsedActor, ProcessedAssData,
};

// 静态正则表达式，用于解析ASS时间戳字符串 (H:MM:SS.CS)。
// - `(\d+)`: 捕获小时 (1位或多位数字)。
// - `(\d{2})`: 捕获分钟 (2位数字)。
// - `(\d{2})`: 捕获秒 (2位数字)。
// - `(\d{2})`: 捕获厘秒 (2位数字)。
static ASS_TIME_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(\d+):(\d{2}):(\d{2})\.(\d{2})").expect("未能编译 ASS_TIME_REGEX") // 正则表达式编译失败时的错误信息
});

// 静态正则表达式，用于解析ASS文本中的卡拉OK标签 `{\k<duration_cs>}`。
// - `\{\\k`: 匹配 `{\k` 字面量。
// - `(\d+)`: 捕获 `\k` 标签的持续时间值 (厘秒)。
// - `\}`: 匹配 `}` 字面量。
static KARAOKE_TAG_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\{\\k(\d+)\}").expect("未能编译 KARAOKE_TAG_REGEX")); // 正则表达式编译失败时的错误信息

// 静态正则表达式，用于解析ASS文件中 [Events] 部分的 Dialogue 或 Comment 行。
// 使用命名捕获组 (?P<GroupName>...) 提取各字段。
static ASS_LINE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r"^(?P<Type>Comment|Dialogue):\s*", // 行类型 (Comment 或 Dialogue)
        r"(?P<Layer>\d+)\s*,",              // Layer (图层)
        r"(?P<Start>\d+:\d{2}:\d{2}\.\d{2})\s*,", // Start Time (开始时间)
        r"(?P<End>\d+:\d{2}:\d{2}\.\d{2})\s*,", // End Time (结束时间)
        r"(?P<Style>[^,]*?)\s*,",           // Style (样式名)
        r"(?P<Actor>[^,]*?)\s*,",           // Name/Actor (演唱者/角色名)
        r"[^,]*,[^,]*,[^,]*,",              // MarginL, MarginR, MarginV (忽略这三个边距字段)
        r"(?P<Effect>[^,]*?)\s*,",          // Effect (效果)
        r"(?P<Text>.*?)\s*$"                // Text (文本内容)
    ))
    .expect("未能编译 ASS_LINE_REGEX") // 正则表达式编译失败时的错误信息
});

// 静态正则表达式，用于从 Actor 字段中解析 iTunes的歌曲组成部分。
// 例如: itunes:song-part="verse 1" 或 itunes:song-part='chorus' 或 itunes:song-part=bridge
static SONG_PART_DIRECTIVE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"itunes:song-part=(?:"([^"]*)"|'([^']*)'|([^\s"']+))"#)
        .expect("未能编译 SONG_PART_DIRECTIVE_REGEX") // 正则表达式编译失败时的错误信息
});

/// 解析 ASS 时间字符串 (H:MM:SS.CS) 并将其转换为总毫秒数。
///
/// # Arguments
/// * `time_str` - `&str` 类型，要解析的ASS时间字符串。
/// * `line_num` - `usize` 类型，当前行号，用于错误报告。
///
/// # Returns
/// `Result<u64, ConvertError>` - 如果解析成功，返回总毫秒数；否则返回错误。
pub fn parse_ass_time(time_str: &str, line_num: usize) -> Result<u64, ConvertError> {
    ASS_TIME_REGEX.captures(time_str).map_or_else(
        // 如果正则表达式不匹配，返回无效ASS时间错误。
        || Err(ConvertError::InvalidAssTime(line_num, time_str.to_string())),
        // 如果匹配成功，则提取各部分并计算总毫秒数。
        |caps| {
            // caps[1] 到 caps[4] 分别对应小时、分钟、秒、厘秒的捕获组。
            let h: u64 = caps[1].parse().map_err(ConvertError::ParseInt)?; // 解析小时
            let m: u64 = caps[2].parse().map_err(ConvertError::ParseInt)?; // 解析分钟
            let s: u64 = caps[3].parse().map_err(ConvertError::ParseInt)?; // 解析秒
            let cs: u64 = caps[4].parse().map_err(ConvertError::ParseInt)?; // 解析厘秒
            // 计算总毫秒数: (小时*3600 + 分钟*60 + 秒)*1000 + 厘秒*10。
            Ok(h * 3_600_000 + m * 60_000 + s * 1000 + cs * 10)
        },
    )
}

/// 解析包含卡拉OK标签 `{\k<duration>}` 的ASS文本，将其分解为带时间信息的音节。
///
/// # Arguments
/// * `text` - `&str` 类型，包含卡拉OK标签的文本内容。
/// * `line_start_ms` - `u64` 类型，该行歌词的起始毫秒时间。
/// * `line_num` - `usize` 类型，当前行号，用于错误报告。
///
/// # Returns
/// `Result<(Vec<AssSyllable>, u64), ConvertError>` - 如果成功，返回一个元组：
///   - `Vec<AssSyllable>`: 解析出的音节列表。
///   - `u64`: 根据卡拉OK标签计算出的该行歌词的实际结束毫秒时间。
///     如果失败，则返回错误。
pub fn parse_karaoke_text(
    text: &str,
    line_start_ms: u64,
    line_num: usize,
) -> Result<(Vec<AssSyllable>, u64), ConvertError> {
    let mut syllables = Vec::new(); // 存储解析出的音节。
    let mut current_char_pos = 0; // 当前在文本中处理到的字符位置索引。
    let mut current_time_ms = line_start_ms; // 当前音节的开始时间，初始为行开始时间。
    let mut max_end_time_ms = line_start_ms; // 追踪通过K标签计算出的最晚结束时间。
    // `previous_duration_cs` 存储的是上一个文本片段（音节）对应的 `\k` 标签时长。
    // 第一个文本片段之前的 `\k` 标签（如果有）决定了第一个文本片段的显示时长。
    let mut previous_duration_cs: u32 = 0;

    // 遍历文本中所有匹配到的卡拉OK标签 `{\k...}`。
    for cap in KARAOKE_TAG_REGEX.captures_iter(text) {
        // `tag_match` 是整个 `{\k...}` 标签的匹配结果。
        let tag_match = cap.get(0).ok_or_else(|| {
            ConvertError::Internal(format!("行 {}: 无法提取K标签的完整匹配", line_num))
        })?;
        // `duration_cs_str` 是 `\k` 标签中的数字部分（厘秒时长）。
        let duration_cs_str = cap
            .get(1)
            .ok_or_else(|| {
                ConvertError::Internal(format!("行 {}: 无法从K标签提取时长值", line_num))
            })?
            .as_str();
        // 将时长字符串解析为 u32 类型的厘秒。
        let current_k_duration_cs: u32 = duration_cs_str.parse().map_err(|_| {
            ConvertError::InvalidAssKaraoke(line_num, format!("无效的K值: {}", duration_cs_str))
        })?;

        // `text_slice` 是上一个 `\k` 标签结束处到当前 `\k` 标签开始处之间的文本。
        // 这个文本片段的显示时长由 `previous_duration_cs` 决定。
        let text_slice = &text[current_char_pos..tag_match.start()];
        if !text_slice.is_empty() {
            // 如果文本片段非空
            let ends_with_space_flag = text_slice.ends_with(|c: char| c.is_whitespace()); // 检查片段是否以空白结尾
            let syllable_text = text_slice.trim_end().to_string(); // 去除尾部空白作为音节文本

            if !syllable_text.is_empty() {
                // 如果去除空白后音节文本仍非空
                let syllable_duration_ms = previous_duration_cs as u64 * 10; // 音节时长 = 上一个K标签值 * 10ms
                let syllable_end_ms = current_time_ms + syllable_duration_ms;
                syllables.push(AssSyllable {
                    text: syllable_text,
                    start_ms: current_time_ms,
                    end_ms: syllable_end_ms,
                    ends_with_space: ends_with_space_flag,
                });
                current_time_ms = syllable_end_ms; // 更新当前时间为该音节的结束时间
                max_end_time_ms = max_end_time_ms.max(syllable_end_ms); // 更新最晚结束时间
            } else {
                // 如果去除空白后音节文本为空（即原 text_slice 只包含空白）
                let whitespace_duration_ms = previous_duration_cs as u64 * 10;
                if whitespace_duration_ms > 0 {
                    current_time_ms += whitespace_duration_ms; // 空白也消耗时间
                    max_end_time_ms = max_end_time_ms.max(current_time_ms);
                    // 如果这个空白音节前有实际音节，则标记前一个音节以空格结尾。
                    if let Some(last_syllable) = syllables.last_mut() {
                        last_syllable.ends_with_space = true;
                    }
                }
            }
        } else {
            // 如果 `text_slice` 为空 (例如 `{\k10}{\k20}text` 中第一个 `\k10` 之后的部分)
            // 这意味着 `previous_duration_cs` 对应的是一个没有文本的延时。
            let syllable_duration_ms = previous_duration_cs as u64 * 10;
            if syllable_duration_ms > 0 {
                current_time_ms += syllable_duration_ms; // 时间照常推进
                max_end_time_ms = max_end_time_ms.max(current_time_ms);
            }
        }
        // 当前 `\k` 标签的 `current_k_duration_cs` 将用于下一个文本片段的计时。
        previous_duration_cs = current_k_duration_cs;
        // 更新字符处理位置到当前 `\k` 标签的末尾。
        current_char_pos = tag_match.end();
    }

    // 处理最后一个 `\k` 标签之后剩余的文本。
    let remaining_text_slice = &text[current_char_pos..];
    if !remaining_text_slice.is_empty() {
        let ends_with_space = remaining_text_slice.ends_with(|c: char| c.is_whitespace());
        let remaining_text = remaining_text_slice.trim_end().to_string();
        if !remaining_text.is_empty() {
            // 这最后一段文本的持续时间由它前面的那个 `\k` 标签 (即 `previous_duration_cs`) 决定。
            let syllable_duration_ms = previous_duration_cs as u64 * 10;
            let syllable_end_ms = current_time_ms + syllable_duration_ms;
            syllables.push(AssSyllable {
                text: remaining_text,
                start_ms: current_time_ms,
                end_ms: syllable_end_ms,
                ends_with_space,
            });
            max_end_time_ms = max_end_time_ms.max(syllable_end_ms);
        } else {
            // 如果剩余部分只有空白
            let whitespace_duration_ms = previous_duration_cs as u64 * 10;
            if whitespace_duration_ms > 0 {
                max_end_time_ms = max_end_time_ms.max(current_time_ms + whitespace_duration_ms);
                if let Some(last_syllable) = syllables.last_mut() {
                    last_syllable.ends_with_space = true;
                }
            }
        }
    } else {
        // 如果文本以 `\k` 标签结尾 (例如 `text{\k10}`)
        // 这个最后的 `\k` 标签表示在最后一个文本片段显示完毕后，还有一段延时。
        let final_advance_ms = previous_duration_cs as u64 * 10;
        if final_advance_ms > 0 {
            max_end_time_ms = max_end_time_ms.max(current_time_ms + final_advance_ms);
        }
    }
    Ok((syllables, max_end_time_ms))
}

/// 解析 ASS Dialogue 行中的 Actor (演唱者/角色名) 字段和 Style 字段，
/// 以确定演唱者角色、是否为背景、语言代码、是否为标记行以及歌曲部分。
///
/// # Arguments
/// * `actor_str_input` - `&str` 类型，原始 Actor 字段的字符串内容。
/// * `style` - `&str` 类型，该 Dialogue 行的 Style 字段内容。
/// * `line_num` - `usize` 类型，当前行号，用于错误和警告报告。
///
/// # Returns
/// `Result<ParsedActor, ConvertError>` - 如果成功，返回解析后的 `ParsedActor` 结构；否则返回错误。
pub fn parse_actor(
    actor_str_input: &str,
    style_input: &str,
    line_num: usize,
) -> Result<ParsedActor, ConvertError> {
    let mut actor_str_for_other_tags = actor_str_input.to_string(); // 可变副本，用于移除已解析的标签
    let mut song_part_val: Option<String> = None; // 存储解析到的歌曲部分

    // --- 解析 itunes:song-part 标签 ---
    // 尝试匹配 actor 字段中的 itunes:song-part="..." 标签
    if let Some(caps) = SONG_PART_DIRECTIVE_REGEX.captures(&actor_str_for_other_tags) {
        let full_match_str = caps.get(0).unwrap().as_str(); // 整个匹配到的标签字符串
        let match_range = caps.get(0).unwrap().range(); // 标签在原始字符串中的范围

        // SONG_PART_DIRECTIVE_REGEX 有三个捕获组，分别对应双引号、单引号和无引号的值
        let double_quoted_val = caps.get(1).map(|m| m.as_str());
        let single_quoted_val = caps.get(2).map(|m| m.as_str());
        let unquoted_val = caps.get(3).map(|m| m.as_str());

        let mut extracted_value: Option<&str> = None; // 存储提取到的有效值
        let mut is_unquoted_capture = false; // 标记是否是无引号捕获

        if let Some(val) = double_quoted_val {
            extracted_value = Some(val);
        } else if let Some(val) = single_quoted_val {
            extracted_value = Some(val);
        } else if unquoted_val.is_some() {
            is_unquoted_capture = true;
            extracted_value = unquoted_val;
        } // 无引号也视为有效值

        if let Some(valid_value) = extracted_value {
            if song_part_val.is_none() {
                // 如果尚未解析到 song_part
                song_part_val = Some(valid_value.to_string());
            } else {
                // 如果已存在 song_part，则警告重复定义
                log::warn!(
                    "行 {}: 发现多个 itunes:song-part 标签。使用第一个 ('{}'), 后续的 ('{}') 将被忽略。",
                    line_num,
                    song_part_val.as_ref().unwrap(),
                    full_match_str
                );
            }
        }
        // 如果是无引号的值被捕获，但规则可能要求引号（取决于具体规范或期望），这里可以添加警告。
        // 当前逻辑：只要正则匹配到，就尝试使用。
        if is_unquoted_capture && extracted_value.is_some() {
            log::info!(
                "行 {}: itunes:song-part 的值 ('{}') 未用引号包裹，但仍被解析。",
                line_num,
                extracted_value.unwrap()
            );
        }

        // 从 actor_str_for_other_tags 中移除已解析的 song-part 标签，避免影响后续解析
        // 确保只替换本次匹配到的实例
        if SONG_PART_DIRECTIVE_REGEX
            .find(&actor_str_for_other_tags)
            .is_some_and(|m| m.range() == match_range)
        {
            actor_str_for_other_tags.replace_range(match_range, "");
        }
    }

    // --- 解析其他 Actor 字段中的标签 (如角色、语言等) ---
    // 按空白分割 Actor 字段，得到标签列表
    let tags: Vec<&str> = actor_str_for_other_tags.split_whitespace().collect();
    let mut is_marker = false; // 是否为标记行 (x-mark)
    let mut found_v1 = false; // 是否找到 v1 (主唱1) 相关标签
    let mut found_v2 = false; // 是否找到 v2 (主唱2) 相关标签
    let mut found_bg = false; // 是否找到 bg (背景) 相关标签
    let mut found_chorus = false; // 是否找到 chorus (合唱) 相关标签
    let mut lang_code: Option<String> = None; // 存储语言代码 (来自 x-lang:xx)
    let mut role_tags_found: Vec<&str> = Vec::new(); // 存储所有找到的与角色相关的标签，用于冲突检测

    let mut is_background_final = false; // 最终确定是否为背景人声

    // 定义各种角色的同义标签
    const V1_TAGS: [&str; 2] = ["左", "v1"]; // "左" 代表左或主唱1
    const V2_TAGS: [&str; 4] = ["右", "x-duet", "x-anti", "v2"]; // "右", "二重唱", "对唱", "主唱2"
    const BG_TAGS: [&str; 2] = ["背", "x-bg"]; // "背景"
    const CHORUS_TAGS: [&str; 2] = ["合", "v1000"]; // "合唱"

    // 遍历所有提取到的标签
    for tag_ref in &tags {
        let tag = *tag_ref;
        if tag == "x-mark" {
            is_marker = true;
        }
        // 标记行
        // 角色标签的识别依赖于 Style 字段。通常 "orig" 或 "default" 样式用于主歌词和背景歌词。
        else if V1_TAGS.contains(&tag) {
            if style_input == "orig" || style_input == "default" {
                found_v1 = true;
                role_tags_found.push(tag);
            }
        } else if V2_TAGS.contains(&tag) {
            if style_input == "orig" || style_input == "default" {
                found_v2 = true;
                role_tags_found.push(tag);
            }
        } else if BG_TAGS.contains(&tag) {
            if style_input == "orig" || style_input == "default" {
                found_bg = true;
                role_tags_found.push(tag);
            }
        } else if CHORUS_TAGS.contains(&tag) {
            if style_input == "orig" || style_input == "default" {
                found_chorus = true;
                role_tags_found.push(tag);
            }
        } else if let Some(code) = tag.strip_prefix("x-lang:") {
            if style_input == "ts" || style_input == "trans" || style_input == "bg-ts" {
                if !code.is_empty() {
                    lang_code = Some(code.to_string());
                } else {
                    log::warn!(
                        "行 {}: 在Actor字段 '{}' 中发现空的语言代码标签 '{}'。",
                        line_num,
                        actor_str_input,
                        tag
                    );
                }
            } else {
                log::warn!(
                    "行 {}: 非翻译行 (样式: {}) 的Actor字段 '{}' 中发现语言标签 '{}'。该标签可能被忽略或导致非预期行为。",
                    line_num,
                    style_input,
                    actor_str_input,
                    tag
                );
            }
        }
    }

    if (style_input == "ts" || style_input == "trans") && lang_code.is_none() {
        lang_code = Some("zh-CN".to_string());
    }

    let mut final_role = None; // 最终确定的演唱者角色

    // 仅当样式为 "orig" 或 "default" (通常代表歌词本身) 时，才判断角色。
    if style_input == "orig" || style_input == "default" {
        // 计算找到的独立角色指示符的数量 (v1, v2, bg, chorus)。
        let role_indicator_count =
            found_v1 as u8 + found_v2 as u8 + found_bg as u8 + found_chorus as u8;
        if role_indicator_count > 1 {
            // 如果多于一个角色指示符，则存在冲突。
            return Err(ConvertError::ConflictingActorTags {
                line_num,
                tags: role_tags_found.iter().map(|s| s.to_string()).collect(),
            });
        }

        // 根据找到的标签确定最终角色。优先级：合唱 > 背景 > 主唱2 > 主唱1 (默认)。
        if found_chorus {
            final_role = Some(ActorRole::Chorus);
        } else if found_bg {
            final_role = Some(ActorRole::Background);
            is_background_final = true; // 标记这是一个背景人声行
        } else if found_v2 {
            final_role = Some(ActorRole::Vocal2);
        } else {
            // 如果没有其他角色标签，或者只有 v1 标签，则默认为 Vocal1。
            // 即使 found_v1 为 false (Actor字段为空或不含v1标签)，对于 orig 样式也应视为 Vocal1。
            final_role = Some(ActorRole::Vocal1);
        }
    } // 对于非 "orig" 样式的行 (如翻译 "ts", "roma"), final_role 保持为 None。
    // 这些行的 Actor 字段主要用于携带 lang_code。

    Ok(ParsedActor {
        role: final_role,
        is_background: is_background_final,
        lang_code,
        is_marker,
        song_part: song_part_val,
    })
}

/// 从字符串加载并处理 ASS 文件内容。
///
/// # Arguments
/// * `ass_content` - `&str` 类型，包含完整 ASS 文件内容的字符串。
///
/// # Returns
/// `Result<ProcessedAssData, ConvertError>` - 如果成功，返回包含所有解析数据的 `ProcessedAssData` 结构；
///                                           否则返回错误。
pub fn load_and_process_ass_from_string(
    ass_content: &str,
) -> Result<ProcessedAssData, ConvertError> {
    let mut line_num = 0; // 当前处理的行号。
    let mut processed_lines: Vec<AssLineInfo> = Vec::new(); // 存储所有成功解析的ASS行信息。
    let mut metadata: Vec<AssMetadata> = Vec::new(); // 存储从Comment元数据行解析的信息。
    let mut markers: Vec<MarkerInfo> = Vec::new(); // 存储标记。

    // 用于从 "meta" Comment行收集特定元数据：
    let mut apple_music_id_val = String::new(); // Apple Music ID
    let mut songwriters_val: Vec<String> = Vec::new(); // 词曲作者列表
    let mut language_code_val: Option<String> = None; // 全局语言代码 (来自 "lang" 元数据)
    let mut agent_names_map: HashMap<String, String> = HashMap::new(); // 演唱者代号与实际名称的映射 (如 "v1": "歌手A")

    // 用于辅助关联主歌词、背景歌词及其翻译/罗马音的状态变量：
    let mut last_main_lyric_line_index: Option<usize> = None; // 最近一个主歌词行在 processed_lines 中的索引。
    let mut last_bg_lyric_line_index: Option<usize> = None; // 最近一个背景歌词行在 processed_lines 中的索引。
    let mut first_detected_translation_lang: Option<String> = None; // 检测到的第一个翻译行的语言代码。

    // 逐行读取ASS内容。
    for line_str_raw in ass_content.lines() {
        line_num += 1;
        let line_str = line_str_raw.trim(); // 去除行首尾空白。

        // 跳过空行或不包含 ':' (ASS行基本分隔符) 的无效行。
        if line_str.is_empty() || !line_str.contains(':') {
            continue;
        }

        // 使用 ASS_LINE_REGEX 正则表达式尝试匹配和解析当前行。
        if let Some(caps) = ASS_LINE_REGEX.captures(line_str) {
            // 从捕获组中提取各个字段。
            let line_type = caps.name("Type").map_or("", |m| m.as_str());
            let start_str = caps.name("Start").map_or("", |m| m.as_str());
            let end_str = caps.name("End").map_or("", |m| m.as_str());
            let style = caps
                .name("Style")
                .map_or("", |m| m.as_str())
                .trim()
                .to_string();
            let actor_raw = caps
                .name("Actor")
                .map_or("", |m| m.as_str())
                .trim()
                .to_string();
            let effect_raw = caps.name("Effect").map_or("", |m| m.as_str()).trim();
            let text_content = caps.name("Text").map_or("", |m| m.as_str()).to_string();

            // 基本有效性检查：行类型、开始时间、结束时间不能为空。
            if line_type.is_empty() || start_str.is_empty() || end_str.is_empty() {
                log::warn!(
                    "行 {}: 无法从行 '{}' 解析出必要的字段 (类型/开始/结束时间)。",
                    line_num,
                    line_str
                );
                continue;
            }

            let style_lower = style.to_lowercase();
            let effect_lower = effect_raw.to_lowercase();
            if (style_lower == "orig"
                || style_lower == "default"
                || style_lower == "ts"
                || style_lower == "trans"
                || style_lower == "roma"
                || style_lower == "bg-ts"
                || style_lower == "bg-roma")
                && !(effect_lower.is_empty() || effect_lower == "karaoke")
            {
                log::info!(
                    "行 {}: 因特效字段为 '{}' (非空或非 'karaoke')，已跳过内容处理 (样式: {}).",
                    line_num,
                    effect_raw,
                    style
                );
                continue; // 跳过此行的主要内容处理逻辑
            }

            // 解析开始和结束时间字符串为毫秒。
            let start_ms = parse_ass_time(start_str, line_num)?;
            let end_ms = parse_ass_time(end_str, line_num)?;
            // 解析 Actor 字段。
            let parsed_actor = parse_actor(&actor_raw, &style, line_num)?;

            // 如果 Actor 字段包含 x-mark 标记，则记录为一个标记。
            if parsed_actor.is_marker {
                markers.push((line_num, text_content.clone()));
                log_marker!(line_num, &text_content);
            }

            let mut current_content: Option<AssLineContent> = None; // 当前行解析出的具体内容。
            let mut calculated_end_ms = end_ms; // 行的结束时间，对于卡拉OK行可能会被音节重新计算。
            let mut add_line_info; // 标记是否应将此行信息添加到 processed_lines。

            // 根据 Style 字段处理不同类型的行。
            match style.as_str() {
                "ts" | "trans" => {
                    // 主翻译行
                    add_line_info = false; // 翻译行本身不直接添加，而是附加到主歌词行
                    if let Some(last_idx) = last_main_lyric_line_index {
                        // 检查翻译行开始时间是否与最后的主歌词行开始时间匹配
                        if processed_lines[last_idx].start_ms == start_ms {
                            // 如果是第一个检测到的翻译，记录其语言代码
                            if first_detected_translation_lang.is_none() {
                                if let Some(lang) = &parsed_actor.lang_code {
                                    if !lang.is_empty() {
                                        first_detected_translation_lang = Some(lang.clone());
                                    }
                                }
                            }
                            // 将翻译内容直接添加到对应的 AssLineInfo 的 content 字段中
                            // （当前 AssLineContent 枚举和 ProcessedAssData 结构需要调整以支持这种直接附加，
                            //  或者，如此处逻辑，创建一个新的 AssLineContent::MainTranslation 并作为新行添加，
                            //  后续在转换为 TtmlParagraph 时再进行合并。）
                            // 当前实现：作为独立行内容存储，后续处理。
                            current_content = Some(AssLineContent::MainTranslation {
                                lang_code: parsed_actor.lang_code.clone(),
                                text: text_content.clone(),
                            });
                            add_line_info = true; // 标记为添加此翻译信息行
                        } else {
                            log::warn!(
                                "行 {}: 主翻译行 (样式 '{}') 的开始时间 {}ms 与上一主歌词行 (行 {}, 开始 {}ms) 不匹配。已忽略。",
                                line_num,
                                style,
                                start_ms,
                                processed_lines[last_idx].line_num,
                                processed_lines[last_idx].start_ms
                            );
                        }
                    } else {
                        log::warn!(
                            "行 {}: 发现主翻译行 (样式 '{}') 但之前没有找到对应的主歌词行。",
                            line_num,
                            style
                        );
                    }
                }
                "bg-ts" => {
                    // 背景翻译行
                    add_line_info = false; // 背景翻译通常附加到背景歌词行
                    if let Some(last_bg_idx) = last_bg_lyric_line_index {
                        if let Some(bg_line_info) = processed_lines.get_mut(last_bg_idx) {
                            if bg_line_info.start_ms == start_ms {
                                if first_detected_translation_lang.is_none() {
                                    // 也用背景翻译来推断主要翻译语言
                                    if let Some(lang) = &parsed_actor.lang_code {
                                        if !lang.is_empty() {
                                            first_detected_translation_lang = Some(lang.clone());
                                        }
                                    }
                                }
                                // 尝试将背景翻译附加到对应的背景歌词行的 LyricLine 结构中
                                if let Some(AssLineContent::LyricLine {
                                    ref mut bg_translation,
                                    ..
                                }) = bg_line_info.content
                                {
                                    *bg_translation =
                                        Some((parsed_actor.lang_code, text_content.clone()));
                                } else {
                                    log::warn!(
                                        "行 {}: 尝试附加背景翻译到行 {}，但该行不是预期的LyricLine类型。",
                                        line_num,
                                        bg_line_info.line_num
                                    );
                                }
                            } else {
                                log::warn!(
                                    "行 {}: 背景翻译行 (样式 'bg-ts') 的开始时间 {}ms 与上一背景歌词行 (行 {}, 开始 {}ms) 不匹配。",
                                    line_num,
                                    start_ms,
                                    bg_line_info.line_num,
                                    bg_line_info.start_ms
                                );
                            }
                        }
                    } else {
                        log::warn!(
                            "行 {}: 发现背景翻译行 (样式 'bg-ts') 但之前没有找到对应的背景歌词行。",
                            line_num
                        );
                    }
                }
                "roma" => {
                    // 主罗马音行
                    add_line_info = false; // 同翻译行，尝试附加或作为新行
                    if let Some(last_idx) = last_main_lyric_line_index {
                        if processed_lines[last_idx].start_ms == start_ms {
                            current_content = Some(AssLineContent::MainRomanization {
                                text: text_content.clone(),
                            });
                            add_line_info = true;
                        } else {
                            log::warn!(
                                "行 {}: 主罗马音行 (样式 'roma') 的开始时间 {}ms 与上一主歌词行 (行 {}, 开始 {}ms) 不匹配。已忽略。",
                                line_num,
                                start_ms,
                                processed_lines[last_idx].line_num,
                                processed_lines[last_idx].start_ms
                            );
                        }
                    } else {
                        log::warn!(
                            "行 {}: 发现主罗马音行 (样式 'roma') 但之前没有找到对应的主歌词行。",
                            line_num
                        );
                    }
                }
                "bg-roma" => {
                    // 背景罗马音行
                    add_line_info = false; // 同背景翻译，尝试附加
                    if let Some(last_bg_idx) = last_bg_lyric_line_index {
                        if let Some(bg_line_info) = processed_lines.get_mut(last_bg_idx) {
                            if bg_line_info.start_ms == start_ms {
                                if let Some(AssLineContent::LyricLine {
                                    ref mut bg_romanization,
                                    ..
                                }) = bg_line_info.content
                                {
                                    *bg_romanization = Some(text_content.clone());
                                } else {
                                    log::warn!(
                                        "行 {}: 尝试附加背景罗马音到行 {}，但该行不是预期的LyricLine类型。",
                                        line_num,
                                        bg_line_info.line_num
                                    );
                                }
                            } else {
                                log::warn!(
                                    "行 {}: 背景罗马音行 (样式 'bg-roma') 的开始时间 {}ms 与上一背景歌词行 (行 {}, 开始 {}ms) 不匹配。",
                                    line_num,
                                    start_ms,
                                    bg_line_info.line_num,
                                    bg_line_info.start_ms
                                );
                            }
                        }
                    } else {
                        log::warn!(
                            "行 {}: 发现背景罗马音行 (样式 'bg-roma') 但之前没有找到对应的背景歌词行。",
                            line_num
                        );
                    }
                }
                "orig" | "default" => {
                    // 主歌词或背景歌词行 (根据Actor字段区分)
                    if line_type == "Dialogue" {
                        // 必须是 Dialogue 类型
                        if let Some(role) = parsed_actor.role {
                            // 必须成功解析出角色
                            let (syllables, line_actual_end_ms) =
                                parse_karaoke_text(&text_content, start_ms, line_num)?;
                            calculated_end_ms = line_actual_end_ms; // 使用卡拉OK解析出的实际结束时间
                            if !syllables.is_empty() {
                                // 必须有音节内容
                                current_content = Some(AssLineContent::LyricLine {
                                    role,
                                    syllables,
                                    bg_translation: None,
                                    bg_romanization: None, // 初始化时无背景的翻译/罗马音
                                });
                                let current_index = processed_lines.len(); // 获取当前行将要插入的索引
                                if parsed_actor.is_background {
                                    // 如果是背景歌词
                                    last_bg_lyric_line_index = Some(current_index);
                                } else {
                                    // 否则是主歌词
                                    last_main_lyric_line_index = Some(current_index);
                                }
                                add_line_info = true;
                            } else {
                                add_line_info = false;
                                log::warn!(
                                    "行 {}: 样式为 '{}' 的行解析后无音节内容。",
                                    line_num,
                                    style
                                );
                            }
                        } else {
                            add_line_info = false;
                            log::warn!(
                                "行 {}: 样式为 '{}' 的行未能解析出有效的演唱者角色。",
                                line_num,
                                style
                            );
                        }
                    } else {
                        add_line_info = false; /* Comment 行使用 orig/default 样式不作为歌词处理 */
                    }
                }
                "meta" => {
                    // 元数据行
                    add_line_info = false; // 元数据不直接添加到 processed_lines，而是存入 metadata 向量
                    if line_type == "Comment" && start_ms == 0 && end_ms == 0 {
                        // 元数据通常是 Layer 0, Start 0, End 0 的 Comment
                        if let Some((key, value)) = text_content.split_once(':') {
                            // 文本格式应为 "key: value"
                            let key_trimmed = key.trim();
                            let value_trimmed = value.trim().to_string();
                            if !key_trimmed.is_empty() {
                                // Key 不能为空
                                // 特定元数据键的处理
                                if key_trimmed.eq_ignore_ascii_case("lang") {
                                    // 全局语言代码
                                    if !value_trimmed.is_empty() {
                                        language_code_val = Some(value_trimmed.clone());
                                    } else {
                                        log::warn!("行 {}: 'lang' 元数据的值为空。", line_num);
                                    }
                                }
                                // 演唱者名称映射
                                if key_trimmed == "v1" || key_trimmed == "v2" {
                                    // 例如 "v1: Singer A"
                                    if !value_trimmed.is_empty() {
                                        agent_names_map
                                            .insert(key_trimmed.to_string(), value_trimmed.clone());
                                    } else {
                                        log::warn!(
                                            "行 {}: 演唱者 '{}' 的名称为空。",
                                            line_num,
                                            key_trimmed
                                        );
                                    }
                                }
                                // Apple Music ID
                                if key_trimmed == "appleMusicId" {
                                    apple_music_id_val = value_trimmed.clone();
                                }
                                // 词曲作者
                                if key_trimmed == "songwriter" {
                                    if !value_trimmed.is_empty() {
                                        songwriters_val.push(value_trimmed.clone());
                                    } else {
                                        log::warn!(
                                            "行 {}: 'songwriter' 元数据的值为空。",
                                            line_num
                                        );
                                    }
                                }
                                // 将所有解析到的元数据存入列表
                                metadata.push(AssMetadata {
                                    key: key_trimmed.to_string(),
                                    value: value_trimmed,
                                });
                            } else {
                                log::warn!(
                                    "行 {}: 元数据行的 Key 为空: '{}'",
                                    line_num,
                                    text_content
                                );
                            }
                        } else {
                            log::warn!(
                                "行 {}: 元数据格式无效 (预期格式为 'key: value'): '{}'",
                                line_num,
                                text_content
                            );
                        }
                    } else {
                        log::warn!(
                            "行 {}: 样式为 'meta' 但不是预期的 Comment 0,0:00:00.00,0:00:00.00 格式。",
                            line_num
                        );
                    }
                }
                _ => {
                    add_line_info = false;
                    log::warn!("行 {}: 遇到未知或当前不支持的样式 '{}'。", line_num, style);
                }
            }

            // 如果标记为添加并且确实有内容，则将解析后的行信息添加到列表中。
            if add_line_info && current_content.is_some() {
                processed_lines.push(AssLineInfo {
                    line_num,
                    start_ms,
                    end_ms: calculated_end_ms, // 使用计算（可能被K标签更新）后的结束时间
                    content: current_content,
                    song_part: parsed_actor.song_part.clone(), // 存储歌曲部分信息
                });
            } else if add_line_info && current_content.is_none() {
                // 这种情况理论上不应发生，因为 add_line_info 为 true 前 current_content 应已 Some。
                log::error!(
                    "行 {}: 逻辑错误 - 标记为添加行但内容为空 (样式: {})。",
                    line_num,
                    style
                );
            }
        } else {
            // 如果行不匹配 ASS_LINE_REGEX，记录为未识别的行。
            // 但也需要考虑是否是 [Script Info] 或 [V4+ Styles] 等部分的合法行。
            // 此处简单忽略不匹配的行，或者可以添加更复杂的节（section）状态判断。
            if !line_str.starts_with('[')
                && !line_str.contains("Format:")
                && !line_str.contains("Style:")
            {}
        }
    } // 结束逐行解析

    // 返回包含所有解析数据的 ProcessedAssData 结构。
    Ok(ProcessedAssData {
        lines: processed_lines,
        metadata,
        markers,
        apple_music_id: apple_music_id_val,
        songwriters: songwriters_val,
        language_code: language_code_val,
        agent_names: agent_names_map,
        detected_translation_language: first_detected_translation_lang,
    })
}
