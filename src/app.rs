use crate::app_definition::UniLyricApp;
use crate::app_update;
use eframe::egui::{self};
use std::time::Duration;

/// TTML 数据库上传用户操作的枚举
#[derive(Clone, Debug)]
pub enum TtmlDbUploadUserAction {
    /// dpaste 已创建，URL已复制到剪贴板，这是打开Issue页面的URL
    PasteReadyAndCopied {
        paste_url: String,                // dpaste 的 URL
        github_issue_url_to_open: String, // GitHub Issue 页面的 URL
    },
    /// 过程中的提示信息
    InProgressUpdate(String),
    /// 准备阶段错误
    PreparationError(String),
    /// 错误信息
    Error(String),
}

impl eframe::App for UniLyricApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 仅供调试，不要开启！
        // ctx.set_debug_on_hover(true);

        app_update::handle_conversion_results(self);
        app_update::handle_search_results(self);
        app_update::handle_download_results(self);

        app_update::handle_provider_load_results(self);

        app_update::process_log_messages(self);
        app_update::handle_auto_fetch_results(self);

        app_update::process_connector_updates(self);

        // let mut desired_repaint_delay = Duration::from_millis(1000);
        // if self.amll_connector.config.lock().unwrap().enabled {
        //     desired_repaint_delay = desired_repaint_delay.min(Duration::from_millis(100));
        // }
        // ctx.request_repaint_after(desired_repaint_delay);

        ctx.request_repaint_after(Duration::from_millis(1000));

        app_update::draw_ui_elements(self, ctx);
        app_update::handle_file_drops(self, ctx);
        app_update::handle_ttml_db_upload_actions(self);

        self.draw_search_lyrics_window(ctx);
        self.ui.toasts.show(ctx);

        let actions = std::mem::take(&mut self.actions_this_frame);

        if !actions.is_empty() {
            self.handle_actions(actions);
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.shutdown_initiated {
            self.shutdown_initiated = true;
            tracing::debug!("[Shutdown] 检测到窗口关闭请求，发送关闭信号...");

            self.send_shutdown_signals();
        }
    }
}
