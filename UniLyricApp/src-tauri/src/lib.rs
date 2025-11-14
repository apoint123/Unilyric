use lyrics_helper_core::{LyricLine, SearchResult, Track};
use lyrics_helper_rs::LyricsHelper;
use serde::Serialize;
use tauri::{Manager, State};
use tokio::sync::Mutex;

struct AppState {
    helper: Mutex<LyricsHelper>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SimpleLyricLine {
    start_ms: u64,
    main_text: Option<String>,
    translation_text: Option<String>,
    romanization_text: Option<String>,
    agent: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewableLyrics {
    lines: Vec<SimpleLyricLine>,
    raw_text: String,
    available_translations: Vec<String>,
    available_romanizations: Vec<String>,
}

#[tauri::command]
async fn search_track(
    title: Option<String>,
    artists: Option<Vec<String>>,
    album: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let artists_vec: Option<Vec<&str>> = artists
        .as_ref()
        .map(|a| a.iter().map(String::as_str).collect());
    let artists_slice: Option<&[&str]> = artists_vec.as_deref();

    let track_meta = Track {
        title: title.as_deref(),
        artists: artists_slice,
        album: album.as_deref(),
        duration: None,
    };

    let helper = state.helper.lock().await;
    helper
        .search_track(&track_meta)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_full_lyrics(
    provider_name: &str,
    song_id: &str,
    state: State<'_, AppState>,
) -> Result<ViewableLyrics, String> {
    let helper = state.helper.lock().await;
    let lyrics_future = helper
        .get_full_lyrics(provider_name, song_id)
        .map_err(|e| e.to_string())?;

    let full_lyrics_result = lyrics_future.await.map_err(|e| e.to_string())?;

    let parsed_data = &full_lyrics_result.parsed;
    let simple_lines: Vec<SimpleLyricLine> = parsed_data
        .lines
        .iter()
        .map(|line: &LyricLine| {
            let main_text = line.main_text();

            let translation_text = line
                .main_track()
                .and_then(|t| t.translations.first())
                .map(|track| track.text());

            let romanization_text = line
                .main_track()
                .and_then(|t| t.romanizations.first())
                .map(|track| track.text());

            SimpleLyricLine {
                start_ms: line.start_ms,
                main_text,
                translation_text,
                romanization_text,
                agent: line.agent.clone(),
            }
        })
        .collect();

    Ok(ViewableLyrics {
        lines: simple_lines,
        raw_text: full_lyrics_result.raw.content,
        available_translations: vec![],
        available_romanizations: vec![],
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let helper = tauri::async_runtime::block_on(async {
                let mut helper = LyricsHelper::new();

                if let Err(e) = helper.load_providers().await {
                    panic!("Failed to load lyrics providers: {}", e);
                }

                helper
            });

            app.manage(AppState {
                helper: Mutex::new(helper),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![search_track, get_full_lyrics])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
