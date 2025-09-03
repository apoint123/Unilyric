//! 用于演示 `lyrics-helper` 库的核心功能。
//!
//! ## 如何运行
//!
//! ```bash
//! cargo run --package lyrics_helper_rs --example demo
//! ```

use std::fs;
use std::io::{self, Write};

use lyrics_helper_core::{MetadataStore, SearchResult, Track, TtmlGenerationOptions};
use lyrics_helper_rs::LyricsHelper;
use lyrics_helper_rs::converter::processors::agent_recognizer;
use lyrics_helper_rs::error::Result;

use tracing::{Level, error, info};
use ttml_processor::generate_ttml;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    info!("正在初始化...");
    let mut helper = LyricsHelper::new();
    helper.load_providers().await?;

    // 这里硬编码了一首歌作为示例。在实际应用中，这些信息来自用户输入或文件元数据。
    let track_to_search = Track {
        title: Some("有点甜"),
        artists: Some(&["汪苏泷", "BY2"]),
        album: Some("万有引力"),
        duration: Some(235_000),
    };
    info!(
        "准备搜索歌曲: '{}' - '{}'",
        track_to_search.title.unwrap_or_default(),
        track_to_search.artists.unwrap_or_default().join(", ")
    );

    let search_results = helper.search_track(&track_to_search).await?;

    if search_results.is_empty() {
        error!("在所有提供商中均未找到相关的歌词，程序退出。");
        return Ok(());
    }

    let chosen_index = prompt_user_for_selection(&search_results)?;

    let selected_result = &search_results[chosen_index];
    info!(
        "选择了来自 '{}' 的歌词 '{}'。正在获取歌词内容...",
        selected_result.provider_name, selected_result.title
    );
    let song_id_for_lyrics = selected_result
        .provider_id_num
        .map_or_else(|| selected_result.provider_id.clone(), |id| id.to_string());
    let mut parsed_lyrics_data = helper
        .get_full_lyrics(&selected_result.provider_name, &song_id_for_lyrics)?
        .await?;

    agent_recognizer::recognize_agents(&mut parsed_lyrics_data.parsed.lines);
    info!("歌词获取并解析成功！");

    // 从解析的歌词文件内部创建基础元数据存储。
    let mut metadata_store = MetadataStore::from(&parsed_lyrics_data.parsed);

    // 使用从 API 获取的 `SearchResult` 信息来覆盖元数据。
    info!("设置标题: {}", &selected_result.title);
    metadata_store.set_single("title", &selected_result.title.clone());

    info!("设置艺术家: {:?}", &selected_result.artists);
    metadata_store.set_multiple(
        "artist",
        selected_result
            .artists
            .iter()
            .map(|a| a.name.clone())
            .collect(),
    );

    if let Some(album) = &selected_result.album {
        info!("设置专辑: {}", album);
        metadata_store.set_single("album", &album.clone());
    }

    info!("正在将歌词转换为 TTML 格式...");

    let agent_store = MetadataStore::to_agent_store(&metadata_store);

    let ttml_options = TtmlGenerationOptions::default();
    let ttml_output = generate_ttml(
        &parsed_lyrics_data.parsed.lines,
        &metadata_store,
        &agent_store,
        &ttml_options,
    )?;

    let output_filename = "lyrics.ttml";
    fs::write(output_filename, &ttml_output)?;

    info!("转换成功！TTML 歌词已保存到文件: {}", output_filename);
    Ok(())
}

/// 将搜索结果打印到控制台，并提示用户进行选择。
fn prompt_user_for_selection(search_results: &[SearchResult]) -> Result<usize> {
    println!(
        "找到了 {} 条歌词，请选择一个进行转换：\n",
        search_results.len()
    );

    for (index, result) in search_results.iter().enumerate() {
        let artists_display = result
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  [{:2}] 标题: {} | 艺术家: {}",
            index + 1,
            result.title,
            artists_display
        );

        let album_display = result.album.as_deref().unwrap_or("N/A");
        let album_id_display = result.album_id.as_deref().unwrap_or("N/A");
        println!("       专辑: {album_display} (ID: {album_id_display})");

        let duration_ms = result.duration.unwrap_or(0);
        let duration_formatted = if duration_ms > 0 {
            let total_secs = duration_ms / 1000;
            format!("{:02}:{:02}", total_secs / 60, total_secs % 60)
        } else {
            "N/A".to_string()
        };
        let language_display = result
            .language
            .as_ref()
            .map_or_else(|| "N/A".to_string(), |l| format!("{l:?}"));
        println!(
            "       来源: {:<10} | 匹配度: {:<15} | 时长: {} | 语言: {}",
            result.provider_name,
            format!("{:?}", result.match_type),
            duration_formatted,
            language_display
        );

        let numeric_id_display = result
            .provider_id_num
            .map_or_else(|| "N/A".to_string(), |id| id.to_string());
        println!(
            "       ID:   Provider ID: {} | 数字 ID: {}",
            result.provider_id, numeric_id_display
        );

        let cover_url_display = result.cover_url.as_deref().unwrap_or("N/A");
        println!("       封面: {cover_url_display}");

        if index < search_results.len() - 1 {
            println!();
        }
    }

    loop {
        print!(
            "\n请输入你想下载并转换的歌词编号 (1-{}): ",
            search_results.len()
        );
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim().parse::<usize>() {
            Ok(num) if num > 0 && num <= search_results.len() => {
                break Ok(num - 1);
            }
            _ => {
                eprintln!("\n输入无效，请输入一个列表中的有效编号。\n");
            }
        }
    }
}
