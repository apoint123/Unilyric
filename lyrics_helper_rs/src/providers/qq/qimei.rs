//! QIMEI 设备指纹获取模块
//!
//! Qimei 是访问 QQ 音乐新版 API 必需的一个关键身份参数。
//! API 来源于 <https://github.com/luren-dc/QQMusicApi>

use crate::http::HttpClient;
use crate::http::HttpMethod;
use crate::providers::qq::device::Device;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{Local, Utc};
use cipher::{BlockEncryptMut, KeyIvInit};
use md5::{Digest, Md5};
use rand::Rng;
use rand::rngs::OsRng;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use std::fmt::Write;

const PUBLIC_KEY: &str = r"-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDEIxgwoutfwoJxcGQeedgP7FG9
qaIuS0qzfR8gWkrkTZKM2iWHn2ajQpBRZjMSoSf6+KJGvar2ORhBfpDXyVtZCKpq
LQ+FLkpncClKVIrBwv6PHyUvuCb0rIarmgDnzkfQAqVufEtR64iazGDKatvJ9y6B
9NMbHddGSAUmRTCrHQIDAQAB
-----END PUBLIC KEY-----";

const SECRET: &str = "ZdJqM15EeO2zWc08";
const APP_KEY: &str = "0AND0HD6FE4HY80F";

/// Qimei 服务器成功响应后返回的数据结构。
#[derive(serde::Deserialize, Debug)]
pub struct QimeiResult {
    /// 16位的 Qimei。
    pub q16: String,
    /// 36位的 Qimei，API 主要使用它。
    pub q36: String,
}

fn rsa_encrypt(content: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let cleaned_key = PUBLIC_KEY.trim();

    let public_key = RsaPublicKey::from_public_key_pem(cleaned_key)?;

    let mut rng = OsRng;
    let encrypted = public_key.encrypt(&mut rng, Pkcs1v15Encrypt, content)?;
    Ok(encrypted)
}

fn aes_encrypt(key: &[u8], content: &[u8]) -> Result<Vec<u8>, &'static str> {
    type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
    const BLOCK_SIZE: usize = 16;

    let mut cipher =
        Aes128CbcEnc::new_from_slices(key, key).map_err(|_| "Invalid key or IV length")?;

    let pad_len = BLOCK_SIZE - (content.len() % BLOCK_SIZE);
    let mut buf = Vec::with_capacity(content.len() + pad_len);
    buf.extend_from_slice(content);
    #[allow(clippy::cast_possible_truncation)]
    buf.resize(content.len() + pad_len, pad_len as u8);

    for chunk in buf.chunks_mut(BLOCK_SIZE) {
        cipher.encrypt_block_mut(chunk.into());
    }

    Ok(buf)
}

fn random_beacon_id() -> String {
    let mut beacon_id = String::with_capacity(1600);
    let mut rng = rand::thread_rng();

    let now = Local::now();
    let time_month = now.format("%Y-%m-01").to_string();
    let rand1: u32 = rng.gen_range(100_000..=999_999);
    let rand2: u64 = rng.gen_range(100_000_000..=999_999_999);

    for i in 1..=40 {
        write!(beacon_id, "k{i}:").unwrap();

        match i {
            1 | 2 | 13 | 14 | 17 | 18 | 21 | 22 | 25 | 26 | 29 | 30 | 33 | 34 | 37 | 38 => {
                write!(beacon_id, "{time_month}{rand1}.{rand2}").unwrap();
            }
            3 => {
                beacon_id.push_str("0000000000000000");
            }
            4 => {
                const CHARSET: &[u8] = b"123456789abcdef";
                let hex_str: String = (0..16)
                    .map(|_| {
                        let idx = rng.gen_range(0..CHARSET.len());
                        CHARSET[idx] as char
                    })
                    .collect();
                beacon_id.push_str(&hex_str);
            }
            _ => {
                beacon_id.push_str(&rng.gen_range(0..=9999).to_string());
            }
        }
        beacon_id.push(';');
    }
    beacon_id
}

/// 从腾讯服务器获取 Qimei 指纹。
///
/// # 参数
/// * `device` - 一个包含了虚拟设备所有信息的 `Device` 实例。
/// * `version` - 当前模拟的 App 版本号字符串。
pub async fn get_qimei(
    http_client: &dyn HttpClient,
    device: &Device,
    version: &str,
) -> Result<QimeiResult, Box<dyn std::error::Error>> {
    const HEX_CHARSET: &[u8] = b"abcdef1234567890";

    let network_result = async {
        let reserved = serde_json::json!({
            "harmony": "0", "clone": "0", "containe": "",
            "oz": "UhYmelwouA+V2nPWbOvLTgN2/m8jwGB+yUB5v9tysQg=",
            "oo": "Xecjt+9S1+f8Pz2VLSxgpw==", "kelong": "0",
            "uptimes": "2024-01-01 08:00:00", "multiUser": "0",
            "bod": device.brand, "dv": device.device,
            "firstLevel": "", "manufact": device.brand,
            "name": device.model, "host": "se.infra",
            "kernel": device.proc_version,
        });

        let payload = serde_json::json!({
            "androidId": device.android_id, "platformId": 1,
            "appKey": APP_KEY, "appVersion": version,
            "beaconIdSrc": random_beacon_id(),
            "brand": device.brand, "channelId": "10003505",
            "cid": "", "imei": device.imei, "imsi": "", "mac": "",
            "model": device.model, "networkType": "unknown", "oaid": "",
            "osVersion": format!("Android {},level {}", device.version.release, device.version.sdk),
            "qimei": "", "qimei36": "", "sdkVersion": "1.2.13.6",
            "targetSdkVersion": "33", "audit": "", "userId": "{}",
            "packageId": "com.tencent.qqmusic",
            "deviceType": "Phone", "sdkName": "",
            "reserved": reserved.to_string(),
        });

        let payload_bytes = serde_json::to_vec(&payload)?;

        let (crypt_key, nonce) = {
            let mut rng = rand::thread_rng();
            let crypt_key: String = (0..16)
                .map(|_| {
                    let idx = rng.gen_range(0..HEX_CHARSET.len());
                    HEX_CHARSET[idx] as char
                })
                .collect();

            let nonce: String = (0..16)
                .map(|_| {
                    let idx = rng.gen_range(0..HEX_CHARSET.len());
                    HEX_CHARSET[idx] as char
                })
                .collect();

            (crypt_key, nonce)
        };

        let key_encrypted = rsa_encrypt(crypt_key.as_bytes())?;
        let key_b64 = STANDARD.encode(key_encrypted);

        let params_encrypted = aes_encrypt(crypt_key.as_bytes(), &payload_bytes)?;
        let params_b64 = STANDARD.encode(params_encrypted);

        let ts = Utc::now().timestamp_millis();

        let extra = format!(r#"{{"appKey":"{APP_KEY}"}}"#);

        let mut signature_hasher = Md5::new();
        signature_hasher.update(key_b64.as_bytes());
        signature_hasher.update(params_b64.as_bytes());
        signature_hasher.update(ts.to_string().as_bytes());
        signature_hasher.update(nonce.as_bytes());
        signature_hasher.update(SECRET.as_bytes());
        signature_hasher.update(extra.as_bytes());
        let sign = hex::encode(signature_hasher.finalize());

        let ts_sec = ts / 1000;
        let mut header_sign_hasher = Md5::new();
        header_sign_hasher.update(format!(
            "qimei_qq_androidpzAuCmaFAaFaHrdakPjLIEqKrGnSOOvH{ts_sec}"
        ));
        let header_sign = hex::encode(header_sign_hasher.finalize());
        let ts_sec_str = ts_sec.to_string();

        let headers = [
            ("method", "GetQimei"),
            ("service", "trpc.tme_datasvr.qimeiproxy.QimeiProxy"),
            ("appid", "qimei_qq_android"),
            ("sign", &header_sign),
            ("user-agent", "QQMusic"),
            ("timestamp", &ts_sec_str),
            ("Content-Type", "application/json"),
        ];

        let body = serde_json::json!({
            "app": 0, "os": 1,
            "qimeiParams": {
                "key": key_b64, "params": params_b64,
                "time": ts.to_string(), "nonce": nonce,
                "sign": sign, "extra": extra
            }
        });
        let body_bytes = serde_json::to_vec(&body)?;

        let response = http_client
            .request_with_headers(
                HttpMethod::Post,
                "https://api.tencentmusic.com/tme/trpc/proxy",
                &headers,
                Some(&body_bytes),
            )
            .await
            .map_err(Box::new)?;

        let response_text = response.text().map_err(Box::new)?;
        let outer_resp: serde_json::Value = serde_json::from_str(&response_text)?;
        let inner_json_str = outer_resp["data"].as_str().ok_or("Inner data not found")?;
        let inner_resp: serde_json::Value = serde_json::from_str(inner_json_str)?;
        let qimei_data = &inner_resp["data"];
        let result: QimeiResult = serde_json::from_value(qimei_data.clone())?;

        Ok::<_, Box<dyn std::error::Error>>(result)
    }
    .await;

    match network_result {
        Ok(result) => Ok(result),
        Err(e) => {
            tracing::warn!("获取 Qimei 失败: {}. 使用缓存或默认值。", e);
            device.qimei.as_ref().map_or_else(
                || {
                    tracing::warn!("未找到缓存的 Qimei，使用硬编码的默认值。");
                    Ok(QimeiResult {
                        q16: String::new(),
                        q36: "6c9d3cd110abca9b16311cee10001e717614".to_string(),
                    })
                },
                |cached_q36| {
                    tracing::info!("使用缓存的 Qimei: {}", cached_q36);
                    Ok(QimeiResult {
                        q16: String::new(), // q16 通常是临时的，所以返回空
                        q36: cached_q36.clone(),
                    })
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{http::ReqwestClient, providers::qq::device::Device};

    #[tokio::test]
    #[ignore]
    async fn test_get_qimei_online() {
        let device = Device::new();
        let api_version = "13.2.5.8";
        let http_client = ReqwestClient::new().unwrap();
        let qimei_result = get_qimei(&http_client, &device, api_version).await;

        assert!(
            qimei_result.is_ok(),
            "获取 Qimei 不应返回错误，收到的错误: {:?}",
            qimei_result.err()
        );

        let result = qimei_result.unwrap();

        assert!(!result.q36.is_empty(), "返回的 q36 字段不应为空");
        assert_eq!(result.q36.len(), 36, "q36 应为 36 个字符的十六进制字符串");

        println!("✅ 成功获取到 Qimei (q36): {}", result.q36);
    }
}
