//! 搜索模块

use std::collections::HashMap;

use futures::future;
use lyrics_helper_core::{MatchType, SearchResult, Track};
use tracing::{debug, info, warn};

use crate::{error::Result, providers::Provider};

pub(crate) mod matcher;
use matcher::compare_track;

/// 在多个提供商中并发搜索歌曲。
///
/// # 参数
/// * `providers` - 一个包含多个提供商实例的切片（通过 `Box<dyn Provider>` 实现动态分发）。
/// * `track` - 原始的、最完整的音轨元数据引用，用作搜索和比较的基准。
/// * `full_search` - 控制每个提供商内部的搜索行为（`true`为全面搜索，`false`为快速搜索）。
///
/// # 返回
/// 一个 `Result`，成功时包含一个 `Vec<SearchResult>`，该列表是来自所有提供商的结果的合集，
/// 并已按匹配度从高到低排序和去重。
pub async fn search_track_in_providers(
    providers: &[Box<dyn Provider>],
    track: &Track<'_>,
    full_search: bool,
) -> Result<Vec<SearchResult>> {
    info!(
        "开始对歌曲 '{}' 在 {} 个提供商中进行搜索...",
        track.title.unwrap_or("未知标题"),
        providers.len()
    );

    let search_futures = providers
        .iter()
        .map(|provider| search_track(provider.as_ref(), track, full_search));

    let results_from_all_providers = future::join_all(search_futures).await;

    let mut combined_results: Vec<SearchResult> = Vec::new();
    for (i, result) in results_from_all_providers.into_iter().enumerate() {
        match result {
            Ok(provider_results) => {
                if !provider_results.is_empty() {
                    info!(
                        "提供商 '{}' 成功返回 {} 条结果。",
                        providers[i].name(),
                        provider_results.len()
                    );
                    combined_results.extend(provider_results);
                }
            }
            Err(e) => {
                warn!(
                    "提供商 '{}' 的搜索失败: {}. 将忽略此提供商的结果。",
                    providers[i].name(),
                    e
                );
            }
        }
    }

    info!("搜索完毕，收集到 {} 条结果", combined_results.len());
    Ok(finalize_multi_provider_results(combined_results))
}

/// 对来自多个提供商的结果进行最终的排序和去重。
fn finalize_multi_provider_results(results: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut best_results_map: HashMap<(String, String), SearchResult> = HashMap::new();

    for result in results {
        let key = (result.provider_name.clone(), result.provider_id.clone());

        best_results_map
            .entry(key)
            .and_modify(|existing| {
                if result.match_type > existing.match_type {
                    *existing = result.clone();
                }
            })
            .or_insert(result);
    }

    let mut unique_results: Vec<SearchResult> = best_results_map.into_values().collect();

    unique_results.sort_unstable_by(|a, b| b.match_type.cmp(&a.match_type));

    unique_results
}

/// 根据歌曲元数据，在指定提供商上进行搜索。
///
/// # 参数
/// * `provider` - 一个实现了 `Provider` trait 的动态引用，代表要搜索的音乐平台。
/// * `track` - 原始的、最完整的歌曲元数据引用，用作搜索和比较的基准。
/// * `full_search` - 一个布尔值，控制搜索行为：
///   - `true`: 执行所有级别的搜索策略，以获得最全面的结果。
///   - `false`: 在任何一个级别找到结果后立即停止，以提高效率。
///
/// # 返回
/// 一个 `Result`，成功时包含一个 `Vec<SearchResult>`，该列表已按匹配度从高到低排序并去重。
pub async fn search_track(
    provider: &dyn Provider,
    track: &Track<'_>,
    full_search: bool,
) -> Result<Vec<SearchResult>> {
    info!(
        "开始对歌曲 '{}' by {:?} 进行搜索 (提供商: {}, 全面搜索: {})",
        track.title.unwrap_or("未知标题"),
        track.artists.unwrap_or(&["未知艺术家"]),
        provider.name(),
        full_search
    );

    let mut all_results: Vec<SearchResult> = Vec::new();

    if track.title.is_some() && track.artists.is_some() {
        let precise_query = Track {
            title: track.title,
            artists: track.artists,
            album: track.album,
            duration: track.duration,
        };
        execute_search_level(provider, &precise_query, track, &mut all_results).await;
    }

    let has_perfect_match = all_results
        .iter()
        .any(|r| r.match_type == MatchType::Perfect);
    if has_perfect_match || (!full_search && !all_results.is_empty()) {
        return Ok(finalize_single_provider_results(all_results));
    }

    if track.title.is_some() {
        let title_only_query = Track {
            title: track.title,
            artists: None,
            album: track.album,
            duration: track.duration,
        };
        execute_search_level(provider, &title_only_query, track, &mut all_results).await;
    }

    Ok(finalize_single_provider_results(all_results))
}

/// 执行单个级别的搜索，处理结果并将其添加到总结果列表中。
async fn execute_search_level(
    provider: &dyn Provider,
    search_query: &Track<'_>,
    original_track: &Track<'_>,
    all_results: &mut Vec<SearchResult>,
) {
    match provider.search_songs(search_query).await {
        Ok(mut results) => {
            if !results.is_empty() {
                debug!("搜索级别命中，找到 {} 个结果。", results.len());
                // 为这批次的结果计算并设置匹配度
                for result in &mut results {
                    result.match_type = compare_track(original_track, result);
                }
                all_results.extend(results);
            }
        }
        Err(e) => {
            warn!(
                "某个搜索级别执行失败 (查询: {:?})，错误: {}。继续执行下一级别。",
                search_query, e
            );
        }
    }
}

/// 对搜索结果进行排序和去重，返回最终的干净列表。
fn finalize_single_provider_results(results: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut best_results_map: HashMap<String, SearchResult> = HashMap::new();

    for result in results {
        let key = result.provider_id.clone();

        best_results_map
            .entry(key)
            .and_modify(|existing| {
                if result.match_type > existing.match_type {
                    *existing = result.clone();
                }
            })
            .or_insert(result);
    }

    let mut unique_results: Vec<SearchResult> = best_results_map.into_values().collect();

    unique_results.sort_unstable_by(|a, b| b.match_type.cmp(&a.match_type));

    unique_results
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;

    use super::*;
    use crate::http::HttpClient;
    use crate::providers::Provider;
    use crate::providers::qq::QQMusic;
    use async_trait::async_trait;
    use lyrics_helper_core::model::generic;
    use lyrics_helper_core::{Artist, FullLyricsResult, ParsedSourceData};

    #[derive(Clone)]
    struct MockProvider {
        name: &'static str,
    }

    #[async_trait]
    impl Provider for MockProvider {
        async fn with_http_client(_http_client: Arc<dyn HttpClient>) -> Result<Self>
        where
            Self: Sized,
        {
            Ok(Self { name: "mock" })
        }

        fn name(&self) -> &'static str {
            self.name
        }

        async fn search_songs(&self, _: &Track<'_>) -> Result<Vec<SearchResult>> {
            let mut results = Vec::new();
            if self.name == "provider_a" {
                // Perfect
                results.push(SearchResult {
                    title: "Song A".to_string(),
                    artists: vec![Artist {
                        id: String::new(),
                        name: "Artist A".to_string(),
                    }],
                    album: Some("Album A".to_string()),
                    duration: Some(240_000), // 4:00
                    provider_name: "provider_a".to_string(),
                    provider_id: "pa_perfect".to_string(),
                    ..Default::default()
                });
                // High or PrettyHigh
                results.push(SearchResult {
                    title: "Song A".to_string(),
                    artists: vec![Artist {
                        id: String::new(),
                        name: "Artist A".to_string(),
                    }],
                    album: Some("Wrong Album".to_string()),
                    duration: Some(300_000), // 5:00
                    provider_name: "provider_a".to_string(),
                    provider_id: "pa_high".to_string(),
                    ..Default::default()
                });
                // 测试 finalize_single_provider_results 的去重
                results.push(SearchResult {
                    title: "Song A".to_string(),
                    artists: vec![Artist {
                        id: String::new(),
                        name: "Artist A".to_string(),
                    }],
                    album: Some("Album A".to_string()),
                    duration: Some(240_000),
                    provider_name: "provider_a".to_string(),
                    provider_id: "pa_perfect".to_string(),
                    ..Default::default()
                });
            }
            if self.name == "provider_b" {
                // Low
                results.push(SearchResult {
                    title: "Song A".to_string(),
                    artists: vec![Artist {
                        id: String::new(),
                        name: "Unknown Artist".to_string(),
                    }],
                    album: Some("Unknown Album".to_string()),
                    duration: Some(180_000), // 3:00
                    provider_name: "provider_b".to_string(),
                    provider_id: "pb_low".to_string(),
                    ..Default::default()
                });
                // None
                results.push(SearchResult {
                    title: "Different Song".to_string(),
                    artists: vec![Artist {
                        id: String::new(),
                        name: "Different Artist".to_string(),
                    }],
                    album: Some("Different Album".to_string()),
                    duration: Some(240_000),
                    provider_name: "provider_b".to_string(),
                    provider_id: "pb_none".to_string(),
                    ..Default::default()
                });
            }
            Ok(results)
        }

        async fn get_full_lyrics(&self, _song_id: &str) -> Result<FullLyricsResult> {
            unimplemented!()
        }
        async fn get_lyrics(&self, _song_id: &str) -> Result<ParsedSourceData> {
            unimplemented!()
        }
        async fn get_album_info(&self, _album_id: &str) -> Result<generic::Album> {
            unimplemented!()
        }
        async fn get_album_songs(
            &self,
            _album_id: &str,
            _page: u32,
            _page_size: u32,
        ) -> Result<Vec<generic::Song>> {
            unimplemented!()
        }
        async fn get_singer_songs(
            &self,
            _singer_id: &str,
            _page: u32,
            _page_size: u32,
        ) -> Result<Vec<generic::Song>> {
            unimplemented!()
        }
        async fn get_playlist(&self, _playlist_id: &str) -> Result<generic::Playlist> {
            unimplemented!()
        }
        async fn get_song_info(&self, _song_id: &str) -> Result<generic::Song> {
            unimplemented!()
        }
        async fn get_album_cover_url(
            &self,
            _album_id: &str,
            _size: generic::CoverSize,
        ) -> Result<String> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn test_search_track_in_multiple_providers() {
        let providers: Vec<Box<dyn Provider>> = vec![
            Box::new(MockProvider { name: "provider_a" }),
            Box::new(MockProvider { name: "provider_b" }),
        ];

        // 这个 track 将被用于和 Mock 返回的数据进行比较
        let track = Track {
            title: Some("Song A"),
            artists: Some(&["Artist A"]),
            album: Some("Album A"),
            duration: None,
        };

        // 我们只对 `search_track_in_providers` 的聚合、排序和去重逻辑感兴趣，
        // 所以直接调用它。full_search 设置为 false 就可以，因为 mock provider 的行为是固定的。
        let results = search_track_in_providers(&providers, &track, false)
            .await
            .unwrap();

        // provider_a 返回 3 个结果，其中 1 个是重复的，去重后剩 2 个。
        // provider_b 返回 2 个结果。
        // 总共应该有4 个结果。
        assert_eq!(results.len(), 4, "结果数量应为 4");

        println!(
            "排序后的结果: {:?}",
            results
                .iter()
                .map(|r| (&r.provider_id, r.match_type))
                .collect::<Vec<_>>()
        );

        assert_eq!(results[0].provider_id, "pa_perfect");
        assert_eq!(results[0].match_type, MatchType::Perfect, "应为完美匹配");

        assert_eq!(results[1].provider_id, "pa_high");
        assert!(results[1].match_type > MatchType::Medium, "应为高度匹配");

        assert_eq!(results[2].provider_id, "pb_low");
        assert!(
            results[2].match_type > MatchType::None && results[2].match_type < MatchType::Medium,
            "应为低匹配"
        );

        assert_eq!(results[3].provider_id, "pb_none");
        assert_eq!(results[3].match_type, MatchType::None, "应为不匹配");
    }

    #[tokio::test]
    #[ignore]
    async fn test_search_track_flow() {
        let qq_music_provider = QQMusic::new().await.unwrap();
        let provider: &dyn Provider = &qq_music_provider;

        let track = Track {
            title: Some("我怕来者不是你"),
            artists: Some(&["小蓝背心"]),
            album: Some("我怕来者不是你"),
            duration: Some(211_000),
        };

        let results = search_track(provider, &track, true).await.unwrap();

        assert!(!results.is_empty(), "搜索结果不应为空");

        let first_result = &results[0];
        println!(
            "找到的最佳匹配: '{}' by {:?}, 匹配类型: {:?}",
            first_result.title, first_result.artists, first_result.match_type
        );

        assert_eq!(
            first_result.match_type,
            MatchType::Perfect,
            "最佳匹配结果的类型应该是 Perfect"
        );

        let is_sorted = results
            .windows(2)
            .all(|w| w[0].match_type >= w[1].match_type);
        assert!(is_sorted, "搜索结果应该按匹配度降序排列");

        let mut provider_ids = HashSet::new();
        for result in &results {
            assert!(
                provider_ids.insert(&result.provider_id),
                "发现重复的 provider_id，去重失败"
            );
        }

        println!(
            "test_search_track_flow: 成功获取 {} 条不重复的结果。",
            results.len()
        );
    }
}
