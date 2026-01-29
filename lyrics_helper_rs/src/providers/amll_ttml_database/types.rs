use serde::{Deserialize, Deserializer};

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub enum DedupKey {
    Ncm(String),
    Qq(String),
    Apple(String),
    Spotify(String),
    RawFile(String),
}

#[derive(Debug, Clone, Default)]
pub struct Metadata {
    pub titles: Vec<String>,
    pub artists: Vec<String>,
    pub albums: Vec<String>,

    pub ncm_music_ids: Vec<String>,
    pub qq_music_ids: Vec<String>,
    pub apple_music_ids: Vec<String>,
    pub spotify_music_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RawLyricFileInfo {
    pub filename: String,
    pub timestamp: u64,
}

impl<'de> Deserialize<'de> for RawLyricFileInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let filename = String::deserialize(deserializer)?;
        let timestamp = filename
            .split('-')
            .next()
            .unwrap_or("0")
            .parse::<u64>()
            .unwrap_or(0);

        Ok(Self {
            filename,
            timestamp,
        })
    }
}

fn deserialize_metadata<'de, D>(deserializer: D) -> Result<Metadata, D::Error>
where
    D: Deserializer<'de>,
{
    let raw: Vec<(String, Vec<String>)> = Deserialize::deserialize(deserializer)?;
    let mut meta = Metadata::default();

    for (key, mut val) in raw {
        match key.as_str() {
            "musicName" => meta.titles = val,
            "artists" => meta.artists = val,
            "album" => meta.albums = val,
            "ncmMusicId" => {
                val.sort();
                meta.ncm_music_ids = val;
            }
            "qqMusicId" => {
                val.sort();
                meta.qq_music_ids = val;
            }
            "appleMusicId" => {
                val.sort();
                meta.apple_music_ids = val;
            }
            "spotifyId" => {
                val.sort();
                meta.spotify_music_ids = val;
            }
            _ => {}
        }
    }
    Ok(meta)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexEntry {
    #[serde(deserialize_with = "deserialize_metadata")]
    pub metadata: Metadata,
    pub raw_lyric_file: RawLyricFileInfo,
}

impl IndexEntry {
    pub fn get_dedup_key(&self) -> DedupKey {
        if let Some(id) = self.metadata.ncm_music_ids.first() {
            return DedupKey::Ncm(id.clone());
        }
        if let Some(id) = self.metadata.qq_music_ids.first() {
            return DedupKey::Qq(id.clone());
        }
        if let Some(id) = self.metadata.apple_music_ids.first() {
            return DedupKey::Apple(id.clone());
        }
        if let Some(id) = self.metadata.spotify_music_ids.first() {
            return DedupKey::Spotify(id.clone());
        }
        DedupKey::RawFile(self.raw_lyric_file.filename.clone())
    }
}

#[derive(Debug, Deserialize)]
pub struct GitHubErrorResponse {
    pub message: String,
}
