// 导入同级模块 error 中定义的错误类型和 Result 别名
use crate::netease_lyrics_fetcher::error::{NeteaseError, Result};
// 导入 aes crate 相关组件，用于 AES 加密
use aes::Aes128;
use aes::cipher::generic_array::GenericArray; // 用于将字节切片转换为密码库所需的固定大小数组
use aes::cipher::{BlockEncryptMut, BlockSizeUser, KeyInit, KeyIvInit}; // AES 加密核心特质和初始化方法 // 使用 AES-128
// 导入 block-padding crate，用于实现 PKCS7 填充
use block_padding::Pkcs7;
// 导入 cbc crate，用于 AES CBC (Cipher Block Chaining) 模式加密
use cbc::Encryptor as CbcModeEncryptor;
// 导入 ecb crate，用于 AES ECB (Electronic Codebook) 模式加密
use ecb::Encryptor as EcbModeEncryptor;

// 导入 md5 crate，用于计算 MD5 哈希
use md5::{Digest, Md5 as Md5Hasher};

// 导入 num_bigint 和 num_traits，用于处理大整数运算（RSA 加密需要）
use num_bigint::BigInt;
use num_traits::Num;
// 导入 rand crate，用于生成随机密钥
use rand::distr::Alphanumeric;
use rand::{Rng, rng};

// --- 常量定义 ---

// WEAPI 加密中使用的固定"随机数"串，实际上是 AES CBC 加密的第一轮密钥
pub(crate) const NONCE_STR: &str = "0CoJUm6Qyw8W8jud";
// WEAPI 和 EAPI 中 AES CBC 加密使用的固定初始化向量 (IV)
pub(crate) const VI_STR: &str = "0102030405060708";

// EAPI 加密中使用的固定 AES ECB 密钥
const EAPI_KEY_STR: &str = "e82ckenh8dichen8";

/// 计算经过 PKCS7 填充后的数据长度。
///
/// # Arguments
/// * `msg_len` - 原始消息的长度（字节）。
/// * `block_size` - 加密算法的块大小（字节，例如 AES 为16）。
///
/// # Returns
/// `usize` - 填充后的总长度。
fn pkcs7_padded_len(msg_len: usize, block_size: usize) -> usize {
    (msg_len / block_size + 1) * block_size
}

/// 生成一个指定长度的随机字母数字字符串。
/// 主要用于为 WEAPI 生成16字节的随机对称密钥 `weapi_secret_key`。
pub fn create_secret_key(length: usize) -> String {
    rng() // 使用线程本地的随机数生成器
        .sample_iter(&Alphanumeric) // 从字母和数字中采样
        .take(length) // 取指定长度的字符
        .map(char::from) // 将 u8 转换为 char
        .collect() // 收集为字符串
}

/// 将十六进制字符串转换为 `BigInt` 大整数。
/// RSA 加密中需要将密钥、指数和模数表示为大整数。
fn hex_str_to_bigint(hex: &str) -> Result<BigInt> {
    BigInt::from_str_radix(hex, 16) // 以16为基数解析
        .map_err(|e| NeteaseError::Crypto(format!("无法解析十六进制字符串: {}", e)))
}

/// 实现 RSA 加密的核心逻辑。
/// 用于 WEAPI 中加密随机生成的对称密钥，得到 `encSecKey`。
///
/// # Arguments
/// * `text` - 明文（通常是 `weapi_secret_key`）。
/// * `pub_key_hex` - RSA 公钥指数的十六进制字符串 (例如 "010001")。
/// * `modulus_hex` - RSA 公钥模数的十六进制字符串。
///
/// # Returns
/// `Result<String>` - RSA 加密后的密文的十六进制字符串，长度固定为256。
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
    let mut key_hex = format!("{:x}", result_bigint);

    // 6. 对结果进行填充或截断，确保长度为256个字符 (对应128字节的 RSA 密钥长度)
    //    如果不足256位，在前面补0；如果超过，则截取低256位 (这部分逻辑可能不完全标准，但符合网易云实现)
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

/// 实现 AES CBC (Cipher Block Chaining) 模式加密。
/// 用于 WEAPI 请求参数的两轮加密。
///
/// # Arguments
/// * `data_str` - 待加密的明文字符串。
/// * `key_str` - AES 密钥字符串 (16字节)。
/// * `iv_str` - AES 初始化向量字符串 (16字节)。
///
/// # Returns
/// `Result<String>` - 加密后的数据的十六进制字符串 (大写)。
pub fn aes_cbc_encrypt(data_str: &str, key_str: &str, iv_str: &str) -> Result<String> {
    let key_bytes = key_str.as_bytes();
    let iv_bytes = iv_str.as_bytes();
    let block_size = Aes128::block_size(); // AES 块大小为16字节

    // 校验密钥和 IV 长度
    if key_bytes.len() != block_size {
        return Err(NeteaseError::Crypto(format!(
            "AES 密钥必须为 {} 字节，但实际为 {}",
            block_size,
            key_bytes.len()
        )));
    }
    if iv_bytes.len() != block_size {
        return Err(NeteaseError::Crypto(format!(
            "AES IV 长度必须为 {} 字节，但实际为 {}",
            block_size,
            iv_bytes.len()
        )));
    }

    // 将密钥和 IV 转换为 GenericArray 类型，这是 `aes` crate 所需的格式
    let key_ga = GenericArray::from_slice(key_bytes);
    let iv_ga = GenericArray::from_slice(iv_bytes);

    // 初始化 AES CBC 加密器
    let cipher = CbcModeEncryptor::<Aes128>::new(key_ga, iv_ga);

    let plaintext_bytes = data_str.as_bytes(); // 明文字节
    let msg_len = plaintext_bytes.len(); // 明文长度
    // 计算 PKCS7 填充后的长度
    let padded_len = pkcs7_padded_len(msg_len, block_size);

    // 创建一个足够容纳填充后数据的缓冲区
    let mut buffer: Vec<u8> = Vec::with_capacity(padded_len);
    buffer.extend_from_slice(plaintext_bytes); // 复制明文到缓冲区
    buffer.resize(padded_len, 0u8); // 用0填充到 padded_len (PKCS7填充会在加密时自动处理)
    // 注意：`encrypt_padded_mut` 会自行处理填充字节的添加，
    // 所以这里 `resize` 填充0可能不是必需的，或者说 `encrypt_padded_mut`
    // 会覆盖这些0。更常见的做法是只 `extend_from_slice`，然后让加密函数处理。
    // 但当前代码结构是先 resize 再调用 `encrypt_padded_mut`，这也能工作。

    // 执行带 PKCS7 填充的 AES CBC 加密
    let ciphertext_slice = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, msg_len) // msg_len 是原始数据长度
        .map_err(|e| NeteaseError::Crypto(format!("AES CBC 加密失败: {:?}", e)))?;

    // 将加密后的字节数据转换为大写的十六进制字符串
    Ok(hex::encode_upper(ciphertext_slice))
}

/// 实现 AES ECB (Electronic Codebook) 模式加密，专用于 EAPI。
///
/// # Arguments
/// * `data_bytes` - 待加密的明文字节切片。
/// * `key_bytes` - AES 密钥字节切片 (16字节)。
///
/// # Returns
/// `Result<String>` - 加密后的数据的十六进制字符串 (大写)。
pub fn aes_ecb_encrypt_eapi(data_bytes: &[u8], key_bytes: &[u8]) -> Result<String> {
    let block_size = Aes128::block_size();
    // 校验密钥长度
    if key_bytes.len() != block_size {
        return Err(NeteaseError::Crypto(format!(
            "EAPI AES 密钥长度必须为 {} 字节，但实际为 {}",
            block_size,
            key_bytes.len()
        )));
    }

    let key_ga = GenericArray::from_slice(key_bytes);
    // 初始化 AES ECB 加密器 (ECB 模式不需要 IV)
    let cipher = EcbModeEncryptor::<Aes128>::new(key_ga);

    let msg_len = data_bytes.len();
    let padded_len = pkcs7_padded_len(msg_len, block_size);

    let mut buffer: Vec<u8> = Vec::with_capacity(padded_len);
    buffer.extend_from_slice(data_bytes);
    buffer.resize(padded_len, 0u8);

    // 执行带 PKCS7 填充的 AES ECB 加密
    let ciphertext_slice = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, msg_len)
        .map_err(|e| NeteaseError::Crypto(format!("AES ECB 加密失败: {:?}", e)))?;

    Ok(hex::encode_upper(ciphertext_slice))
}

/// 准备 EAPI 请求的加密参数。
/// 这是 EAPI 加密流程的核心编排函数。
///
/// # Arguments
/// * `url_path` - API 的 URL 路径段 (例如 "/api/song/lyric/v1")。
/// * `params_obj` - 原始请求参数对象 (需要实现 `serde::Serialize`)。
///
/// # Returns
/// `Result<String>` - 最终加密后的参数的十六进制字符串。
pub fn prepare_eapi_params<T: serde::Serialize>(url_path: &str, params_obj: &T) -> Result<String> {
    // 1. 将原始请求参数对象序列化为 JSON 字符串
    let text = serde_json::to_string(params_obj)?;
    // 2. 构造特定格式的消息字符串
    let message = format!("nobody{}use{}md5forencrypt", url_path, text);
    // 3. 计算该消息字符串的 MD5 哈希
    let mut md5_hasher = Md5Hasher::new_with_prefix(""); // 初始化 MD5 哈希器
    md5_hasher.update(message.as_bytes()); // 更新哈希内容
    let digest = format!("{:x}", md5_hasher.finalize()); // 获取十六进制的 MD5摘要

    // 4. 构造一个新的待加密字符串，格式为：url_path + "-36cd479b6b5-" + json_payload + "-36cd479b6b5-" + md5_digest
    let data_to_encrypt_str = format!("{}-36cd479b6b5-{}-36cd479b6b5-{}", url_path, text, digest);

    // 5. 使用固定的 EAPI_KEY_STR 对构造的字符串进行 AES ECB 加密
    aes_ecb_encrypt_eapi(data_to_encrypt_str.as_bytes(), EAPI_KEY_STR.as_bytes())
}
