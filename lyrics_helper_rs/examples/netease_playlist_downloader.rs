use std::env;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use lyrics_helper_rs::http::reqwest_client::ReqwestClient;
use lyrics_helper_rs::providers::Provider;
use lyrics_helper_rs::providers::netease::NeteaseClient;

#[tokio::main]
async fn main() -> Result<()> {
    let cookie = env::var("NETEASE_COOKIE").expect("请设置 NETEASE_COOKIE 环境变量");
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("用法: netease_playlist_downloader <playlist_id>");
        return Ok(());
    }
    let playlist_id = &args[1];

    let http_client = Arc::new(ReqwestClient::new()?);
    let provider = NeteaseClient::with_http_client(http_client)
        .await?
        .with_cookie(cookie);

    println!("正在获取歌单信息...");
    let playlist = provider.get_playlist(playlist_id).await?;
    println!("成功获取歌单: {}", playlist.name);

    if let Some(songs) = playlist.songs {
        println!("歌单包含 {} 首歌曲，开始下载...", songs.len());

        for song in songs {
            let artists_name = song
                .artists
                .iter()
                .map(|a| a.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            println!("正在处理歌曲: {} - {}", song.name, artists_name);
            match provider.get_song_link_v1(&song.id).await {
                Ok(url) => {
                    let raw_file_name = format!("{} - {}.flac", song.name, artists_name);
                    let file_name = raw_file_name
                        .replace(&['<', '>', ':', '"', '/', '\\', '|', '?', '*'][..], "_");
                    let path = Path::new(&file_name);
                    if path.exists() {
                        println!("歌曲 {} 已存在，跳过下载。", file_name);
                        continue;
                    }

                    println!("  获取到下载链接，开始下载...");
                    let response = reqwest::get(&url).await?;
                    let content = response.bytes().await?;
                    tokio::fs::write(path, &content).await?;
                    println!("  成功下载并保存为 {}", file_name);
                }
                Err(e) => {
                    println!("  获取歌曲 {} 的下载链接失败: {}", song.name, e);
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    } else {
        println!("歌单中没有歌曲。");
    }

    Ok(())
}
