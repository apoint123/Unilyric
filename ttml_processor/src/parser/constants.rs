//! # TTML 解析器 - 常量定义
//!
//! 该模块包含了在解析 TTML 文件时用到的所有 XML 标签和属性的常量定义。

pub const TAG_TT: &[u8] = b"tt";
pub const TAG_METADATA: &[u8] = b"metadata";
pub const TAG_BODY: &[u8] = b"body";
pub const TAG_DIV: &[u8] = b"div";
pub const TAG_P: &[u8] = b"p";
pub const TAG_SPAN: &[u8] = b"span";
pub const TAG_BR: &[u8] = b"br";

pub const TAG_AGENT: &[u8] = b"agent";
pub const TAG_AGENT_TTM: &[u8] = b"ttm:agent";
pub const TAG_NAME: &[u8] = b"name";
pub const TAG_NAME_TTM: &[u8] = b"ttm:name";
pub const TAG_META: &[u8] = b"meta";
pub const TAG_META_AMLL: &[u8] = b"amll:meta";
pub const TAG_ITUNES_METADATA: &[u8] = b"iTunesMetadata";
pub const TAG_SONGWRITER: &[u8] = b"songwriter";
pub const TAG_TRANSLATIONS: &[u8] = b"translations";
pub const TAG_TRANSLITERATIONS: &[u8] = b"transliterations";
pub const TAG_TRANSLATION: &[u8] = b"translation";
pub const TAG_TRANSLITERATION: &[u8] = b"transliteration";
pub const TAG_TEXT: &[u8] = b"text";
pub const TAG_AUDIO: &[u8] = b"audio";

pub const ATTR_ITUNES_TIMING: &[u8] = b"itunes:timing";
pub const ATTR_XML_LANG: &[u8] = b"xml:lang";
pub const ATTR_ITUNES_SONG_PART: &[u8] = b"itunes:song-part";
pub const ATTR_ITUNES_SONG_PART_NEW: &[u8] = b"itunes:songPart";
pub const ATTR_BEGIN: &[u8] = b"begin";
pub const ATTR_END: &[u8] = b"end";
pub const ATTR_AGENT: &[u8] = b"ttm:agent";
pub const ATTR_AGENT_ALIAS: &[u8] = b"agent";
pub const ATTR_ITUNES_KEY: &[u8] = b"itunes:key";
pub const ATTR_ROLE: &[u8] = b"ttm:role";
pub const ATTR_ROLE_ALIAS: &[u8] = b"role";
pub const ATTR_XML_SCHEME: &[u8] = b"xml:scheme";
pub const ATTR_XML_ID: &[u8] = b"xml:id";
pub const ATTR_KEY: &[u8] = b"key";
pub const ATTR_VALUE: &[u8] = b"value";
pub const ATTR_FOR: &[u8] = b"for";
pub const ATTR_LYRIC_OFFSET: &[u8] = b"lyricOffset";

pub const ROLE_TRANSLATION: &[u8] = b"x-translation";
pub const ROLE_ROMANIZATION: &[u8] = b"x-roman";
pub const ROLE_BACKGROUND: &[u8] = b"x-bg";
