use crate::ProviderName;
use futures::Sink;
use futures_core::stream::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use std::time::Duration;
use thiserror::Error;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    /// 用户的唯一 ID
    pub user_id: i64,
    /// 用户的昵称
    pub nickname: String,
    /// 用户头像的 URL
    pub avatar_url: String,
}

#[derive(Debug, Error, Clone)]
pub enum LoginError {
    #[error("请求超时")]
    TimedOut,
    #[error("用户取消了操作")]
    UserCancelled,
    #[error("网络错误: {0}")]
    Network(String),
    #[error("无效的凭据: {0}")]
    InvalidCredentials(String),
    #[error("提供商 API 错误: {0}")]
    ProviderError(String),
    #[error("内部错误: {0}")]
    Internal(String),
}

pub struct LoginFlow {
    pub events: Pin<Box<dyn Stream<Item = LoginEvent> + Send>>,
    pub actions: Pin<Box<dyn Sink<LoginAction, Error = LoginError> + Send>>,
}

#[derive(Debug)]
pub enum LoginEvent {
    Initiating,
    QRCodeReady { image_data: Vec<u8> },
    WaitingForScan,
    ScannedWaitingForConfirmation,
    SmsCodeSent { expires_in: Duration },
    SmsCodeInvalid,
    Success(LoginResult),
    Failure(LoginError),
}

#[derive(Debug)]
pub enum LoginAction {
    SubmitSmsCode(String),
    Cancel,
}

#[derive(Debug)]
pub enum LoginMethod {
    QQMusicByCookie { cookies: String },
    QQMusicByQRCode,
    NeteaseByCookie { music_u: String },
    SmsRequestCode { phone_number: String },
}

#[derive(Clone, Debug)]
pub struct LoginResult {
    pub profile: UserProfile,
    pub auth_state: ProviderAuthState,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum ProviderAuthState {
    QQMusic {
        musicid: u64,
        musickey: String,
        refresh_key: Option<String>,
        encrypt_uin: Option<String>,
    },
    Netease {
        cookies_json: String,
    },
}

#[derive(Serialize, Deserialize, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSession {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cookies: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_state: Option<ProviderAuthState>,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Session {
    pub provider_sessions: HashMap<ProviderName, ProviderSession>,
}
