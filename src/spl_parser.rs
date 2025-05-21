// 导入项目中定义的类型：
// AssMetadata: 用于存储元数据（尽管SPL本身不包含元数据标签，但为了与其他格式兼容可能保留）。
// ConvertError: 错误处理枚举，用于表示解析过程中可能发生的错误。
// LysSyllable: 代表一个歌词音节（或文本片段）及其计时信息，是逐字歌词处理的基础。
use crate::types::{AssMetadata, ConvertError, LysSyllable};

// 导入 once_cell::sync::Lazy 用于延迟初始化静态变量（这里是正则表达式）。
// 这样可以确保正则表达式只在首次使用时编译一次，提高效率。
use once_cell::sync::Lazy;
// 导入 regex::Regex 用于处理正则表达式匹配。
use regex::Regex;

// 定义静态的正则表达式 SPL_SINGLE_LINE_START_REGEX。
// Lazy确保它在首次访问时才被编译。
// 这个正则表达式用于捕获一行文本开头的一个或多个标准SPL时间戳 `[分:秒.毫秒]`。
// - `^`: 匹配行的开始。
// - `\[`: 匹配左方括号 `[`。
// - `(\d{1,3}:\d{1,2}\.\d{1,6})`: 捕获组1，匹配时间戳内容。
//   - `\d{1,3}`: 匹配1到3位数字（分钟部分）。
//   - `:`: 匹配冒号。
//   - `\d{1,2}`: 匹配1到2位数字（秒钟部分）。
//   - `\.`: 匹配点号。
//   - `\d{1,6}`: 匹配1到6位数字（毫秒部分）。
// - `\]`: 匹配右方括号 `]`。
// `expect` 用于在正则表达式编译失败时 panic，因为这是程序启动时的关键部分。
static SPL_SINGLE_LINE_START_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\[(\d{1,3}:\d{1,2}\.\d{1,6})\]").expect("未能编译 SPL_SINGLE_LINE_START_REGEX")
});

// 定义静态的正则表达式 SPL_INLINE_TIMESTAMP_REGEX。
// 这个正则表达式用于查找行内的时间戳，可以是方括号 `[mm:ss.xx]` 或尖括号 `<mm:ss.xx>`。
// 这些内联时间戳用于逐字歌词（卡拉OK效果）或标记显式的行结束时间。
// - `\[(\d{1,3}:\d{1,2}\.\d{1,6})\]`: 匹配并捕获方括号内的时间戳（捕获组1）。
// - `|`: 或操作符。
// - `<(\d{1,3}:\d{1,2}\.\d{1,6})>`: 匹配并捕获尖括号内的时间戳（捕获组2）。
//   注意：实际使用时，需要检查哪个捕获组匹配成功。
//   如果方括号匹配，则caps.get(1)有值；如果尖括号匹配，则caps.get(2)有值。
//    后续代码中 `inline_match.as_str()` 获取整个匹配，然后判断首尾字符来提取内部时间戳，这样更简单。
static SPL_INLINE_TIMESTAMP_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\[(\d{1,3}:\d{1,2}\.\d{1,6})\]|<(\d{1,3}:\d{1,2}\.\d{1,6})>")
        .expect("未能编译 SPL_INLINE_TIMESTAMP_REGEX")
});

/// 代表一个已解析的 SPL 歌词块 (SplLineBlock)。
/// 一个歌词块通常由一个主歌词行和所有与之关联的翻译行组成。
/// 它也可能包含多个起始时间戳（用于重复行）。
#[derive(Debug, Clone, PartialEq)] // 为结构体派生 Debug, Clone, PartialEq traits
pub struct SplLineBlock {
    /// `Vec<u64>`: 存储此歌词块主歌词行的所有行首时间戳（已转换为毫秒）。
    /// 例如 `[00:10.00][00:25.00]歌词` 会在这里存储 `vec![10000, 25000]`。
    pub start_times_ms: Vec<u64>,
    /// `String`: 主歌词行的文本内容。这部分文本可能仍然包含内联的逐字时间戳
    /// (如 `你好<00:01.00>世界[00:02.00]`)，需要后续进一步解析。
    pub main_text_with_inline_ts: String,
    /// `Vec<String>`: 存储所有属于这个歌词块的翻译行文本。
    /// 这包括了同时间戳的翻译行和紧跟在主歌词行或同时间戳翻译行之后的无时间戳翻译行。
    pub all_translation_lines: Vec<String>,
    /// `Option<u64>`: 存储整个歌词块的显式结束时间（已转换为毫秒）。
    /// 例如 `[00:01.00]歌词[00:03.00]`，这里的 `3000` 会被存储。
    /// 如果没有显式的块结束时间戳，则为 `None`。
    pub explicit_block_end_ms: Option<u64>,
}

/// 将 SPL 时间戳字符串（如 "mm:ss.xx" 或 "mm:ss.xxx"）解析为总毫秒数。
///
/// # Arguments
/// * `timestamp_str` - `&str` 类型，表示要解析的时间戳字符串 (不包含外层的 `[]` 或 `<>`)。
///
/// # Returns
/// `Result<u64, ConvertError>` - 如果解析成功，返回 `Ok(u64)` 包含总毫秒数；
///                               否则返回 `Err(ConvertError::InvalidTime)` 并附带错误信息。
fn parse_spl_timestamp_to_ms(timestamp_str: &str) -> Result<u64, ConvertError> {
    // 按 ':' 和 '.' 分割时间戳字符串
    let parts: Vec<&str> = timestamp_str.split([':', '.']).collect();
    // SPL 时间戳必须由三部分组成：分、秒、毫秒
    if parts.len() != 3 {
        return Err(ConvertError::InvalidTime(format!(
            "无效的SPL时间戳格式: {}",
            timestamp_str
        )));
    }

    // 解析分钟部分
    let minutes: u64 = parts[0].parse().map_err(|e| {
        ConvertError::InvalidTime(format!("时间戳 '{}' 中的分钟无效: {}", timestamp_str, e))
    })?;
    // 解析秒钟部分
    let seconds: u64 = parts[1].parse().map_err(|e| {
        ConvertError::InvalidTime(format!("时间戳 '{}' 中的秒钟无效: {}", timestamp_str, e))
    })?;

    // 解析毫秒部分
    let fraction_str = parts[2];
    let milliseconds: u64 = match fraction_str.len() {
        // SPL规范：毫秒限制1至6位数字。不足3位的写法将视为在后位省略了0。
        // 例如：.1 视为 .100 (100毫秒)，.02 视为 .020 (20毫秒)。
        1 => {
            fraction_str.parse::<u64>().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "时间戳中的第一位秒钟 '{}' 无效: {}",
                    timestamp_str, e
                ))
            })? * 100
        } // 1位毫秒，乘以100
        2 => {
            fraction_str.parse::<u64>().map_err(|e| {
                ConvertError::InvalidTime(format!(
                    "时间戳中的第二位秒钟 '{}' 无效: {}",
                    timestamp_str, e
                ))
            })? * 10
        } // 2位毫秒，乘以10
        3 => fraction_str.parse::<u64>().map_err(|e| {
            ConvertError::InvalidTime(format!(
                "时间戳中的第三位秒钟 '{}' 无效: {}",
                timestamp_str, e
            ))
        })?, // 3位毫秒，直接使用
        // SPL规范允许最多6位毫秒，但通常只使用前3位。这里截取前3位进行解析。
        4..=6 => fraction_str[0..3].parse::<u64>().map_err(|e| {
            ConvertError::InvalidTime(format!(
                "时间戳中的第四至六位秒钟 '{}' 无效: {}",
                timestamp_str, e
            ))
        })?, // 4到6位毫秒，取前3位
        // 其他长度的毫秒部分视为无效
        _ => {
            return Err(ConvertError::InvalidTime(format!(
                "时间戳中的毫秒部分无效: '{}'",
                parts[2]
            )));
        }
    };

    // 计算总毫秒数：(分钟 * 60 + 秒) * 1000 + 毫秒
    Ok((minutes * 60 + seconds) * 1000 + milliseconds)
}

/// 辅助函数：从一行文本的开头提取所有连续的行首时间戳，并返回这些时间戳（转换为毫秒）以及剩余的文本。
/// 例如，输入 `"[00:01.00][00:02.00]Some text"`
/// 会返回 `(vec![1000, 2000], "Some text".to_string())`。
///
/// # Arguments
/// * `line_text` - `&str` 类型，表示要处理的单行文本。
/// * `line_num_for_log` - `usize` 类型，当前行号，用于日志记录。
///
/// # Returns
/// `(Vec<u64>, String)` - 一个元组，包含：
///   - `Vec<u64>`: 提取到的所有行首时间戳的毫秒值列表。
///   - `String`: 移除了所有行首时间戳后，剩余的文本部分（已去除前导空白）。
fn extract_leading_timestamps(line_text: &str, line_num_for_log: usize) -> (Vec<u64>, String) {
    let mut timestamps_ms: Vec<u64> = Vec::new(); // 初始化存储时间戳的向量
    let mut remaining_text = line_text; // 初始化剩余文本为完整行文本

    // 循环匹配行首时间戳
    // `SPL_SINGLE_LINE_START_REGEX.captures(remaining_text)` 尝试在 `remaining_text` 的开头匹配时间戳
    while let Some(caps) = SPL_SINGLE_LINE_START_REGEX.captures(remaining_text) {
        // `caps.get(1).unwrap().as_str()` 获取第一个捕获组的内容（即时间戳 "mm:ss.xxx" 部分）
        let ts_str = caps.get(1).unwrap().as_str();
        // 解析提取到的时间戳字符串为毫秒
        match parse_spl_timestamp_to_ms(ts_str) {
            Ok(ms) => timestamps_ms.push(ms), // 解析成功，添加到列表中
            Err(e) => {
                // 解析失败，记录警告日志，包含行号、错误的时间戳和错误信息
                log::warn!(
                    "[SPL 处理] 行 {}: 无效的行首时间戳 '{}': {}",
                    line_num_for_log,
                    ts_str,
                    e
                );
            }
        }
        // 更新 `remaining_text`，移除已匹配的时间戳部分，并去除可能的前导空格
        // `caps.get(0).unwrap().end()` 获取整个匹配（包括 `[]`）的结束位置索引
        remaining_text = remaining_text[caps.get(0).unwrap().end()..].trim_start();
    }
    (timestamps_ms, remaining_text.to_string()) // 返回提取的时间戳列表和剩余文本
}

/// 从字符串加载 SPL (Salt Player Lyric) 内容，并将其解析为 `SplLineBlock` 的向量。
/// SPL 文件没有特定的元数据标签，所以元数据部分通常为空。
///
/// # Arguments
/// * `spl_content` - `&str` 类型，包含完整 SPL 文件内容的字符串。
///
/// # Returns
/// `Result<(Vec<SplLineBlock>, Vec<AssMetadata>), ConvertError>` - 如果解析成功，返回：
///   - `Ok((spl_blocks, spl_metadata))`：
///     - `spl_blocks`: 一个 `Vec<SplLineBlock>`，包含了所有解析出的歌词块。
///     - `spl_metadata`: 一个空的 `Vec<AssMetadata>`，因为 SPL 不支持元数据。
///   - 如果发生不可恢复的错误，则返回 `Err(ConvertError)`。
pub fn load_spl_from_string(
    spl_content: &str,
) -> Result<(Vec<SplLineBlock>, Vec<AssMetadata>), ConvertError> {
    let mut spl_blocks: Vec<SplLineBlock> = Vec::new(); // 初始化存储解析结果的歌词块向量
    let spl_metadata: Vec<AssMetadata> = Vec::new(); // SPL没有元数据，所以元数据向量为空

    // 将输入内容按行分割，并使用 `enumerate` 获取行号（从0开始），`peekable` 允许查看下一行而不消耗它。
    let mut line_iterator = spl_content.lines().enumerate().peekable();

    // 遍历每一行
    while let Some((line_idx, raw_line_str)) = line_iterator.next() {
        let current_log_line_num = line_idx + 1; // 日志中使用的行号（从1开始）
        let trimmed_line_str = raw_line_str.trim(); // 去除当前行的首尾空白

        // 如果行去除空白后为空，则跳过
        if trimmed_line_str.is_empty() {
            continue;
        }

        // 提取当前行的所有行首时间戳和剩余文本
        let (current_block_start_times_ms, mut current_block_main_text_line) =
            extract_leading_timestamps(trimmed_line_str, current_log_line_num);

        // 如果当前行没有行首时间戳
        if current_block_start_times_ms.is_empty() {
            // 根据SPL规范，没有时间戳的行如果紧跟在一个有时间戳的行之后，可以被视作该行的翻译。
            // 这个逻辑由下面的 `peekable` 循环处理（当它查找隐式翻译时）。
            // 如果一个无时间戳行出现在文件开头，或者前面是空行，它就是孤立的。
            // 记录警告并跳过这种孤立的无时间戳行。
            log::warn!(
                "[SPL 处理] 行 {}: 跳过无行首时间戳的行: '{}'",
                current_log_line_num,
                trimmed_line_str
            );
            continue;
        }

        // 到这里，`current_block_start_times_ms` 至少包含一个时间戳，
        // `current_block_main_text_line` 是这一行移除了所有行首时间戳后的文本。
        // 这个文本被视为主歌词（可能包含内联时间戳）。

        let mut current_block_all_translations: Vec<String> = Vec::new(); // 初始化当前歌词块的翻译行列表
        // `last_line_of_block_text` 用于追踪当前块中处理的最后一行文本。
        // 这很重要，因为显式的块结束时间戳 `[...end_time]` 可能出现在主歌词行尾，也可能出现在最后一行翻译的行尾。
        let mut last_line_of_block_text = current_block_main_text_line.clone();

        // --- 收集翻译行 ---
        // 查看下一行（如果存在），以判断它是否是当前块的翻译。
        // 循环会持续到下一行不是当前块的翻译为止。
        while let Some((peek_line_idx, peek_raw_line_str)) = line_iterator.peek() {
            let peek_trimmed_line = peek_raw_line_str.trim(); // 去除下一行的首尾空白
            let peek_log_line_num = peek_line_idx + 1;

            // 如果下一行是空行，则消耗它并继续查看更下一行
            if peek_trimmed_line.is_empty() {
                line_iterator.next(); // 消耗空行
                continue;
            }

            // 提取下一行的行首时间戳和文本
            let (peek_line_timestamps, peek_line_text) =
                extract_leading_timestamps(peek_trimmed_line, peek_log_line_num);

            if !peek_line_timestamps.is_empty() {
                // 如果下一行有行首时间戳
                // 检查这些时间戳是否与当前块的主歌词行的时间戳完全相同
                if peek_line_timestamps == current_block_start_times_ms {
                    // 时间戳相同，说明这是“同时间戳翻译”。
                    current_block_all_translations.push(peek_line_text.clone());
                    last_line_of_block_text = peek_line_text; // 更新块的最后一行文本
                    line_iterator.next(); // 消耗这一行翻译
                } else {
                    // 时间戳不同，说明下一行是一个新的歌词块的开始。
                    // 停止收集当前块的翻译。
                    break;
                }
            } else {
                // 下一行没有行首时间戳。根据SPL规范，这被视为“隐式翻译”。
                // `peek_line_text` 此时是整行文本（因为它没有行首时间戳被移除）。
                current_block_all_translations.push(peek_line_text.clone());
                last_line_of_block_text = peek_line_text; // 更新块的最后一行文本
                line_iterator.next(); // 消耗这一行翻译
            }
        } // 结束翻译行收集循环

        // --- 检查块的显式结束时间戳 ---
        // 显式结束时间戳 `[mm:ss.xxx]` 可能出现在 `last_line_of_block_text` 的末尾。
        let mut explicit_block_end_ms: Option<u64> = None;

        // 步骤1: 尝试从当前块的最后一行文本（主歌词或最后一条翻译）的末尾提取结束时间戳
        if let Some(last_ts_match) = SPL_INLINE_TIMESTAMP_REGEX
            .find_iter(&last_line_of_block_text)
            .last()
        {
            // 检查这个最后匹配的时间戳是否确实位于该行文本的末尾
            if last_ts_match.end() == last_line_of_block_text.len() {
                // 提取时间戳字符串 (例如 `[00:03.00]` 或 `<00:03.00>`)
                let ts_str_with_brackets = last_ts_match.as_str();
                // 提取括号内的纯时间戳部分 (例如 `00:03.00`)
                // SPL规范中，行尾结束时间戳必须是 `[]`。但解析时可能灵活处理 `<>` 也作为行尾（如果它是最后一个）。
                // 此处代码不区分 `[]` 和 `<>` 作为行尾标记，只要是最后一个即可。
                // （规范：“在一句的歌词最后添加时间戳将视为歌词的结尾时间”）
                let inner_ts_str = &ts_str_with_brackets[1..ts_str_with_brackets.len() - 1];
                if let Ok(ms) = parse_spl_timestamp_to_ms(inner_ts_str) {
                    explicit_block_end_ms = Some(ms);

                    // 从包含该结束时间戳的文本行中移除它。
                    // 首先检查这个结束时间戳是否来自最后一行翻译
                    if current_block_all_translations
                        .last_mut()
                        .is_some_and(|last_trans| last_line_of_block_text == *last_trans)
                    {
                        let last_trans_mut = current_block_all_translations.last_mut().unwrap();
                        last_trans_mut.truncate(last_ts_match.start()); // 截断字符串，移除时间戳
                        *last_trans_mut = last_trans_mut.trim_end().to_string(); // 去除可能的前导空格
                        // 如果移除时间戳后翻译行为空，则从列表中移除该空翻译行
                        if last_trans_mut.is_empty() {
                            current_block_all_translations.pop();
                        }
                    }
                    // 否则，检查是否来自主歌词行
                    else if last_line_of_block_text == current_block_main_text_line {
                        current_block_main_text_line.truncate(last_ts_match.start());
                        current_block_main_text_line =
                            current_block_main_text_line.trim_end().to_string();
                    }
                    // 如果 `current_block_main_text_line` 移除时间戳后变为空，它仍然是块的一部分，除非没有翻译。
                }
            }
        }

        // 步骤2: 如果步骤1没有找到结束时间戳，则检查下一行是否为纯时间戳行
        if explicit_block_end_ms.is_none() {
            if let Some((_peek_line_idx, peek_raw_line_str)) = line_iterator.peek() {
                let peek_trimmed_line = peek_raw_line_str.trim();
                // 确保peek的行不是空行（虽然理论上外层循环已处理）
                if !peek_trimmed_line.is_empty() {
                    let (peek_line_timestamps, peek_line_remaining_text) =
                        extract_leading_timestamps(peek_trimmed_line, _peek_line_idx + 1);

                    // 如果下一行有行首时间戳，并且去除这些时间戳后文本为空
                    if !peek_line_timestamps.is_empty() && peek_line_remaining_text.is_empty() {
                        // 将下一行的第一个时间戳作为当前块的结束时间
                        explicit_block_end_ms = Some(peek_line_timestamps[0]);
                        // 消耗掉这个纯时间戳行，因为它已经被用于确定上一行的结束时间
                        line_iterator.next();
                        log::info!(
                            "[SPL 处理] 行 {}: 使用下一行 '{}' 的时间戳 {}ms 作为结束时间。",
                            current_log_line_num,
                            peek_trimmed_line,
                            peek_line_timestamps[0]
                        );
                    }
                }
            }
        }

        // --- 构建并添加 SplLineBlock ---
        // 只有当主歌词行在处理完行首和可能的行尾时间戳后仍然非空，
        // 或者即使主歌词行为空但存在翻译行时，才认为这是一个有效的歌词块。
        // 例如，`[00:01.00][00:02.00]` 这样的行，`current_block_main_text_line` 会是空。
        // 如果它没有翻译，则不应被添加为一个歌词块（它代表静默，通常由 `TtmlParagraph` 的 p_start/end_ms 处理）。
        // 但如果它有翻译，如 `[00:01.00]\nTranslation`，则应添加。
        // （此逻辑可能需要根据具体如何处理纯时间戳行来调整，当前实现是如果主文本为空且无翻译，则不添加）
        if !current_block_main_text_line.is_empty() || !current_block_all_translations.is_empty() {
            spl_blocks.push(SplLineBlock {
                start_times_ms: current_block_start_times_ms,
                main_text_with_inline_ts: current_block_main_text_line,
                all_translation_lines: current_block_all_translations,
                explicit_block_end_ms,
            });
        } else if explicit_block_end_ms.is_some()
            && current_block_start_times_ms
                .first()
                .is_some_and(|s| s < &explicit_block_end_ms.unwrap_or(0))
        {
            // 处理纯时间戳行（如 [start][end]）的情况，如果它没有被用作前一行的结束时间
            // 这种情况是：一个行只有行首时间戳，然后它的结束时间来自下一行的纯时间戳行，
            // 导致 main_text 和 all_translations 都为空，但它确实定义了一个有持续时间的静默。
            // 或者，一个行是 [start_time][end_time] 自身定义了静默。
            spl_blocks.push(SplLineBlock {
                start_times_ms: current_block_start_times_ms,
                main_text_with_inline_ts: String::new(), // 主文本为空
                all_translation_lines: Vec::new(),       // 翻译为空
                explicit_block_end_ms,                   // 但有显式结束时间
            });
        }
    } // 结束对所有输入行的遍历

    Ok((spl_blocks, spl_metadata)) // 返回解析得到的歌词块列表和空的元数据列表
}

/// 解析 `SplLineBlock` 中的主歌词行文本（可能包含内联时间戳），
/// 将其分解为一系列 `LysSyllable`（歌词音节/片段）。
/// 这用于处理 SPL 的逐字歌词特性。
///
/// # Arguments
/// * `main_text_line_content` - `&str` 类型，主歌词行的文本内容，例如 "你好<00:01.00>世界[00:02.00]"。
/// * `line_start_time_ms` - `u64` 类型，该歌词行的起始时间（来自行首时间戳）。
/// * `line_overall_end_time_ms` - `u64` 类型，该歌词行的整体结束时间。
///   这通常是 `SplLineBlock` 的 `explicit_block_end_ms`，
///   或者是下一行歌词的开始时间（如果是隐式结尾）。
///   这个值用于确定最后一个音节的持续时间。
/// * `line_num_for_log` - `usize` 类型，当前行号，用于日志记录。
///
/// # Returns
/// `Result<Vec<LysSyllable>, ConvertError>` - 如果解析成功，返回 `Ok(Vec<LysSyllable>)` 包含音节列表；
///                                           否则返回 `Err(ConvertError)`。
pub fn parse_spl_main_text_to_syllables(
    main_text_line_content: &str,
    line_start_time_ms: u64,
    line_overall_end_time_ms: u64,
    line_num_for_log: usize, // 当前未使用，但保留用于可能的详细日志
) -> Result<Vec<LysSyllable>, ConvertError> {
    let mut syllables: Vec<LysSyllable> = Vec::new(); // 初始化音节列表
    let mut current_segment_start_ms = line_start_time_ms; // 当前文本段的开始时间，初始为行的开始时间
    let mut last_match_end_char_idx = 0; // 上一个内联时间戳匹配结束的字符索引

    // 遍历主歌词行文本中所有匹配到的内联时间戳
    for inline_match in SPL_INLINE_TIMESTAMP_REGEX.find_iter(main_text_line_content) {
        // 提取当前内联时间戳之前的文本段
        let segment_text_str =
            &main_text_line_content[last_match_end_char_idx..inline_match.start()];

        // 提取并解析内联时间戳
        let ts_str_with_brackets = inline_match.as_str(); // 例如 "[00:01.00]" 或 "<00:01.00>"
        let inner_ts_str = &ts_str_with_brackets[1..ts_str_with_brackets.len() - 1]; // "00:01.00"
        let inline_timestamp_ms = parse_spl_timestamp_to_ms(inner_ts_str)?; // 解析为毫秒

        // SPL规范：逐字标记时间戳需要递增。如果出现时间戳不在行开始时间和结束时间之间，
        // 或者小于之前的时间戳，那么此逐字标记时间戳会被忽略。
        // 此处进行检查并记录警告，但仍然使用该时间戳作为分割点。
        // 当前实现是：即使时间戳无序，也用它来分割文本，但时长计算可能会受影响。
        if inline_timestamp_ms < current_segment_start_ms
            || inline_timestamp_ms > line_overall_end_time_ms
        {
            log::warn!(
                "[SPL 处理] 第 ~{} 行：时间戳 {}ms (从 '{}' 解析) 顺序错误或越界。当前开始时间 {}ms，行结束时间 {}ms。文本: '{}'。",
                line_num_for_log,
                inline_timestamp_ms,
                ts_str_with_brackets,
                current_segment_start_ms,
                line_overall_end_time_ms,
                segment_text_str
            );
            // 如果需要严格忽略，可以考虑：
            // 1. continue; (忽略这个时间戳和它之前的文本段，可能导致文本丢失)
            // 2. 或者将这个时间戳调整为 current_segment_start_ms (如果小于) 或 line_overall_end_time_ms (如果大于)
            //    但这样会改变原始计时。当前选择是记录警告并按原样使用。
        }

        // 计算当前文本段的持续时间
        // `saturating_sub` 防止下溢（如果 inline_timestamp_ms < current_segment_start_ms，结果为0）
        let duration_ms = inline_timestamp_ms.saturating_sub(current_segment_start_ms);

        // 创建 LysSyllable 对象并添加到列表中
        // 只有当文本段非空，或者即使文本为空但有明确时长时，才添加音节
        if !segment_text_str.is_empty() || duration_ms > 0 {
            syllables.push(LysSyllable {
                text: segment_text_str.to_string(), // 文本段内容
                start_ms: current_segment_start_ms, // 开始时间
                duration_ms,                        // 持续时间
            });
        }

        // 更新下一个文本段的开始时间为当前内联时间戳的时间
        current_segment_start_ms = inline_timestamp_ms;
        // 更新上一个匹配结束的字符索引
        last_match_end_char_idx = inline_match.end();
    } // 结束对内联时间戳的遍历

    // 处理最后一个内联时间戳之后（或者如果没有内联时间戳，则是整行）的剩余文本
    let last_segment_text_str = &main_text_line_content[last_match_end_char_idx..];
    // 计算最后一个文本段的持续时间
    // 它的结束时间是整个歌词行的结束时间 `line_overall_end_time_ms`
    let last_segment_duration = line_overall_end_time_ms.saturating_sub(current_segment_start_ms);

    // 如果最后一个文本段非空，或者如果整行都没有内联时间戳（此时 `syllables` 为空，`last_segment_text_str` 是整行文本），
    // 并且有实际时长，则创建并添加最后一个音节。
    // 这确保了即使一行歌词没有内联时间戳，也会被当作一个单独的音节处理。
    if !last_segment_text_str.is_empty() || (syllables.is_empty() && last_segment_duration > 0) {
        syllables.push(LysSyllable {
            text: last_segment_text_str.to_string(),
            start_ms: current_segment_start_ms,
            duration_ms: last_segment_duration,
        });
    }

    Ok(syllables) // 返回解析得到的音节列表
}
