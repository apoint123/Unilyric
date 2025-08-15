//! # TTML 生成器 - 工具函数模块
//!
//! 该模块提供了 TTML 生成过程中所需的各种辅助函数。

/// 将毫秒时间戳格式化为 TTML 标准的时间字符串。
/// 例如：123456ms -> "2:03.456"
pub(super) fn format_ttml_time(ms: u64) -> String {
    let hours = ms / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    let seconds = (ms % 60_000) / 1000;
    let millis = ms % 1000;

    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}.{millis:03}")
    } else if minutes > 0 {
        format!("{minutes}:{seconds:02}.{millis:03}")
    } else {
        format!("{seconds}.{millis:03}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_ttml_time() {
        assert_eq!(format_ttml_time(3_723_456), "1:02:03.456");
        assert_eq!(format_ttml_time(310_100), "5:10.100");
        assert_eq!(format_ttml_time(7123), "7.123");
        assert_eq!(format_ttml_time(0), "0.000");
        assert_eq!(format_ttml_time(59999), "59.999");
        assert_eq!(format_ttml_time(60000), "1:00.000");
    }
}
