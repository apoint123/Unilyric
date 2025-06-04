// 在 src/netease_lyrics_fetcher/mod.rs 文件中

pub mod api;
pub mod crypto;
pub mod error;
pub mod neteasemodel;

use crate::netease_lyrics_fetcher::neteasemodel::Song;
pub use error::{NeteaseError, Result as NeteaseResult};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FetchedNeteaseLyrics {
    pub song_id: Option<String>,
    pub song_name: Option<String>,
    pub artists_name: Vec<String>,
    pub album_name: Option<String>,
    pub main_lrc: Option<String>,
    pub translation_lrc: Option<String>,
    pub romanization_lrc: Option<String>,
    pub karaoke_lrc: Option<String>,
}

impl FetchedNeteaseLyrics {}

pub async fn search_and_fetch_first_netease_lyrics(
    client: &api::NeteaseClient,
    keywords: &str,
) -> NeteaseResult<FetchedNeteaseLyrics> {
    let search_response = client.search_songs_unified(keywords, 1, 0).await?;

    let selected_song: Song = search_response
        .result
        .and_then(|r| r.songs.into_iter().next())
        .ok_or_else(|| {
            // log::warn!(
            //     "[NeteaseLyricsFetcher] 使用关键词 '{}' 未找到歌曲",
            //     keywords
            // );
            NeteaseError::SongNotFound(keywords.to_string())
        })?;

    log::info!(
        "[NeteaseLyricsFetcher] 找到歌曲: '{}' 歌手: {} (ID: {})",
        selected_song.name,
        selected_song
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join("/"),
        selected_song.id
    );

    let lyrics_data = client.fetch_lyrics_unified(selected_song.id).await?;

    // 主LRC歌词 (来自 lrc.lyric)
    let main_lrc = lyrics_data.lrc.as_ref().and_then(|l| l.lyric.clone());

    let translation_lrc = lyrics_data
        .tlyric
        .as_ref()
        .and_then(|l| l.lyric.clone())
        .or_else(|| {
            lyrics_data.ytlrc.as_ref().and_then(|y_val| {
                y_val
                    .as_object()
                    .and_then(|obj| obj.get("lyric")?.as_str().map(String::from))
            })
        });

    let romanization_lrc = lyrics_data
        .romalrc
        .as_ref()
        .and_then(|l| l.lyric.clone())
        .or_else(|| {
            lyrics_data.yromalrc.as_ref().and_then(|y_val| {
                y_val
                    .as_object()
                    .and_then(|obj| obj.get("lyric")?.as_str().map(String::from))
            })
        });

    // 主歌词的YRC/KLyric内容
    let mut karaoke_lrc_content: Option<String> = None;
    let mut source_of_karaoke = "None";

    if let Some(yrc_val) = &lyrics_data.yrc {
        // 主YRC
        if let Some(lyric_obj) = yrc_val.as_object() {
            if let Some(lyric_str_val) = lyric_obj.get("lyric") {
                if let Some(lyric_str) = lyric_str_val.as_str() {
                    if !lyric_str.is_empty() {
                        karaoke_lrc_content = Some(lyric_str.to_string());
                        source_of_karaoke = "YRC";
                        log::info!("[NeteaseLyricsFetcher] 加载 YRC 逐字歌词");
                    }
                }
            }
        }
    }
    if karaoke_lrc_content.is_none() {
        // 如果主YRC没有，尝试KLyric
        if let Some(klyric_content_obj) = &lyrics_data.klyric {
            if let Some(klyric_str) = &klyric_content_obj.lyric {
                if !klyric_str.is_empty() {
                    karaoke_lrc_content = Some(klyric_str.clone());
                    source_of_karaoke = "klyric";
                    log::info!("[NeteaseLyricsFetcher] 加载逐字歌词");
                }
            }
        }
    }

    log::info!(
        "[NeteaseLyricsFetcher] 已获取到歌词，歌曲ID {}: 主LRC: {}, 翻译LRC: {}, 罗马音LRC: {}, 逐字歌词 (来自 {}): {}",
        selected_song.id,
        main_lrc.is_some(),
        translation_lrc.is_some(),
        romanization_lrc.is_some(),
        source_of_karaoke,
        karaoke_lrc_content.is_some()
    );

    let song_id_str = Some(selected_song.id.to_string());
    let song_name_str = if selected_song.name.is_empty() {
        None
    } else {
        Some(selected_song.name.clone())
    };
    let artists_name_vec: Vec<String> = selected_song
        .artists
        .iter()
        .map(|a| a.name.clone())
        .filter(|name| !name.is_empty())
        .collect();
    let album_name_str: Option<String> = selected_song.album.as_ref().and_then(|album_simple| {
        if album_simple.name.is_empty() {
            None
        } else {
            Some(album_simple.name.clone())
        }
    });

    Ok(FetchedNeteaseLyrics {
        song_id: song_id_str,
        song_name: song_name_str,
        artists_name: artists_name_vec,
        album_name: album_name_str,
        main_lrc,
        translation_lrc,
        romanization_lrc,
        karaoke_lrc: karaoke_lrc_content,
    })
}
