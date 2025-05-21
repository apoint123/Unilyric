// 导入同级模块 error 中定义的错误类型 KugouError 和 Result 类型别名
use super::error::{KugouError, Result};
// 导入 base64 库的 Engine 特征，用于 Base64 编解码
use base64::Engine;
// 导入 flate2 库的 ZlibDecoder，用于 Zlib 解压缩
use flate2::read::ZlibDecoder;
// 导入标准库的 Read trait，ZlibDecoder 需要它
use std::io::Read;

// 定义用于 KRC 歌词解密的16字节固定密钥。
// 这个密钥是 KRC 解密算法的核心部分。
const DECRYPT_KEY: [u8; 16] = [
    0x40, 0x47, 0x61, 0x77, 0x5E, 0x32, 0x74, 0x47, 0x51, 0x36, 0x31, 0x2D, 0xCE, 0xD2, 0x6E, 0x69,
]; // 对应的ASCII字符大致是 "@Gaw^2tGQ61-ÎÒni" (部分不可打印)

/// 解密 KRC 格式的歌词内容。
///
/// KRC 歌词通常经过加密和压缩。此函数执行以下步骤：
/// 1. Base64 解码输入的字符串。
/// 2. 移除解码后数据头部的4个字节。
/// 3. 对剩余数据使用固定的 `DECRYPT_KEY` 进行逐字节异或解密。
/// 4. 使用 Zlib 解压缩经过异或解密的数据。
/// 5. 将解压缩后的字节数据转换为 UTF-8 字符串。
///
/// # Arguments
/// * `encrypted_lyrics_base64` - Base64 编码的加密 KRC 歌词字符串。
///
/// # Returns
/// `Result<String>` - 如果成功，返回解密和解压缩后的 KRC 歌词文本 (UTF-8 字符串)；
///                    否则返回 `KugouError`。
pub fn decrypt_krc_lyrics(encrypted_lyrics_base64: &str) -> Result<String> {
    // 1. Base64 解码
    // 使用标准的 Base64 引擎解码输入的字符串。
    // `?` 操作符会在解码失败时提前返回 `base64::DecodeError`，该错误会被转换为 `KugouError::Base64`。
    let mut data =
        base64::engine::general_purpose::STANDARD.decode(encrypted_lyrics_base64.as_bytes())?;

    // 2. 移除头部
    // KRC 加密数据的前4个字节通常不参与异或解密，是某种头部信息或校验。
    // `split_off(4)` 会将 `data` 在索引4处分割，`data` 自身保留前4个字节，
    // 返回后半部分（从索引4开始到末尾）作为 `data_to_decrypt`。
    let mut data_to_decrypt = data.split_off(4);

    // 3. 逐字节异或解密
    // 遍历需要解密的字节数据
    for i in 0..data_to_decrypt.len() {
        // 将每个字节与 `DECRYPT_KEY` 中对应的字节进行异或操作。
        // `DECRYPT_KEY` 循环使用 (通过取模运算 `i % DECRYPT_KEY.len()`)。
        data_to_decrypt[i] ^= DECRYPT_KEY[i % DECRYPT_KEY.len()];
    }

    // 4. Zlib 解压缩
    // 创建一个 Zlib 解码器，输入是经过异或解密后的字节数据。
    let mut decoder = ZlibDecoder::new(&data_to_decrypt[..]);
    let mut decompressed_data = Vec::new(); // 用于存储解压缩后的数据
    // 读取所有解压缩数据到 decompressed_data 向量中。
    // 如果解压缩失败，`map_err` 会将 `std::io::Error` 转换为 `KugouError::Decompression`。
    decoder
        .read_to_end(&mut decompressed_data)
        .map_err(KugouError::Decompression)?;

    // 5. 转换为 UTF-8 字符串
    // 将解压缩后的字节数据尝试转换为 UTF-8 字符串。
    // 如果转换失败（例如，数据不是有效的 UTF-8），则返回 `std::string::FromUtf8Error`，
    // 该错误会被转换为 `KugouError::Utf8`。
    let krc_string = String::from_utf8(decompressed_data)?;

    // 移除第一个字符，对应Lyricify Lyrics Helper里的 return res[1..];
    // 似乎并不是必要的
    // krc_string.remove(0);

    Ok(krc_string) // 返回最终解密和解压缩后的 KRC 歌词文本
}
