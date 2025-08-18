//! 本模块用于加密发送给网易云音乐 API 的请求。
//! 本实现仅用于加密网易云音乐 API 请求，不应用于实际安全目的。
//!
//! 加密逻辑来源于 <https://github.com/Binaryify/NeteaseCloudMusicApi>

use aes::{
    Aes128,
    cipher::{BlockSizeUser, KeyIvInit, generic_array::GenericArray},
};
use base64::{Engine, prelude::BASE64_STANDARD};
use block_padding::Pkcs7;
use cbc::Encryptor as CbcModeEncryptor;
use cipher::{BlockEncryptMut, KeyInit};
use ecb::Encryptor as EcbModeEncryptor;
use md5::{Digest, Md5 as Md5Hasher};
use num_bigint::BigInt;
use num_traits::Num;
use rand::{Rng, distributions::Alphanumeric, thread_rng};

use crate::error::{LyricsHelperError, Result};

/// WEAPI 加密中使用的固定"随机数"串，实际上是 AES CBC 加密的第一轮密钥
pub(crate) const NONCE_STR: &str = "0CoJUm6Qyw8W8jud";
/// WEAPI 和 EAPI 中 AES CBC 加密使用的固定初始化向量 (IV)
pub(crate) const VI_STR: &str = "0102030405060708";

/// EAPI 加密中使用的固定 AES ECB 密钥
const EAPI_KEY_STR: &str = "e82ckenh8dichen8";
/// WEAPI 加密中使用的 RSA 公钥指数 ("010001"，即 65537)
pub(crate) const PUBKEY_STR_API: &str = "010001";
/// WEAPI 加密中使用的 RSA 公钥模数 (一个很长的十六进制字符串)
pub(crate) const MODULUS_STR_API: &str = "00e0b509f6259df8642dbc35662901477df22677ec152b5ff68ace615bb7b725152b3ab17a876aea8a5aa76d2e417629ec4ee341f56135fccf695280104e0312ecbda92557c93870114af6c9d05c4f7f0c3685b7a46bee255932575cce10b424d813cfe4875d3e82047b97ddef52741d546b8e289dc6935b3ece0462db0a22b8e7";

/// 生成一个指定长度的随机字母数字字符串。
///
/// 此函数主要用于为 WEAPI 生成 16 字节的随机对称密钥 `weapi_secret_key`。
pub fn create_secret_key(length: usize) -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

/// 将十六进制字符串转换为 `BigInt` 大整数。
/// RSA 加密中需要将密钥、指数和模数表示为大整数。
fn hex_str_to_bigint(hex: &str) -> Result<BigInt> {
    BigInt::from_str_radix(hex, 16)
        .map_err(|e| LyricsHelperError::Encryption(format!("无法解析十六进制字符串: {e}")))
}

/// 实现 RSA 加密的核心逻辑。
///
/// 此函数用于 WEAPI 中加密随机生成的对称密钥，得到 `encSecKey`。
///
/// # 参数
/// * `text` - 明文（通常是 `weapi_secret_key`）。
/// * `pub_key_hex` - RSA 公钥指数的十六进制字符串 (例如 "010001")。
/// * `modulus_hex` - RSA 公钥模数的十六进制字符串。
///
/// # 返回
/// - `Result<String>`: RSA 加密后的密文的十六进制字符串，长度固定为 256。
pub fn rsa_encode(text: &str, pub_key_hex: &str, modulus_hex: &str) -> Result<String> {
    // 1. 将明文反转 (网易云特定的预处理步骤)
    let reversed_text: String = text.chars().rev().collect();
    // 2. 将反转后的明文转换为十六进制字符串
    let text_hex = hex::encode(reversed_text.as_bytes());

    // 3. 将十六进制的明文、公钥指数、模数转换为 BigInt
    let a = hex_str_to_bigint(&text_hex)?; // 明文的大整数表示
    let b = hex_str_to_bigint(pub_key_hex)?; // 公钥指数的大整数表示 (e)
    let c = hex_str_to_bigint(modulus_hex)?; // 公钥模数的大整数表示 (n)

    // 4. 执行 RSA 加密核心操作: result = a^b mod c
    let result_bigint = a.modpow(&b, &c);
    // 5. 将加密结果大整数转换为十六进制字符串
    let mut key_hex = format!("{result_bigint:x}");

    // 6. 对结果进行填充或截断，确保长度为256个字符 (对应128字节的 RSA 密钥长度)
    //    如果不足256位，在前面补0；如果超过，则截取低256位 (这部分不怎么标准，但符合网易云实现)
    match key_hex.len().cmp(&256) {
        std::cmp::Ordering::Less => {
            // 长度不足，前补0
            key_hex = format!("{}{}", "0".repeat(256 - key_hex.len()), key_hex);
        }
        std::cmp::Ordering::Greater => {
            // 长度超出，取后256位
            key_hex = key_hex.split_at(key_hex.len() - 256).1.to_string();
        }
        std::cmp::Ordering::Equal => {} // 长度正好，无需操作
    }
    Ok(key_hex)
}

/// 实现 AES ECB 模式加密，专用于 EAPI。
///
/// # 参数
/// * `data_bytes` - 待加密的明文字节切片。
/// * `key_bytes` - AES 密钥字节切片 (必须为 16 字节)。
///
/// # 返回
/// - `Result<String>`: 加密后的数据的十六进制字符串 (大写)。
pub fn aes_ecb_encrypt_eapi(data_bytes: &[u8], key_bytes: &[u8]) -> Result<String> {
    let block_size = Aes128::block_size();
    if key_bytes.len() != block_size {
        return Err(LyricsHelperError::Encryption(format!(
            "EAPI AES 密钥长度必须为 {} 字节，但实际为 {}",
            block_size,
            key_bytes.len()
        )));
    }

    let key_ga = GenericArray::from_slice(key_bytes);
    let cipher = EcbModeEncryptor::<Aes128>::new(key_ga);

    let mut buffer = data_bytes.to_vec();
    let msg_len = buffer.len();

    let block_size = Aes128::block_size();
    let padded_len = (msg_len / block_size + 1) * block_size;
    buffer.resize(padded_len, 0);

    let ciphertext_slice = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, msg_len)
        .map_err(|e| LyricsHelperError::Encryption(format!("AES ECB 加密失败: {e:?}")))?;

    Ok(hex::encode_upper(ciphertext_slice))
}

/// 准备 EAPI 请求的加密参数。
///
/// # 参数
/// * `url_path` - API 的 URL 路径段 (例如 "/api/song/lyric/v1")。
/// * `params_obj` - 原始请求参数对象 (需要实现 `serde::Serialize`)。
///
/// # 返回
/// - `Result<String>`: 最终加密后的参数的十六进制字符串。
pub fn prepare_eapi_params<T: serde::Serialize>(url_path: &str, params_obj: &T) -> Result<String> {
    // 1. 将原始请求参数对象序列化为 JSON 字符串
    let text = serde_json::to_string(params_obj)?;

    // 2. 构造特定格式的消息字符串
    let message = format!("nobody{url_path}use{text}md5forencrypt");

    // 3. 计算该消息字符串的 MD5 哈希
    let mut md5_hasher = Md5Hasher::new_with_prefix("");
    md5_hasher.update(message.as_bytes());
    let digest = hex::encode(md5_hasher.finalize());

    // 4. 构造一个新的待加密字符串
    let data_to_encrypt_str = format!("{url_path}-36cd479b6b5-{text}-36cd479b6b5-{digest}");

    // 5. 进行 AES ECB 加密
    aes_ecb_encrypt_eapi(data_to_encrypt_str.as_bytes(), EAPI_KEY_STR.as_bytes())
}

/// 实现 AES CBC 模式加密，并返回 Base64 编码的字符串。
///
/// # 参数
/// * `data_str` - 待加密的明文字符串。
/// * `key_str` - 密钥字符串 (ASCII)。
/// * `iv_str` - 初始化向量字符串 (ASCII)。
///
/// # 返回
/// - `Result<String>`: 加密后并经过 Base64 编码的字符串。
pub fn aes_cbc_encrypt_base64(data_str: &str, key_str: &str, iv_str: &str) -> Result<String> {
    let key_bytes = key_str.as_bytes();
    let iv_bytes = iv_str.as_bytes();
    let block_size = Aes128::block_size();

    if key_bytes.len() != block_size {
        return Err(LyricsHelperError::Encryption(format!(
            "AES 密钥长度必须为 {} 字节，当前为 {}",
            block_size,
            key_bytes.len()
        )));
    }
    if iv_bytes.len() != block_size {
        return Err(LyricsHelperError::Encryption(format!(
            "AES 初始化向量长度必须为 {} 字节，当前为 {}",
            block_size,
            iv_bytes.len()
        )));
    }

    let key_ga = GenericArray::from_slice(key_bytes);
    let iv_ga = GenericArray::from_slice(iv_bytes);
    let cipher = CbcModeEncryptor::<Aes128>::new(key_ga, iv_ga);

    let mut buffer = data_str.as_bytes().to_vec();
    let msg_len = buffer.len();

    let block_size = Aes128::block_size();
    let padded_len = (msg_len / block_size + 1) * block_size;
    buffer.resize(padded_len, 0);

    let ciphertext_slice = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, msg_len)
        .map_err(|e| LyricsHelperError::Encryption(format!("AES CBC 模式加密失败: {e:?}")))?;

    Ok(BASE64_STANDARD.encode(ciphertext_slice))
}
