// 导入 once_cell 用于静态初始化 Regex，以及 regex 本身
use once_cell::sync::Lazy;
use regex::Regex;
// 从项目中导入类型定义：ConvertError (错误类型) 和 ParsedLyricifyLine (LYL行结构)
use crate::types::{ConvertError, ParsedLyricifyLine};

// 静态正则表达式：匹配 Lyricify Lines 的行格式
// ^\[(\d+),(\d+)\](.*)$
// - `^`: 匹配行首
// - `\[`: 匹配开方括号
// - `(\d+)`: 捕获第一个数字序列（开始时间，毫秒）到捕获组1
// - `,`: 匹配逗号
// - `(\d+)`: 捕获第二个数字序列（结束时间，毫秒）到捕获组2
// - `\]`: 匹配闭方括号
// - `(.*)`: 捕获方括号之后的所有剩余字符（歌词文本）到捕获组3
// - `$`: 匹配行尾
static LYRICIFY_LINE_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\[(\d+),(\d+)\](.*)$").expect("未能编译 LYRICIFY_LINE_REGEX"));

/// 解析 Lyricify Lines (LYL) 格式的文本内容。
///
/// LYL 格式通常是：
/// `[type:LyricifyLines]` (可选的头部，指示格式类型)
/// `[开始时间,结束时间]歌词文本`
/// `[开始时间,结束时间]另一行歌词文本`
/// ...
/// 时间单位为毫秒。
///
/// # Arguments
/// * `content` - 包含完整 LYL 文件内容的字符串。
///
/// # Returns
/// `Result<Vec<ParsedLyricifyLine>, ConvertError>` -
/// 如果成功，返回一个包含所有解析出的 `ParsedLyricifyLine` 结构体的向量；否则返回错误。
pub fn parse_lyricify_lines(content: &str) -> Result<Vec<ParsedLyricifyLine>, ConvertError> {
    let mut lines_vec: Vec<ParsedLyricifyLine> = Vec::new(); // 用于存储解析结果的向量

    // 逐行处理输入内容
    for (i, line_str_raw) in content.lines().enumerate() {
        let line_num = i + 1; // 行号从1开始，用于日志和错误报告
        let trimmed_line = line_str_raw.trim(); // 去除行首尾空格

        // 跳过空行和可选的类型声明头部 "[type:LyricifyLines]"
        if trimmed_line.is_empty() || trimmed_line.eq_ignore_ascii_case("[type:LyricifyLines]") {
            continue;
        }

        // 尝试使用正则表达式匹配当前行
        if let Some(caps) = LYRICIFY_LINE_REGEX.captures(trimmed_line) {
            // 从捕获组中提取开始时间、结束时间和文本内容
            // caps.get(0) 是整个匹配的字符串
            // caps.get(1) 是第一个捕获组 (开始时间)
            // caps.get(2) 是第二个捕获组 (结束时间)
            // caps.get(3) 是第三个捕获组 (歌词文本)
            let start_ms_str = caps.get(1).map_or("", |m| m.as_str());
            let end_ms_str = caps.get(2).map_or("", |m| m.as_str());
            let text_content = caps.get(3).map_or("", |m| m.as_str()).to_string();

            // 解析开始时间字符串为 u64 毫秒数
            let start_ms: u64 = start_ms_str.parse().map_err(|e| {
                log::error!(
                    "Lyricify Line 处理错误 (行 {line_num}): 无效的开始时间 '{start_ms_str}': {e}"
                );
                ConvertError::InvalidTime(format!(
                    "行 {line_num}：无效的开始时间 '{start_ms_str}' : {e}"
                ))
            })?;
            // 解析结束时间字符串为 u64 毫秒数
            let end_ms: u64 = end_ms_str.parse().map_err(|e| {
                log::error!(
                    "Lyricify Line 处理错误 (行 {line_num}): 无效的结束时间 '{end_ms_str}': {e}"
                );
                ConvertError::InvalidTime(format!(
                    "行 {line_num}: 无效的结束时间 '{end_ms_str}': {e}"
                ))
            })?;

            // 检查结束时间是否小于开始时间，如果是，则记录一个警告
            // 歌词行本身仍然会被创建，但这个时间戳问题可能会在后续处理中导致非预期行为
            if end_ms < start_ms {
                log::warn!(
                    "Lyricify Line {line_num}: 结束时间 {end_ms} 毫秒 在开始时间 {start_ms} 毫秒之前。行: '{trimmed_line}'"
                );
            }

            // 创建 ParsedLyricifyLine 结构并添加到结果向量中
            lines_vec.push(ParsedLyricifyLine {
                start_ms,
                end_ms,
                text: text_content,
            });
        } else {
            // 如果行不符合 LYL 格式，记录一个警告
            log::warn!("无效的 Lyricify Line 格式 (行 {line_num}): '{trimmed_line}'");
        }
    }
    Ok(lines_vec) // 返回解析出的所有行
}
