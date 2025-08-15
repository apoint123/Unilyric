//! 模块化设备模拟与缓存
//!
//! 负责创建、管理和缓存一个持久化的虚拟设备身份。
//! 目的是让 API 请求看起来像是从一个真实的 QQ 音乐移动端 App 发出的。
//! API 来源于 <https://github.com/luren-dc/QQMusicApi>

use rand::Rng;
use rand::distr::Alphanumeric;
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use uuid::Uuid;

/// 描述操作系统的版本信息。
#[allow(missing_docs)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct OsVersion {
    pub incremental: String,
    pub release: String,
    pub codename: String,
    pub sdk: u32,
}

/// 封装了一个虚拟设备的所有相关属性。
///
/// 这些属性在获取 Qimei 时被用来生成 payload。
#[allow(missing_docs)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Device {
    pub display: String,
    pub product: String,
    pub device: String,
    pub board: String,
    pub model: String,
    pub fingerprint: String,
    pub boot_id: String,
    pub proc_version: String,
    pub imei: String,
    pub brand: String,
    pub bootloader: String,
    pub base_band: String,
    pub version: OsVersion,
    pub sim_info: String,
    pub os_type: String,
    pub mac_address: String,
    pub wifi_bssid: String,
    pub wifi_ssid: String,
    pub android_id: String,
    pub apn: String,
    pub vendor_name: String,
    pub vendor_os_name: String,
    /// 从服务器获取并缓存的 Qimei36 值。
    pub qimei: Option<String>,
}

/// 根据 Luhn 算法生成一个随机的 IMEI 号码。
fn random_imei() -> String {
    let mut rng = rand::rng();
    let mut imei_digits: Vec<u32> = Vec::with_capacity(15);
    let mut sum = 0;

    for i in 0..14 {
        let digit = rng.random_range(0..=9);
        imei_digits.push(digit);

        let mut temp = digit;
        if i % 2 == 0 {
            temp *= 2;
            if temp >= 10 {
                temp = (temp % 10) + 1;
            }
        }
        sum += temp;
    }

    let control_digit = (sum * 9) % 10;
    imei_digits.push(control_digit);

    imei_digits.into_iter().map(|d| d.to_string()).collect()
}

impl Default for Device {
    fn default() -> Self {
        Self::new()
    }
}

impl Device {
    /// 创建一个新的随机设备。
    pub fn new() -> Self {
        let mut rng = rand::rng();
        let mut android_id = String::with_capacity(16);
        for _ in 0..8 {
            let byte = rng.random::<u8>();
            let _ = write!(android_id, "{byte:02x}");
        }

        Self {
            display: format!("QMAPI.{}.001", rng.random_range(100_000..999_999)),
            product: "iarim".to_string(),
            device: "sagit".to_string(),
            board: "eomam".to_string(),
            model: "MI 6".to_string(),
            fingerprint: format!(
                "xiaomi/iarim/sagit:10/eomam.200122.001/{}:user/release-keys",
                rng.random_range(1_000_000..9_999_999)
            ),
            boot_id: Uuid::new_v4().to_string(),
            proc_version: format!(
                "Linux 5.4.0-54-generic-{} (android-build@google.com)",
                (&mut rng)
                    .sample_iter(Alphanumeric)
                    .take(8)
                    .map(char::from)
                    .collect::<String>()
            ),

            imei: random_imei(),
            brand: "Xiaomi".to_string(),
            bootloader: "U-boot".to_string(),
            base_band: String::new(),
            version: OsVersion {
                incremental: "5891938".to_string(),
                release: "10".to_string(),
                codename: "REL".to_string(),
                sdk: 29,
            },
            sim_info: "T-Mobile".to_string(),
            os_type: "android".to_string(),
            mac_address: "00:50:56:C0:00:08".to_string(),
            wifi_bssid: "00:50:56:C0:00:08".to_string(),
            wifi_ssid: "<unknown ssid>".to_string(),
            android_id,
            apn: "wifi".to_string(),
            vendor_name: "MIUI".to_string(),
            vendor_os_name: "qmapi".to_string(),
            qimei: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{get_config_file_path, load_cached_config, save_cached_config};

    use super::*;
    use std::fs;

    #[test]
    fn test_device_creation() {
        let device = Device::new();

        assert!(!device.display.is_empty(), "Display 字段不应为空");
        assert!(!device.fingerprint.is_empty(), "Fingerprint 字段不应为空");
        assert_eq!(device.imei.len(), 15, "IMEI 长度应为 15 位");
        assert_eq!(device.android_id.len(), 16, "Android ID 长度应为 16 位");

        assert!(device.qimei.is_none(), "新设备的 qimei 字段应为 None");

        println!("✅ Device::new() 功能正常。");
    }

    #[test]
    fn test_device_caching_flow() {
        const CACHE_FILENAME: &str = "qq_device_test.json";
        let cache_path = get_config_file_path(CACHE_FILENAME).expect("无法获取缓存路径");

        if cache_path.exists() {
            fs::remove_file(&cache_path).expect("无法删除旧的缓存文件");
        }

        let first_device = Device::new();
        println!("第一次生成的设备 IMEI: {}", first_device.imei);
        save_cached_config(CACHE_FILENAME, &first_device).expect("第一次保存设备失败");

        assert!(cache_path.exists(), "缓存文件应在第一次保存后被创建");

        let second_device_cached =
            load_cached_config::<Device>(CACHE_FILENAME).expect("第二次加载设备失败");
        let second_device = second_device_cached.data;
        println!("从缓存加载的设备 IMEI: {}", second_device.imei);

        assert_eq!(
            first_device, second_device,
            "从缓存加载的设备与初次创建的设备不一致"
        );

        fs::remove_file(&cache_path).expect("无法删除测试缓存文件");

        println!("✅ 设备缓存读写流程正常。");
    }
}
