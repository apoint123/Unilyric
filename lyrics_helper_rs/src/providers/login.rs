use crate::error::Result;
use crate::model::auth::{LoginFlow, LoginMethod, ProviderAuthState};
use crate::providers::Provider;
use async_trait::async_trait;

/// 为支持登录功能的 Provider 定义的接口
#[async_trait]
pub trait LoginProvider: Provider {
    /// 发起登录流程，返回一个双向事件流
    ///
    /// # 参数
    /// * `method` - 描述要使用的登录方式，例如二维码或Cookie。
    ///
    /// # 返回
    /// 一个 `LoginFlow`，包含了 `events` 流和 `actions` 接收器。
    fn initiate_login(&self, method: LoginMethod) -> LoginFlow;

    /// 将持久化的认证状态应用到 Provider 实例。
    ///
    /// 用于在应用启动时恢复用户的登录会话。
    ///
    /// # 参数
    /// * `auth_state` - 从会话文件中反序列化出的特定于此 Provider 的认证状态。
    fn set_auth_state(&self, auth_state: &ProviderAuthState) -> Result<()>;

    /// 获取当前 Provider 实例的认证状态，用于持久化。
    ///
    /// 用于在应用关闭前保存用户的登录会话。
    ///
    /// # 返回
    /// 如果用户已登录，则返回 `Some(ProviderAuthState)`，否则返回 `None`。
    fn get_auth_state(&self) -> Option<ProviderAuthState>;

    /// 验证当前会话是否仍然有效。
    ///
    /// 应该在 `import_session` 之后被调用，以确保登陆状态有效。
    ///
    /// # 返回
    /// * `Ok(())` - 如果会话有效。
    /// * `Err(LyricsHelperError::LoginFailed(_))` - 如果会话已过期或无效。
    async fn verify_session(&self) -> Result<()>;
}
