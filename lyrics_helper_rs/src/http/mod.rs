//! HTTP客户端抽象层，用于解耦不同环境下的HTTP请求实现。

use std::fmt::Debug;

use async_trait::async_trait;
use serde::de::DeserializeOwned;

use crate::error::{LyricsHelperError, Result};

pub mod wreq_client;

pub use self::wreq_client::WreqClient;

/// HTTP请求方法枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    /// GET方法
    Get,
    /// POST方法
    Post,
    /// PUT方法
    Put,
    /// DELETE方法
    Delete,
    /// PATCH方法
    Patch,
}

/// 统一的HTTP响应数据结构
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP状态码
    pub status: u16,
    /// 响应头
    pub headers: Vec<(String, String)>,
    /// 响应体
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// 将响应体解析为UTF-8字符串
    pub fn text(&self) -> Result<String> {
        String::from_utf8(self.body.clone()).map_err(|e| LyricsHelperError::Encoding(e.to_string()))
    }

    /// 将响应体解析为JSON对象
    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(|e| LyricsHelperError::Parser(e.to_string()))
    }

    /// 获取原始的响应体字节
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.body
    }
}

/// 统一的HTTP客户端接口
#[async_trait]
pub trait HttpClient: Send + Sync + Debug {
    /// 发送GET请求
    async fn get(&self, url: &str) -> Result<HttpResponse>;

    /// 发送POST请求，携带JSON数据
    async fn post_json(&self, url: &str, json: &serde_json::Value) -> Result<HttpResponse>;

    /// 发送POST请求，携带表单数据
    async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<HttpResponse>;

    /// 发送带自定义headers的请求
    async fn request_with_headers(
        &self,
        method: HttpMethod,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<HttpResponse>;

    /// 发送带查询参数和自定义头部的GET请求
    async fn get_with_params_and_headers(
        &self,
        url: &str,
        params: &[(&str, &str)],
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse> {
        let query_string = serde_urlencoded::to_string(params)
            .map_err(|e| LyricsHelperError::Internal(format!("无法对查询参数进行编码: {e}")))?;
        let full_url = format!("{url}?{query_string}");

        self.request_with_headers(HttpMethod::Get, &full_url, headers, None)
            .await
    }

    /// 导出当前 `HttpClient` 中的所有 Cookies 为 JSON 字符串
    fn get_cookies(&self) -> Result<String>;

    /// 从 JSON 字符串中导入 Cookies，覆盖现有状态
    fn set_cookies(&self, cookies_json: &str) -> Result<()>;

    /// 发送POST表单请求，但不遵循重定向，而是返回 Location 头
    async fn post_form_for_redirect(&self, url: &str, form: &[(&str, &str)]) -> Result<String>;
}
