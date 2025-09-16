use std::env;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use lyrics_helper_rs::http::WreqClient;
use lyrics_helper_rs::model::auth::{LoginEvent, LoginMethod};
use lyrics_helper_rs::providers::LoginProvider;
use lyrics_helper_rs::providers::Provider;
use lyrics_helper_rs::providers::netease::NeteaseClient;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> Result<()> {
    let music_u = env::var("NETEASE_MUSIC_U").expect("请设置 NETEASE_MUSIC_U 环境变量");
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("用法: netease_playlist_downloader <playlist_id>");
        return Ok(());
    }
    let playlist_id = &args[1];

    let http_client = Arc::new(WreqClient::new()?);
    let provider = NeteaseClient::with_http_client(http_client).await?;

    let login_method = LoginMethod::NeteaseByCookie { music_u };
    let mut flow = provider.initiate_login(login_method);

    loop {
        match flow.events.next().await {
            Some(LoginEvent::Success(result)) => {
                println!("登录成功！用户: {}", result.profile.nickname);
                break;
            }
            Some(LoginEvent::Failure(e)) => {
                anyhow::bail!("登录失败: {}", e);
            }
            Some(event) => {
                println!("收到事件: {event:?}");
            }
            None => {
                anyhow::bail!("登录流程意外终止");
            }
        }
    }

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
                        println!("歌曲 {file_name} 已存在，跳过下载。");
                        continue;
                    }

                    println!("  获取到下载链接，开始下载...");
                    let response = reqwest::get(&url).await?;
                    let content = response.bytes().await?;
                    tokio::fs::write(path, &content).await?;
                    println!("  成功下载并保存为 {file_name}");
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
