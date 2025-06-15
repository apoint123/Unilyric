use reqwest::Client; // HTTP 客户端，用于发送网络请求
use reqwest::header::{ACCEPT, USER_AGENT}; // HTTP 请求头常量
use serde::Deserialize; // 用于反序列化数据 (例如从 JSON)
use serde_json; // JSON 处理库
use std::fs::{self, File}; // 文件系统操作，如创建、打开文件
use std::io::{self, BufRead, BufReader, Write}; // I/O 操作，如缓冲读取、写入
use std::path::Path; // 路径操作

// 从父模块导入自定义类型
use super::types::{AmllIndexEntry, AmllSearchField, FetchedAmllTtmlLyrics};
// 从 crate 根导入自定义错误类型
use crate::types::ConvertError;

/// 用于反序列化 GitHub API 返回的 commit信息的结构体
#[derive(Deserialize, Debug)]
struct GitHubCommitInfo {
    sha: String, // commit 的 SHA 哈希值
}

// --- 常量定义 ---
/// GitHub API 的基础 URL
const GITHUB_API_BASE_URL: &str = "https://api.github.com";
/// 索引文件在仓库中的路径
const INDEX_FILE_PATH_IN_REPO: &str = "metadata/raw-lyrics-index.jsonl";
/// 仓库所有者名称
const REPO_OWNER: &str = "Steve-xmh";
/// 仓库名称
const REPO_NAME: &str = "amll-ttml-db";
/// 仓库分支名称
const REPO_BRANCH: &str = "main";

/// 异步函数：获取远程索引文件的最新 commit SHA (HEAD)。
/// 用于判断远程索引文件是否有更新。
///
/// # 参数
/// * `client`: `reqwest::Client` 的引用，用于发送 HTTP 请求。
///
/// # 返回
/// * `Result<String, ConvertError>`: 成功时返回最新 commit SHA 字符串，失败时返回 `ConvertError`。
pub async fn fetch_remote_index_head(client: &Client) -> Result<String, ConvertError> {
    // 构建获取指定文件最新 commit 信息的 GitHub API URL
    let url = format!(
        "{GITHUB_API_BASE_URL}/repos/{REPO_OWNER}/{REPO_NAME}/commits?path={INDEX_FILE_PATH_IN_REPO}&sha={REPO_BRANCH}&per_page=1"
    );
    log::info!("[AMLLFetcher] 正在获取索引文件的 HEAD commit SHA: {url}");

    // 发送 GET 请求
    let response = client
        .get(&url)
        .header(USER_AGENT, "UniLyricApp/1.0") // GitHub API 推荐设置 User-Agent
        .header(ACCEPT, "application/vnd.github.v3+json") // 指定接受 GitHub API v3 JSON 格式的响应
        .send()
        .await?; // `?` 会将 reqwest::Error 转换为 ConvertError (如果 ConvertError 实现了 From<reqwest::Error>)

    // 检查响应状态码
    if !response.status().is_success() {
        let err_msg = format!("获取远程索引 HEAD 失败，HTTP 状态码: {}", response.status());
        log::error!("[AMLLFetcher] {err_msg}");
        // 当状态码表示失败时，response.error_for_status() 会返回 Err(reqwest::Error)
        // 我们使用 unwrap_err() 来获取这个 Error
        return Err(ConvertError::NetworkRequest(
            response.error_for_status().unwrap_err(),
        ));
    }

    // 解析 JSON 响应体为 Vec<GitHubCommitInfo>
    let commits: Vec<GitHubCommitInfo> = response.json().await?; // `?` 会将 reqwest::Error 转换为 ConvertError
    if let Some(latest_commit) = commits.first() {
        // 获取列表中的第一个 commit (即最新的)
        log::info!(
            "[AMLLFetcher] 成功获取索引 HEAD commit SHA: {}",
            latest_commit.sha
        );
        Ok(latest_commit.sha.clone())
    } else {
        let err_msg = "未找到索引文件的 commit 信息。".to_string();
        log::error!("[AMLLFetcher] {err_msg}");
        Err(ConvertError::Internal(err_msg)) // 内部逻辑错误或 API 返回了非预期的空数据
    }
}

/// 将索引内容和当前的 HEAD SHA 保存到缓存文件。
/// 索引内容保存到 `cache_file_path` (例如 .../index.jsonl)。
/// HEAD SHA 保存到 `cache_file_path` 同名但扩展名为 `.head` 的文件 (例如 .../index.jsonl.head)。
///
/// # 参数
/// * `cache_file_path`: 索引缓存文件的路径。
/// * `index_content`: 要保存的索引文件内容字符串。
/// * `current_head_sha`: 当前远程索引的 HEAD commit SHA。
///
/// # 返回
/// * `Result<(), ConvertError>`: 成功时返回 `Ok(())`，失败时返回 `ConvertError`。
fn save_index_to_cache(
    cache_file_path: &Path,
    index_content: &str,
    current_head_sha: &str,
) -> Result<(), ConvertError> {
    log::info!(
        "[AMLLFetcher] 准备将索引保存到缓存文件: {cache_file_path:?} (HEAD SHA: {current_head_sha})"
    );
    // 确保缓存文件所在的父目录存在，如果不存在则创建
    if let Some(parent_dir) = cache_file_path.parent() {
        fs::create_dir_all(parent_dir).map_err(|e| {
            log::error!("[AMLLFetcher] 创建缓存目录 {parent_dir:?} 失败: {e}");
            ConvertError::Io(e) // 将 std::io::Error 转换为 ConvertError::Io
        })?;
    }

    // 创建或覆盖索引缓存文件并写入内容
    let mut file = File::create(cache_file_path).map_err(|e| {
        log::error!("[AMLLFetcher] 创建索引缓存文件 {cache_file_path:?} 失败: {e}");
        ConvertError::Io(e)
    })?;
    file.write_all(index_content.as_bytes()).map_err(|e| {
        log::error!("[AMLLFetcher] 写入索引内容到缓存文件 {cache_file_path:?} 失败: {e}");
        ConvertError::Io(e)
    })?;
    log::info!("[AMLLFetcher] 索引已保存到文件: {cache_file_path:?}");

    // 创建或覆盖 .head 文件并写入当前的 HEAD SHA
    let head_file_path = cache_file_path.with_extension("jsonl.head");
    fs::write(&head_file_path, current_head_sha).map_err(|e| {
        log::error!("[AMLLFetcher] 写入 HEAD SHA 到文件 {head_file_path:?} 失败: {e}");
        ConvertError::Io(e)
    })?;
    log::info!("[AMLLFetcher] HEAD SHA 已成功保存到文件: {head_file_path:?}");

    Ok(())
}

/// 从缓存加载索引文件的 HEAD SHA。
/// HEAD SHA 存储在与索引缓存文件同名但扩展名为 `.head` 的文件中。
///
/// # 参数
/// * `cache_file_path`: 主索引缓存文件的路径 (例如 .../index.jsonl)。
///
/// # 返回
/// * `Result<Option<String>, io::Error>`:
///   - `Ok(Some(String))` 如果 .head 文件存在且包含有效的 SHA。
///   - `Ok(None)` 如果 .head 文件不存在或为空。
///   - `Err(io::Error)` 如果读取文件失败。
pub fn load_cached_index_head(cache_file_path: &Path) -> Result<Option<String>, io::Error> {
    let head_file_path = cache_file_path.with_extension("jsonl.head"); // 构建 .head 文件的路径
    if head_file_path.exists() {
        // 检查 .head 文件是否存在
        match fs::read_to_string(&head_file_path) {
            // 读取文件内容
            Ok(head) => {
                let trimmed_head = head.trim(); // 去除可能存在的前后空白字符
                if !trimmed_head.is_empty() {
                    // 如果内容不为空
                    log::info!(
                        "[AMLLFetcher] 从缓存文件 {head_file_path:?} 加载到 HEAD SHA: {trimmed_head}"
                    );
                    Ok(Some(trimmed_head.to_string()))
                } else {
                    // 文件存在但内容为空
                    log::warn!("[AMLLFetcher] 缓存的 HEAD SHA 文件 {head_file_path:?} 内容为空。");
                    Ok(None)
                }
            }
            Err(e) => {
                // 读取文件失败
                log::error!("[AMLLFetcher] 读取缓存的 HEAD SHA 文件 {head_file_path:?} 失败: {e}");
                Err(e)
            }
        }
    } else {
        // .head 文件不存在
        log::info!("[AMLLFetcher] 缓存的 HEAD SHA 文件 {head_file_path:?} 未找到。");
        Ok(None)
    }
}

/// 从本地缓存文件加载并解析歌词索引。
/// 索引文件应为 JSONL 格式，每行一个 JSON 对象代表一个索引条目。
///
/// # 参数
/// * `cache_file_path`: 索引缓存文件的路径。
///
/// # 返回
/// * `Result<Vec<AmllIndexEntry>, ConvertError>`: 成功时返回包含所有索引条目的向量，失败时返回 `ConvertError`。
pub fn load_index_from_cache(cache_file_path: &Path) -> Result<Vec<AmllIndexEntry>, ConvertError> {
    if !cache_file_path.exists() {
        // 检查缓存文件是否存在
        log::info!("[AMLLFetcher] 索引缓存文件 {cache_file_path:?} 不存在。");
        // 返回特定的 NotFound 错误
        return Err(ConvertError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            "索引缓存文件不存在",
        )));
    }

    // 打开缓存文件
    let file = File::open(cache_file_path).map_err(|e| {
        log::error!("[AMLLFetcher] 打开索引缓存文件 {cache_file_path:?} 失败: {e}");
        ConvertError::Io(e)
    })?;

    let reader = BufReader::new(file); // 使用缓冲读取器提高效率
    let mut entries = Vec::new(); // 用于存储解析后的索引条目

    // 逐行读取并解析
    for (line_num, line_result) in reader.lines().enumerate() {
        let line = line_result.map_err(|e| {
            log::error!(
                "[AMLLFetcher] 读取索引缓存文件 {:?} 第 {} 行失败: {}",
                cache_file_path,
                line_num + 1, // 行号从1开始计数
                e
            );
            ConvertError::Io(e)
        })?;

        if line.trim().is_empty() {
            // 跳过空行
            continue;
        }
        // 尝试将每行文本解析为 AmllIndexEntry 结构体
        match serde_json::from_str::<AmllIndexEntry>(&line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                // 解析失败
                log::warn!(
                    "[AMLLFetcher] 解析索引缓存文件 {:?} 第 {} 行失败: 内容 '{}', 错误: {}",
                    cache_file_path,
                    line_num + 1,
                    line, // 记录原始行内容以帮助调试
                    e
                );
                return Err(ConvertError::JsonParse(e));
            }
        }
    }

    if entries.is_empty()
        && Path::new(cache_file_path)
            .metadata()
            .is_ok_and(|m| m.len() > 0)
    {
        // 文件非空，但没有解析出任何条目，这可能表明文件格式有问题
        log::error!(
            "[AMLLFetcher] 从缓存文件 {cache_file_path:?} 加载的索引为空，但文件本身非空。请检查文件格式。"
        );
    } else if entries.is_empty() {
        log::warn!("[AMLLFetcher] 从缓存文件 {cache_file_path:?} 加载的索引为空。");
    }

    log::info!(
        "[AMLLFetcher] 从缓存文件 {:?} 成功加载并解析了 {} 条索引条目。",
        cache_file_path,
        entries.len()
    );
    Ok(entries)
}

/// 异步下载远程索引文件，解析内容，并保存到缓存。
///
/// # 参数
/// * `client`: `reqwest::Client` 的引用。
/// * `repo_base_url`: 仓库中原始文件内容的基础 URL (例如 `https://raw.githubusercontent.com/{OWNER}/{REPO}/{BRANCH}/`)。
/// * `cache_file_path`: 本地索引缓存文件的路径。
/// * `remote_head_sha`: 从 `fetch_remote_index_head` 获取到的远程最新 commit SHA，用于保存缓存时关联版本。
///
/// # 返回
/// * `Result<Vec<AmllIndexEntry>, ConvertError>`: 成功时返回解析后的索引条目向量，失败时返回 `ConvertError`。
pub async fn download_and_parse_index(
    client: &Client,
    repo_base_url: &str, // 例如 "https://raw.githubusercontent.com/Steve-xmh/amll-ttml-db/main"
    cache_file_path: &Path,
    remote_head_sha: String, // 传入期望保存的远程 HEAD SHA
) -> Result<Vec<AmllIndexEntry>, ConvertError> {
    // 构建索引文件的直接下载 URL
    let index_url = format!("{repo_base_url}/{INDEX_FILE_PATH_IN_REPO}"); // INDEX_FILE_PATH_IN_REPO 是 "metadata/raw-lyrics-index.jsonl"
    log::info!("[AMLLFetcher] 开始下载索引文件: {index_url}");

    // 发送 GET 请求下载文件内容
    let response = client.get(&index_url).send().await.map_err(|e| {
        log::error!("[AMLLFetcher] 下载索引文件时发生网络请求错误: {e}");
        ConvertError::NetworkRequest(e)
    })?;

    // 检查 HTTP 状态码
    if !response.status().is_success() {
        let err_msg = format!("下载索引文件失败，HTTP 状态码: {}", response.status());
        log::error!("[AMLLFetcher] {err_msg}");
        return Err(ConvertError::NetworkRequest(
            response.error_for_status().unwrap_err(),
        ));
    }

    // 读取响应体文本内容
    let response_text = response.text().await.map_err(|e| {
        log::error!("[AMLLFetcher] 读取索引文件响应体为文本时出错: {e}");
        ConvertError::NetworkRequest(e)
    })?;

    log::info!("[AMLLFetcher] 索引文件下载成功。");

    // 将下载的索引内容和当前的远程 HEAD SHA 保存到缓存
    if let Err(e) = save_index_to_cache(cache_file_path, &response_text, &remote_head_sha) {
        log::warn!(
            "[AMLLFetcher] 保存下载的索引文件到本地缓存失败: {e}。后续操作将使用内存中的数据。"
        );
        // 即使保存缓存失败，仍然尝试解析并返回内存中的数据
    }

    let mut entries = Vec::new();
    // 逐行解析下载的文本内容
    for (line_num, line) in response_text.lines().enumerate() {
        if line.trim().is_empty() {
            // 跳过空行
            continue;
        }
        match serde_json::from_str::<AmllIndexEntry>(line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                // 解析某一行失败
                log::warn!(
                    "[AMLLFetcher] 解析下载的索引文件第 {} 行失败: 内容 '{}', 错误: {}",
                    line_num + 1,
                    line,
                    e
                );
                // 选择继续解析下一行，而不是因单行错误而中止整个过程
            }
        }
    }

    if entries.is_empty() && !response_text.trim().is_empty() {
        // 下载的内容非空，但未能解析出任何条目
        log::error!(
            "[AMLLFetcher] 下载的索引文件内容非空，但未能解析出任何有效条目。请检查文件格式或远程数据源。"
        );
        return Err(ConvertError::Internal(
            "无法从下载的索引内容中解析出条目".to_string(),
        ));
    }

    log::info!(
        "[AMLLFetcher] 索引文件解析完成，共 {} 条索引。",
        entries.len()
    );
    Ok(entries)
}

/// 在已加载的索引条目列表中搜索匹配的歌词。
///
/// # 参数
/// * `query`: 搜索查询字符串。
/// * `search_field`: 指定要在哪个字段中进行搜索 (`AmllSearchField` 枚举)。
/// * `index_entries`: 包含所有歌词索引条目的切片。
///
/// # 返回
/// * `Vec<AmllIndexEntry>`: 包含所有匹配搜索条件的索引条目的向量 (克隆)。
pub fn search_lyrics_in_index(
    query: &str,
    search_field: &AmllSearchField,
    index_entries: &[AmllIndexEntry],
) -> Vec<AmllIndexEntry> {
    if query.trim().is_empty() {
        // 如果查询字符串为空，则返回空结果
        return Vec::new();
    }
    let lower_query = query.to_lowercase(); // 将查询字符串转换为小写，以进行不区分大小写的搜索
    let search_key = search_field.to_key_string(); // 获取当前搜索字段对应的元数据键名 (例如 "music_name", "artists")

    index_entries
        .iter()
        .filter(|entry| {
            // 过滤每个索引条目
            // 遍历条目的元数据 (HashMap<String, Vec<String>>)
            for (key, values) in &entry.metadata {
                if key == search_key {
                    // 如果元数据的键与当前搜索字段的键匹配
                    // 根据搜索字段的类型，应用不同的匹配逻辑
                    return match search_field {
                        // 对于歌曲名、艺术家、专辑名、TTML作者GitHub登录名，使用包含匹配 (contains)
                        AmllSearchField::MusicName
                        | AmllSearchField::Artists
                        | AmllSearchField::Album
                        | AmllSearchField::TtmlAuthorGithubLogin => values
                            .iter()
                            .any(|v| v.to_lowercase().contains(&lower_query)), // 任何一个值包含查询即可
                        // 对于各种 ID 和 ISRC，以及 TTML 作者 GitHub 用户名，
                        // 使用精确匹配 (==) 或包含匹配 (contains)。
                        // 注意：对于ID类字段，通常更期望精确匹配。这里的混合匹配可能是特定需求。
                        AmllSearchField::NcmMusicId
                        | AmllSearchField::QqMusicId
                        | AmllSearchField::SpotifyId
                        | AmllSearchField::AppleMusicId // AppleMusicId 也通过 metadata 搜索
                        | AmllSearchField::Isrc
                        | AmllSearchField::TtmlAuthorGithub => values
                            .iter()
                            .any(|v| v.to_lowercase() == lower_query || v.to_lowercase().contains(&lower_query)), // 任何一个值精确匹配或包含查询
                    };
                }
            }
            false // 如果条目的元数据中没有找到匹配的键，则此条目不匹配
        })
        .cloned() // 克隆匹配的条目，因为 filter 返回的是引用
        .collect() // 收集到新的 Vec 中
}

/// 根据索引条目信息，异步下载对应的 TTML 歌词文件内容。
///
/// # 参数
/// * `client`: `reqwest::Client` 的引用。
/// * `repo_base_url`: 仓库中原始文件内容的基础 URL。
/// * `index_entry`: 包含 TTML 文件路径等信息的索引条目。
///
/// # 返回
/// * `Result<FetchedAmllTtmlLyrics, ConvertError>`: 成功时返回包含歌词内容和元数据的结构体，失败时返回 `ConvertError`。
pub async fn download_ttml_from_entry(
    client: &Client,
    repo_base_url: &str,
    index_entry: &AmllIndexEntry,
) -> Result<FetchedAmllTtmlLyrics, ConvertError> {
    // 构建 TTML 文件的直接下载 URL
    let ttml_file_url = format!(
        "{}/{}/{}", // base_url / "raw-lyrics" / "filename.ttml"
        repo_base_url,
        "raw-lyrics",
        index_entry.raw_lyric_file // raw_lyric_file 字段存储文件名
    );
    log::info!("[AMLLFetcher] 开始下载 TTML 歌词文件: {ttml_file_url}");

    // 发送 GET 请求下载文件
    let response = client.get(&ttml_file_url).send().await.map_err(|e| {
        log::error!("[AMLLFetcher] 下载 TTML 文件时发生网络请求错误: {e}");
        ConvertError::NetworkRequest(e)
    })?;

    // 检查 HTTP 状态码
    if !response.status().is_success() {
        let err_msg = format!(
            "下载 TTML 文件 '{}' 失败，HTTP 状态码: {}",
            index_entry.raw_lyric_file,
            response.status()
        );
        log::error!("[AMLLFetcher] {err_msg}");
        return Err(ConvertError::NetworkRequest(
            response.error_for_status().unwrap_err(),
        ));
    }

    // 读取响应体为文本 (即 TTML 内容)
    let ttml_content = response.text().await.map_err(|e| {
        log::error!(
            "[AMLLFetcher] 读取 TTML 文件 '{}' 响应体为文本时出错: {}",
            index_entry.raw_lyric_file,
            e
        );
        ConvertError::NetworkRequest(e)
    })?;

    if ttml_content.trim().is_empty() {
        // 检查下载的 TTML 内容是否为空
        log::warn!(
            // 使用警告级别，因为这可能是一个有效但空的歌词文件
            "[AMLLFetcher] 下载的 TTML 文件 '{}' 内容为空或只包含空白字符。",
            index_entry.raw_lyric_file
        );
    }
    log::info!(
        "[AMLLFetcher] TTML 文件 '{}' 下载成功。",
        index_entry.raw_lyric_file,
    );

    // 从索引条目的元数据中提取歌曲名、艺术家、专辑名和 Apple Music ID (作为 source_id)
    let mut song_name = None;
    let mut artists_name = Vec::new();
    let mut album_name = None;
    let mut source_id: Option<String> = None; // 用于存储 Apple Music ID

    for (key, values) in &index_entry.metadata {
        if values.is_empty() {
            // 如果某个元数据键对应的值列表为空，则跳过
            continue;
        }
        // 使用 AmllSearchField 的 to_key_string() 方法来匹配键名，确保一致性
        let key_str = key.as_str();
        if key_str == AmllSearchField::MusicName.to_key_string() {
            song_name = values.first().cloned(); // 取第一个值作为歌曲名
        } else if key_str == AmllSearchField::Artists.to_key_string() {
            artists_name.extend(values.iter().cloned()); // 将所有艺术家名添加到列表中
        } else if key_str == AmllSearchField::Album.to_key_string() {
            album_name = values.first().cloned(); // 取第一个值作为专辑名
        } else if key_str == AmllSearchField::AppleMusicId.to_key_string() {
            source_id = values.first().cloned(); // 取第一个 Apple Music ID 作为源 ID
        }
    }

    // 构建并返回 FetchedAmllTtmlLyrics 结构体
    Ok(FetchedAmllTtmlLyrics {
        song_name,
        artists_name,
        album_name,
        ttml_content,
        source_id, // 使用从元数据中提取的 Apple Music ID
        all_metadata_from_index: index_entry.metadata.clone(), // 包含所有原始元数据
    })
}
