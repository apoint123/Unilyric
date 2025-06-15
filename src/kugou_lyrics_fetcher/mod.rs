// Copyright (c) 2025 [WXRIW]
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

pub mod decrypter;
pub mod error;
pub mod kugoumodel;
use crate::krc_parser;
use crate::types::AssMetadata;

use crate::kugou_lyrics_fetcher::error::{KugouError, Result};
use crate::kugou_lyrics_fetcher::kugoumodel::{
    Candidate, KugouLyricsDownloadResponse, SearchLyricsResponse, SearchSongResponse, SongInfoItem,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};

const SEARCH_SONG_URL: &str = "http://mobilecdn.kugou.com/api/v3/search/song";
const SEARCH_LYRICS_URL: &str = "https://lyrics.kugou.com/search";
const DOWNLOAD_LYRICS_URL: &str = "https://lyrics.kugou.com/download";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FetchedKrcLyrics {
    pub song_name: Option<String>,
    pub artists_name: Vec<String>,
    pub album_name: Option<String>,
    pub krc_content: String,
    pub translation_lines: Option<Vec<String>>,
    pub krc_embedded_metadata: Vec<AssMetadata>,
}

pub async fn search_song_info_async(
    client: &Client,
    keywords: &str,
    page: Option<u32>,
    pagesize: Option<u32>,
) -> Result<Vec<SongInfoItem>> {
    let page_str = page.unwrap_or(1).to_string();
    let pagesize_str = pagesize.unwrap_or(5).to_string();

    let params = [
        ("format", "json"),
        ("keyword", keywords),
        ("page", page_str.as_str()),
        ("pagesize", pagesize_str.as_str()),
        ("showtype", "1"),
    ];

    let response = client
        .get(SEARCH_SONG_URL)
        .query(&params)
        .send()
        .await?
        .error_for_status()?;

    let song_response: SearchSongResponse = response.json().await?;

    if song_response.status != 1 && song_response.error_code != 0 {
        return Err(KugouError::LyricsNotFound(format!(
            "搜索API错误: 状态 {}, 错误代码 {}, 响应: {:?}",
            song_response.status, song_response.error_code, song_response.error
        )));
    }

    match song_response.song_data {
        Some(data) => Ok(data.info),
        None => {
            if song_response.status == 1
                && (song_response.error_code == 0 || song_response.error_code == 200)
            {
                Ok(Vec::new())
            } else {
                Err(KugouError::LyricsNotFound(format!(
                    "搜索API未返回任何数据，状态 {}, 错误代码 {}, 响应: {:?}",
                    song_response.status, song_response.error_code, song_response.error
                )))
            }
        }
    }
}

pub async fn search_lyrics_candidates_async(
    client: &Client,
    keyword: &str,
    duration_ms: Option<i32>,
    hash: Option<&str>,
) -> Result<Vec<Candidate>> {
    let mut params = vec![
        ("ver", "1"),
        ("man", "yes"),
        ("client", "pc"),
        ("keyword", keyword),
    ];

    let duration_sec_str: String;
    if let Some(dur_ms) = duration_ms {
        duration_sec_str = (dur_ms / 1000).to_string();
        params.push(("duration", &duration_sec_str));
    }

    let hash_str_owned: String;
    if let Some(h) = hash
        && !h.is_empty()
    {
        hash_str_owned = h.to_string();
        params.push(("hash", &hash_str_owned));
    }

    let response = client
        .get(SEARCH_LYRICS_URL)
        .query(&params)
        .send()
        .await?
        .error_for_status()?;

    let search_response: SearchLyricsResponse = response.json().await?;

    if search_response.status != 200 {
        return Err(KugouError::LyricsNotFound(format!(
            "搜索歌词候选API错误: {}, 响应: {:?}",
            search_response.status, search_response.error_message
        )));
    }

    if search_response.error_code != 0 && search_response.candidates.is_empty() {
        return Err(KugouError::LyricsNotFound(format!(
            "搜索歌词候选API错误: 状态 {}, 错误代码 {}, 响应: {:?}. 未找到歌词",
            search_response.status, search_response.error_code, search_response.error_message
        )));
    }
    Ok(search_response.candidates)
}

pub async fn download_and_decrypt_lyrics_async(
    client: &Client,
    id: &str,
    access_key: &str,
) -> Result<(String, Option<Vec<String>>, Vec<AssMetadata>)> {
    if id.is_empty() || access_key.is_empty() {
        return Err(KugouError::MissingCredentials);
    }

    let params = [
        ("ver", "1"),
        ("client", "pc"),
        ("id", id),
        ("accesskey", access_key),
        ("fmt", "krc"),
        ("charset", "utf8"),
    ];

    let response = client
        .get(DOWNLOAD_LYRICS_URL)
        .query(&params)
        .send()
        .await?
        .error_for_status()?;

    let download_response: KugouLyricsDownloadResponse = response.json().await?;

    if download_response.status != 200 {
        return Err(KugouError::LyricsNotFound(format!(
            "下载歌词错误: 状态 {}",
            download_response.status
        )));
    }

    match download_response.content {
        Some(encrypted_content) if !encrypted_content.is_empty() => {
            let decrypted_krc = decrypter::decrypt_krc_lyrics(&encrypted_content)?;
            let (_krc_lines_from_parser, embedded_metadata) =
                krc_parser::load_krc_from_string(&decrypted_krc).map_err(|e| {
                    KugouError::InvalidKrcData(format!("解析KRC内嵌元数据失败: {e}"))
                })?;

            let translations = krc_parser::extract_translation_from_krc(&decrypted_krc)?;
            Ok((decrypted_krc, translations, embedded_metadata))
        }
        _ => {
            if download_response.error_code != 0 {
                Err(KugouError::LyricsNotFound(format!(
                    "下载歌词错误: 状态 {}, 代码 {}",
                    download_response.status, download_response.error_code
                )))
            } else {
                Err(KugouError::EmptyLyricContent)
            }
        }
    }
}

pub async fn fetch_lyrics_for_song_async(
    client: &Client,
    song_keywords: &str,
) -> Result<FetchedKrcLyrics> {
    let song_infos = search_song_info_async(client, song_keywords, None, Some(5)).await?;

    if song_infos.is_empty() {
        // log::warn!(
        //     "[KugouFetcher] 使用关键词 '{}' 未找到任何歌曲信息。",
        //     song_keywords
        // );
        return Err(KugouError::LyricsNotFound("未找到歌曲".to_string()));
    }

    let selected_song = song_infos.first().ok_or_else(|| {
        log::error!("[KugouFetcher] song_infos 为空，逻辑错误。");
        KugouError::LyricsNotFound(String::from("未找到歌曲（内部错误）"))
    })?;

    let mut parsed_song_name: Option<String> = None;
    let mut parsed_artists_name: Vec<String> = Vec::new();
    let mut parsed_album_name: Option<String> = None;

    if !selected_song.song_name.is_empty() {
        parsed_song_name = Some(selected_song.song_name.trim().to_string());
    } else {
        log::warn!(
            "[KugouFetcher] API 返回的 song_name 为空 for hash: {}",
            selected_song.hash
        );
    }

    if !selected_song.singer_name.is_empty() {
        let full_singer_name = selected_song.singer_name.trim().to_string();
        if !full_singer_name.is_empty() {
            parsed_artists_name.push(full_singer_name);
        }
    } else {
        log::warn!(
            "[KugouFetcher] API 返回的 singer_name 为空 for hash: {}",
            selected_song.hash
        );
    }

    if let Some(album) = &selected_song.album_name
        && !album.trim().is_empty()
    {
        parsed_album_name = Some(album.trim().to_string());
    }

    let lyric_search_keyword = {
        let artist_part = parsed_artists_name.first().map_or("", |s| s.as_str());
        let title_part = parsed_song_name.as_deref().unwrap_or("");
        if !artist_part.is_empty() && !title_part.is_empty() {
            format!("{artist_part} - {title_part}")
        } else if !title_part.is_empty() {
            title_part.to_string()
        } else if !artist_part.is_empty() {
            artist_part.to_string()
        } else {
            log::warn!(
                "[KugouFetcher] 歌名和艺术家均为空（来自API），使用原始输入进行歌词搜索: {song_keywords}"
            );
            song_keywords.to_string()
        }
    };
    let duration_ms = Some(selected_song.duration);
    let hash_for_search = Some(selected_song.hash.as_str());
    let lyrics_candidates = match search_lyrics_candidates_async(
        client,
        &lyric_search_keyword,
        duration_ms,
        hash_for_search,
    )
    .await
    {
        Ok(_) => {
            // log::warn!(
            //     "[KugouFetcher] 使用关键词 '{}' 首次搜索歌词候选为空，尝试使用原始关键词 '{}'",
            //     lyric_search_keyword,
            //     song_keywords
            // );
            search_lyrics_candidates_async(client, song_keywords, duration_ms, hash_for_search)
                .await?
        }
        Err(e) => {
            log::warn!(
                "[KugouFetcher] 使用关键词 '{lyric_search_keyword}' 首次搜索歌词候选失败: {e:?}。尝试使用原始关键词 '{song_keywords}'"
            );
            search_lyrics_candidates_async(client, song_keywords, duration_ms, hash_for_search)
                .await?
        }
    };
    if lyrics_candidates.is_empty() {
        log::warn!("[KugouFetcher] 未找到歌词候选 : {lyric_search_keyword}");
        return Err(KugouError::NoCandidatesFound);
    }
    let best_lyric_candidate = lyrics_candidates
        .first()
        .ok_or(KugouError::NoCandidatesFound)?;
    log::info!(
        "[KugouFetcher] 选择的歌词候选: ID {}, AccessKey {}",
        best_lyric_candidate.id,
        best_lyric_candidate.access_key
    );

    let (krc_content, translations_opt, embedded_metadata) = download_and_decrypt_lyrics_async(
        client,
        &best_lyric_candidate.id,
        &best_lyric_candidate.access_key,
    )
    .await?;

    if krc_content.is_empty() {
        log::warn!(
            "[KugouFetcher] 下载的KRC内容为空， ID: {}",
            best_lyric_candidate.id
        );
        return Err(KugouError::EmptyLyricContent);
    }

    Ok(FetchedKrcLyrics {
        song_name: parsed_song_name,
        artists_name: parsed_artists_name,
        album_name: parsed_album_name,
        krc_content,
        translation_lines: translations_opt,
        krc_embedded_metadata: embedded_metadata,
    })
}
