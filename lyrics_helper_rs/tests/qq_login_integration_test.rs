#![allow(dead_code)]

use futures_util::StreamExt;
use lyrics_helper_rs::{
    http::WreqClient,
    model::auth::{LoginEvent, LoginMethod, ProviderAuthState},
    providers::{LoginProvider, Provider, qq::QQMusic},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tracing::info;

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, FmtSubscriber};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("trace"));
    let _ = FmtSubscriber::builder()
        .with_env_filter(filter)
        .with_test_writer()
        .try_init();
}

// #[tokio::test]
#[ignore]
async fn test_full_qr_code_login_flow() {
    init_tracing();
    let http_client = Arc::new(WreqClient::new().unwrap());
    let provider = QQMusic::with_http_client(http_client).await.unwrap();

    info!("正在启动二维码登录流程...");
    let method = LoginMethod::QQMusicByQRCode;
    let mut flow = provider.initiate_login(method);

    let start_time = Instant::now();
    let timeout = Duration::from_secs(60);

    while let Some(event) = flow.events.next().await {
        assert!(start_time.elapsed() <= timeout, "登录超时(超过60秒)");

        match event {
            LoginEvent::Initiating => {
                info!("流程已启动...");
            }
            LoginEvent::QRCodeReady { image_data } => {
                info!("二维码已就绪，请扫描文件 'qrcode.png'。");
                assert!(!image_data.is_empty(), "二维码图片数据不应为空");
                std::fs::write("qrcode.png", &image_data).expect("保存二维码图片到文件失败");
            }
            LoginEvent::WaitingForScan => {
                info!("等待扫描...");
            }
            LoginEvent::ScannedWaitingForConfirmation => {
                info!("已扫描，等待确认...");
            }
            LoginEvent::Success(login_result) => {
                info!("登录成功");
                if let ProviderAuthState::QQMusic {
                    musicid, musickey, ..
                } = login_result.auth_state
                {
                    assert!(musicid > 0, "musicid 无效");
                    assert!(!musickey.is_empty(), "musickey 不应为空");
                    println!("获取到的 musicid: {musicid}");
                } else {
                    panic!("获取到的认证信息类型不正确");
                }
                return;
            }
            LoginEvent::Failure(e) => {
                panic!("登录流程失败: {e:?}");
            }
            _ => {}
        }
    }

    panic!("登录事件流意外结束，未收到 Success 或 Failure 事件。");
}
