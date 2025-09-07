use lyrics_helper_rs::{
    http::WreqClient,
    model::auth::ProviderAuthState,
    providers::{
        Provider,
        qq::{QQMusic, models::QRCodeStatus},
    },
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::time::sleep;

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

    println!("正在获取登录二维码...");
    let qr_info_result = provider.get_qrcode().await;
    assert!(qr_info_result.is_ok(), "获取二维码失败");
    let qr_info = qr_info_result.unwrap();
    assert!(!qr_info.image_data.is_empty(), "二维码图片数据不应为空");
    assert!(!qr_info.qrsig.is_empty(), "qrsig不应为空");

    let qr_code_path = "qrcode.png";
    let write_result = std::fs::write(qr_code_path, &qr_info.image_data);
    assert!(write_result.is_ok(), "保存二维码图片到文件失败");

    let start_time = Instant::now();
    let timeout = Duration::from_secs(60);

    let final_redirect_url = loop {
        assert!((start_time.elapsed() <= timeout), "登录超时(超过60秒)");

        let status_result = provider.check_qrcode_status(&qr_info.qrsig).await;
        assert!(status_result.is_ok(), "轮询二维码状态失败");
        let status = status_result.unwrap();

        match status {
            QRCodeStatus::WaitingForScan => {
                println!("等待扫描...");
            }
            QRCodeStatus::Scanned => {
                println!("已扫描, 等待确认...");
            }
            QRCodeStatus::Confirmed { url } => {
                println!("已确认登录");
                break url;
            }
            QRCodeStatus::TimedOut => {
                panic!("二维码已过期");
            }
            QRCodeStatus::Refused => {
                panic!("用户拒绝登录");
            }
            QRCodeStatus::Error(e) => {
                panic!("轮询时发生错误: {e}");
            }
        }

        sleep(Duration::from_secs(2)).await;
    };

    let final_auth_state_result = provider.finalize_login_with_url(&final_redirect_url).await;
    assert!(final_auth_state_result.is_ok(), "使用URL完成最终登录失败");

    let final_auth_state = final_auth_state_result.unwrap();
    println!("登录成功！");

    if let ProviderAuthState::QQMusic {
        musicid, musickey, ..
    } = final_auth_state
    {
        assert!(musicid > 0, "musicid 无效");
        assert!(!musickey.is_empty(), "musickey 不应为空");
        println!("获取到的 musicid: {musicid}");
    } else {
        panic!("获取到的认证信息类型不正确");
    }
}
