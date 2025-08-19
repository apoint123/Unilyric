//! `reqwest` 客户端的默认实现。

use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::header::HeaderMap;

use crate::error::{LyricsHelperError, Result};
use crate::http::{HttpClient, HttpMethod, HttpResponse};

/// 包装了 `reqwest::Client` 的 `HttpClient` 实现。
#[derive(Debug)]
pub struct ReqwestClient {
    client: reqwest::Client,
}

impl Default for ReqwestClient {
    fn default() -> Self {
        Self::new().expect("Failed to create default ReqwestClient")
    }
}

impl ReqwestClient {
    /// 创建一个新的 `ReqwestClient` 实例。
    pub fn new() -> Result<Self> {
        let builder = reqwest::Client::builder();

        #[cfg(not(target_arch = "wasm32"))]
        let builder = { builder.timeout(std::time::Duration::from_secs(10)) };
        #[cfg(target_arch = "wasm32")]
        let builder = builder;

        let client = builder
            .build()
            .map_err(|e| LyricsHelperError::Http(e.to_string()))?;
        Ok(Self { client })
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl HttpClient for ReqwestClient {
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
    let mut headers = HashMap::new();
    for (name, value) in header_map {
        if let Ok(value_str) = value.to_str() {
            headers.insert(name.as_str().to_string(), value_str.to_string());
        }
    }
    headers
}

impl From<HttpMethod> for reqwest::Method {
    fn from(method: HttpMethod) -> Self {
        match method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Delete => reqwest::Method::DELETE,
            HttpMethod::Patch => reqwest::Method::PATCH,
        }
    }
}
