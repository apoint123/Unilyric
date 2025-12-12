use crate::app_actions::{LyricsAction, UserAction};
use crate::app_definition::UniLyricApp;
use eframe::egui;
use lyrics_helper_core::CanonicalMetadataKey;
use std::str::FromStr;
use strum::IntoEnumIterator;

pub fn draw_metadata_editor_window_contents(
    app: &mut UniLyricApp,
    ui: &mut egui::Ui,
    _open: &mut bool,
) {
    let mut actions_to_send = Vec::new();

    egui::ScrollArea::vertical().show(ui, |scroll_ui| {
        if app.lyrics.metadata_manager.ui_entries.is_empty() {
            scroll_ui
                .label(egui::RichText::new("无元数据可编辑。\n可从文件加载，或手动添加。").weak());
            return;
        }

        let mut deletion_index: Option<usize> = None;
        let mut previous_key: Option<&CanonicalMetadataKey> = None;

        for (index, entry) in app
            .lyrics
            .metadata_manager
            .ui_entries
            .iter_mut()
            .enumerate()
        {
            let item_id = entry.id;
            let is_first_in_group = previous_key != Some(&entry.key);
            if is_first_in_group && index > 0 {
                scroll_ui.separator();
            }
            scroll_ui.horizontal(|row_ui| {
                if row_ui.checkbox(&mut entry.is_pinned, "").changed() {
                    actions_to_send.push(UserAction::Lyrics(Box::new(
                        LyricsAction::ToggleMetadataPinned,
                    )));
                }
                row_ui
                    .label("固定")
                    .on_hover_text("勾选后, 此条元数据在加载新歌词时将尝试保留其值");

                let key_editor_width = row_ui.available_width() * 0.3;
                let mut key_changed_this_frame = false;

                if is_first_in_group {
                    row_ui.add_space(5.0);
                    row_ui.label("键:");
                    if let CanonicalMetadataKey::Custom(custom_key_str) = &mut entry.key {
                        let response = row_ui.add_sized(
                            [key_editor_width, 0.0],
                            egui::TextEdit::singleline(custom_key_str)
                                .id_salt(item_id.with("key_edit_custom")),
                        );
                        if response.lost_focus() && response.changed() {
                            if let Ok(parsed_key) = CanonicalMetadataKey::from_str(custom_key_str) {
                                entry.key = parsed_key;
                            }
                            key_changed_this_frame = true;
                        }
                    } else {
                        egui::ComboBox::from_id_salt(item_id.with("key_combo"))
                            .selected_text(entry.key.to_string())
                            .width(key_editor_width)
                            .show_ui(row_ui, |combo_ui| {
                                for key_variant in CanonicalMetadataKey::iter() {
                                    if combo_ui
                                        .selectable_value(
                                            &mut entry.key,
                                            key_variant.clone(),
                                            key_variant.to_string(),
                                        )
                                        .changed()
                                    {
                                        key_changed_this_frame = true;
                                    }
                                }
                                combo_ui.separator();
                                if combo_ui.selectable_label(false, "自定义").clicked() {
                                    entry.key = CanonicalMetadataKey::Custom("custom".to_string());
                                    key_changed_this_frame = true;
                                }
                            });
                    }
                } else {
                    let style = row_ui.style();
                    let space_for_pin_label = row_ui.text_style_height(&egui::TextStyle::Body);
                    let space_for_key_label =
                        style.spacing.item_spacing.x + style.spacing.interact_size.x;

                    row_ui.add_space(space_for_pin_label + space_for_key_label + key_editor_width);
                }

                if key_changed_this_frame {
                    actions_to_send.push(UserAction::Lyrics(Box::new(
                        LyricsAction::UpdateMetadataKey,
                    )));
                }

                row_ui.add_space(5.0);
                row_ui.label("值:");
                let value_edit_response = row_ui.add(
                    egui::TextEdit::singleline(&mut entry.value)
                        .id_salt(item_id.with("value_edit"))
                        .hint_text("元数据值"),
                );
                if value_edit_response.lost_focus() {
                    actions_to_send.push(UserAction::Lyrics(Box::new(
                        LyricsAction::UpdateMetadataValue,
                    )));
                }

                if row_ui.button("🗑").on_hover_text("删除此条元数据").clicked() {
                    deletion_index = Some(index);
                }
            });
            previous_key = Some(&entry.key);
        }

        if let Some(index_to_delete) = deletion_index {
            actions_to_send.push(UserAction::Lyrics(Box::new(LyricsAction::DeleteMetadata(
                index_to_delete,
            ))));
        }

        scroll_ui.separator();

        scroll_ui.menu_button("添加新元数据...", |menu| {
            for key_variant in CanonicalMetadataKey::iter() {
                if menu.button(key_variant.to_string()).clicked() {
                    actions_to_send.push(UserAction::Lyrics(Box::new(LyricsAction::AddMetadata(
                        key_variant,
                    ))));
                    menu.close();
                }
            }
            menu.separator();
            if menu.button("自定义键").clicked() {
                actions_to_send.push(UserAction::Lyrics(Box::new(LyricsAction::AddMetadata(
                    CanonicalMetadataKey::Custom("custom".to_string()),
                ))));
                menu.close();
            }
        });
    });

    for action in actions_to_send {
        app.send_action(action);
    }
}
