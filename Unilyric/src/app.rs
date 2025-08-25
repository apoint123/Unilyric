use crate::app_definition::UniLyricApp;
use crate::app_update;
use eframe::egui::{self};
use std::time::Duration;

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

        if let Some(trigger_time) = self.auto_fetch_trigger_time {
            if std::time::Instant::now() >= trigger_time {
                self.auto_fetch_trigger_time = None;

                let track_info = self.player.current_now_playing.clone();
                if track_info.title.as_deref().is_some() {
                    crate::app_fetch_core::initial_auto_fetch_and_send_lyrics(self, track_info);
                }
            } else {
                ctx.request_repaint_after(trigger_time - std::time::Instant::now());
            }
        }

        // let mut desired_repaint_delay = Duration::from_millis(1000);
        // if self.amll_connector.config.lock().unwrap().enabled {
        //     desired_repaint_delay = desired_repaint_delay.min(Duration::from_millis(100));
        // }
        // ctx.request_repaint_after(desired_repaint_delay);

        ctx.request_repaint_after(Duration::from_millis(1000));

        app_update::draw_ui_elements(self, ctx);
        app_update::handle_file_drops(self, ctx);

        self.draw_search_lyrics_window(ctx);
        self.ui.toasts.show(ctx);

        while let Ok(action) = self.action_rx.try_recv() {
            self.actions_this_frame.push(action);
        }

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
