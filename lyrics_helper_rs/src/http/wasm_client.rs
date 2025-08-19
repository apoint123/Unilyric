use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::collections::HashMap;

use super::{HttpClient, HttpMethod, HttpResponse};
use crate::error::{LyricsHelperError, Result};

#[derive(Debug)]
pub struct WasmClient {
    client: reqwest::Client,
    proxy_url: Option<String>,
}

impl WasmClient {
    pub fn new(proxy_url: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            proxy_url,
        }
    }
}

#[async_trait(?Send)]
impl HttpClient for WasmClient {
    async fn get(&self, url: &str) -> Result<HttpResponse> {
        self.request_with_headers(HttpMethod::Get, url, &[], None)
            .await
    }

    async fn post_json(&self, url: &str, json: &serde_json::Value) -> Result<HttpResponse> {
        let body_bytes =
            serde_json::to_vec(json).map_err(|e| LyricsHelperError::Parser(e.to_string()))?;
        self.request_with_headers(
            HttpMethod::Post,
            url,
            &[("Content-Type", "application/json")],
            Some(&body_bytes),
        )
        .await
    }

    async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<HttpResponse> {
        let body_str = serde_urlencoded::to_string(form)
            .map_err(|e| LyricsHelperError::Parser(e.to_string()))?;
        self.request_with_headers(
            HttpMethod::Post,
            url,
            &[("Content-Type", "application/x-www-form-urlencoded")],
            Some(body_str.as_bytes()),
        )
        .await
    }

    async fn request_with_headers(
        &self,
        method: HttpMethod,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<HttpResponse> {
        let final_url = if let Some(proxy) = &self.proxy_url {
            format!("{}{}", proxy, url)
        } else {
            url.to_string()
        };

        let req_method = match method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
            HttpMethod::Put => reqwest::Method::PUT,
            HttpMethod::Delete => reqwest::Method::DELETE,
            HttpMethod::Patch => reqwest::Method::PATCH,
        };

        let mut header_map = HeaderMap::new();
        for (key, value) in headers {
            let header_name = HeaderName::from_bytes(key.as_bytes())
                .map_err(|e| LyricsHelperError::Internal(format!("无效的 Header key: {}", e)))?;
            let header_value = HeaderValue::from_str(value)
                .map_err(|e| LyricsHelperError::Internal(format!("无效的 Header value: {}", e)))?;
            header_map.insert(header_name, header_value);
        }

        let mut request_builder = self
            .client
            .request(req_method, &final_url)
            .headers(header_map);
        if let Some(body_data) = body {
            request_builder = request_builder.body(body_data.to_vec());
        }

        let response = request_builder
            .send()
            .await
            .map_err(|e| LyricsHelperError::Http(e.to_string()))?;

        let status = response.status().as_u16();
        let response_headers: HashMap<String, String> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let body = response
            .bytes()
            .await
            .map_err(|e| LyricsHelperError::Http(e.to_string()))?
            .to_vec();

        Ok(HttpResponse {
            status,
            headers: response_headers,
            body,
        })
    }
}
