//! # TTML 解析器 - 常量定义
//!
//! 该模块包含了在解析 TTML 文件时用到的所有 XML 标签和属性的常量定义。

pub(super) const TAG_TT: &[u8] = b"tt";
pub(super) const TAG_METADATA: &[u8] = b"metadata";
pub(super) const TAG_BODY: &[u8] = b"body";
pub(super) const TAG_DIV: &[u8] = b"div";
pub(super) const TAG_P: &[u8] = b"p";
pub(super) const TAG_SPAN: &[u8] = b"span";
pub(super) const TAG_BR: &[u8] = b"br";

pub(super) const TAG_AGENT: &[u8] = b"agent";
pub(super) const TAG_AGENT_TTM: &[u8] = b"ttm:agent";
pub(super) const TAG_NAME: &[u8] = b"name";
pub(super) const TAG_NAME_TTM: &[u8] = b"ttm:name";
pub(super) const TAG_META: &[u8] = b"meta";
pub(super) const TAG_META_AMLL: &[u8] = b"amll:meta";
pub(super) const TAG_ITUNES_METADATA: &[u8] = b"iTunesMetadata";
pub(super) const TAG_SONGWRITER: &[u8] = b"songwriter";
pub(super) const TAG_TRANSLATIONS: &[u8] = b"translations";
pub(super) const TAG_TRANSLITERATIONS: &[u8] = b"transliterations";
pub(super) const TAG_TRANSLATION: &[u8] = b"translation";
pub(super) const TAG_TRANSLITERATION: &[u8] = b"transliteration";
pub(super) const TAG_TEXT: &[u8] = b"text";

pub(super) const ATTR_ITUNES_TIMING: &[u8] = b"itunes:timing";
pub(super) const ATTR_XML_LANG: &[u8] = b"xml:lang";
pub(super) const ATTR_ITUNES_SONG_PART: &[u8] = b"itunes:song-part";
pub(super) const ATTR_ITUNES_SONG_PART_NEW: &[u8] = b"itunes:songPart";
pub(super) const ATTR_BEGIN: &[u8] = b"begin";
pub(super) const ATTR_END: &[u8] = b"end";
pub(super) const ATTR_AGENT: &[u8] = b"ttm:agent";
pub(super) const ATTR_AGENT_ALIAS: &[u8] = b"agent";
pub(super) const ATTR_ITUNES_KEY: &[u8] = b"itunes:key";
pub(super) const ATTR_ROLE: &[u8] = b"ttm:role";
pub(super) const ATTR_ROLE_ALIAS: &[u8] = b"role";
pub(super) const ATTR_XML_SCHEME: &[u8] = b"xml:scheme";
pub(super) const ATTR_XML_ID: &[u8] = b"xml:id";
pub(super) const ATTR_KEY: &[u8] = b"key";
pub(super) const ATTR_VALUE: &[u8] = b"value";
pub(super) const ATTR_FOR: &[u8] = b"for";

pub(super) const ROLE_TRANSLATION: &[u8] = b"x-translation";
pub(super) const ROLE_ROMANIZATION: &[u8] = b"x-roman";
pub(super) const ROLE_BACKGROUND: &[u8] = b"x-bg";
