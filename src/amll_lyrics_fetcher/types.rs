use serde::Deserialize;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum AmllSearchField {
    #[default]
    MusicName,
    Artists,
    Album,
    NcmMusicId,
    QqMusicId,
    SpotifyId,
    AppleMusicId,
    Isrc,
    TtmlAuthorGithub,
    TtmlAuthorGithubLogin,
}

impl AmllSearchField {
    pub fn to_key_string(&self) -> &str {
        match self {
            AmllSearchField::MusicName => "musicName",
            AmllSearchField::Artists => "artists",
            AmllSearchField::Album => "album",
            AmllSearchField::NcmMusicId => "ncmMusicId",
            AmllSearchField::QqMusicId => "qqMusicId",
            AmllSearchField::SpotifyId => "spotifyId",
            AmllSearchField::AppleMusicId => "appleMusicId",
            AmllSearchField::Isrc => "isrc",
            AmllSearchField::TtmlAuthorGithub => "ttmlAuthorGithub",
            AmllSearchField::TtmlAuthorGithubLogin => "ttmlAuthorGithubLogin",
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            AmllSearchField::MusicName => "歌曲名称",
            AmllSearchField::Artists => "艺术家",
            AmllSearchField::Album => "专辑名",
            AmllSearchField::NcmMusicId => "网易云音乐 ID",
            AmllSearchField::QqMusicId => "QQ音乐 ID",
            AmllSearchField::SpotifyId => "Spotify音乐 ID",
            AmllSearchField::AppleMusicId => "Apple Music 音乐 ID",
            AmllSearchField::Isrc => "ISRC 号码",
            AmllSearchField::TtmlAuthorGithub => "逐词歌词作者 GitHub ID",
            AmllSearchField::TtmlAuthorGithubLogin => "逐词歌词作者 GitHub 用户名",
        }
    }

    pub fn all_fields() -> Vec<Self> {
        vec![
            Self::MusicName,
            Self::Artists,
            Self::Album,
            Self::AppleMusicId,
            Self::NcmMusicId,
            Self::QqMusicId,
            Self::SpotifyId,
            Self::Isrc,
            Self::TtmlAuthorGithub,
            Self::TtmlAuthorGithubLogin,
        ]
    }
}

impl fmt::Display for AmllSearchField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AmllIndexEntry {
    pub metadata: Vec<(String, Vec<String>)>,
    #[serde(rename = "rawLyricFile")]
    pub raw_lyric_file: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FetchedAmllTtmlLyrics {
    pub song_name: Option<String>,
    pub artists_name: Vec<String>,
    pub album_name: Option<String>,
    pub ttml_content: String,
    pub source_id: Option<String>,
    pub all_metadata_from_index: Vec<(String, Vec<String>)>,
}
