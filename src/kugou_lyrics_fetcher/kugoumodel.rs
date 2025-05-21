use serde::Deserialize;

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct SearchSongResponse {
    #[serde(alias = "Status")]
    pub status: i32,
    #[serde(alias = "error", default)]
    pub error: Option<String>,
    #[serde(alias = "errcode", default)]
    pub error_code: i32,
    #[serde(alias = "data")]
    pub song_data: Option<SongDataItem>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct SongDataItem {
    #[serde(alias = "timestamp")]
    pub timestamp: Option<i64>,
    #[serde(alias = "total")]
    pub total: i32,
    #[serde(alias = "info")]
    pub info: Vec<SongInfoItem>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct SongInfoItem {
    #[serde(alias = "hash", alias = "Hash")]
    pub hash: String,
    #[serde(alias = "songname")]
    pub song_name: String,
    #[serde(alias = "album_name", default)]
    pub album_name: Option<String>,
    #[serde(alias = "songname_original", default)]
    pub song_name_original: Option<String>,
    #[serde(alias = "singername")]
    pub singer_name: String,
    #[serde(alias = "duration")]
    pub duration: i32,
    #[serde(alias = "filename")]
    pub filename: String,
    #[serde(alias = "group", default)]
    pub group: Vec<SongInfoItem>,
    #[serde(alias = "FileHash", default)]
    pub file_hash: Option<String>,
    #[serde(alias = "HQFileHash", default)]
    pub hq_file_hash: Option<String>,
    #[serde(alias = "SQFileHash", default)]
    pub sq_file_hash: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct SearchLyricsResponse {
    pub status: i32,
    #[serde(alias = "errcode", default)]
    pub error_code: i32,
    #[serde(alias = "errmsg", default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub proposal: Option<String>,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct Candidate {
    #[serde(alias = "Id")]
    pub id: String,
    #[serde(alias = "accesskey")]
    pub access_key: String,
    pub singer: Option<String>,
    pub song: Option<String>,
    pub duration: Option<i32>,
    pub language: Option<String>,
    #[serde(alias = "krctype")]
    pub krc_type: Option<i32>,
    pub score: Option<i32>,
    pub nickname: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct KugouLyricsDownloadResponse {
    pub status: i32,
    #[serde(alias = "error_code", default)]
    pub error_code: i32,
    pub content: Option<String>,
    pub info: Option<String>,
    #[serde(alias = "fmt")]
    pub format: Option<String>,
    #[serde(alias = "contenttype")]
    pub content_type: Option<i32>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct KugouTranslation {
    #[serde(default)]
    pub version: i32,
    #[serde(default)]
    pub content: Vec<TranslationContentItem>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct TranslationContentItem {
    pub language: i32,
    #[serde(alias = "type")]
    pub item_type: i32,
    #[serde(alias = "lyricContent", default)]
    pub lyric_content: Vec<Vec<String>>,
}
