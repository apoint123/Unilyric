// 导入项目中定义的类型，LysSyllable 是原始解析器（如LYS, QRC, KRC解析器）输出的音节结构，
// TtmlSyllable 是转换为TTML中间表示时使用的音节结构。
use crate::types::{LysSyllable, TtmlSyllable};

/// 声明一个宏 `log_marker!`，用于输出带有 "MARKER" 目标和行号的日志信息。
/// 这通常用于在代码中标记特定的执行点或调试信息。
#[macro_export] // 导出宏，使其在其他模块中可用
macro_rules! log_marker {
    // 宏接受两个参数：$line (行号表达式) 和 $text (要记录的文本表达式)
    ($line:expr, $text:expr) => {
        // 使用 log::info! 宏记录信息，指定目标为 "MARKER"
        log::info!(target: "MARKER", "行 {}: {}", $line, $text)
    };
}

/// 清理背景文本两端的括号。
/// 例如，输入 "(一些文本)" 会返回 "一些文本"。
/// 它会移除字符串开头和结尾的单个半角括号，并去除结果两端的空白。
///
/// # Arguments
/// * `text` - 需要清理的文本字符串切片。
///
/// # Returns
/// `String` - 清理后的字符串。
pub fn clean_parentheses_from_bg_text(text: &str) -> String {
    let mut cleaned_slice = text.trim(); // 首先去除原始文本两端的空白

    // 检查并移除开头的括号
    if cleaned_slice.starts_with('(') && !cleaned_slice.is_empty() {
        cleaned_slice = &cleaned_slice[1..]; // 切掉第一个字符
    }
    // 检查并移除结尾的括号
    if cleaned_slice.ends_with(')') && !cleaned_slice.is_empty() {
        cleaned_slice = &cleaned_slice[..cleaned_slice.len() - 1]; // 切掉最后一个字符
    }

    cleaned_slice.trim().to_string() // 再次去除可能因移除括号而产生的空白，并转换为String
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
        // 如果希望保留纯空格行（例如，一个文本为" "的音节），此逻辑可能需要调整。
        // 当前的组合效果是，纯粹由空音节或仅含一个空格音节（且被视为空）的行会被完全移除。
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
    format!("[{:02}:{:02}.{:03}]", minutes, seconds, milliseconds) // 格式化输出
}
