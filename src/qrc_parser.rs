// 导入所需的模块和类型
use once_cell::sync::Lazy; // 用于创建静态的、只初始化一次的全局变量
use regex::Regex; // 用于正则表达式匹配

// 从项目类型模块中导入 AssMetadata（用于存储元数据）、ConvertError（错误类型）、
// LysSyllable（QRC音节结构，与LYS和KRC共用）和 QrcLine（QRC行结构）
use crate::types::{AssMetadata, ConvertError, LysSyllable, QrcLine};

// 静态正则表达式，用于匹配QRC行级别的时间戳，例如 "[12345,5000]"
// (?P<start>\d+) 捕获开始时间（毫秒）到名为 "start" 的组
// (?P<duration>\d+) 捕获持续时间（毫秒）到名为 "duration" 的组
static QRC_LINE_TIMESTAMP_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\[(?P<start>\d+),(?P<duration>\d+)\]").expect("未能编译 QRC_LINE_TIMESTAMP_REGEX") // 如果编译失败则 panic
});

// 静态正则表达式，用于匹配QRC音节级别的时间戳和文本，例如 "歌词(12345,500)"
// \((?P<start>\d+),(?P<duration>\d+)\) 捕获音节的开始时间和持续时间
// 注意：QRC的音节文本是在时间戳之前的，这个正则主要用于定位时间戳本身，文本通过切片获取
static WORD_TIMESTAMP_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\((?P<start>\d+),(?P<duration>\d+)\)").expect("未能编译 WORD_TIMESTAMP_REGEX")
});

// 静态正则表达式，用于匹配通用的元数据标签，例如 "[ti:歌曲标题]"
// (?P<key>[a-zA-Z0-9_]+) 捕获元数据键到名为 "key" 的组
// (?P<value>.*?) 捕获元数据值到名为 "value" 的组 (非贪婪匹配)
static METADATA_TAG_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\[(?P<key>[a-zA-Z0-9_]+):(?P<value>.*?)\]$").expect("未能编译 METADATA_TAG_REGEX")
});

/// 解析单行 QRC 歌词文本。
///
/// # Arguments
/// * `line_str` - 要解析的单行 QRC 文本。
/// * `line_num` - 当前行号，用于错误报告。
///
/// # Returns
/// `Result<QrcLine, ConvertError>` - 如果解析成功，返回包含行信息的 `QrcLine`；否则返回错误。
pub fn parse_qrc_line(line_str: &str, line_num: usize) -> Result<QrcLine, ConvertError> {
    // 尝试匹配行级别的时间戳
    let line_ts_cap = QRC_LINE_TIMESTAMP_REGEX.captures(line_str).ok_or_else(|| {
        // 如果匹配失败，返回格式错误
        ConvertError::InvalidQrcFormat {
            line_num,
            message: "行首缺少行时间戳标记 [start,duration]".to_string(),
        }
    })?;

    // 从捕获组中提取行开始时间和持续时间的字符串
    let line_start_ms_str = line_ts_cap.name("start").unwrap().as_str();
    let line_duration_ms_str = line_ts_cap.name("duration").unwrap().as_str();

    // 将字符串转换为 u64 类型的毫秒数
    let line_start_ms: u64 =
        line_start_ms_str
            .parse()
            .map_err(|_| ConvertError::InvalidQrcLineTimestamp {
                line_num,
                timestamp_str: format!("[{line_start_ms_str},{line_duration_ms_str}]"),
            })?;
    let line_duration_ms: u64 =
        line_duration_ms_str
            .parse()
            .map_err(|_| ConvertError::InvalidQrcLineTimestamp {
                line_num,
                timestamp_str: format!("[{line_start_ms_str},{line_duration_ms_str}]"),
            })?;

    // 获取行时间戳之后的内容，这部分包含音节文本和音节时间戳
    let content_after_line_ts = &line_str[line_ts_cap.get(0).unwrap().end()..];
    let mut syllables: Vec<LysSyllable> = Vec::new(); // 用于存储解析出的音节
    let mut last_text_segment_end = 0; // 跟踪上一个音节时间戳之后文本部分的结束位置

    // 遍历所有匹配到的音节时间戳
    for ts_cap_match in WORD_TIMESTAMP_REGEX.find_iter(content_after_line_ts) {
        let timestamp_tag_start = ts_cap_match.start(); // 音节时间戳标记的开始位置
        let timestamp_tag_end = ts_cap_match.end(); // 音节时间戳标记的结束位置

        // 提取位于上一个时间戳标记之后、当前时间戳标记之前的文本作为音节文本
        let text_slice = &content_after_line_ts[last_text_segment_end..timestamp_tag_start];

        // 再次捕获当前时间戳标记内的具体时间和时长
        let captures = WORD_TIMESTAMP_REGEX
            .captures(ts_cap_match.as_str())
            .unwrap();
        let syl_start_ms_str = captures.name("start").unwrap().as_str();
        let syl_duration_ms_str = captures.name("duration").unwrap().as_str();

        // 解析音节的开始时间和持续时间
        let syl_start_ms: u64 =
            syl_start_ms_str
                .parse()
                .map_err(|e| ConvertError::InvalidLysSyllable {
                    line_num,
                    text: text_slice.to_string(),
                    message: format!("音节开始时间无效 '{syl_start_ms_str}': {e}"),
                })?;
        let syl_duration_ms: u64 =
            syl_duration_ms_str
                .parse()
                .map_err(|e| ConvertError::InvalidLysSyllable {
                    line_num,
                    text: text_slice.to_string(),
                    message: format!("音节持续时长无效 '{syl_duration_ms_str}': {e}"),
                })?;

        // 创建 LysSyllable 结构并添加到列表中
        syllables.push(LysSyllable {
            text: text_slice.to_string(), // 音节文本
            start_ms: syl_start_ms,       // 音节绝对开始时间
            duration_ms: syl_duration_ms, // 音节持续时间
        });

        // 更新下一个文本段的开始位置
        last_text_segment_end = timestamp_tag_end;
    }

    // 处理最后一个音节时间戳之后的文本（如果有）
    if last_text_segment_end < content_after_line_ts.len() {
        let remaining_text = &content_after_line_ts[last_text_segment_end..];
        if !remaining_text.trim().is_empty() {
            // QRC 格式要求所有文本都必须有关联的时间戳，所以行尾不应有多余文本
            log::warn!(
                "行 {line_num}: 在最后一个音节时间戳标记后发现未处理的文本: '{remaining_text}' ，已忽略"
            );
        }
    }

    // 如果音节列表为空，但行内容（除行时间戳外）非空，说明格式可能有问题
    if syllables.is_empty() && !content_after_line_ts.trim().is_empty() {
        return Err(ConvertError::InvalidQrcFormat {
            line_num,
            message: format!("无法从内容 '{content_after_line_ts}' 中解析出任何有效音节时间戳。"),
        });
    }

    // 返回解析成功的 QrcLine
    Ok(QrcLine {
        line_start_ms,
        line_duration_ms,
        syllables,
    })
}

/// 从字符串加载并解析 QRC 内容。
///
/// # Arguments
/// * `qrc_content` - 包含完整 QRC 文件内容的字符串。
///
/// # Returns
/// `Result<(Vec<QrcLine>, Vec<AssMetadata>), ConvertError>` -
/// 如果成功，返回一个元组，包含解析出的 QRC 行列表和元数据列表；否则返回错误。
pub fn load_qrc_from_string(
    qrc_content: &str,
) -> Result<(Vec<QrcLine>, Vec<AssMetadata>), ConvertError> {
    let mut qrc_lines_vec: Vec<QrcLine> = Vec::new(); // 存储解析后的歌词行
    let mut qrc_metadata_vec: Vec<AssMetadata> = Vec::new(); // 存储解析后的元数据

    // 逐行处理输入内容
    for (i, line_str_raw) in qrc_content.lines().enumerate() {
        let line_num = i + 1; // 行号从1开始
        let trimmed_line = line_str_raw.trim(); // 去除行首尾空格

        if trimmed_line.is_empty() {
            continue; // 跳过空行
        }

        // 尝试匹配元数据标签
        if let Some(meta_caps) = METADATA_TAG_REGEX.captures(trimmed_line) {
            let key = meta_caps.name("key").map_or("", |m| m.as_str());
            let value = meta_caps
                .name("value")
                .map_or("", |m| m.as_str())
                .trim()
                .to_string();

            // 根据键名进行特定处理或存储
            match key {
                "ti" => {
                    // 歌曲标题
                    qrc_metadata_vec.push(AssMetadata {
                        key: "musicName".to_string(),
                        value,
                    });
                }
                "ar" => {
                    // 艺术家
                    // QRC的艺术家字段可能包含多个艺术家，用 "/" 分隔
                    if value.contains('/') {
                        for artist_part in value.split('/') {
                            let trimmed_artist_part = artist_part.trim();
                            if !trimmed_artist_part.is_empty() {
                                qrc_metadata_vec.push(AssMetadata {
                                    key: "artists".to_string(), // 内部统一使用 "artists"
                                    value: trimmed_artist_part.to_string(),
                                });
                            }
                        }
                    } else if !value.is_empty() {
                        qrc_metadata_vec.push(AssMetadata {
                            key: "artists".to_string(),
                            value,
                        });
                    }
                }
                "al" => {
                    // 专辑
                    qrc_metadata_vec.push(AssMetadata {
                        key: "album".to_string(),
                        value,
                    });
                }
                "by" => {
                    // LRC制作者/信息来源
                    qrc_metadata_vec.push(AssMetadata {
                        key: "ttmlAuthorGithubLogin".to_string(),
                        value,
                    });
                }
                "kana" => { // QRC特有的假名/罗马音信息，这里暂不特殊处理，按原样存
                    // qrc_metadata_vec.push(AssMetadata { key: "kana".to_string(), value });
                }
                "offset" => { // 时间偏移
                    // qrc_metadata_vec.push(AssMetadata { key: "offset".to_string(), value });
                }
                _ => {
                    // 其他未识别的元数据标签
                    log::warn!(
                        "行 {line_num}: 未知的 QRC/LYS 元数据类型 '{key}' (内容: '{value}')"
                    );
                    // 也可以选择将未知标签作为自定义元数据存储
                    // qrc_metadata_vec.push(AssMetadata { key: key.to_string(), value });
                }
            }
        }
        // 尝试匹配歌词行的时间戳
        else if QRC_LINE_TIMESTAMP_REGEX.is_match(trimmed_line) {
            match parse_qrc_line(trimmed_line, line_num) {
                Ok(parsed_line) => {
                    // 只有当解析出的行包含音节时才添加
                    // 或者，如果行本身（除去行时间戳）非空，即使没有音节时间戳，也可能需要警告或特殊处理
                    if !parsed_line.syllables.is_empty() {
                        qrc_lines_vec.push(parsed_line);
                    } else if !trimmed_line
                        .trim_start_matches(|c: char| {
                            c == '[' || c.is_ascii_digit() || c == ',' || c == ']'
                        })
                        .is_empty()
                    {
                        // 如果行内容（除去行时间戳的部分）不为空，但没有解析出音节，记录警告
                        log::warn!("行 {line_num}: '{trimmed_line}' 解析后没有有效音节。");
                    }
                    // 如果行内容也为空（纯时间戳行），则通常忽略
                }
                Err(e) => {
                    // 解析单行失败，记录错误
                    log::error!("解析 QRC 行 {line_num} ('{trimmed_line}') 失败: {e}");
                }
            }
        }
        // 如果行既不是元数据也不是有效的歌词行格式
        else {
            log::warn!("行 {line_num}: 无法识别的 QRC 行: '{trimmed_line}'");
        }
    }
    // 返回解析结果
    Ok((qrc_lines_vec, qrc_metadata_vec))
}
