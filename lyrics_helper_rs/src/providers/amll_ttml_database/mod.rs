//! 此模块实现了与 AMLL TTML Database 进行交互的 `Provider`。

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;

use crate::{
    config::AmllMirror,
    converter::{self},
    error::{LyricsHelperError, Result},
    http::HttpClient,
    model::match_type::MatchScorable,
    providers::{Provider, amll_ttml_database::types::SearchField},
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
const REPO_OWNER: &str = "Steve-xmh";
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

impl AmllTtmlDatabase {
    /// 根据特定字段进行精确或模糊搜索。
    ///
    /// 这是一个此 Provider 特有的高级搜索功能。
    ///
    /// # 参数
    /// * `query` - 要搜索的文本或 ID。
    /// * `field` - 指定在哪一个字段中进行搜索 (`SearchField` 枚举)。
    ///
    /// # 返回
    /// 返回一个包含所有匹配条目的 `Vec<IndexEntry>`。
    #[must_use]
    pub fn search_by_field(&self, query: &str, field: &SearchField) -> Vec<IndexEntry> {
        if query.trim().is_empty() {
            return vec![];
        }

        let lower_query = query.to_lowercase();
        let metadata_key = field.to_metadata_key();

        self.index
            .iter()
            .filter(|entry| {
                entry
                    .metadata
                    .get(metadata_key)
                    .is_some_and(|values| match field {
                        SearchField::NcmMusicId
                        | SearchField::QqMusicId
                        | SearchField::SpotifyId
                        | SearchField::AppleMusicId
                        | SearchField::Isrc
                        | SearchField::TtmlAuthorGithub
                        | SearchField::TtmlAuthorGithubLogin => {
                            values.iter().any(|v| v.to_lowercase() == lower_query)
                        }
                        SearchField::MusicName | SearchField::Artists | SearchField::Album => {
                            values
                                .iter()
                                .any(|v| v.to_lowercase().contains(&lower_query))
                        }
                    })
            })
            .cloned()
            .collect()
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
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
                format!(
                    "{RAW_CONTENT_BASE_URL}/{REPO_OWNER}/{REPO_NAME}/{REPO_BRANCH}/{INDEX_FILE_PATH_IN_REPO}"
                ),
                "https://amll.bikonoo.com/raw-lyrics/{song_id}".to_string(),
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
                tracing::warn!("[AMLL] {msg}");
                tracing::warn!("[AMLL] 无法检查索引更新，将使用本地缓存（如果存在）。");
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

    /// 在索引中搜索歌曲。
    async fn search_songs(&self, track: &Track<'_>) -> Result<Vec<SearchResult>> {
        let title_to_search = track.title.unwrap_or_default();
        if title_to_search.trim().is_empty() {
            return Ok(vec![]);
        }
        let lower_title_to_search = title_to_search.to_lowercase();

        // 快速从索引中找出所有标题可能相关的条目
        let candidates: Vec<IndexEntry> = self
            .index
            .iter()
            .rev() // 越下面的越新
            .filter(|entry| {
                entry.get_meta_vec("musicName").is_some_and(|titles| {
                    titles
                        .iter()
                        .any(|v| v.to_lowercase().contains(&lower_title_to_search))
                })
            })
            .cloned()
            .collect();

        if candidates.is_empty() {
            return Ok(vec![]);
        }

        let mut scored_results: Vec<(IndexEntry, MatchType)> = candidates
            .into_iter()
            .map(|entry| {
                // 临时转换为 SearchResult 以便评分
                let temp_search_result = SearchResult {
                    title: entry
                        .get_meta_str("musicName")
                        .unwrap_or_default()
                        .to_string(),
                    artists: entry
                        .get_meta_vec("artists")
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|name| generic::Artist {
                            id: String::new(),
                            name,
                        })
                        .collect(),
                    album: entry.get_meta_str("album").map(String::from),
                    ..Default::default()
                };
                let match_type = compare_track(track, &temp_search_result);
                (entry, match_type)
            })
            .filter(|(_, match_type)| *match_type >= MatchType::VeryLow)
            .collect();

        scored_results.sort_by(|a, b| b.1.get_score().cmp(&a.1.get_score()));

        let final_results = scored_results
            .into_iter()
            .take(20)
            .map(|(entry, _)| SearchResult {
                provider_id: entry.raw_lyric_file.clone(),
                title: entry
                    .get_meta_str("musicName")
                    .unwrap_or_default()
                    .to_string(),
                artists: entry
                    .get_meta_vec("artists")
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|name| generic::Artist {
                        id: String::new(),
                        name,
                    })
                    .collect(),
                album: entry.get_meta_str("album").map(String::from),
                provider_name: self.name().to_string(),
                ..Default::default()
            })
            .collect();

        Ok(final_results)
    }

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
        };

        let mut parsed_data =
            converter::parse_and_merge(&conversion_input, &ConversionOptions::default())
                .map_err(|e| LyricsHelperError::Parser(e.to_string()))?;

        parsed_data.source_name = "amll-ttml-database".to_string();

        let raw_lyrics = RawLyrics {
            format: "ttml".to_string(),
            content: response_text,
            translation: None,
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

    async fn get_song_link(&self, _song_id: &str) -> Result<String> {
        Err(LyricsHelperError::ProviderNotSupported(
            "amll-ttml-database 不支持 get_song_link".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_provider() -> (AmllTtmlDatabase, IndexEntry) {
        let sample_json = r#"{"metadata":[["musicName",["明明 (深爱着你) (Live)"]],["artists",["李宇春","丁肆Dicey"]],["album",["有歌2024 第4期"]],["ncmMusicId",["2642164541"]],["qqMusicId",["000pF84f1Mqkf7"]],["spotifyId",["29OlvJxVuNd8BJazjvaYpP"]],["isrc",["CNUM72400589"]],["ttmlAuthorGithub",["108002475"]],["ttmlAuthorGithubLogin",["apoint123"]]],"rawLyricFile":"1746678978875-108002475-0a0fb081.ttml"}"#;

        let index_entry: IndexEntry = serde_json::from_str(sample_json).unwrap();

        let provider = AmllTtmlDatabase {
            index: Arc::new(vec![index_entry.clone()]),
            http_client: Arc::new(crate::http::ReqwestClient::new().unwrap()),
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
        assert_eq!(results1[0].provider_id, expected_entry.raw_lyric_file);
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
        let song_id = &entry.raw_lyric_file;

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

    #[tokio::test]
    async fn test_amll_search_by_specific_field() {
        let (provider, _expected_entry) = create_test_provider();

        let results1 = provider.search_by_field("2642164541", &SearchField::NcmMusicId);
        assert_eq!(results1.len(), 1, "使用 NcmMusicId 搜索应该找到一个结果");
        assert_eq!(
            results1[0].get_meta_vec("ncmMusicId").unwrap(),
            &vec!["2642164541"]
        );

        let results2 = provider.search_by_field("apoint123", &SearchField::TtmlAuthorGithubLogin);
        assert_eq!(results2.len(), 1, "使用 Github 登录名搜索应该找到一个结果");

        let results3 = provider.search_by_field("李宇春", &SearchField::Artists);
        assert_eq!(results3.len(), 1, "使用艺术家包含搜索应该找到一个结果");

        let results4 = provider.search_by_field("1234567890", &SearchField::NcmMusicId);
        assert!(results4.is_empty(), "用错误的 ID 搜索应该找不到结果");
    }
}
