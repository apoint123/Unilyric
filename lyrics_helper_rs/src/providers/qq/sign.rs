//! QQ音乐请求签名算法的实现

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_ENGINE};
use sha1::{Digest, Sha1};

const PART_1_INDEXES: [usize; 7] = [23, 14, 6, 36, 16, 7, 19];
const PART_2_INDEXES: [usize; 8] = [16, 1, 32, 12, 19, 27, 8, 5];
const SCRAMBLE_VALUES: [u8; 20] = [
    89, 39, 179, 150, 218, 82, 58, 252, 177, 52, 186, 123, 120, 64, 242, 133, 143, 161, 121, 179,
];

pub fn sign(request: &serde_json::Value) -> Result<String, serde_json::Error> {
    let request_str = serde_json::to_string(request)?;

    let mut hasher = Sha1::new();
    hasher.update(request_str.as_bytes());
    let hash = hasher.finalize();
    let hash_hex = format!("{hash:X}");

    let hash_chars: Vec<char> = hash_hex.chars().collect();

    let part1: String = PART_1_INDEXES.iter().map(|&i| hash_chars[i]).collect();
    let part2: String = PART_2_INDEXES.iter().map(|&i| hash_chars[i]).collect();

    let mut part3 = Vec::with_capacity(20);
    for (i, &scramble_val) in SCRAMBLE_VALUES.iter().enumerate() {
        let hex_pair = &hash_hex[i * 2..i * 2 + 2];
        let byte_val = u8::from_str_radix(hex_pair, 16).unwrap_or(0);
        part3.push(scramble_val ^ byte_val);
    }

    let b64_part = BASE64_ENGINE.encode(&part3);
    let b64_part_cleaned = b64_part.replace(['/', '+', '='], "");

    let final_sign = format!("zzc{part1}{b64_part_cleaned}{part2}").to_lowercase();
    Ok(final_sign)
}
