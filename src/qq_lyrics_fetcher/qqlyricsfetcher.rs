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

// 导入同级模块 qqmusic_api，该模块负责与QQ音乐API的实际交互
use crate::qq_lyrics_fetcher::qqmusic_api::{self, Song};
// 导入项目中定义的通用错误类型 ConvertError
use crate::types::ConvertError;
// 导入 reqwest::Client 用于发送HTTP请求
use reqwest::Client;
// 导入 serde 的 Deserialize 和 Serialize 特征，用于数据的序列化和反序列化
use serde::{Deserialize, Serialize};
// 从项目类型模块中导入 ConvertError，并重命名为 UniLyricConvertError 以避免与当前模块的 QQLyricsFetcherError 冲突
use crate::types::ConvertError as UniLyricConvertError;

/// 定义 QQ 音乐歌词获取过程中可能发生的特定错误。
/// 使用 thiserror 宏可以方便地为每个错误变体生成 Display 和 Error 特征的实现。
#[derive(Debug, thiserror::Error)]
pub enum QQLyricsFetcherError {
    #[error("API调用失败: {0}")] // API 调用或处理过程中的通用错误，源自 UniLyricConvertError
    ApiProcess(#[from] UniLyricConvertError), // #[from] 允许从 UniLyricConvertError 自动转换
    #[error("未找到歌词 (或必要信息缺失)")] // 未找到歌曲信息或歌词信息不完整
    SongInfoMissing,
    #[error("未找到歌词")] // 明确表示未找到选定歌曲的歌词
    LyricNotFoundForSelectedSong,
    #[error("API错误: {0}")] // 更具体的API错误，源自 qqmusic_api 模块可能返回的 ConvertError
    ApiError(ConvertError), // 注意：这里的 ConvertError 是 crate::types::ConvertError
    #[error("QQ音乐服务器拒绝了你的搜索请求 (代码2001)，请稍后再试")]
    // QQ音乐API返回的特定错误代码
    RequestRejected,
}

// 为下载结果定义一个类型别名，方便使用
type DownloadResult<T> = std::result::Result<T, QQLyricsFetcherError>;

/// 结构体，用于存储从 QQ 音乐获取到的歌词数据和相关元数据。
/// 实现了 Debug, Clone, Default, Serialize, Deserialize 特征。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FetchedQqLyrics {
    // --- 元数据字段 ---
    pub song_id: Option<String>,    // QQ音乐歌曲ID (通常是 songmid)
    pub song_name: Option<String>,  // 歌曲名 (来自API返回的 songname)
    pub artists_name: Vec<String>,  // 艺术家列表 (来自API返回的 singer 结构，可能有多个艺术家)
    pub album_name: Option<String>, // 专辑名 (可能为空或由其他地方填充)

    // --- 歌词内容字段 ---
    pub main_lyrics_qrc: Option<String>, // 主歌词内容 (通常是原始QRC格式)
    pub translation_lrc: Option<String>, // 翻译歌词内容 (通常是LRC格式)
    pub romanization_qrc: Option<String>, // 罗马音歌词内容 (通常是原始QRC格式)
}

/// 根据查询关键词从 QQ 音乐搜索歌曲，并下载第一个匹配结果的歌词。
///
/// # Arguments
/// * `client` - 一个 `reqwest::Client` 的引用，用于执行 HTTP 请求。
/// * `query` - 用户输入的搜索关键词（例如 "歌曲名 - 歌手"）。
///
/// # Returns
/// `DownloadResult<FetchedQqLyrics>` -
///   - `Ok(FetchedQqLyrics)`: 如果成功获取到歌词数据。
///   - `Err(QQLyricsFetcherError)`: 如果在过程中发生任何错误。
pub async fn download_lyrics_by_query_first_match(
    client: &Client,
    query: &str,
) -> DownloadResult<FetchedQqLyrics> {
    // 1. 调用 qqmusic_api::search_song 搜索歌曲
    //    该函数返回一个元组 (歌曲列表, 原始搜索响应文本) 或一个错误
    //    这里的 `?` 操作符会在 search_song 返回 Err 时提前返回错误
    let (songs, _raw_search_resp) = qqmusic_api::search_song(client, query).await?;

    // 2. 检查搜索结果
    if songs.is_empty() {
        // 如果没有找到任何歌曲，记录错误并返回 SongInfoMissing 错误
        log::error!("[QQLyricsFetcher] 未找到任何歌曲: {}", query);
        return Err(QQLyricsFetcherError::SongInfoMissing);
    }

    // 3. 选择第一首匹配的歌曲
    //    这里简单地取搜索结果列表中的第一个元素。
    //    可以根据需求添加更复杂的选择逻辑（例如，匹配度最高的、用户选择的等）。
    let selected_song: Song = songs.first().unwrap().clone(); // .unwrap() 在这里是安全的，因为上面已检查 songs 非空
    let song_name_for_log = selected_song.name.clone();
    let artists_for_log = selected_song
        .singer
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join("/");
    log::info!(
        "[QQLyricsFetcher] 自动选择第一首歌: {} - {} (ID: {}, MID: {})",
        song_name_for_log,
        artists_for_log,
        selected_song.id,  // QQ音乐的 ID
        selected_song.mid  // QQ音乐的 songmid，通常用于其他API调用
    );

    // 4. 根据选定歌曲的 ID 获取歌词
    //    qqmusic_api::get_lyrics_by_id 需要歌曲的数字 ID。
    //    它返回一个元组 (可选的QqLyricsResponse, 原始歌词响应文本) 或一个错误。
    match qqmusic_api::get_lyrics_by_id(client, &selected_song.id.to_string())
        .await
        .map_err(|e| {
            // 如果 get_lyrics_by_id 返回错误，进行映射处理
            log::error!(
                "[QQLyricsFetcher] 获取歌词失败 for song ID {}: {}",
                selected_song.id,
                e
            );
            // 根据具体的错误类型从 ConvertError 转换为 QQLyricsFetcherError
            match e {
                ConvertError::RequestRejected => QQLyricsFetcherError::RequestRejected,
                ConvertError::LyricNotFound => QQLyricsFetcherError::LyricNotFoundForSelectedSong,
                _ => QQLyricsFetcherError::ApiError(e), // 其他错误包装为 ApiError
            }
        })? {
        // 如果成功获取到歌词响应 (qrc_resp 可能为 Some 或 None)
        (Some(qrc_resp), _raw_qrc_text) => {
            // 提取主歌词 (QRC格式)
            let main_lyrics_qrc = if !qrc_resp.lyrics.is_empty() {
                Some(qrc_resp.lyrics)
            } else {
                None
            };

            // 提取翻译歌词 (通常是LRC或纯文本)
            let translation_lrc = if !qrc_resp.trans.is_empty() {
                Some(qrc_resp.trans)
            } else {
                None
            };

            // 提取罗马音歌词 (QRC格式)
            let romanization_qrc = if !qrc_resp.roma.is_empty() {
                Some(qrc_resp.roma)
            } else {
                None
            };

            // 提取艺术家名称列表
            let artists_name_vec: Vec<String> = selected_song
                .singer
                .iter()
                .map(|s| s.name.clone())
                .collect();

            // QQ音乐的歌词接口通常不直接返回专辑名，所以这里设为 None。
            // 如果需要专辑名，可能需要通过其他API或从歌曲搜索结果中更深入地提取。
            let album_name: Option<String> = None;

            // 构建并返回 FetchedQqLyrics 结构体
            Ok(FetchedQqLyrics {
                song_id: Some(selected_song.mid.clone()), // 使用 songmid 作为平台的歌曲ID
                song_name: Some(selected_song.name.clone()),
                artists_name: artists_name_vec,
                album_name, // 当前为 None
                main_lyrics_qrc,
                translation_lrc,
                romanization_qrc,
            })
        }
        // 如果 get_lyrics_by_id 返回 (None, ...)，表示API调用成功但未找到歌词内容
        (None, _raw_qrc_text_if_none) => {
            log::warn!(
                "[QQLyricsFetcher] 未找到歌词内容，歌曲ID： {}",
                selected_song.id
            );
            Err(QQLyricsFetcherError::LyricNotFoundForSelectedSong)
        }
    }
}
