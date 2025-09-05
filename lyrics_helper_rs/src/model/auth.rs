use crate::ProviderName;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

#[derive(Clone, Debug)]
pub enum LoginCredentials<'a> {
    NeteaseByCookie { music_u: &'a str },
}

#[derive(Clone, Debug)]
pub struct LoginResult {
    pub profile: UserProfile,
    pub auth_state: ProviderAuthState,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ProviderAuthState {
    QQMusic { qimei36: String, qimei_key: String },
    Netease,
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
