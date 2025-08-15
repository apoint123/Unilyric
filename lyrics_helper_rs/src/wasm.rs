// 随便写的，只保证基本功能正常

use crate::{
    LyricsHelper, SearchMode,
    converter::types::{ConversionInput, ConversionOptions},
    model::track::Track,
};
use serde::Deserialize;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn main_js() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    tracing_wasm::set_as_global_default();
    Ok(())
}

#[wasm_bindgen]
pub struct WasmLyricsHelper {
    helper: LyricsHelper,
}

#[wasm_bindgen]
impl WasmLyricsHelper {
    pub async fn new() -> Result<WasmLyricsHelper, JsValue> {
        let helper = LyricsHelper::new()
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(Self { helper })
    }

    #[wasm_bindgen(js_name = searchLyrics)]
    pub async fn search_lyrics(
        &self,
        track_meta_js: JsValue,
        mode_js: JsValue,
    ) -> Result<JsValue, JsValue> {
        #[derive(Deserialize)]
        struct OwnedTrack {
            title: Option<String>,
            artists: Option<Vec<String>>,
            album: Option<String>,
        }

        let owned_track: OwnedTrack = serde_wasm_bindgen::from_value(track_meta_js)?;

        let artists_vec: Vec<&str> = owned_track.artists.as_ref().map_or(Vec::new(), |artists| {
            artists.iter().map(String::as_str).collect()
        });

        let track_to_search = Track {
            title: owned_track.title.as_deref(),
            artists: if artists_vec.is_empty() {
                None
            } else {
                Some(&artists_vec)
            },
            album: owned_track.album.as_deref(),
        };

        let mode_str: String = serde_wasm_bindgen::from_value(mode_js)?;
        let mode = match mode_str.as_str() {
            "Ordered" => SearchMode::Ordered,
            "Parallel" => SearchMode::Parallel,
            _ => SearchMode::Specific(mode_str),
        };

        let result = self
            .helper
            .search_lyrics(&track_to_search, mode)
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        Ok(serde_wasm_bindgen::to_value(&result)?)
    }

    #[wasm_bindgen(js_name = convertLyrics)]
    pub async fn convert_lyrics(
        &self,
        input_js: JsValue,
        options_js: JsValue,
    ) -> Result<String, JsValue> {
        let input: ConversionInput = serde_wasm_bindgen::from_value(input_js)?;
        let options: ConversionOptions = serde_wasm_bindgen::from_value(options_js)?;

        let result = crate::converter::convert_single_lyric(&input, &options)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        Ok(result)
    }
}
