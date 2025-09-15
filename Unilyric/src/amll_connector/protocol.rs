use super::protocol_strings::NullString;
use binrw::{BinRead, BinWrite, binrw};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

#[binrw]
#[brw(little)]
#[derive(Deserialize, Serialize, PartialEq, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    pub id: NullString,
    pub name: NullString,
}

#[binrw]
#[brw(little)]
#[derive(Deserialize, Serialize, PartialEq, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct LyricWord {
    pub start_time: u64,
    pub end_time: u64,
    pub word: NullString,
    #[brw(ignore)]
    #[serde(default)]
    pub roman_word: NullString,
}

#[binrw]
#[brw(little)]
#[derive(Deserialize, Serialize, PartialEq, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub start_time: u64,
    pub end_time: u64,

    #[serde(skip)]
    #[bw(try_calc = u32::try_from(words.len()))]
    word_count: u32,

    #[br(count = word_count)]
    pub words: Vec<LyricWord>,

    pub translated_lyric: NullString,
    pub roman_lyric: NullString,

    #[serde(skip)]
    #[bw(calc = *is_bg as u8 | ((*is_duet as u8) << 1))]
    flags: u8,

    #[br(calc = flags & 0b01 != 0)]
    #[bw(ignore)]
    #[serde(default, rename = "isBG")]
    pub is_bg: bool,

    #[br(calc = flags & 0b10 != 0)]
    #[bw(ignore)]
    pub is_duet: bool,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum ClientMessage {
    InitializeV2,
    Ping,
    Pong,
    #[serde(rename_all = "camelCase")]
    SetMusicInfo {
        music_id: String,
        music_name: String,
        album_id: String,
        album_name: String,
        artists: Vec<Artist>,
        duration: u64,
    },
    OnPlayProgress {
        progress: u64,
    },
    OnPaused,
    OnResumed,
    SetLyric {
        data: Vec<LyricLine>,
    },
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum ServerMessage {
    Ping,
    Pong,
    Pause,
    Resume,
    ForwardSong,
    BackwardSong,
    SetVolume { volume: f64 },
    SeekPlayProgress { progress: u64 },
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, PartialEq)]
pub enum BinClientMessage {
    #[brw(magic(0u16))]
    Ping,
    #[brw(magic(1u16))]
    Pong,
    #[brw(magic(2u16))]
    SetMusicInfo {
        music_id: NullString,
        music_name: NullString,
        album_id: NullString,
        album_name: NullString,
        #[bw(try_calc = u32::try_from(artists.len()))]
        artist_count: u32,
        #[br(count = artist_count)]
        artists: Vec<Artist>,
        duration: u64,
    },
    #[brw(magic(4u16))]
    SetMusicAlbumCoverImageData {
        #[bw(try_calc = u32::try_from(data.len()))]
        size: u32,
        #[br(count = size)]
        data: Vec<u8>,
    },
    #[brw(magic(5u16))]
    OnPlayProgress { progress: u64 },
    #[brw(magic(7u16))]
    OnPaused,
    #[brw(magic(8u16))]
    OnResumed,
    #[brw(magic(9u16))]
    OnAudioData {
        #[bw(try_calc = u32::try_from(data.len()))]
        size: u32,
        #[br(count = size)]
        data: Vec<u8>,
    },
    #[brw(magic(10u16))]
    SetLyric {
        #[bw(try_calc = u32::try_from(data.len()))]
        line_count: u32,
        #[br(count = line_count)]
        data: Vec<LyricLine>,
    },
    #[brw(magic(11u16))]
    SetLyricFromTTML { data: NullString },
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, PartialEq)]
pub enum BinServerMessage {
    #[brw(magic(0u16))]
    Ping,
    #[brw(magic(1u16))]
    Pong,
    #[brw(magic(12u16))]
    Pause,
    #[brw(magic(13u16))]
    Resume,
    #[brw(magic(14u16))]
    ForwardSong,
    #[brw(magic(15u16))]
    BackwardSong,
    #[brw(magic(16u16))]
    SetVolume { volume: f64 },
    #[brw(magic(17u16))]
    SeekPlayProgress { progress: u64 },
}

#[derive(Debug, Clone)]
pub enum OutgoingMessage {
    Json(ClientMessage),
    LegacyBinary(BinClientMessage),
}

impl BinClientMessage {
    pub fn encode(&self) -> binrw::BinResult<Vec<u8>> {
        let mut writer = Cursor::new(Vec::new());
        self.write_le(&mut writer)?;
        Ok(writer.into_inner())
    }

    pub fn _decode(bytes: &[u8]) -> binrw::BinResult<Self> {
        let mut reader = Cursor::new(bytes);
        Self::read_le(&mut reader)
    }
}

impl BinServerMessage {
    pub fn decode(bytes: &[u8]) -> binrw::BinResult<Self> {
        let mut reader = Cursor::new(bytes);
        Self::read_le(&mut reader)
    }
}
