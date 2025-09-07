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
use rand::{Rng, rngs::OsRng};
use rsa::{Pkcs1v15Encrypt, RsaPublicKey, pkcs8::DecodePublicKey};
use serde_json::json;
use std::fmt::Write;
use thiserror::Error;
use tracing::{info, warn};

const PUBLIC_KEY: &str = r"-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDEIxgwoutfwoJxcGQeedgP7FG9
qaIuS0qzfR8gWkrkTZKM2iWHn2ajQpBRZjMSoSf6+KJGvar2ORhBfpDXyVtZCKpq
LQ+FLkpncClKVIrBwv6PHyUvuCb0rIarmgDnzkfQAqVufEtR64iazGDKatvJ9y6B
9NMbHddGSAUmRTCrHQIDAQAB
-----END PUBLIC KEY-----";

const SECRET: &str = "ZdJqM15EeO2zWc08";
const APP_KEY: &str = "0AND0HD6FE4HY80F";
const HEX_CHARSET: &[u8] = b"abcdef1234567890";

#[derive(Debug, Error)]
pub enum QimeiError {
    #[error("加密失败")]
    Encryption(String),

    #[error("HTTP 请求失败")]
    Network(#[from] crate::LyricsHelperError),

    #[error("JSON 序列化或反序列化失败")]
    Json(#[from] serde_json::Error),

    #[error("API 响应解析失败: {0}")]
    ResponseParsing(String),
}

/// Qimei 服务器成功响应后返回的数据结构。
#[derive(serde::Deserialize, Debug)]
pub struct QimeiResult {
    /// 16位的 Qimei。
    pub q16: String,
    /// 36位的 Qimei，API 主要使用它。
    pub q36: String,
}

fn rsa_encrypt(content: &[u8]) -> Result<Vec<u8>, rsa::Error> {
    let cleaned_key = PUBLIC_KEY.trim();
    let public_key = RsaPublicKey::from_public_key_pem(cleaned_key).expect("解析公钥失败");
    let mut rng = OsRng;
    public_key.encrypt(&mut rng, Pkcs1v15Encrypt, content)
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

fn build_payload(device: &Device, version: &str) -> serde_json::Value {
    let reserved = json!({
        "harmony": "0", "clone": "0", "containe": "",
        "oz": "UhYmelwouA+V2nPWbOvLTgN2/m8jwGB+yUB5v9tysQg=",
        "oo": "Xecjt+9S1+f8Pz2VLSxgpw==", "kelong": "0",
        "uptimes": "2024-01-01 08:00:00", "multiUser": "0",
        "bod": device.brand, "dv": device.device,
        "firstLevel": "", "manufact": device.brand,
        "name": device.model, "host": "se.infra",
        "kernel": device.proc_version,
    });

    json!({
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
    })
}

fn prepare_qimei_params(payload_bytes: &[u8]) -> Result<(serde_json::Value, i64), QimeiError> {
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

    let key_encrypted =
        rsa_encrypt(crypt_key.as_bytes()).map_err(|e| QimeiError::Encryption(e.to_string()))?;
    let key_b64 = STANDARD.encode(key_encrypted);

    let params_encrypted = aes_encrypt(crypt_key.as_bytes(), payload_bytes)
        .map_err(|e| QimeiError::Encryption(e.to_string()))?;
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

    let qimei_params = json!({
        "key": key_b64, "params": params_b64,
        "time": ts.to_string(), "nonce": nonce,
        "sign": sign, "extra": extra
    });

    Ok((qimei_params, ts))
}

async fn send_and_parse_response(
    http_client: &dyn HttpClient,
    qimei_params: serde_json::Value,
    ts: i64,
) -> Result<QimeiResult, QimeiError> {
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

    let body = json!({ "app": 0, "os": 1, "qimeiParams": qimei_params });
    let body_bytes = serde_json::to_vec(&body)?;

    let response = http_client
        .request_with_headers(
            HttpMethod::Post,
            "https://api.tencentmusic.com/tme/trpc/proxy",
            &headers,
            Some(&body_bytes),
        )
        .await?;

    let response_text = response.text()?;
    let outer_resp: serde_json::Value = serde_json::from_str(&response_text)?;
    let inner_json_str = outer_resp["data"]
        .as_str()
        .ok_or_else(|| QimeiError::ResponseParsing("Inner data not found".to_string()))?;
    let inner_resp: serde_json::Value = serde_json::from_str(inner_json_str)?;
    let qimei_data = &inner_resp["data"];

    serde_json::from_value(qimei_data.clone())
        .map_err(|e| QimeiError::ResponseParsing(format!("Failed to parse final Qimei data: {e}")))
}

async fn try_get_qimei_from_network(
    http_client: &dyn HttpClient,
    device: &Device,
    version: &str,
) -> Result<QimeiResult, QimeiError> {
    let payload = build_payload(device, version);
    let payload_bytes = serde_json::to_vec(&payload)?;

    let (qimei_params, ts) = prepare_qimei_params(&payload_bytes)?;

    send_and_parse_response(http_client, qimei_params, ts).await
}

pub async fn get_qimei(
    http_client: &dyn HttpClient,
    device: &Device,
    version: &str,
) -> Result<QimeiResult, QimeiError> {
    match try_get_qimei_from_network(http_client, device, version).await {
        Ok(result) => Ok(result),
        Err(e) => {
            warn!("获取 Qimei 失败: {}. 使用缓存或默认值。", e);
            device.qimei.as_ref().map_or_else(
                || {
                    warn!("未找到缓存的 Qimei，使用硬编码的默认值。");
                    Ok(QimeiResult {
                        q16: String::new(),
                        q36: "6c9d3cd110abca9b16311cee10001e717614".to_string(),
                    })
                },
                |cached_q36| {
                    info!("使用缓存的 Qimei: {}", cached_q36);
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
    use crate::{http::WreqClient, providers::qq::device::Device};

    #[tokio::test]
    #[ignore]
    async fn test_get_qimei_online() {
        let device = Device::new();
        let api_version = "13.2.5.8";
        let http_client = WreqClient::new().unwrap();
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
