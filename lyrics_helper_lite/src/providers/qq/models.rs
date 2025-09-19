use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SongData {
    pub list: Vec<SongItem>,
}

#[derive(Debug, Deserialize)]
pub struct SongItem {
    pub id: u64,
    pub mid: String,
    pub name: String,
    pub singer: Vec<Artist>,
    pub album: Album,
    pub interval: u64, // seconds
}

#[derive(Debug, Deserialize)]
pub struct Artist {
    pub mid: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct Album {
    pub mid: String,
    pub name: String,
}
