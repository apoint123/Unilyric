//! 此模块实现了与 AMLL TTML Database 进行交互的 `Provider`。

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    config::AmllMirror,
    converter::{self},
    error::{LyricsHelperError, Result},
    http::HttpClient,
    model::match_type::MatchScorable,
    providers::{Provider, amll_ttml_database::types::DedupKey},
    search::matcher::compare_track,
};

use lyrics_helper_core::{
    ConversionInput, ConversionOptions, CoverSize, FullLyricsResult, InputFile, LyricFormat,
    MatchType, ParsedSourceData, RawLyrics, SearchResult, Track, model::generic,
};

mod types;
use types::IndexEntry;

const GITHUB_API_BASE_URL: &str = "https://api.github.com";
const RAW_CONTENT_BASE_URL: &str = "https://raw.githubusercontent.com";
const INDEX_FILE_PATH_IN_REPO: &str = "metadata/raw-lyrics-index.jsonl";
const REPO_OWNER: &str = "amll-dev";
const REPO_NAME: &str = "amll-ttml-db";
const REPO_BRANCH: &str = "main";
const USER_AGENT: &str = "lyrics-helper-rs/0.1.0";
const INDEX_CACHE_FILENAME: &str = "amll_ttml_db/index.jsonl";
const HEAD_CACHE_FILENAME: &str = "amll_ttml_db/index.jsonl.head";

/// 用于反序列化 GitHub commit API 响应的辅助结构体。
#[derive(Deserialize)]
struct GitHubCommitInfo {
    sha: String,
}

/// AMLL TTML Database 提供商的实现。
pub struct AmllTtmlDatabase {
    index: Arc<Vec<IndexEntry>>,
    http_client: Arc<dyn HttpClient>,
    lyrics_url_template: String,
}

#[async_trait]
impl Provider for AmllTtmlDatabase {
    fn name(&self) -> &'static str {
        "amll-ttml-database"
    }

    async fn with_http_client(http_client: Arc<dyn HttpClient>) -> Result<Self>
    where
        Self: Sized,
    {
        let config = crate::config::load_amll_config().unwrap_or_else(|e| {
            tracing::error!("[AMLL] 加载 AMLL 镜像配置失败: {}. 使用默认 GitHub 源。", e);
            crate::config::AmllConfig::default()
        });

        let (index_url, lyrics_url_template) = match &config.mirror {
            AmllMirror::GitHub => (
                format!(
                    "{RAW_CONTENT_BASE_URL}/{REPO_OWNER}/{REPO_NAME}/{REPO_BRANCH}/{INDEX_FILE_PATH_IN_REPO}"
                ),
                format!(
                    "{RAW_CONTENT_BASE_URL}/{REPO_OWNER}/{REPO_NAME}/{REPO_BRANCH}/raw-lyrics/{{song_id}}"
                ),
            ),
            AmllMirror::Dimeta => (
                format!(
                    "{RAW_CONTENT_BASE_URL}/{REPO_OWNER}/{REPO_NAME}/{REPO_BRANCH}/{INDEX_FILE_PATH_IN_REPO}"
                ),
                "https://amll.mirror.dimeta.top/api/db/raw-lyrics/{song_id}".to_string(),
            ),
            AmllMirror::Bikonoo => (
                "https://amlldb.bikonoo.com/metadata/raw-lyrics-index.jsonl".to_string(),
                "https://amlldb.bikonoo.com/raw-lyrics/{song_id}".to_string(),
            ),
            AmllMirror::Custom {
                index_url,
                lyrics_url_template,
            } => (index_url.clone(), lyrics_url_template.clone()),
        };

        let remote_head_result = fetch_remote_index_head(http_client.as_ref()).await;

        let (should_update, remote_head) = match remote_head_result {
            Ok(sha) => {
                let local_head = load_cached_index_head();
                (Some(sha.clone()) != local_head, Some(sha))
            }
            Err(LyricsHelperError::RateLimited(msg)) => {
                tracing::info!("[AMLL] GitHub API 速率限制，无法获取索引文件: {msg}");
                (false, None)
            }
            Err(e) => return Err(e),
        };

        let index_entries_result = load_index_from_cache();

        let index_entries = match (should_update, remote_head, index_entries_result) {
            (false, _, Ok(entries)) => {
                tracing::info!("[AMLL] 索引缓存有效，从本地加载...");
                entries
            }
            (true, Some(sha), _) | (false, Some(sha), Err(_)) => {
                tracing::info!(
                    "[AMLL] 索引需要更新或本地缓存不可用，正在从 {} 下载...",
                    index_url
                );
                download_and_parse_index(&sha, http_client.as_ref(), &index_url).await?
            }
            (true, None, Ok(entries)) => {
                tracing::warn!("[AMLL] 无法检查更新，将使用可能已过期的本地缓存。");
                entries
            }
            (_, _, Err(_)) => {
                return Err(LyricsHelperError::Internal(
                    "AMLL 数据库初始化失败：被速率限制且无本地缓存可用。".to_string(),
                ));
            }
        };

        tracing::info!("[AMLL] 索引加载完成，共 {} 条记录。", index_entries.len());
        Ok(Self {
            index: Arc::new(index_entries),
            http_client,
            lyrics_url_template,
        })
    }

    async fn search_songs(&self, track: &Track<'_>) -> Result<Vec<SearchResult>> {
        let Some(title_to_search) = track.title else {
            return Ok(vec![]);
        };

        let candidates = find_candidates(&self.index, title_to_search);

        if candidates.is_empty() {
            return Ok(vec![]);
        }

        let scored_results: Vec<_> = candidates
            .into_iter()
            .map(|entry| {
                let (match_type, best_title, best_album) = calculate_best_match(&entry, track);
                (entry, match_type, best_title, best_album)
            })
            .filter(|(_, match_type, _, _)| *match_type >= MatchType::VeryLow)
            .collect();

        let unique_results = deduplicate_and_sort(scored_results);
        let provider_name = self.name();
        let final_results: Vec<SearchResult> = unique_results
            .into_iter()
            .take(20)
            .map(|(entry, _, best_title, best_album)| {
                into_search_result(&entry, best_title, best_album, provider_name)
            })
            .collect();

        Ok(final_results)
    }

    #[allow(clippy::literal_string_with_formatting_args)]
    /// 获取并解析完整的 TTML 歌词文件。
    async fn get_full_lyrics(&self, song_id: &str) -> Result<FullLyricsResult> {
        let ttml_url = self.lyrics_url_template.replace("{song_id}", song_id);
        tracing::info!("[AMLL] 下载并解析 TTML: {}", ttml_url);

        let response = self.http_client.get(&ttml_url).await?;
        if response.status >= 400 {
            return Err(LyricsHelperError::Http(format!(
                "下载 AMLL 歌词失败，状态码: {}",
                response.status
            )));
        }
        let response_text = response.text()?;

        let conversion_input = ConversionInput {
            main_lyric: InputFile {
                content: response_text.clone(),
                format: LyricFormat::Ttml,
                language: None,
                filename: Some(song_id.to_string()),
            },
            translations: vec![],
            romanizations: vec![],
            target_format: LyricFormat::default(),
            user_metadata_overrides: None,
            additional_metadata: None,
        };

        let mut parsed_data =
            converter::parse_and_merge(&conversion_input, &ConversionOptions::default())
                .map_err(|e| LyricsHelperError::Parser(e.to_string()))?;

        parsed_data.source_name = "amll-ttml-database".to_string();

        let raw_lyrics = RawLyrics {
            format: "ttml".to_string(),
            content: response_text,
            translation: None,  // 由 TTML 解析器自己处理
            romanization: None, // 由 TTML 解析器自己处理
        };

        Ok(FullLyricsResult {
            parsed: parsed_data,
            raw: raw_lyrics,
        })
    }

    /// 获取歌词。`song_id` 就是 TTML 文件名。
    async fn get_lyrics(&self, song_id: &str) -> Result<ParsedSourceData> {
        // 普通歌词和完整歌词没有区别，都返回最完整的 TTML 解析结果。
        Ok(self.get_full_lyrics(song_id).await?.parsed)
    }

    async fn get_album_info(&self, _: &str) -> Result<generic::Album> {
        Err(LyricsHelperError::ProviderNotSupported(
            "amll-ttml-database 不支持 get_album_info".into(),
        ))
    }

    async fn get_album_songs(
        &self,
        _album_id: &str,
        _page: u32,
        _page_size: u32,
    ) -> Result<Vec<generic::Song>> {
        Err(LyricsHelperError::ProviderNotSupported(
            "amll-ttml-database 不支持 get_album_songs".to_string(),
        ))
    }

    async fn get_singer_songs(
        &self,
        _singer_id: &str,
        _page: u32,
        _page_size: u32,
    ) -> Result<Vec<generic::Song>> {
        Err(LyricsHelperError::ProviderNotSupported(
            "amll-ttml-database 不支持 get_singer_songs".to_string(),
        ))
    }

    async fn get_playlist(&self, _playlist_id: &str) -> Result<generic::Playlist> {
        Err(LyricsHelperError::ProviderNotSupported(
            "amll-ttml-database 不支持 get_playlist".to_string(),
        ))
    }

    async fn get_song_info(&self, _song_id: &str) -> Result<generic::Song> {
        Err(LyricsHelperError::ProviderNotSupported(
            "amll-ttml-database 不支持 get_song_info".to_string(),
        ))
    }

    async fn get_album_cover_url(&self, _album_id: &str, _size: CoverSize) -> Result<String> {
        Err(LyricsHelperError::ProviderNotSupported(
            "amll-ttml-database 不支持 get_album_cover_url".into(),
        ))
    }
}

/// 从 GitHub API 获取索引文件的最新 commit SHA。
async fn fetch_remote_index_head(http_client: &dyn HttpClient) -> Result<String> {
    let url = format!(
        "{GITHUB_API_BASE_URL}/repos/{REPO_OWNER}/{REPO_NAME}/commits?path={INDEX_FILE_PATH_IN_REPO}&sha={REPO_BRANCH}&per_page=1"
    );
    let headers = [
        ("User-Agent", USER_AGENT),
        ("Accept", "application/vnd.github.v3+json"),
    ];
    let response = http_client
        .request_with_headers(crate::http::HttpMethod::Get, &url, &headers, None)
        .await?;

    if response.status >= 400 {
        let status = response.status;
        if status == 403
            && let Ok(err_resp) = response.json::<types::GitHubErrorResponse>()
            && err_resp.message.contains("rate limit exceeded")
        {
            return Err(LyricsHelperError::RateLimited(err_resp.message));
        }
        return Err(LyricsHelperError::Http(format!(
            "GitHub API 返回错误: {status}"
        )));
    }

    let commits: Vec<GitHubCommitInfo> = response.json()?;

    commits
        .first()
        .map(|c| c.sha.clone())
        .ok_or_else(|| LyricsHelperError::Internal("未找到索引文件的 commit 信息".into()))
}

/// 从本地 `.head` 文件加载缓存的 commit SHA。
fn load_cached_index_head() -> Option<String> {
    crate::config::read_from_cache(HEAD_CACHE_FILENAME).map_or(None, |head| {
        let trimmed_head = head.trim();
        if trimmed_head.is_empty() {
            None
        } else {
            Some(trimmed_head.to_string())
        }
    })
}

/// 从本地缓存文件加载索引。
fn load_index_from_cache() -> Result<Vec<IndexEntry>> {
    let content = crate::config::read_from_cache(INDEX_CACHE_FILENAME)
        .map_err(|e| LyricsHelperError::Internal(format!("Failed to read index cache: {e}")))?;

    let entries = content
        .lines()
        .filter_map(|line| {
            if line.trim().is_empty() {
                None
            } else {
                serde_json::from_str::<IndexEntry>(line).ok()
            }
        })
        .collect();
    Ok(entries)
}

/// 下载、解析索引文件，并更新本地缓存。
async fn download_and_parse_index(
    remote_head_sha: &str,
    http_client: &dyn HttpClient,
    index_url: &str,
) -> Result<Vec<IndexEntry>> {
    let response = http_client.get(index_url).await?;
    if response.status >= 400 {
        return Err(LyricsHelperError::Http(format!(
            "下载 AMLL 索引失败，状态码: {}",
            response.status
        )));
    }
    let response_text = response.text()?;

    let entries: Vec<IndexEntry> = response_text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| match serde_json::from_str(line) {
            Ok(entry) => Some(entry),
            Err(e) => {
                tracing::warn!("[AMLL] 索引文件中有损坏的行，已忽略。错误: {e}, 行内容: '{line}'");
                None
            }
        })
        .collect();

    if entries.is_empty() && !response_text.trim().is_empty() {
        return Err(LyricsHelperError::Internal(
            "下载的索引文件内容非空但无法解析出任何条目".into(),
        ));
    }

    save_index_to_cache(&response_text, remote_head_sha)?;
    Ok(entries)
}

/// 将下载的内容和最新的 SHA 写入本地缓存文件。
fn save_index_to_cache(content: &str, head_sha: &str) -> Result<()> {
    crate::config::write_to_cache(INDEX_CACHE_FILENAME, content)
        .map_err(|e| LyricsHelperError::Internal(format!("写入索引缓存失败: {e}")))?;

    crate::config::write_to_cache(HEAD_CACHE_FILENAME, head_sha)
        .map_err(|e| LyricsHelperError::Internal(format!("写入 HEAD 缓存失败: {e}")))?;

    Ok(())
}

fn find_candidates(index: &[IndexEntry], keyword: &str) -> Vec<IndexEntry> {
    let lower_keyword = keyword.to_lowercase();
    index
        .iter()
        .rev()
        .filter(|entry| {
            entry
                .metadata
                .titles
                .iter()
                .any(|v| v.to_lowercase().contains(&lower_keyword))
        })
        .cloned()
        .collect()
}

fn calculate_best_match(
    entry: &IndexEntry,
    track: &Track<'_>,
) -> (MatchType, String, Option<String>) {
    let default_title = entry.metadata.titles.first().cloned().unwrap_or_default();

    if entry.metadata.titles.is_empty() {
        return (MatchType::None, default_title, None);
    }

    let artists_for_scoring: Vec<generic::Artist> = entry
        .metadata
        .artists
        .iter()
        .map(|name| generic::Artist {
            id: String::new(),
            name: name.clone(),
        })
        .collect();

    let mut album_candidates: Vec<Option<String>> = entry
        .metadata
        .albums
        .iter()
        .map(|s| Some(s.clone()))
        .collect();

    if album_candidates.is_empty() {
        album_candidates.push(None);
    }

    let (best_match, best_title_ref, best_album_val) = entry
        .metadata
        .titles
        .iter()
        .flat_map(|title_candidate| {
            let artists_ref = &artists_for_scoring;
            album_candidates.iter().map(move |album_candidate| {
                let temp_search_result = SearchResult {
                    title: title_candidate.clone(),
                    album: album_candidate.clone(),
                    artists: artists_ref.clone(),
                    ..Default::default()
                };

                let match_type = compare_track(track, &temp_search_result);
                (match_type, title_candidate, album_candidate.clone())
            })
        })
        .max_by_key(|(mt, _, _)| mt.get_score())
        .unwrap_or((MatchType::None, &default_title, None));

    (best_match, best_title_ref.clone(), best_album_val)
}

fn deduplicate_and_sort(
    results: Vec<(IndexEntry, MatchType, String, Option<String>)>,
) -> Vec<(IndexEntry, MatchType, String, Option<String>)> {
    let mut best_results_map: HashMap<DedupKey, (IndexEntry, MatchType, String, Option<String>)> =
        HashMap::new();

    for item in results {
        let (entry, match_type, _, _) = &item;
        let dedup_key = entry.get_dedup_key();
        let timestamp = entry.raw_lyric_file.timestamp;
        let score = match_type.get_score();

        best_results_map
            .entry(dedup_key)
            .and_modify(|existing| {
                let (exist_entry, exist_match_type, _, _) = existing;
                let exist_score = exist_match_type.get_score();
                let exist_timestamp = exist_entry.raw_lyric_file.timestamp;
                if score > exist_score || (score == exist_score && timestamp > exist_timestamp) {
                    *existing = item.clone();
                }
            })
            .or_insert(item);
    }

    let mut unique_results: Vec<_> = best_results_map.into_values().collect();
    unique_results.sort_by(|a, b| b.1.get_score().cmp(&a.1.get_score()));
    unique_results
}

fn into_search_result(
    entry: &IndexEntry,
    best_title: String,
    best_album: Option<String>,
    provider_name: &str,
) -> SearchResult {
    SearchResult {
        provider_id: entry.raw_lyric_file.filename.clone(),
        title: best_title,
        artists: entry
            .metadata
            .artists
            .iter()
            .map(|name| generic::Artist {
                id: String::new(),
                name: name.clone(),
            })
            .collect(),
        album: best_album,
        provider_name: provider_name.to_string(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_provider() -> (AmllTtmlDatabase, IndexEntry) {
        let sample_json = r#"{"metadata":[["musicName",["明明 (深爱着你) (Live)"]],["artists",["李宇春","丁肆Dicey"]],["album",["有歌2024 第4期"]],["ncmMusicId",["2642164541"]],["qqMusicId",["000pF84f1Mqkf7"]],["spotifyId",["29OlvJxVuNd8BJazjvaYpP"]],["isrc",["CNUM72400589"]],["ttmlAuthorGithub",["108002475"]],["ttmlAuthorGithubLogin",["apoint123"]]],"rawLyricFile":"1746678978875-108002475-0a0fb081.ttml"}"#;

        let index_entry: IndexEntry = serde_json::from_str(sample_json).unwrap();

        let provider = AmllTtmlDatabase {
            index: Arc::new(vec![index_entry.clone()]),
            http_client: Arc::new(crate::http::WreqClient::new().unwrap()),
            lyrics_url_template: format!(
                "{RAW_CONTENT_BASE_URL}/{REPO_OWNER}/{REPO_NAME}/{REPO_BRANCH}/raw-lyrics/{{song_id}}"
            ),
        };

        (provider, index_entry)
    }

    #[tokio::test]
    async fn test_amll_search() {
        let (provider, expected_entry) = create_test_provider();

        // --- 案例 1: 仅按标题搜索 ---
        let search_query1 = Track {
            title: Some("明明"),
            artists: None,
            album: None,
            duration: None,
        };
        let results1 = provider.search_songs(&search_query1).await.unwrap();
        assert_eq!(results1.len(), 1, "应该找到一个结果");
        assert_eq!(
            results1[0].provider_id,
            expected_entry.raw_lyric_file.filename
        );
        assert_eq!(results1[0].title, "明明 (深爱着你) (Live)");

        // --- 案例 2: 标题和部分艺术家匹配 ---
        let search_query2 = Track {
            title: Some("明明 (深爱着你) (Live)"),
            artists: Some(&["李宇春"]),
            album: None,
            duration: None,
        };
        let results2 = provider.search_songs(&search_query2).await.unwrap();
        assert_eq!(results2.len(), 1, "应该找到一个结果");
        let artist_names: Vec<String> =
            results2[0].artists.iter().map(|a| a.name.clone()).collect();
        assert_eq!(artist_names, vec!["李宇春", "丁肆Dicey"]);

        // --- 案例 3: 大小写不敏感的艺术家匹配 ---
        let search_query3 = Track {
            title: Some("明明"),
            artists: Some(&["丁肆dicey"]),
            album: None,
            duration: None,
        };
        let results3 = provider.search_songs(&search_query3).await.unwrap();
        assert_eq!(results3.len(), 1, "大小写不敏感的搜索应该工作");

        // --- 案例 4: 艺术家不匹配，但标题匹配 ---
        let search_query4 = Track {
            title: Some("明明"),
            artists: Some(&["周杰伦"]),
            album: None,
            duration: None,
        };
        let results4 = provider.search_songs(&search_query4).await.unwrap();
        assert_eq!(
            results4.len(),
            1,
            "即使艺术家不匹配，只要标题匹配，也应返回一个低分结果"
        );
        assert_eq!(
            results4[0].title, "明明 (深爱着你) (Live)",
            "应返回正确的歌曲条目"
        );

        // --- 案例 5: 标题不匹配 ---
        let search_query5 = Track {
            title: Some("不爱"),
            artists: None,
            album: None,
            duration: None,
        };
        let results5 = provider.search_songs(&search_query5).await.unwrap();
        assert!(results5.is_empty(), "用错误的歌曲名应该搜索不到结果");
    }

    #[tokio::test]
    #[ignore]
    async fn test_amll_fetch_lyrics() {
        let (provider, entry) = create_test_provider();
        let song_id = &entry.raw_lyric_file.filename;

        println!("正在获取 id 为 {song_id} 的歌词");

        let result = provider.get_full_lyrics(song_id).await;

        assert!(
            result.is_ok(),
            "获取歌词不应该出错。错误: {:?}",
            result.err()
        );

        let parsed_data = result.unwrap();
        assert!(
            !parsed_data.parsed.lines.is_empty(),
            "解析后的歌词不应该是空的"
        );

        let first_line = &parsed_data.parsed.lines[0];
        println!("第一行的开始时间: {}ms", first_line.start_ms);
        assert!(first_line.start_ms > 0, "第一行应该有开始时间");
    }
}
