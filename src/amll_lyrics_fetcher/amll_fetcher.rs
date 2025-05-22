use reqwest::Client;
use serde_json;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use super::types::{AmllIndexEntry, AmllSearchField, FetchedAmllTtmlLyrics};
use crate::types::ConvertError;

fn save_index_to_cache(cache_file_path: &Path, index_content: &str) -> Result<(), ConvertError> {
    log::info!(
        "[AMLLLyricsFetcher] 准备保存缓存文件: {:?}",
        cache_file_path
    );
    if let Some(parent_dir) = cache_file_path.parent() {
        std::fs::create_dir_all(parent_dir).map_err(|e| {
            log::error!(
                "[AMLLLyricsFetcher] 创建缓存目录 {:?} 失败: {}",
                parent_dir,
                e
            );
            ConvertError::Io(e)
        })?;
    }

    let mut file = File::create(cache_file_path).map_err(|e| {
        log::error!(
            "[AMLLLyricsFetcher] 创建缓存文件 {:?} 失败: {}",
            cache_file_path,
            e
        );
        ConvertError::Io(e)
    })?;

    file.write_all(index_content.as_bytes()).map_err(|e| {
        log::error!(
            "[AMLLLyricsFetcher] 写入缓存文件 {:?} 失败: {}",
            cache_file_path,
            e
        );
        ConvertError::Io(e)
    })?;
    log::info!("[AMLLLyricsFetcher] 缓存文件已保存: {:?}", cache_file_path);
    Ok(())
}

pub fn load_index_from_cache(cache_file_path: &Path) -> Result<Vec<AmllIndexEntry>, ConvertError> {
    if !cache_file_path.exists() {
        log::info!(
            "[AMLLLyricsFetcher] 缓存文件 {:?} 不存在。",
            cache_file_path
        );
        return Err(ConvertError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            "缓存文件不存在",
        )));
    }

    let file = File::open(cache_file_path).map_err(|e| {
        log::error!(
            "[AMLLLyricsFetcher] 打开缓存文件 {:?} 失败: {}",
            cache_file_path,
            e
        );
        ConvertError::Io(e)
    })?;

    let reader = BufReader::new(file);
    let mut entries = Vec::new();
    for (line_num, line_result) in reader.lines().enumerate() {
        let line = line_result.map_err(|e| {
            log::error!(
                "[AMLLLyricsFetcher] 读取缓存文件 {:?} 第 {} 行失败: {}",
                cache_file_path,
                line_num + 1,
                e
            );
            ConvertError::Io(e)
        })?;

        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<AmllIndexEntry>(&line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                log::warn!(
                    "[AMLLLyricsFetcher] 解析缓存文件 {:?} 第 {} 行失败: '{}', 错误: {}",
                    cache_file_path,
                    line_num + 1,
                    line,
                    e
                );
                return Err(ConvertError::JsonParse(e));
            }
        }
    }

    if entries.is_empty() {
        log::error!(
            "[AMLLLyricsFetcher] 从缓存文件 {:?} 加载的索引为空。",
            cache_file_path
        );
    }

    log::info!(
        "[AMLLLyricsFetcher] 从缓存 {:?} 成功加载并解析 {} 条索引。",
        cache_file_path,
        entries.len()
    );
    Ok(entries)
}

pub async fn download_and_parse_index(
    client: &Client,
    repo_base_url: &str,
    cache_file_path: &Path,
) -> Result<Vec<AmllIndexEntry>, ConvertError> {
    let index_url = format!("{}/{}", repo_base_url, "am-lyrics/index.jsonl");
    log::info!(
        "[AMLLLyricsFetcher] 开始下载 index.jsonl 索引文件: {}",
        index_url
    );

    let response = client.get(&index_url).send().await.map_err(|e| {
        log::error!("[AMLLLyricsFetcher] 下载索引文件失败: {}", e);
        ConvertError::NetworkRequest(e)
    })?;

    if !response.status().is_success() {
        let err_msg = format!(
            "[AMLLLyricsFetcher] 下载索引文件失败，状态码: {}",
            response.status()
        );
        log::error!("{}", err_msg);
        let response_err = response.error_for_status().unwrap_err();
        return Err(ConvertError::NetworkRequest(response_err));
    }

    let response_text = response.text().await.map_err(|e| {
        log::error!("[AMLLLyricsFetcher] 读取索引文件失败: {}", e);
        ConvertError::NetworkRequest(e)
    })?;

    log::info!("[AMLLLyricsFetcher] 索引文件下载完毕",);

    if let Err(e) = save_index_to_cache(cache_file_path, &response_text) {
        log::warn!("[AMLLLyricsFetcher] 保存索引文件到缓存失败: {}", e);
    }

    let mut entries = Vec::new();
    for (line_num, line) in response_text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<AmllIndexEntry>(line) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                log::warn!(
                    "[AMLLLyricsFetcher] 解析索引文件第 {} 行失败: '{}', 错误: {}",
                    line_num + 1,
                    line,
                    e
                );
            }
        }
    }

    if entries.is_empty() && !response_text.trim().is_empty() {
        log::error!("[AMLLLyricsFetcher] 索引文件未能解析出任何条目。请检查文件格式或缓存。");
    }

    log::info!(
        "[AMLLLyricsFetcher] 索引文件解析完毕，共 {} 条目。",
        entries.len()
    );
    Ok(entries)
}

pub fn search_lyrics_in_index(
    query: &str,
    search_field: &AmllSearchField,
    index_entries: &[AmllIndexEntry],
) -> Vec<AmllIndexEntry> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let lower_query = query.to_lowercase();
    let search_key = search_field.to_key_string();
    index_entries
        .iter()
        .filter(|entry| {
            if *search_field == AmllSearchField::AppleMusicId
                && entry.id.to_lowercase().contains(&lower_query)
            {
                return true;
            }
            for (key, values) in &entry.metadata {
                if key == search_key {
                    return match search_field {
                        AmllSearchField::MusicName
                        | AmllSearchField::Artists
                        | AmllSearchField::Album
                        | AmllSearchField::TtmlAuthorGithubLogin => values
                            .iter()
                            .any(|v| v.to_lowercase().contains(&lower_query)),
                        AmllSearchField::NcmMusicId
                        | AmllSearchField::QqMusicId
                        | AmllSearchField::SpotifyId
                        | AmllSearchField::AppleMusicId
                        | AmllSearchField::Isrc
                        | AmllSearchField::TtmlAuthorGithub => values
                            .iter()
                            .any(|v| v.to_lowercase() == lower_query || v.contains(&lower_query)),
                    };
                }
            }
            false
        })
        .cloned()
        .collect()
}

pub async fn download_ttml_from_entry(
    client: &Client,
    repo_base_url: &str,
    index_entry: &AmllIndexEntry,
) -> Result<FetchedAmllTtmlLyrics, ConvertError> {
    let ttml_file_url = format!(
        "{}/{}/{}",
        repo_base_url, "raw-lyrics", index_entry.raw_lyric_file
    );
    log::info!("[AMLLLyricsFetcher] 开始下载 TTML 文件: {}", ttml_file_url);
    let response = client.get(&ttml_file_url).send().await.map_err(|e| {
        log::error!("[AMLLLyricsFetcher] 网络请求失败: {}", e);
        ConvertError::NetworkRequest(e)
    })?;
    if !response.status().is_success() {
        let err_msg = format!(
            "[AMLLLyricsFetcher] 下载 TTML 文件 '{}' 失败，状态码: {}",
            index_entry.raw_lyric_file,
            response.status()
        );
        log::error!("{}", err_msg);
        let response_err = response.error_for_status().unwrap_err();
        return Err(ConvertError::NetworkRequest(response_err));
    }
    let ttml_content = response.text().await.map_err(|e| {
        log::error!(
            "[AMLLLyricsFetcher] 读取 TTML 文件 '{}' 失败: {}",
            index_entry.raw_lyric_file,
            e
        );
        ConvertError::NetworkRequest(e)
    })?;
    if ttml_content.trim().is_empty() {
        log::error!(
            "[AMLLLyricsFetcher] 下载的 TTML 文件 '{}' 内容为空。",
            index_entry.raw_lyric_file
        );
    }
    log::info!(
        "[AMLLLyricsFetcher] TTML 文件 '{}' 下载完毕。",
        index_entry.raw_lyric_file,
    );
    let mut song_name = None;
    let mut artists_name = Vec::new();
    let mut album_name = None;
    for (key, values) in &index_entry.metadata {
        if values.is_empty() {
            continue;
        }
        match key.as_str() {
            _ if key == AmllSearchField::MusicName.to_key_string() => {
                song_name = values.first().cloned();
            }
            _ if key == AmllSearchField::Artists.to_key_string() => {
                artists_name.extend(values.iter().cloned());
            }
            _ if key == AmllSearchField::Album.to_key_string() => {
                album_name = values.first().cloned();
            }
            _ => {}
        }
    }
    Ok(FetchedAmllTtmlLyrics {
        song_name,
        artists_name,
        album_name,
        ttml_content,
        source_id: Some(index_entry.id.clone()),
        all_metadata_from_index: index_entry.metadata.clone(),
    })
}
