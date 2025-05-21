// 导入项目中定义的类型：
// ConvertError: 错误处理枚举
// LysLine: LYS解析器输出的单行歌词数据结构
// TtmlParagraph: 目标中间数据结构，表示一个歌词段落
// ActorRole: 演唱者角色枚举 (Vocal1, Vocal2, Background, Chorus)
// BackgroundSection: TTML段落中用于存储背景和声的部分
// AssMetadata: 用于存储元数据 (虽然LYS到TTML转换通常不直接处理文件级元数据，但函数签名保持一致性)
use crate::types::{
    ActorRole, AssMetadata, BackgroundSection, ConvertError, LysLine, TtmlParagraph,
};
// 导入工具函数：
// process_parsed_syllables_to_ttml: 将原始音节列表（如LysSyllable）转换为TTML音节列表（TtmlSyllable）
// post_process_ttml_syllable_line_spacing: 对整行TTML音节进行行级别的空格后处理
use crate::utils::{post_process_ttml_syllable_line_spacing, process_parsed_syllables_to_ttml};

// LYS 行属性常量定义
// 这些常量代表 LYS 文件中行首 `[属性]` 的数字含义。
// const LYS_PROPERTY_UNSET: u8 = 0; // 未设置或默认 (通常视为左/主唱1, 非背景)
// const LYS_PROPERTY_LEFT: u8 = 1; // 左/主唱1, 背景未定
const LYS_PROPERTY_RIGHT: u8 = 2; // 右/主唱2, 背景未定
// const LYS_PROPERTY_NO_BACK_UNSET: u8 = 3; // 未设置，但明确非背景
// const LYS_PROPERTY_NO_BACK_LEFT: u8 = 4; // 左/主唱1, 明确非背景
const LYS_PROPERTY_NO_BACK_RIGHT: u8 = 5; // 右/主唱2, 明确非背景
const LYS_PROPERTY_BACK_UNSET: u8 = 6; // 未设置，但为背景和声
const LYS_PROPERTY_BACK_LEFT: u8 = 7; // 左/主唱1, 且为背景和声
const LYS_PROPERTY_BACK_RIGHT: u8 = 8; // 右/主唱2, 且为背景和声

/// 将 LYS 行属性数字映射到 TTML 的演唱者角色 (ActorRole) 和是否为背景人声。
///
/// # Arguments
/// * `property` - LYS 行的属性数字。
///
/// # Returns
/// 元组 `(ActorRole, bool)`:
///   - `ActorRole`: 对应的演唱者角色 (Vocal1, Vocal2)。
///   - `bool`: `true` 如果该属性表示背景和声，否则为 `false`。
fn map_lys_property_to_role_and_background(property: u8) -> (ActorRole, bool) {
    match property {
        // 属性 2, 5, 8 通常表示右或第二演唱者 (Vocal2)
        LYS_PROPERTY_RIGHT | LYS_PROPERTY_NO_BACK_RIGHT | LYS_PROPERTY_BACK_RIGHT => {
            (ActorRole::Vocal2, property == LYS_PROPERTY_BACK_RIGHT) // 只有属性8明确是背景
        }
        // 属性 6, 7 表示背景和声，通常关联到主唱1
        LYS_PROPERTY_BACK_UNSET | LYS_PROPERTY_BACK_LEFT => {
            (ActorRole::Vocal1, true) // 明确是背景
        }
        // 其他所有属性 (0, 1, 3, 4, 以及未定义的) 都默认为主唱1 (Vocal1)，非背景
        _ => (ActorRole::Vocal1, false),
    }
}

/// 将解析后的 LYS 数据 (`Vec<LysLine>`) 转换为 TTML 段落数据 (`Vec<TtmlParagraph>`)。
///
/// 根据 LYS 格式规范，音节的开始时间 (`start_ms` in `LysSyllable`) 是相对于歌曲开始的绝对时间。
/// LYS 的行属性会影响生成的 TTML 段落的 `agent` 和是否包含 `background_section`。
///
/// # Arguments
/// * `lys_lines` - 一个包含 `LysLine` 结构体的切片，代表从LYS文件解析出的所有歌词行。
///
/// # Returns
/// `Result<(Vec<TtmlParagraph>, Vec<AssMetadata>), ConvertError>` -
/// 如果成功，返回一个元组，包含转换后的 TTML 段落列表和空的元数据列表。
/// LYS 文件级元数据（如 `[ti:]`）由 `lys_parser` 解析并由上层逻辑（如 `MetadataStore`）处理，
/// 此函数不直接处理文件级元数据到 TTML `<head>` 的转换。
/// 失败时返回错误。
pub fn convert_lys_to_ttml_data(
    lys_lines: &[LysLine],
) -> Result<(Vec<TtmlParagraph>, Vec<AssMetadata>), ConvertError> {
    let mut ttml_paragraphs: Vec<TtmlParagraph> = Vec::new(); // 存储转换结果
    let metadata_out: Vec<AssMetadata> = Vec::new(); // LYS 文件级元数据不在此函数处理

    let mut i = 0; // 使用索引遍历 lys_lines，因为可能需要一次处理多行（主歌词+背景）
    while i < lys_lines.len() {
        let current_lys_line = &lys_lines[i]; // 获取当前处理的 LYS 行
        let line_num_for_log = i + 1; // 日志用的行号

        // 如果当前行没有音节，记录警告并跳过
        if current_lys_line.syllables.is_empty() {
            log::warn!(
                "[LYS -> TTML] 行 {} (属性: {}) 无音节，已跳过",
                line_num_for_log,
                current_lys_line.property
            );
            i += 1;
            continue;
        }

        // 根据行属性映射角色和是否为背景
        let (current_actor_role, current_is_background) =
            map_lys_property_to_role_and_background(current_lys_line.property);

        // 初始化一个 TtmlParagraph 结构
        let mut paragraph = TtmlParagraph {
            // 根据角色设置 agent
            agent: match current_actor_role {
                ActorRole::Vocal1 => "v1".to_string(),
                ActorRole::Vocal2 => "v2".to_string(),
                _ => "v1".to_string(), // LYS 不直接表示 Chorus 等角色，默认为 v1
            },
            ..Default::default() // 其他字段使用默认值
        };

        // LYS 音节的 start_ms 已经是绝对时间。
        // `process_parsed_syllables_to_ttml` 期望输入的 `LysSyllable.start_ms` 是音节的起始时间戳，
        // 它会基于此 `start_ms` 和 `duration_ms` 计算 `TtmlSyllable` 的 `start_ms` 和 `end_ms`。
        // 由于 LYS 的 `start_ms` 是绝对时间，所以这里可以直接使用。
        let syllables_for_processing = current_lys_line.syllables.clone();

        // 将音节列表转换为 TTML 音节列表，并进行行级别空格处理
        let mut processed_syllables =
            process_parsed_syllables_to_ttml(&syllables_for_processing, "LYS");
        post_process_ttml_syllable_line_spacing(&mut processed_syllables);

        // 如果处理后音节列表为空（例如，原始行只有无效音节或纯粹的格式标记），则跳过
        if processed_syllables.is_empty() {
            log::warn!(
                "[LYS -> TTML] 行 {} (属性: {}) 处理后无有效音节，已跳过",
                line_num_for_log,
                current_lys_line.property
            );
            i += 1;
            continue;
        }

        // 根据当前行是否为背景，填充 TtmlParagraph 的相应字段
        if current_is_background {
            // 当前行是背景和声行
            paragraph.background_section = Some(BackgroundSection {
                // 背景部分的开始和结束时间由其音节（已经是绝对时间）决定
                start_ms: processed_syllables.first().map_or(0, |s| s.start_ms),
                end_ms: processed_syllables.last().map_or(0, |s| s.end_ms),
                syllables: processed_syllables, // 存储处理后的音节
                ..Default::default()
            });
            // 更新整个段落的开始和结束时间以包含背景部分
            // （如果段落只有背景，则其时间即为背景时间；如果后续有主歌词，会被主歌词时间覆盖或合并）
            if let Some(bg_sec) = &paragraph.background_section {
                if !bg_sec.syllables.is_empty() {
                    paragraph.p_start_ms = bg_sec.start_ms;
                    paragraph.p_end_ms = bg_sec.end_ms;
                }
            }
            ttml_paragraphs.push(paragraph); // 添加到结果列表
            i += 1; // 处理下一行
        } else {
            // 当前行是主歌词行
            paragraph.main_syllables = processed_syllables; // 存储处理后的音节
            // 主歌词段落的开始和结束时间由其音节（已经是绝对时间）决定
            paragraph.p_start_ms = paragraph.main_syllables.first().unwrap().start_ms;
            paragraph.p_end_ms = paragraph.main_syllables.last().unwrap().end_ms;

            let mut consumed_next_line = false; // 标记是否消耗了下一行（作为当前行的背景）
            // 检查下一行是否是与当前主歌词行匹配的背景和声行
            if i + 1 < lys_lines.len() {
                let next_lys_line = &lys_lines[i + 1];
                if !next_lys_line.syllables.is_empty() {
                    let (next_actor_role, next_is_background) =
                        map_lys_property_to_role_and_background(next_lys_line.property);

                    // 条件：下一行是背景行，并且其角色与当前主歌词行角色匹配
                    // 或者下一行的属性是 LYS_PROPERTY_BACK_UNSET (通用背景，不限声道)
                    if next_is_background
                        && (next_actor_role == current_actor_role
                            || next_lys_line.property == LYS_PROPERTY_BACK_UNSET)
                    {
                        // 下一行的 LYS 音节的 start_ms 也是绝对时间
                        let next_line_syllables_for_processing = next_lys_line.syllables.clone();

                        let mut bg_syllables_from_next_line = process_parsed_syllables_to_ttml(
                            &next_line_syllables_for_processing,
                            "LYS_BG",
                        );
                        post_process_ttml_syllable_line_spacing(&mut bg_syllables_from_next_line);

                        if !bg_syllables_from_next_line.is_empty() {
                            paragraph.background_section = Some(BackgroundSection {
                                start_ms: bg_syllables_from_next_line.first().unwrap().start_ms,
                                end_ms: bg_syllables_from_next_line.last().unwrap().end_ms,
                                syllables: bg_syllables_from_next_line,
                                ..Default::default()
                            });
                            // 更新整个段落的开始和结束时间以包含背景部分
                            if let Some(bg_sec) = &paragraph.background_section {
                                paragraph.p_end_ms = paragraph.p_end_ms.max(bg_sec.end_ms);
                                paragraph.p_start_ms = paragraph.p_start_ms.min(bg_sec.start_ms);
                            }
                        }
                        consumed_next_line = true; // 标记下一行已被作为背景处理
                    }
                }
            }
            ttml_paragraphs.push(paragraph); // 添加到结果列表
            i += if consumed_next_line { 2 } else { 1 }; // 根据是否消耗下一行来推进索引
        }
    }
    Ok((ttml_paragraphs, metadata_out)) // 返回转换结果
}
