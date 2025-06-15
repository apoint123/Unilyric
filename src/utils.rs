use std::path::PathBuf;

use crate::types::{LysSyllable, TtmlSyllable};

use directories::ProjectDirs;
use ferrous_opencc::{OpenCC, error::OpenCCError};
use log::{error, info, trace};
use once_cell::sync::Lazy;

#[macro_export]
macro_rules! log_marker {
    ($line:expr, $text:expr) => {
        log::info!(target: "MARKER", "行 {}: {}", $line, $text)
    };
}

/// 清理文本两端的括号（单个或成对）
pub fn clean_parentheses_from_bg_text(text: &str) -> String {
    if text.is_empty() {
        return "".to_string();
    }

    let first_char = text.chars().next();
    let last_char = text.chars().last();

    let has_leading_paren = first_char == Some('(');
    let has_trailing_paren = last_char == Some(')');

    let mut start_index = 0;
    let mut end_index = text.len();

    if has_leading_paren && has_trailing_paren && text.len() >= 2 {
        // 同时有开头和结尾括号
        start_index = text.chars().next().unwrap().len_utf8();
        end_index = text.char_indices().last().unwrap().0;
    } else if has_leading_paren {
        // 只有开头括号
        start_index = text.chars().next().unwrap().len_utf8();
    } else if has_trailing_paren {
        // 只有结尾括号
        end_index = text.char_indices().last().unwrap().0;
    }
    let final_content_slice = if start_index >= end_index {
        ""
    } else {
        &text[start_index..end_index]
    };

    final_content_slice.to_string()
}

/// 将原始音节列表（如来自 LYS, QRC, KRC 等格式的 `LysSyllable`）
/// 转换为 TTML 使用的音节列表 (`TtmlSyllable`)。
///
/// 此函数的核心逻辑包括：
/// 1.  **处理零时长空格**: LYS 中的 ` (时间,0)` 形式的空格标记，不会生成独立的 `TtmlSyllable`，
///     而是将其效果（一个尾随空格）附加到其前一个有效的 `TtmlSyllable` 上。
/// 2.  **处理音节文本中的空格**:
///     a.  **前导空格**: 如果一个音节文本以空格开头（如 `" text"`），这个前导空格会被视为
///     其前一个 `TtmlSyllable` 的尾随空格。
///     b.  **尾随空格**: 如果一个音节文本以空格结尾（如 `"text "`），则生成的对应 `TtmlSyllable`
///     会被标记为 `ends_with_space = true`。
///     c.  **纯空格音节 (有时长)**: 如果一个音节文本完全由空格组成但有持续时间（如 `"   "`，时长 > 0），
///     它会被规范化为一个文本内容是单个空格 `" "` 的 `TtmlSyllable`。
///     d.  **静默音节**: 如果一个音节文本为空但有持续时间（如 `""`，时长 > 0），它会生成一个
///     文本内容为空字符串的 `TtmlSyllable`。
/// 3.  **音节构建**: 只有当处理后的音节文本非空，或者原始音节有明确的持续时间时，
///     才会创建并添加 `TtmlSyllable` 到结果列表中。
///
/// # Arguments
/// * `lys_syllables` - 一个 `LysSyllable` 的切片，代表原始解析出的音节。
/// * `_source_format_hint` - 源格式提示字符串 (例如 "LYS", "QRC")，当前版本未使用，但保留用于未来可能的扩展。
///
/// # Returns
/// `Vec<TtmlSyllable>` - 转换后的 TTML 音节列表。
pub fn process_parsed_syllables_to_ttml(
    lys_syllables: &[LysSyllable],
    _source_format_hint: &str, // 参数当前未使用，但保留以备将来扩展
) -> Vec<TtmlSyllable> {
    let mut ttml_syllables_result: Vec<TtmlSyllable> = Vec::with_capacity(lys_syllables.len());

    for lys_syl in lys_syllables {
        // --- 步骤 1: 特殊处理零时长空格音节 ---
        // 例如 LYS 中的 ` (任意时间,0)` 标记，它代表一个逻辑空格，但不占用时间。
        if lys_syl.text == " " && lys_syl.duration_ms == 0 {
            // 如果是零时长空格，并且结果列表中已有音节，
            // 则将上一个音节的 `ends_with_space` 标记为 true，表示其后应有一个空格。
            if let Some(prev_ttml_syl) = ttml_syllables_result.last_mut() {
                // 这个复杂条件的目的是避免不必要的或重复的空格标记：
                // - `!prev_ttml_syl.text.chars().all(char::is_whitespace)`: 确保前一个音节不是纯粹的空白音节。
                //   如果是，那么这个零时长空格可能多余，或者其空格效果已由前一个音节体现。
                // - `!prev_ttml_syl.ends_with_space`: 确保前一个音节尚未被标记为尾部有空格。
                // 如果以上任一条件满足（即前一个音节有内容，或者前一个音节虽是空白但未标记尾随空格），则标记。
                if !prev_ttml_syl.text.chars().all(char::is_whitespace)
                    || !prev_ttml_syl.ends_with_space
                {
                    prev_ttml_syl.ends_with_space = true;
                }
            }
            // 零时长空格本身不生成独立的 TtmlSyllable，处理完毕后跳到下一个 LysSyllable。
            continue;
        }

        // --- 步骤 2: 处理非零时长音节（包括有时长的空格、静默或带内容的音节）---
        let current_text_input = &lys_syl.text; // 当前处理的原始音节文本 (类型: &String)
        let final_text_for_current_syl: String; // 存储处理后用于当前 TtmlSyllable 的文本
        let mut current_syl_ends_with_space: bool = false; // 标记当前 TtmlSyllable 是否逻辑上以空格结束

        if current_text_input.chars().all(char::is_whitespace) {
            // --- 情况 A: 原始音节文本完全由空白字符组成 (或为空字符串)，但有持续时间 ---
            if !current_text_input.is_empty() {
                // A.1: 例如 LYS 中的 `"   "` (时长 > 0)。
                // 对于有持续时间的纯空格音节，其内容规范化为单个空格。
                final_text_for_current_syl = " ".to_string();
            } else {
                // A.2: 例如 LYS 中的 `""` (时长 > 0)。这代表一个有持续时间的静默。
                final_text_for_current_syl = "".to_string();
            }
        } else {
            // --- 情况 B: 原始音节文本包含非空白字符 ---
            let mut text_to_process: &str = current_text_input.as_str();

            // B.1: 处理前导空格。
            // 如果当前音节文本以空格开头（且仅仅是纯空格），这个前导空格的效应
            // 是让前一个 TtmlSyllable 标记为 `ends_with_space = true`。
            if text_to_process.starts_with(char::is_whitespace) {
                if let Some(prev_ttml_syl) = ttml_syllables_result.last_mut() {
                    // 同样使用上述的复杂条件来避免不当的空格标记。
                    if !prev_ttml_syl.text.chars().all(char::is_whitespace)
                        || !prev_ttml_syl.ends_with_space
                    {
                        prev_ttml_syl.ends_with_space = true;
                    }
                }
                text_to_process = text_to_process.trim_start(); // 移除前导空格，继续处理剩余部分
            }

            // B.2: 处理尾随空格和核心文本。
            // `core_text` 是移除了所有前导和尾随空格后的文本。
            let core_text = text_to_process.trim_end(); // trim_end() 作用于 &str, 返回 &str
            final_text_for_current_syl = core_text.to_string(); // 音节的文本内容是这个核心文本。

            // 检查在移除前导空格后、移除尾随空格前的文本 (`text_to_process`) 是否以空格结尾。
            // 如果是，并且核心文本非空，则当前 TtmlSyllable 应标记为尾部有空格。
            // 例如，对于 "Word  "，`text_to_process` (在 trim_start 后) 是 "Word  "，`core_text` 是 "Word"。
            if text_to_process.ends_with(char::is_whitespace) && !core_text.is_empty() {
                current_syl_ends_with_space = true;
            }
        }

        // --- 步骤 3: 构建并添加 TtmlSyllable ---
        // 只有当处理后的最终文本非空，或者原始 LysSyllable 有明确的持续时间（代表一个有意义的静默或空格）时，
        // 才创建并添加 TtmlSyllable。
        if !final_text_for_current_syl.is_empty() || lys_syl.duration_ms > 0 {
            ttml_syllables_result.push(TtmlSyllable {
                text: final_text_for_current_syl,
                start_ms: lys_syl.start_ms, // 音节的开始时间直接继承
                end_ms: lys_syl.start_ms.saturating_add(lys_syl.duration_ms), // 结束时间 = 开始时间 + 持续时间
                ends_with_space: current_syl_ends_with_space, // 标记此音节后是否应有空格
            });
        }
    }
    ttml_syllables_result
}

/// 对已转换为 `TtmlSyllable` 列表的整行歌词进行行级别的空格后处理。
/// 主要目的是：
/// 1. 移除行首所有完全是空文本的音节。
/// 2. 确保行尾最后一个有文本内容的音节其 `ends_with_space` 标志为 `false`。
/// 3. 移除行尾在最后一个有文本内容的音节之后的所有空文本音节。
///
/// # Arguments
/// * `syllables` - 一个可变的 `TtmlSyllable` 向量，代表一行歌词。
pub fn post_process_ttml_syllable_line_spacing(syllables: &mut Vec<TtmlSyllable>) {
    if syllables.is_empty() {
        return;
    }

    // 步骤 1: 移除行首所有文本为空 ("") 的音节
    // 找到第一个文本非空的音节的索引
    let mut first_meaningful_syl_idx = 0;
    while first_meaningful_syl_idx < syllables.len()
        && syllables[first_meaningful_syl_idx].text.is_empty()
    // 只检查文本是否为空
    {
        first_meaningful_syl_idx += 1;
    }

    // 如果找到了这样的空音节（即 first_meaningful_syl_idx > 0），则移除它们
    if first_meaningful_syl_idx > 0 {
        syllables.drain(0..first_meaningful_syl_idx);
    }

    // 如果移除后列表变空，则直接返回
    if syllables.is_empty() {
        return;
    }

    // 步骤 2 & 3: 处理行尾空格和空音节
    // 找到最后一个文本非空的音节的索引
    if let Some(last_meaningful_syl_idx) = syllables.iter().rposition(|syl| !syl.text.is_empty()) {
        // 将最后一个有意义音节的 ends_with_space 设置为 false，因为行尾不应有逻辑上的尾随空格
        syllables[last_meaningful_syl_idx].ends_with_space = false;

        // 移除最后一个有意义音节之后的所有音节（这些音节必然是文本为空的）
        if last_meaningful_syl_idx + 1 < syllables.len() {
            syllables.drain(last_meaningful_syl_idx + 1..);
        }
    } else {
        // 如果所有音节的文本都为空（例如，一行纯粹的静默或空格音节，经过步骤1后可能还剩下一个空格音节），
        // 则清空整个列表。这确保不会输出只有空音节的行（除非它们有特殊含义且被保留）。
        // 实际行为取决于步骤1：如果一行全是空文本音节，步骤1后可能为空。如果剩下一个空格音节，这里会清空。
        syllables.clear();
    }
}

/// 将毫秒时间格式化为 LRC 标准的时间字符串 `[mm:ss.xxx]` 或 `[mm:ss.xx]`。
/// LRC 标准通常使用厘秒 (xx) 或毫秒 (xxx)。此函数输出毫秒 (xxx)。
///
/// # Arguments
/// * `ms` - 需要格式化的总毫秒数。
///
/// # Returns
/// `String` - 格式化后的 LRC 时间标签字符串。
pub fn format_lrc_time_ms(ms: u64) -> String {
    let minutes = ms / 60000; // 计算分钟
    let seconds = (ms % 60000) / 1000; // 计算秒
    let milliseconds = ms % 1000; // 计算毫秒
    format!("[{minutes:02}:{seconds:02}.{milliseconds:03}]") // 格式化输出
}

pub fn get_app_data_dir() -> Option<PathBuf> {
    if let Some(proj_dirs) = ProjectDirs::from("com", "Unilyric", "Unilyric") {
        let data_dir = proj_dirs.data_local_dir();
        if !data_dir.exists()
            && let Err(e) = std::fs::create_dir_all(data_dir)
        {
            log::error!("[UniLyric] 无法创建应用数据目录 {data_dir:?}: {e}");
            return None;
        }
        Some(data_dir.to_path_buf())
    } else {
        log::error!("[UniLyric] 无法获取应用数据目录。");
        None
    }
}

/// 预处理LRC内容字符串，主要用于处理QQ音乐下载的翻译LRC。
/// 将文本内容为 "//" 的LRC行转换为空文本行（只保留时间戳部分）。
///
/// # Arguments
/// * `lrc_content` - 原始的LRC多行文本字符串。
///
/// # Returns
/// `String` - 处理后的LRC多行文本字符串。
pub fn preprocess_qq_translation_lrc_content(lrc_content: String) -> String {
    lrc_content
        .lines() // 将输入字符串按行分割成迭代器
        .map(|line_str| {
            // 对每一行进行处理
            // 尝试找到最后一个 ']' 字符，这通常是LRC时间戳的结束位置
            if let Some(text_start_idx) = line_str.rfind(']') {
                let timestamp_part = &line_str[..=text_start_idx]; // 提取时间戳部分 (包括 ']')
                let text_part = line_str[text_start_idx + 1..].trim(); // 提取文本部分并去除首尾空格

                if text_part == "//" {
                    // 如果文本部分正好是 "//"，则只返回时间戳部分（即文本变为空）
                    timestamp_part.to_string()
                } else {
                    // 否则，返回原始行字符串
                    line_str.to_string()
                }
            } else {
                // 如果行不包含 ']'，说明它可能不是标准的LRC行
                String::new()
            }
        })
        .collect::<Vec<String>>() // 将处理过的所有行收集到一个Vec<String>
        .join("\n") // 再用换行符将它们连接回一个多行字符串
}

// 全局静态变量，用于存储一次性初始化后的 OpenCC 转换器实例或初始化错误。
// Lazy 块仅在首次访问时执行。
static OPENCC_T2S_CONVERTER: Lazy<Result<OpenCC, OpenCCError>> = Lazy::new(|| {
    trace!("[简繁转换] 首次尝试初始化 ferrous-opencc T2S 转换器 (Lazy 模式)...");
    OpenCC::from_config_name("t2s.json")
});

/// 获取对已初始化的 T2S (繁体转简体) OpenCC 转换器的静态引用。
fn get_t2s_converter() -> Result<&'static OpenCC, &'static OpenCCError> {
    OPENCC_T2S_CONVERTER.as_ref()
}

/// 将文本从繁体中文转换为简体中文 (使用 ferrous-opencc)。
/// 如果转换失败或初始化 OpenCC 转换器失败，会记录错误并返回原始文本。
pub fn convert_traditional_to_simplified(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    match get_t2s_converter() {
        Ok(converter) => {
            let simplified_text = converter.convert(text);

            if text != simplified_text {
                info!("[简繁转换] 原文: '{text}' -> 简体: '{simplified_text}'");
            } else if !text.is_empty() && text.chars().any(|c| c as u32 > 127 && c != ' ') {
                trace!("[简繁转换] 文本 '{text}' 转换为简体后无变化 (可能已是简体或无对应转换)。");
            }
            simplified_text
        }
        Err(e) => {
            // 初始化或获取转换器失败
            error!("[简繁转换] 获取 OpenCC T2S 转换器失败: {e}。繁简转换功能将不可用。返回原文。");
            text.to_string()
        }
    }
}
