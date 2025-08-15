//! 酷狗歌词解密工具模块。
//! 该模块包含用于解密酷狗音乐歌词的函数。
//!
//! ## 致谢
//!
//! 本模块的解密逻辑（包括固定的16字节密钥和异或算法）源于
//! `LyricDecoder` 项目。
//!
//! - Copyright (c) `SuJiKiNen` (`LyricDecoder` Project)
//! - Licensed under the MIT License.
//!
//! <https://github.com/SuJiKiNen/LyricDecoder>

use std::io::Read;

use base64::{Engine as _, engine::general_purpose};
use flate2::read::ZlibDecoder;

use crate::error::{LyricsHelperError, Result};

/// 酷狗 KRC 歌词解密所使用的固定16字节密钥。
const KRC_DECRYPT_KEY: [u8; 16] = [
    0x40, 0x47, 0x61, 0x77, 0x5E, 0x32, 0x74, 0x47, 0x51, 0x36, 0x31, 0x2D, 0xCE, 0xD2, 0x6E, 0x69,
];

/// 解密酷狗音乐的 KRC 格式歌词。
///
/// # 参数
///
/// * `encrypted_krc_base64` - 一个经过 Base64 编码的加密 KRC 文本字符串。
///
/// # 返回
///
/// * `Result<String>` - 成功时返回解密后的 KRC 文本（通常是 KRC 格式的字符串）。
///
/// # 错误
///
/// * `LyricsHelperError::Base64Decode` - 如果输入的字符串不是有效的 Base64。
/// * `LyricsHelperError::Decryption` - 如果输入的数据长度不足。
/// * `LyricsHelperError::Io` - 如果 Zlib 解压缩失败。
/// * `LyricsHelperError` (From `FromUtf8Error`) - 如果解密后的数据不是有效的 UTF-8 编码。
pub fn decrypt_krc(encrypted_krc_base64: &str) -> Result<String> {
    // Base64 解码
    let mut data = general_purpose::STANDARD
        .decode(encrypted_krc_base64.as_bytes())
        .map_err(LyricsHelperError::Base64Decode)?;

    if data.len() < 4 {
        return Err(LyricsHelperError::Decryption(
            "KRC 加密数据过短，至少需要4字节的头部。".into(),
        ));
    }

    // 移除前4个字节的头部
    let data_to_decrypt = &mut data[4..];

    // 异或
    for (i, byte) in data_to_decrypt.iter_mut().enumerate() {
        *byte ^= KRC_DECRYPT_KEY[i % KRC_DECRYPT_KEY.len()];
    }

    // Zlib 解压缩
    let mut decoder = ZlibDecoder::new(&*data_to_decrypt);
    let mut decompressed_data = Vec::new();
    decoder
        .read_to_end(&mut decompressed_data)
        .map_err(LyricsHelperError::Io)?;

    // 转换为 UTF-8 字符串
    String::from_utf8(decompressed_data).map_err(LyricsHelperError::from)
}
