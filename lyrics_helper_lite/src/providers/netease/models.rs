use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SearchResultData {
    pub songs: Vec<SongItem>,
}

#[derive(Debug, Deserialize)]
pub struct SongItem {
    pub id: u64,
    pub name: String,
    #[serde(rename = "ar")]
    pub artists: Vec<Artist>,
    #[serde(rename = "al")]
    pub album: Album,
    #[serde(rename = "dt")]
    pub duration: u64, // milliseconds
}

#[derive(Debug, Deserialize)]
pub struct Artist {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct Album {
    pub id: u64,
    pub name: String,
    #[serde(rename = "picUrl")]
    pub pic_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LyricApiResponse {
    pub lrc: Option<LyricData>,
    pub tlyric: Option<LyricData>,
    pub romalrc: Option<LyricData>,
    pub yrc: Option<LyricData>,
}

#[derive(Debug, Deserialize)]
pub struct LyricData {
    pub lyric: Option<String>,
}
