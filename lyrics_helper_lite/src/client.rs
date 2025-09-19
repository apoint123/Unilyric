use crate::error::Result;
use async_trait::async_trait;
use reqwest::{Client, header};

#[async_trait(?Send)]
pub trait HttpClient {
    async fn get(&self, url: &str) -> Result<String>;
    async fn post_json(&self, url: &str, body: &str) -> Result<String>;
    async fn post_form(&self, url: &str, body: &str) -> Result<String>;
}

fn create_client() -> Client {
    let mut headers = header::HeaderMap::new();
    headers.insert(header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/108.0.0.0 Safari/537.36".parse().unwrap());
    headers.insert(header::REFERER, "https://y.qq.com/".parse().unwrap());

    Client::builder().default_headers(headers).build().unwrap()
}

pub struct ReqwestClient(Client);

impl ReqwestClient {
    #[must_use]
    pub fn new() -> Self {
        Self(create_client())
    }
}

impl Default for ReqwestClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait(?Send)]
impl HttpClient for ReqwestClient {
    async fn get(&self, url: &str) -> Result<String> {
        Ok(self.0.get(url).send().await?.text().await?)
    }

    async fn post_json(&self, url: &str, body: &str) -> Result<String> {
        Ok(self
            .0
            .post(url)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.to_owned())
            .send()
            .await?
            .text()
            .await?)
    }

    async fn post_form(&self, url: &str, body: &str) -> Result<String> {
        Ok(self
            .0
            .post(url)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body.to_owned())
            .send()
            .await?
            .text()
            .await?)
    }
}

#[must_use]
pub fn create_http_client() -> Box<dyn HttpClient> {
    Box::new(ReqwestClient::new())
}
