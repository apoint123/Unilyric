use super::protocol_strings::NullString;
use binrw::{BinRead, BinWrite, binrw};
use std::io::Cursor;

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, PartialEq)]
pub struct Artist {
    pub id: NullString,
    pub name: NullString,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, PartialEq)]
pub struct LyricWord {
    pub start_time: u64,
    pub end_time: u64,
    pub word: NullString,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LyricLine {
    pub start_time: u64,
    pub end_time: u64,
    #[bw(try_calc = u32::try_from(words.len()))]
    word_count: u32,
    #[br(count = word_count)]
    pub words: Vec<LyricWord>,
    pub translated_lyric: NullString,
    pub roman_lyric: NullString,
    #[bw(calc = *is_bg as u8 | ((*is_duet as u8) << 1))]
    flags: u8,
    #[br(calc = flags & 0b01 != 0)]
    #[bw(ignore)]
    pub is_bg: bool,
    #[br(calc = flags & 0b10 != 0)]
    #[bw(ignore)]
    pub is_duet: bool,
}

/// 本程序发送的消息
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, PartialEq)]
pub enum ClientMessage {
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

/// 从服务器接收的消息
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
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

impl ClientMessage {
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

impl ServerMessage {
    pub fn decode(bytes: &[u8]) -> binrw::BinResult<Self> {
        let mut reader = Cursor::new(bytes);
        Self::read_le(&mut reader)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_message_encoding() {
        let ping = ClientMessage::Ping;
        let encoded_ping = ping.encode().unwrap();
        assert_eq!(encoded_ping, vec![0x00, 0x00]);

        let pong = ServerMessage::Pong;
        let encoded_pong_bytes = vec![0x01, 0x00];
        let decoded_pong = ServerMessage::decode(&encoded_pong_bytes).unwrap();
        assert_eq!(pong, decoded_pong);
    }

    #[test]
    fn test_set_music_info_roundtrip() {
        let original_message = ClientMessage::SetMusicInfo {
            music_id: "id123".into(),
            music_name: "歌曲名".into(),
            album_id: "album456".into(),
            album_name: "专辑名".into(),
            artists: vec![Artist {
                id: "artist789".into(),
                name: "歌手名".into(),
            }],
            duration: 180000,
        };

        let encoded_bytes = original_message.encode().unwrap();
        let decoded_message = ClientMessage::_decode(&encoded_bytes).unwrap();

        assert_eq!(original_message, decoded_message);
    }

    #[test]
    fn test_set_lyric_roundtrip() {
        let original_message = ClientMessage::SetLyric {
            data: vec![LyricLine {
                start_time: 1000,
                end_time: 5000,
                words: vec![
                    LyricWord {
                        start_time: 1000,
                        end_time: 2000,
                        word: "Hello".into(),
                    },
                    LyricWord {
                        start_time: 2000,
                        end_time: 4000,
                        word: "World".into(),
                    },
                ],
                translated_lyric: "你好世界".into(),
                roman_lyric: "Konnichiwa Sekai".into(),
                is_bg: false,
                is_duet: true,
            }],
        };

        let encoded = original_message.encode().unwrap();
        let decoded = ClientMessage::_decode(&encoded).unwrap();

        assert_eq!(original_message, decoded);
    }

    #[test]
    fn test_seek_command_decoding() {
        let seek_bytes: Vec<u8> = vec![0x11, 0x00, 0x30, 0x75, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

        let expected_message = ServerMessage::SeekPlayProgress { progress: 30000 };
        let decoded_message = ServerMessage::decode(&seek_bytes).unwrap();

        assert_eq!(expected_message, decoded_message);
    }
}
