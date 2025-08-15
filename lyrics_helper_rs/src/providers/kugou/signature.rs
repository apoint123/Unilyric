//! 此模块包含为酷狗安卓版 API 请求生成签名的函数。
//! API 来源于 <https://github.com/MakcRe/KuGouMusicApi>

use md5::{self, Digest, Md5};
use std::collections::BTreeMap;
use std::fmt::Write;

const KUGOU_ANDROID_SALT: &str = "OIlwieks28dk2k092lksi2UIkp";
const KUGOU_LITE_ANDROID_SALT: &str = "LnT6xpN3khm36zse0QzvmgTZ3waWdRSA";

/// 为酷狗安卓版 API 请求生成 `signature`。
///
/// # 参数
/// * `params` - 一个包含所有 URL 查询参数的 `BTreeMap`。
/// * `body` - POST 请求的请求体字符串。对于 GET 请求，应传入空字符串。
/// * `is_lite` - 是否使用概念版 (lite) 的盐。
///
/// # 返回
/// 返回计算出的 32 位小写 MD5 签名字符串。
#[must_use]
pub fn signature_android_params(
    params: &BTreeMap<String, String>,
    body: &str,
    is_lite: bool,
) -> String {
    // 选择 salt
    let salt = if is_lite {
        KUGOU_LITE_ANDROID_SALT
    } else {
        KUGOU_ANDROID_SALT
    };

    // 构建排序后的参数字符串
    // BTreeMap 的迭代器已经按 key 的字典序排好序
    let mut params_string = String::with_capacity(params.len() * 10); // 估算一个初始容量
    for (k, v) in params {
        write!(&mut params_string, "{k}={v}").unwrap();
    }

    // 构建待哈希的完整字符串
    let mut string_to_sign =
        String::with_capacity(salt.len() * 2 + params_string.len() + body.len());
    string_to_sign.push_str(salt);
    string_to_sign.push_str(&params_string);
    string_to_sign.push_str(body);
    string_to_sign.push_str(salt);

    // 计算 MD5 并格式化为十六进制字符串
    let mut hasher = Md5::new();
    hasher.update(string_to_sign.as_bytes());
    let digest = hasher.finalize();

    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut output, "{byte:02x}").unwrap();
    }
    output
}

const KUGOU_SIGN_KEY_SALT: &str = "57ae12eb6890223e355ccfcb74edf70d";
const KUGOU_LITE_SIGN_KEY_SALT: &str = "185672dd44712f60bb1736df5a377e82";

/// 为获取歌曲 URL 等接口生成 `key` 参数。
#[must_use]
pub fn sign_key(hash: &str, mid: &str, userid: u64, appid: &str, is_lite: bool) -> String {
    let salt = if is_lite {
        KUGOU_LITE_SIGN_KEY_SALT
    } else {
        KUGOU_SIGN_KEY_SALT
    };
    let input = format!("{hash}{salt}{appid}{mid}{userid}");
    let digest = Md5::digest(input.as_bytes());

    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut output, "{byte:02x}").unwrap();
    }
    output
}

/// 为新的 /kmr/ 接口生成 body 中的 `key` 参数。
#[must_use]
pub fn sign_params_key(appid: &str, clientver: &str, clienttime: &str) -> String {
    // data 是 clienttime, str 是 KUGOU_ANDROID_SALT
    let input = format!("{appid}{KUGOU_ANDROID_SALT}{clientver}{clienttime}");
    let digest = Md5::digest(input.as_bytes());

    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut output, "{byte:02x}").unwrap();
    }
    output
}

/// 为酷狗设备注册接口生成 `signature`。
/// 算法: MD5("1014" + `sorted_values` + "1014")
#[must_use]
pub fn signature_register_params(params: &BTreeMap<String, String>) -> String {
    let mut values: Vec<&str> = params.values().map(String::as_str).collect();
    values.sort_unstable();

    let params_string: String = values.join("");

    let string_to_sign = format!("1014{params_string}1014");

    let digest = Md5::digest(string_to_sign.as_bytes());

    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut output, "{byte:02x}").unwrap();
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signature() {
        let mut params = BTreeMap::new();
        params.insert("appid".to_string(), "1005".to_string());
        params.insert("clientver".to_string(), "11083".to_string());
        params.insert("clienttime".to_string(), "1678886400".to_string());

        let body = "{\"data\":[{\"album_id\":\"12345\"}],\"is_buy\":0}";

        let signature = signature_android_params(&params, body, false);

        assert_eq!(signature, "f02fe39da9cc0f24a97aa5063da7de2f");
    }

    #[test]
    fn test_sign_key() {
        let hash = "HASH_TEST";
        let mid = "MID_TEST";
        let userid = 12345;
        let appid = "1005";

        let actual_key = sign_key(hash, mid, userid, appid, false);

        let expected_key = "e4e63e21332f2c1c28f325b6248531f4";

        assert_eq!(actual_key, expected_key);
    }

    #[test]
    fn test_sign_params_key() {
        let appid = "1005";
        let clientver = "12569";
        let clienttime = "1678886400";

        let actual_key = sign_params_key(appid, clientver, clienttime);
        let expected_key = "d750b0eda64e5a973df8ecca4b0d2e80";

        assert_eq!(actual_key, expected_key);
    }
}
