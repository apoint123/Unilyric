//! `reqwest` 客户端的默认实现。

use async_trait::async_trait;
use cookie_store::{CookieStore, RawCookie, serde::json};
use parking_lot::Mutex;
use reqwest::{Url, header::HeaderMap, header::HeaderValue};
use std::{collections::HashMap, sync::Arc};

use crate::{
    error::{LyricsHelperError, Result},
    http::{HttpClient, HttpMethod, HttpResponse},
};

#[derive(Debug, Clone, Default)]
struct SharedCookieStore {
    store: Arc<Mutex<CookieStore>>,
}

impl SharedCookieStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl reqwest::cookie::CookieStore for SharedCookieStore {
    fn set_cookies(&self, cookie_headers: &mut dyn Iterator<Item = &HeaderValue>, url: &Url) {
        let mut store = self.store.lock();

        let cookies = cookie_headers.filter_map(|val| {
            val.to_str()
                .ok()
                .and_then(|s| RawCookie::parse(s.to_owned()).ok())
        });
        store.store_response_cookies(cookies, url);
    }

    fn cookies(&self, url: &Url) -> Option<HeaderValue> {
        let cookie_string = self
            .store
            .lock()
            .get_request_values(url)
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");

        if cookie_string.is_empty() {
            None
        } else {
            HeaderValue::from_str(&cookie_string).ok()
        }
    }
}

/// 包装了 `reqwest::Client` 的 `HttpClient` 实现。
#[derive(Debug, Clone)]
pub struct ReqwestClient {
    client: reqwest::Client,
    cookie_store: Arc<SharedCookieStore>,
}

impl Default for ReqwestClient {
    fn default() -> Self {
        Self::new().expect("Failed to create default ReqwestClient")
    }
}

impl ReqwestClient {
    /// 创建一个新的 `ReqwestClient` 实例。
    pub fn new() -> Result<Self> {
        let cookie_store = Arc::new(SharedCookieStore::new());
        let builder = reqwest::Client::builder().cookie_provider(Arc::clone(&cookie_store));

        #[cfg(not(target_arch = "wasm32"))]
        let builder = builder.timeout(std::time::Duration::from_secs(10));
        #[cfg(target_arch = "wasm32")]
        let builder = builder;

        let client = builder
            .build()
            .map_err(|e| LyricsHelperError::Http(e.to_string()))?;

        Ok(Self {
            client,
            cookie_store,
        })
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl HttpClient for ReqwestClient {
    fn get_cookies(&self) -> Result<String> {
        let store = self.cookie_store.store.lock();
        let mut writer = Vec::new();
        json::save_incl_expired_and_nonpersistent(&store, &mut writer)
            .map_err(|e| LyricsHelperError::Internal(e.to_string()))?;
        Ok(String::from_utf8(writer).unwrap_or_default())
    }

    fn set_cookies(&self, cookies_json: &str) -> Result<()> {
        if cookies_json.is_empty() {
            *self.cookie_store.store.lock() = CookieStore::default();
            return Ok(());
        }
        let new_store = json::load_all(cookies_json.as_bytes())
            .map_err(|e| LyricsHelperError::Internal(e.to_string()))?;

        *self.cookie_store.store.lock() = new_store;
        Ok(())
    }

    async fn get(&self, url: &str) -> Result<HttpResponse> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| LyricsHelperError::Http(e.to_string()))?;
        convert_response(response).await
    }

    async fn post_json(&self, url: &str, json: &serde_json::Value) -> Result<HttpResponse> {
        let response = self
            .client
            .post(url)
            .json(json)
            .send()
            .await
            .map_err(|e| LyricsHelperError::Http(e.to_string()))?;
        convert_response(response).await
    }

    async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<HttpResponse> {
        let response = self
            .client
            .post(url)
            .form(form)
            .send()
            .await
            .map_err(|e| LyricsHelperError::Http(e.to_string()))?;
        convert_response(response).await
    }

    async fn request_with_headers(
        &self,
        method: HttpMethod,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<HttpResponse> {
        let mut request_builder = self.client.request(method.into(), url);

        for (key, value) in headers {
            request_builder = request_builder.header(*key, *value);
        }

        if let Some(body_data) = body {
            request_builder = request_builder.body(body_data.to_vec());
        }

        let response = request_builder
            .send()
            .await
            .map_err(|e| LyricsHelperError::Http(e.to_string()))?;

        convert_response(response).await
    }

    async fn get_with_params_and_headers(
        &self,
        url: &str,
        params: &[(&str, &str)],
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse> {
        let mut request_builder = self.client.get(url).query(params);

        for (key, value) in headers {
            request_builder = request_builder.header(*key, *value);
        }

        let response = request_builder
            .send()
            .await
            .map_err(|e| LyricsHelperError::Http(e.to_string()))?;

        convert_response(response).await
    }
}

/// 将 `reqwest::Response` 转换为自定义的 `HttpResponse`。
async fn convert_response(response: reqwest::Response) -> Result<HttpResponse> {
    let status = response.status().as_u16();
    let headers = convert_headers(response.headers());
    let body = response
        .bytes()
        .await
        .map_err(|e| LyricsHelperError::Http(e.to_string()))?
        .to_vec();

    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

/// 将 `reqwest` 的 `HeaderMap` 转换为 `HashMap<String, String>`。
fn convert_headers(header_map: &HeaderMap) -> HashMap<String, String> {
    header_map
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value_str| (name.as_str().to_string(), value_str.to_string()))
        })
        .collect()
}

impl From<HttpMethod> for reqwest::Method {
    fn from(method: HttpMethod) -> Self {
        match method {
            HttpMethod::Get => Self::GET,
            HttpMethod::Post => Self::POST,
            HttpMethod::Put => Self::PUT,
            HttpMethod::Delete => Self::DELETE,
            HttpMethod::Patch => Self::PATCH,
        }
    }
}
