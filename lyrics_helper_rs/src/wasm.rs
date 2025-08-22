use std::str::FromStr;
use std::sync::{Arc, Once};
use tracing::Level;

use crate::http::WasmClient;
use crate::{LyricsHelper, SearchMode};
use lyrics_helper_core::{ConversionInput, ConversionOptions, Track};
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::*;

static INIT: Once = Once::new();

#[wasm_bindgen]
pub struct WasmLyricsHelper {
    helper: LyricsHelper,
}

#[derive(Serialize)]
pub struct ProviderInfo {
    id: String,
    name: String,
}

#[wasm_bindgen]
impl WasmLyricsHelper {
    #[wasm_bindgen(constructor)]
    pub async fn new(
        proxy_url: Option<String>,
        log_level: Option<String>,
    ) -> Result<WasmLyricsHelper, JsValue> {
        INIT.call_once(|| {
            let log_level_str = log_level.as_deref().unwrap_or("info");
            let level = Level::from_str(log_level_str).unwrap_or(Level::INFO);
            let config = tracing_wasm::WASMLayerConfigBuilder::new()
                .set_max_level(level)
                .build();
            tracing_wasm::set_as_global_default_with_config(config);
        });

        let wasm_http_client = Arc::new(WasmClient::new(proxy_url));
        let mut helper = LyricsHelper::new_with_http_client(wasm_http_client);

        helper
            .load_providers()
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        Ok(Self { helper })
    }

    #[wasm_bindgen(js_name = getProviders)]
    pub fn get_providers(&self) -> Result<JsValue, JsValue> {
        let providers = crate::ProviderName::all()
            .iter()
            .map(|p| ProviderInfo {
                id: p.as_str().to_string(),
                name: p.display_name().to_string(),
            })
            .collect::<Vec<_>>();
        Ok(serde_wasm_bindgen::to_value(&providers)?)
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
            duration: Option<u64>,
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
            duration: owned_track.duration,
        };

        let mode: SearchMode;

        if mode_js.is_string() {
            let mode_str: String = serde_wasm_bindgen::from_value(mode_js)?;
            mode = match mode_str.as_str() {
                "Ordered" => SearchMode::Ordered,
                "Parallel" => SearchMode::Parallel,
                provider_str => {
                    if let Some(provider) = crate::ProviderName::try_from_str(provider_str) {
                        SearchMode::Specific(provider)
                    } else {
                        return Err(JsValue::from_str(&format!(
                            "不支持的提供商: {provider_str}"
                        )));
                    }
                }
            };
        } else if mode_js.is_array() {
            let provider_ids: Vec<String> = serde_wasm_bindgen::from_value(mode_js)?;
            let providers: std::result::Result<Vec<_>, _> = provider_ids
                .into_iter()
                .map(|id| crate::ProviderName::from_str(&id))
                .collect();

            match providers {
                Ok(p_vec) => {
                    mode = SearchMode::Subset(p_vec);
                }
                Err(e) => return Err(JsValue::from_str(&e)),
            }
        } else {
            return Err(JsValue::from_str("搜索模式必须是字符串或字符串数组。"));
        }

        let result = self
            .helper
            .search_lyrics(&track_to_search, mode)
            .map_err(|e| JsValue::from_str(&e.to_string()))?
            .await
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        Ok(serde_wasm_bindgen::to_value(&result)?)
    }

    #[wasm_bindgen(js_name = convertLyrics)]
    pub fn convert_lyrics(
        &self,
        input_js: JsValue,
        options_js: JsValue,
    ) -> Result<String, JsValue> {
        let input: ConversionInput = serde_wasm_bindgen::from_value(input_js)?;
        let options: ConversionOptions = serde_wasm_bindgen::from_value(options_js)?;

        let result = crate::converter::convert_single_lyric(&input, &options)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

        Ok(result.output_lyrics)
    }
}
