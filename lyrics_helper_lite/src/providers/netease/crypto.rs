use crate::error::{FetcherError, Result};
use aes::{Aes128, cipher::KeyInit};
use block_padding::Pkcs7;
use cipher::{BlockEncryptMut, generic_array::GenericArray};
use ecb::Encryptor as EcbModeEncryptor;
use md5::{Digest, Md5 as Md5Hasher};

const EAPI_KEY_STR: &str = "e82ckenh8dichen8";

fn aes_ecb_encrypt_eapi(data_bytes: &[u8], key_bytes: &[u8]) -> Result<String> {
    let key_ga = GenericArray::from_slice(key_bytes);
    let cipher = EcbModeEncryptor::<Aes128>::new(key_ga);

    let mut buffer = data_bytes.to_vec();
    let msg_len = buffer.len();

    let ciphertext_slice = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buffer, msg_len)
        .map_err(|e| FetcherError::Provider(format!("AES ECB encryption failed: {e:?}")))?;

    Ok(hex::encode_upper(ciphertext_slice))
}

pub fn prepare_eapi_params<T: serde::Serialize>(url_path: &str, params_obj: &T) -> Result<String> {
    let text = serde_json::to_string(params_obj)?;
    let message = format!("nobody{url_path}use{text}md5forencrypt");

    let mut md5_hasher = Md5Hasher::new();
    md5_hasher.update(message.as_bytes());
    let digest = hex::encode(md5_hasher.finalize());

    let data_to_encrypt_str = format!("{url_path}-36cd479b6b5-{text}-36cd479b6b5-{digest}");

    aes_ecb_encrypt_eapi(data_to_encrypt_str.as_bytes(), EAPI_KEY_STR.as_bytes())
}
