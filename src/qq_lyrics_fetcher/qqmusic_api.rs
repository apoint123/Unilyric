use regex::Regex;
// 导入 reqwest::Client 用于发送 HTTP 请求
use reqwest::Client;
// 导入 serde 的 Deserialize 和 Serialize 特征，用于数据的序列化和反序列化
use serde::{Deserialize, Serialize};
// 导入项目中定义的通用错误类型 ConvertError
use crate::types::ConvertError;
// 导入 quick_xml 库，用于解析 QQ 音乐返回的 XML 格式歌词数据
use quick_xml::{Reader, events::Event};

// 为 API 结果定义一个类型别名，方便使用
type ApiResult<T> = std::result::Result<T, ConvertError>;

// 模块内部的配置，定义了 QQ 音乐的 API 端点
mod config {
    // QQ 音乐歌曲搜索 API 的 URL
    pub const SEARCH_API_URL: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
    // QQ 音乐歌词下载 API 的 URL (返回 QRC 等格式)
    pub const QRC_API_URL: &str = "https://c.y.qq.com/qqmusic/fcgi-bin/lyric_download.fcg";
}

/// 定义发送到 QQ 音乐搜索 API 的 JSON 请求体的顶层结构。
#[derive(Debug, Serialize)]
struct SearchRequest {
    req_1: SearchRequestBody, // 包含实际的请求参数
}

/// 定义搜索请求体中的 `req_1` 字段的结构。
#[derive(Debug, Serialize)]
struct SearchRequestBody {
    method: String,     // 请求的方法名，例如 "DoSearchForQQMusicDesktop"
    module: String,     // 请求的模块名，例如 "music.search.SearchCgiService"
    param: SearchParam, // 具体的搜索参数
}

/// 定义搜索参数的结构。
#[derive(Debug, Serialize)]
struct SearchParam {
    num_per_page: u32, // 每页返回的结果数量
    page_num: u32,     // 请求的页码
    query: String,     // 搜索关键词
    search_type: u32,  // 搜索类型
}

/// 定义从 QQ 音乐搜索 API 返回的 JSON 响应的顶层结构。
#[derive(Debug, Deserialize, Clone)]
pub struct MusicFcgApiResult {
    code: i32,                      // 顶层响应状态码
    req_1: SearchResponseContainer, // 包含实际搜索结果的容器
}

/// 定义搜索响应中 `req_1` 字段的结构。
#[derive(Debug, Deserialize, Clone)]
struct SearchResponseContainer {
    code: i32,        // 搜索操作本身的状态码 (例如 0 表示成功, 2001 表示请求被拒绝)
    data: SearchData, // 包含搜索结果数据
}

/// 定义搜索结果数据的主要部分。
#[derive(Debug, Deserialize, Clone)]
struct SearchData {
    body: SearchBody, // 结果主体
}

/// 定义搜索结果主体中的歌曲信息部分。
#[derive(Debug, Deserialize, Clone)]
struct SearchBody {
    song: SearchSong, // 歌曲列表的容器
}

/// 定义歌曲列表的结构。
#[derive(Debug, Deserialize, Clone)]
struct SearchSong {
    list: Vec<Song>, // 包含多个 Song 对象的向量
}

/// 定义单个歌曲信息的结构。
#[derive(Debug, Deserialize, Clone)]
pub struct Song {
    pub mid: String,         // 歌曲的媒体 ID (songmid)，非常重要，常用于其他 API 调用
    pub name: String,        // 歌曲名称
    pub singer: Vec<Singer>, // 歌手列表 (一个歌曲可能有多个演唱者)
    pub id: u64,             // 歌曲的数字 ID
}

/// 定义单个歌手信息的结构。
#[derive(Debug, Deserialize, Clone)]
pub struct Singer {
    pub name: String, // 歌手名称
}

/// 结构体，用于存储从 QQ 音乐歌词接口获取并处理后的歌词内容。
#[derive(Debug, Clone, Default)]
pub struct QqLyricsResponse {
    pub lyrics: String, // 主歌词内容 (通常是 QRC 格式)
    pub trans: String,  // 翻译歌词内容 (通常是 LRC 格式)
    pub roma: String,   // 罗马音歌词内容 (通常是 QRC 格式)
}

/// 异步函数，根据关键词搜索 QQ 音乐歌曲。
///
/// # Arguments
/// * `client` - `reqwest::Client` 的引用，用于发送 HTTP POST 请求。
/// * `keyword` - 搜索关键词。
///
/// # Returns
/// `ApiResult<(Vec<Song>, String)>` -
///   - `Ok((Vec<Song>, String))`：成功时返回歌曲列表和原始 JSON 响应字符串。
///   - `Err(ConvertError)`：失败时返回错误。
pub async fn search_song(client: &Client, keyword: &str) -> ApiResult<(Vec<Song>, String)> {
    // 构建搜索请求体
    let search_request = SearchRequest {
        req_1: SearchRequestBody {
            method: "DoSearchForQQMusicDesktop".to_string(),
            module: "music.search.SearchCgiService".to_string(),
            param: SearchParam {
                num_per_page: 20, // 默认请求20条结果
                page_num: 1,      // 默认请求第一页
                query: keyword.to_string(),
                search_type: 0,
            },
        },
    };

    // 发送 POST 请求并获取响应文本
    let resp_text = client
        .post(config::SEARCH_API_URL)
        .json(&search_request) // 将请求体序列化为 JSON
        .send()
        .await
        .map_err(|e| {
            // 处理网络请求错误
            log::error!("[QQMusicAPI] 网络错误: {}", e);
            ConvertError::NetworkRequest(e)
        })?
        .text() // 获取响应体文本
        .await
        .map_err(|e| {
            // 处理文本转换错误
            log::error!("[QQMusicAPI] 转换错误: {}", e);
            ConvertError::NetworkRequest(e) // 复用 NetworkRequest 错误类型可能不太精确，但能工作
        })?;

    let raw_response = resp_text.clone(); // 克隆原始响应文本，可能用于调试或缓存

    // 尝试将 JSON 响应文本反序列化为 MusicFcgApiResult 结构体
    let resp: MusicFcgApiResult = match serde_json::from_str(&resp_text) {
        Ok(r) => r,
        Err(e) => {
            // 处理 JSON 解析错误
            log::error!(
                "[QQMusicAPI] JSON 处理错误: {}. 原始响应: {}",
                e,
                raw_response
            );
            return Err(ConvertError::JsonParse(e));
        }
    };

    // 检查 API 返回的状态码
    if resp.code == 0 {
        // 顶层状态码为0通常表示请求被服务器接受
        if resp.req_1.code == 2001 {
            // 特定错误码：请求被拒绝
            log::error!("[QQMusicAPI] 服务器拒绝了你的搜索请求（代码2001），请稍后再试");
            Err(ConvertError::RequestRejected)
        } else if resp.req_1.code == 0 {
            // 内部状态码也为0表示搜索成功
            Ok((resp.req_1.data.body.song.list, raw_response)) // 返回歌曲列表和原始响应
        } else {
            // 其他内部错误码
            Err(ConvertError::QqMusicApiError(format!(
                "内部错误 (返回代码: {}), ",
                resp.req_1.code
            )))
        }
    } else {
        // 顶层状态码非0，表示请求处理失败
        Err(ConvertError::QqMusicApiError(format!(
            "顶层错误 (返回代码: {})",
            resp.code
        )))
    }
}

/// 异步函数，根据歌曲的数字 ID 获取歌词。
///
/// QQ 音乐的歌词接口返回的是 XML 格式，其中歌词内容（主歌词、翻译、罗马音）
/// 通常是加密的，并位于 CDATA 区块内。
/// 此函数会获取原始 XML，然后提取、解密并解析这些歌词内容。
///
/// # Arguments
/// * `client` - `reqwest::Client` 的引用。
/// * `id` - 歌曲的数字 ID (字符串形式)。
///
/// # Returns
/// `ApiResult<(Option<QqLyricsResponse>, String)>` -
///   - `Ok((Some(QqLyricsResponse), String))`：成功获取并处理歌词时。
///   - `Ok((None, String))`：API 调用成功但未找到歌词内容时。
///   - `Err(ConvertError)`：发生错误时。
pub async fn get_lyrics_by_id(
    client: &Client,
    id: &str,
) -> ApiResult<(Option<QqLyricsResponse>, String)> {
    // 构建歌词下载 API 的查询参数
    let params = [
        ("version", "15"),
        ("miniversion", "82"),
        ("lrctype", "4"),
        ("musicid", id), // 歌曲 ID
    ];

    // 发送 GET 请求并获取响应文本
    let initial_resp_text = client
        .get(config::QRC_API_URL)
        .query(&params)
        .send()
        .await
        .map_err(|e| {
            log::error!("[QQMusicAPI] 网络错误: {}", e);
            ConvertError::NetworkRequest(e)
        })?
        .text()
        .await
        .map_err(|e| {
            log::error!("[QQMusicAPI] 转换错误: {}", e);
            ConvertError::NetworkRequest(e)
        })?;

    let raw_response_for_log = initial_resp_text.clone(); // 克隆原始响应用于日志或返回

    // --- XML 解析和 CDATA 提取 ---
    let mut temp_cleaned_xml = initial_resp_text.trim(); // 去除首尾空格

    // 实际的内容包含在注释中
    if temp_cleaned_xml.starts_with("<!--") {
        if let Some(comment_end_index) = temp_cleaned_xml.find("-->") {
            temp_cleaned_xml = &temp_cleaned_xml[4..comment_end_index];
            temp_cleaned_xml = temp_cleaned_xml.trim();
        }
    }
    let cleaned_xml_for_cdata_extraction = temp_cleaned_xml.to_string();

    // 初始化用于存储提取和解密后歌词的变量
    let mut main_lyrics_cdata_decrypted = String::new(); // 主歌词 (QRC)
    let mut trans_text_decrypted_lrc = String::new(); // 翻译 (LRC)
    let mut roma_cdata_decrypted = String::new(); // 罗马音 (QRC)

    // 使用 quick_xml 解析清理后的 XML 响应
    let mut initial_xml_reader = Reader::from_str(&cleaned_xml_for_cdata_extraction);
    initial_xml_reader.config_mut().trim_text_start = true; // 配置解析器去除文本节点前后的空白
    initial_xml_reader.config_mut().trim_text_end = true;

    let mut current_cdata_target_tag: Option<String> = None; // 当前期望的 CDATA 所属的标签名
    let mut cdata_buf = Vec::new(); // 用于读取 XML 事件的缓冲区

    loop {
        // 循环读取 XML 事件
        match initial_xml_reader.read_event_into(&mut cdata_buf) {
            Ok(Event::Start(e)) => {
                // 处理开始标签 <tag>
                let tag_name_bytes = e.name(); // 获取标签名 (字节形式)
                // 根据标签名设置当前期望的 CDATA 目标
                match tag_name_bytes.as_ref() {
                    b"content" => current_cdata_target_tag = Some("content".to_string()), // 主歌词
                    b"contentts" => current_cdata_target_tag = Some("contentts".to_string()), // 翻译
                    b"contentroma" => current_cdata_target_tag = Some("contentroma".to_string()), // 罗马音
                    _ => { /* 其他标签忽略 */ }
                }
            }
            Ok(Event::CData(e)) => {
                // 处理 CDATA 区块 <![CDATA[...]]>
                if let Some(ref target_tag) = current_cdata_target_tag {
                    // 如果当前正在期望某个标签的 CDATA
                    let cdata_text = String::from_utf8(e.to_vec())?; // 将 CDATA 内容转换为字符串
                    if !cdata_text.is_empty() {
                        // 调用解密函数 (在 decrypto.rs 中定义)
                        let decrypted_text =
                            crate::qq_lyrics_fetcher::decrypto::decrypt_lyrics(&cdata_text)
                                .map_err(|de| {
                                    ConvertError::Internal(format!(
                                        "解密 {} 错误: {}",
                                        target_tag, de
                                    ))
                                })?;

                        // 将解密后的文本存入相应的变量
                        match target_tag.as_str() {
                            "content" => main_lyrics_cdata_decrypted = decrypted_text,
                            "contentts" => trans_text_decrypted_lrc = decrypted_text,
                            "contentroma" => roma_cdata_decrypted = decrypted_text,
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                // 处理结束标签 </tag>
                let tag_name_bytes = e.name();
                // 如果结束的标签是当前期望的 CDATA 目标标签，则重置目标
                if let Some(ref target_tag) = current_cdata_target_tag {
                    if std::str::from_utf8(tag_name_bytes.as_ref()) == Ok(target_tag.as_str()) {
                        current_cdata_target_tag = None;
                    }
                }
            }
            Ok(Event::Eof) => break, // XML 文档结束
            Err(e) => {
                // XML 解析错误
                log::error!("[QQMusicAPI] 服务器返回的XML解析错误： {}", e);
                return Err(ConvertError::Xml(e));
            }
            _ => {} // 其他 XML 事件忽略
        }
        cdata_buf.clear(); // 清空缓冲区为下一次读取做准备
    }

    // --- 解析解密后的内部 XML (通常是 QRC 内容) ---
    // 解密后的 main_lyrics_cdata_decrypted 和 roma_cdata_decrypted 本身通常还是 XML 片段，
    // 格式类似 <Lyric_1 LyricType="1" LyricContent="[QRC文本]"/>
    // 需要再次解析这个内部 XML 来提取真正的歌词文本。

    let final_lyrics_qrc = if !main_lyrics_cdata_decrypted.is_empty() {
        // 调用辅助函数提取 QRC 内容
        extract_lyric_content_from_qrcinfos_xml(&main_lyrics_cdata_decrypted, "1")?
    } else {
        log::info!("[QQMusicAPI] 主歌词CDATA解密后为空");
        String::new()
    };

    // 翻译内容 (trans_text_decrypted_lrc) 通常已经是 LRC 或纯文本，不需要再次 XML 解析
    let final_trans_lrc = trans_text_decrypted_lrc;

    let final_roma_qrc_candidate = if !roma_cdata_decrypted.is_empty() {
        extract_lyric_content_from_qrcinfos_xml(&roma_cdata_decrypted, "1")? // 罗马音也可能是QRC
    } else {
        log::info!("[QQMusicAPI] 罗马音数据解密后为空");
        String::new()
    };

    // 如果所有歌词内容（主歌词、翻译、处理后的罗马音）都为空，则认为未找到歌词
    if final_lyrics_qrc.is_empty()
        && final_trans_lrc.is_empty()
        && final_roma_qrc_candidate.is_empty()
    {
        Err(ConvertError::LyricNotFound)
    } else {
        // 否则，构建 QqLyricsResponse 并返回
        Ok((
            Some(QqLyricsResponse {
                lyrics: final_lyrics_qrc,
                trans: final_trans_lrc,
                roma: final_roma_qrc_candidate,
            }),
            raw_response_for_log, // 同时返回原始响应文本
        ))
    }
}

/// 从解密后的 QRC 信息 XML 字符串中提取特定 LyricType 的 LyricContent。
/// 此版本使用正则表达式提取 LyricContent 的值，以避免因内容中未转义引号导致的 XML 解析错误，
/// 然后逐行处理提取出的内容，保留以 '[' 开头的行。
///
/// # 参数
/// * `xml_string`: &str - 包含QRC信息的完整XML字符串。
/// * `target_lyric_type_str`: &str - 目标歌词类型的字符串标识，例如 "1" 代表主QRC。
///
/// # 返回
/// `Result<String, crate::types::ConvertError>` - 成功时返回提取到的歌词内容字符串，否则返回错误或空字符串。
fn extract_lyric_content_from_qrcinfos_xml(
    // 你可以将函数名改回原来的，或使用此新名称
    xml_string: &str,
    target_lyric_type_str: &str,
) -> Result<String, crate::types::ConvertError> {
    // 确保这里的 ConvertError 路径正确
    // 检查输入XML是否为空或仅包含空白字符
    if xml_string.trim().is_empty() {
        log::warn!("[QQMusicAPI - 提取] 输入的XML内容为空，直接返回空字符串。");
        return Ok(String::new());
    }

    // 调试日志：记录尝试提取的歌词类型和XML片段（截断以避免日志过长）
    log::trace!(
        "[QQMusicAPI - 提取] 尝试使用正则表达式提取 LyricType '{}' 的内容，XML片段: '{}'",
        target_lyric_type_str,
        if xml_string.len() > 300 {
            format!("{}...", &xml_string[..300])
        } else {
            xml_string.to_string()
        }
    );

    // 构建正则表达式:
    // - 匹配 <Lyric_数字 LyricType="目标类型"[其他属性]LyricContent="实际QRC内容"[其他属性]/>
    //   - <Lyric_\d+\s+                    : 匹配 <Lyric_ 后跟一个或多个数字和一个或多个空格
    //   - LyricType\s*=\s*"{}"             : 匹配 LyricType 属性，其值为 target_lyric_type_str。
    //                                       允许等号和引号周围有空格。目标类型字符串会经过 regex::escape 转义。
    //   - [^>]*?                           : 非贪婪匹配 LyricType 和 LyricContent 之间的任何其他属性或字符（直到遇到 LyricContent 或标签结束）。
    //   - LyricContent\s*=\s*"((?:.|\s)*?)" : 匹配 LyricContent 属性，并捕获引号内的所有内容（捕获组1）。
    //                                       ((?:.|\s)*?) 表示非贪婪匹配任何字符（包括换行符）。
    //   - \s*/>                            : 匹配标签的自闭合部分，允许前面的空格。
    let regex_pattern = format!(
        r#"<Lyric_\d+\s+LyricType\s*=\s*"{}"[^>]*?LyricContent\s*=\s*"((?:.|\s)*?)"\s*/>"#,
        regex::escape(target_lyric_type_str) // 转义目标类型字符串，以防包含正则特殊字符
    );

    // 编译正则表达式
    let re = match Regex::new(&regex_pattern) {
        Ok(r) => r,
        Err(e) => {
            log::error!(
                "[QQMusicAPI - 提取] 正则表达式编译失败，模式: '{}'，错误: {}",
                regex_pattern,
                e
            );
            // 返回内部错误，指示正则表达式构建问题
            return Err(crate::types::ConvertError::Internal(format!(
                "正则表达式编译失败: {}",
                e
            )));
        }
    };

    // 在XML字符串中执行正则匹配
    match re.captures(xml_string) {
        Some(caps) => {
            // 正则表达式成功匹配
            // 尝试获取第一个捕获组 (即 LyricContent 的值)
            match caps.get(1) {
                Some(qrc_content_match) => {
                    let qrc_data = qrc_content_match.as_str();
                    log::trace!(
                        "[QQMusicAPI - 提取] 正则表达式成功捕获 LyricType '{}' 的QRC数据块。长度: {}",
                        target_lyric_type_str,
                        qrc_data.len()
                    );

                    // 逐行处理捕获到的QRC数据，提取以 '[' 开头的行
                    let extracted_lines: Vec<String> = qrc_data
                        .lines() // 按行分割
                        .filter(|line| line.trim_start().starts_with('[')) // 筛选以 '[' 开头的行 (去除行首空格后判断)
                        .map(String::from) // 将 &str 转换为 String
                        .collect(); // 收集结果到 Vec<String>

                    if !extracted_lines.is_empty() {
                        let final_content = extracted_lines.join("\n"); // 将提取的行用换行符重新组合
                        log::info!(
                            "[QQMusicAPI - 提取] 成功提取并过滤 LyricType '{}' 的QRC内容。输出长度: {}",
                            target_lyric_type_str,
                            final_content.len()
                        );
                        Ok(final_content) // 返回处理后的歌词内容
                    } else {
                        log::warn!(
                            "[QQMusicAPI - 提取] 正则匹配并捕获到LyricContent，但在QRC数据中未找到以 '[' 开头的有效行。LyricType: '{}'。捕获的QRC数据片段: '{}'",
                            target_lyric_type_str,
                            if qrc_data.len() > 200 {
                                // 截断显示的QRC数据
                                format!("{}...", &qrc_data[..200])
                            } else {
                                qrc_data.to_string()
                            }
                        );
                        Ok(String::new()) // 未找到有效歌词行，返回空字符串
                    }
                }
                None => {
                    // 这种情况理论上不应发生：如果 re.captures 返回 Some，则捕获组1也应该存在。
                    // 若发生，可能表示正则表达式逻辑本身或XML结构有预料之外的细微变化。
                    log::error!(
                        "[QQMusicAPI - 提取] 正则表达式匹配成功，但捕获组1 (LyricContent) 未捕获到任何内容。LyricType: '{}'。",
                        target_lyric_type_str
                    );
                    Ok(String::new()) // 捕获组问题，返回空字符串
                }
            }
        }
        None => {
            // 正则表达式未在XML中找到匹配项
            log::warn!(
                "[QQMusicAPI - 提取] 正则表达式模式未在XML中匹配到 LyricType '{}' 的内容。XML片段: '{}'",
                target_lyric_type_str,
                if xml_string.len() > 300 {
                    // 截断显示的XML片段
                    format!("{}...", &xml_string[..300])
                } else {
                    xml_string.to_string()
                }
            );
            Ok(String::new()) // 未匹配，返回空字符串
        }
    }
}
