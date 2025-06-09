use crate::amll_connector::amll_connector_manager::{self, run_progress_timer_task};
use crate::amll_connector::{ConnectorCommand, WebsocketStatus};
use crate::amll_lyrics_fetcher::{AmllIndexEntry, FetchedAmllTtmlLyrics};
use crate::app_definition::UniLyricApp;
use crate::app_fetch_core;
use crate::app_update;
use crate::ass_parser;
use crate::json_parser;
use crate::krc_parser;
use crate::kugou_lyrics_fetcher;
use crate::lrc_parser;
use crate::lyric_processor;
use crate::lyricify_lines_parser;
use crate::lyricify_lines_to_ttml_data;
use crate::lyrics_merger;
use crate::lys_parser;
use crate::lys_to_ttml_data;
use crate::metadata_processor::MetadataStore;
use crate::netease_lyrics_fetcher;
use crate::qq_lyrics_fetcher;
use crate::qrc_parser;
use crate::qrc_to_ttml_data;
use crate::spl_parser;
use crate::ttml_generator;
use crate::ttml_parser;
use crate::types::{
    AmllIndexDownloadState, AmllTtmlDownloadState, AssMetadata, AutoSearchSource, AutoSearchStatus,
    CanonicalMetadataKey, ConvertError, DisplayLrcLine, EditableMetadataEntry, KrcDownloadState,
    LocalLyricCacheEntry, LrcContentType, LrcLine, LyricFormat, LysSyllable, NeteaseDownloadState,
    ParsedSourceData, PlatformFetchedData, ProcessedAssData, ProcessedLyricsSourceData,
    QqMusicDownloadState, TtmlParagraph, TtmlSyllable,
};
use crate::websocket_server::{PlaybackInfoPayload, ServerCommand, TimeUpdatePayload};
use crate::yrc_parser;
use crate::yrc_to_ttml_data;
use eframe::egui::{self};
use egui_toast::{Toast, ToastKind, ToastOptions};
use log::{info, warn};
use rand::Rng;
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ws_protocol::Body as ProtocolBody;

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

// UniLyricApp 的实现块
impl UniLyricApp {
    /// 将UI元数据编辑器中的当前状态同步回内部的 `MetadataStore`，
    /// 更新 `persistent_canonical_keys` 集合，并将固定的元数据保存到应用设置中，
    /// 最后触发目标格式歌词的重新生成。
    pub fn sync_store_from_editable_list_and_trigger_conversion(&mut self) {
        // 使用代码块限制锁的范围
        {
            // 获取元数据存储的可写锁
            let mut store = self.metadata_store.lock().unwrap();
            store.clear(); // 清空当前的元数据存储

            self.persistent_canonical_keys.clear(); // 清空持久化（固定）的规范键集合
            let mut current_pinned_for_settings: HashMap<String, Vec<String>> = HashMap::new(); // 用于保存到设置的固定元数据

            // 遍历UI上的可编辑元数据列表
            for entry_ui in &self.editable_metadata {
                if !entry_ui.key.trim().is_empty() {
                    // 确保键不为空
                    // 尝试将UI条目添加到内部存储
                    if let Err(e) = store.add(&entry_ui.key, entry_ui.value.clone()) {
                        log::warn!(
                            "[Unilyric UI同步] 添加元数据 '{}' 到Store失败: {}",
                            entry_ui.key,
                            e
                        );
                    }

                    // 如果此条目被标记为固定
                    if entry_ui.is_pinned {
                        let value_to_pin = entry_ui.value.clone(); // 获取要固定的值

                        // 尝试将UI上的键解析为规范元数据键
                        match entry_ui.key.trim().parse::<CanonicalMetadataKey>() {
                            Ok(canonical_key) => {
                                // 如果是规范键
                                let key_for_settings = canonical_key.to_display_key(); // 获取用于设置的显示键

                                // 将固定项添加到待保存的哈希图中
                                current_pinned_for_settings
                                    .entry(key_for_settings)
                                    .or_default()
                                    .push(value_to_pin);

                                // 将规范键添加到持久化键集合中
                                self.persistent_canonical_keys.insert(canonical_key);
                            }
                            Err(_) => {
                                // 如果是自定义键
                                let custom_key_for_settings = entry_ui.key.trim().to_string();
                                current_pinned_for_settings
                                    .entry(custom_key_for_settings.clone())
                                    .or_default()
                                    .push(value_to_pin);

                                // 将自定义键（包装在CanonicalMetadataKey::Custom中）添加到持久化键集合
                                self.persistent_canonical_keys
                                    .insert(CanonicalMetadataKey::Custom(custom_key_for_settings));
                            }
                        }
                    }
                }
            }

            // 保存固定元数据到应用设置
            {
                let mut app_settings_locked = self.app_settings.lock().unwrap();
                app_settings_locked.pinned_metadata = current_pinned_for_settings; // 更新设置中的固定元数据
                // 尝试保存设置文件
                if let Err(e) = app_settings_locked.save() {
                    log::error!("[Unilyric UI同步] 保存固定元数据到设置文件失败: {}", e);
                } else {
                    log::info!(
                        "[Unilyric UI同步] 已将 {} 个键的固定元数据保存到设置文件。",
                        app_settings_locked.pinned_metadata.len()
                    );
                }
            }
        } //元数据存储的锁在此释放

        log::info!(
            "[Unilyric UI同步] MetadataStore已从UI编辑器同步。固定键类型数量: {}. 总元数据条目数量: {}",
            self.persistent_canonical_keys.len(),
            self.metadata_store.lock().unwrap().iter_all().count()
        );
        log::trace!(
            "[Unilyric UI同步] 当前固定的元数据键类型列表: {:?}",
            self.persistent_canonical_keys
        );

        // 如果解析后的TTML段落存在，或者元数据存储不为空，则触发目标格式的生成
        let store_is_empty = self.metadata_store.lock().unwrap().is_empty();
        if self.parsed_ttml_paragraphs.is_some() || !store_is_empty {
            self.generate_target_format_output();
        }
    }

    /// 根据当前的 `MetadataStore` 重建UI上显示的元数据编辑列表 (`self.editable_metadata`)。
    ///
    /// 此函数在以下情况被调用：
    /// 1. 应用初始化时 (`new` 函数末尾)。
    /// 2. 从文件加载或网络下载歌词并解析元数据后 (`update_app_state_from_parsed_data` 函数末尾)。
    /// 3. 用户在元数据编辑器中手动添加/删除条目后，或更改固定状态后，可能需要刷新列表以正确排序和显示状态。
    ///
    /// 主要逻辑：
    /// - 遍历 `self.metadata_store` 中的所有元数据项。
    /// - 为每个存储的元数据项创建一个 `EditableMetadataEntry` 对象。
    ///   - `key`: 使用 `CanonicalMetadataKey::to_display_key()` 获取规范的显示键名。
    ///   - `value`: 存储的值。
    ///   - `is_pinned`: 通过检查 `canonical_key_from_store` 是否存在于 `self.persistent_canonical_keys` 集合中来确定。
    ///     这确保了UI上的“固定”状态与内部的持久化意图一致。
    ///   - `is_from_file`: 初始标记为 `true`，表示这些条目是直接从当前的权威数据源（`MetadataStore`）加载的。
    ///   - `id`: 为每个条目生成一个唯一的 `egui::Id`，用于UI元素的追踪。
    /// - 对新生成的 `EditableMetadataEntry` 列表进行排序：
    ///   - 固定项 (`is_pinned == true`) 排在非固定项之前。
    ///   - 同为固定项或同为非固定项时，按键名（不区分大小写）的字母顺序排序。
    /// - 最后，用这个新生成的、排序后的列表替换 `self.editable_metadata`。
    pub fn rebuild_editable_metadata_from_store(&mut self) {
        // 获取 MetadataStore 的只读锁
        let store_guard = self.metadata_store.lock().unwrap();
        // 初始化一个新的可编辑元数据列表
        let mut new_editable_list: Vec<EditableMetadataEntry> = Vec::new();
        // 用于为 egui::Id 生成唯一后缀的计数器
        let mut id_seed_counter = 0;

        // 遍历 MetadataStore 中的所有元数据项。
        // store_guard.iter_all() 返回一个迭代器，其元素是 (CanonicalMetadataKey, &Vec<String>)
        // 即每个规范化的键及其对应的值列表。
        for (canonical_key_from_store, values_vec) in store_guard.iter_all() {
            // 获取该规范键对应的用户友好显示名称 (例如，CanonicalMetadataKey::Title -> "Title")
            let display_key_name = canonical_key_from_store.to_display_key();

            // MetadataStore 可能为一个键存储多个值，所以遍历值列表
            for value_str in values_vec {
                id_seed_counter += 1; // 增加ID种子，确保每个条目的ID唯一
                // 为UI条目创建一个唯一的egui ID，这对于egui正确处理用户交互（如文本框编辑）很重要。
                // ID基于显示键名和计数器生成，替换特殊字符以避免ID格式问题。
                let new_id = egui::Id::new(format!(
                    "editable_meta_{}_{}",
                    display_key_name.replace([':', '/', ' '], "_"), // 替换键名中的特殊字符
                    id_seed_counter
                ));

                // 创建一个新的 EditableMetadataEntry 实例
                new_editable_list.push(EditableMetadataEntry {
                    key: display_key_name.clone(), // UI上显示的键名
                    value: value_str.clone(),      // UI上显示/编辑的值
                    // 核心逻辑：判断此条目在UI上是否应显示为“固定”。
                    // 这是通过检查其规范键是否存在于 self.persistent_canonical_keys 集合中来决定的。
                    // self.persistent_canonical_keys 反映了用户当前希望哪些类型的元数据是固定的。
                    is_pinned: self
                        .persistent_canonical_keys
                        .contains(canonical_key_from_store),
                    is_from_file: true, // 标记此条目是直接从 MetadataStore 加载的
                    id: new_id,         // egui 用的唯一ID
                });
            }
        }

        // 对新构建的可编辑元数据列表进行排序。
        // 排序规则：
        // 1. “固定”的条目 (is_pinned == true) 排在前面。
        // 2. 在“固定”或“非固定”的组内，按键名 (key) 的字母顺序（不区分大小写）排序。
        new_editable_list.sort_by(|a, b| {
            match (a.is_pinned, b.is_pinned) {
                (true, false) => std::cmp::Ordering::Less, // a是固定的，b不是，则a排在前面
                (false, true) => std::cmp::Ordering::Greater, // a不是固定的，b是，则b排在前面
                _ => a.key.to_lowercase().cmp(&b.key.to_lowercase()), // 两者固定状态相同，按键名排序
            }
        });

        // 用新生成的、排序好的列表替换 self.editable_metadata
        self.editable_metadata = new_editable_list;
        log::info!(
            "[Unilyric] 已从内部存储重建UI元数据列表。共 {} 个条目。",
            self.editable_metadata.len()
        );
    }

    /// 将当前歌词保存到本地缓存。
    pub fn save_current_lyrics_to_local_cache(&mut self) {
        // 检查输出文本是否为空
        if self.output_text.is_empty() {
            log::warn!("[本地缓存] 输出文本为空，无法保存到本地缓存。");
            self.toasts.add(Toast {
                text: "缓存失败：无歌词内容".into(),
                kind: ToastKind::Error,
                options: ToastOptions::default()
                    .duration_in_seconds(3.0)
                    .show_progress(true),
                style: Default::default(),
            });
            return;
        }

        // 尝试获取当前媒体信息的锁
        let current_media_info_guard = match self.current_media_info.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                log::error!("[本地缓存] 无法获取当前媒体信息锁，无法保存。");
                self.toasts.add(Toast {
                    text: "缓存失败：无法获取播放信息".into(),
                    kind: ToastKind::Error,
                    options: ToastOptions::default()
                        .duration_in_seconds(3.0)
                        .show_progress(true),
                    style: Default::default(),
                });
                return;
            }
        };

        // 从媒体信息中提取标题和艺术家
        let (smtc_title_opt, smtc_artists_str_opt) = match &*current_media_info_guard {
            Some(info) => (info.title.clone(), info.artist.clone()),
            None => {
                log::error!("[本地缓存] 无当前 SMTC 信息，无法确定歌曲以保存缓存。");
                self.toasts.add(Toast {
                    text: "缓存失败：无SMTC信息".into(),
                    kind: ToastKind::Error,
                    options: ToastOptions::default()
                        .duration_in_seconds(3.0)
                        .show_progress(true),
                    style: Default::default(),
                });
                return;
            }
        };
        drop(current_media_info_guard); // 释放锁

        // 校验歌曲标题
        let title_to_save = match smtc_title_opt {
            Some(t) if !t.is_empty() && t != "无歌曲" && t != "无活动会话" => t,
            _ => {
                log::error!("[本地缓存] 无有效的 SMTC 歌曲标题，无法保存缓存。");
                self.toasts.add(Toast {
                    text: "缓存失败：歌曲标题无效".into(),
                    kind: ToastKind::Error,
                    options: ToastOptions::default()
                        .duration_in_seconds(3.0)
                        .show_progress(true),
                    style: Default::default(),
                });
                return;
            }
        };

        // 处理艺术家列表
        let artists_to_save: Vec<String> = smtc_artists_str_opt
            .map(|s| {
                s.split(['/', '、', ',', ';']) // 按多种分隔符分割
                    .map(|name| name.trim().to_string()) // 去除首尾空格
                    .filter(|name| !name.is_empty()) // 过滤空艺术家名
                    .collect()
            })
            .unwrap_or_else(Vec::new); // 如果没有艺术家信息，则为空Vec

        // 生成随机ID和时间戳，用于文件名
        let mut rng = rand::rng(); // 使用线程本地随机数生成器
        let random_id: u32 = rng.random_range(0..u32::MAX); // 生成 u32 范围内的随机数
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        // 创建安全的文件名组件 (只保留字母数字和空格，并将空格替换为下划线)
        let safe_title = title_to_save
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ')
            .collect::<String>()
            .replace(' ', "_");
        let safe_artist = artists_to_save.first().map_or_else(
            || "未知艺术家".to_string(), // 如果没有艺术家，默认为 "未知艺术家"
            |a| {
                a.chars()
                    .filter(|c| c.is_alphanumeric() || *c == ' ')
                    .collect::<String>()
                    .replace(' ', "_")
            },
        );
        // 组装最终文件名
        let filename = format!(
            "{}_{}_{}_{:x}.ttml", // 时间戳_安全标题_安全艺术家_随机ID(十六进制).ttml
            timestamp,
            safe_title.chars().take(20).collect::<String>(), // 限制标题长度
            safe_artist.chars().take(15).collect::<String>(), // 限制艺术家长度
            random_id
        );

        // 获取本地缓存目录路径
        let Some(cache_dir) = self.local_lyrics_cache_dir_path.as_ref() else {
            log::error!("[本地缓存] 本地缓存目录路径未设置，无法保存文件。");
            self.toasts.add(Toast {
                text: "缓存失败：内部错误 (目录路径)".into(),
                kind: ToastKind::Error,
                options: ToastOptions::default()
                    .duration_in_seconds(3.0)
                    .show_progress(true),
                style: Default::default(),
            });
            return;
        };
        let file_path = cache_dir.join(&filename); // 完整文件路径

        // 写入歌词到文件
        match std::fs::write(&file_path, &self.output_text) {
            Ok(_) => log::info!("[本地缓存] TTML 歌词已保存到: {:?}", file_path),
            Err(e) => {
                log::error!("[本地缓存] 保存 TTML 文件到 {:?} 失败: {}", file_path, e);
                self.toasts.add(Toast {
                    text: "缓存失败：写入文件错误".into(),
                    kind: ToastKind::Error,
                    options: ToastOptions::default()
                        .duration_in_seconds(3.0)
                        .show_progress(true),
                    style: Default::default(),
                });
                return;
            }
        }

        // 创建新的缓存索引条目
        let new_entry = LocalLyricCacheEntry::new(
            title_to_save.clone(),
            artists_to_save.clone(),
            filename.clone(),
            self.last_auto_fetch_source_format, // 记录原始获取格式
        );

        // 更新内存中的缓存索引
        let mut index_guard = self.local_lyrics_cache_index.lock().unwrap();
        if let Some(existing_idx) = index_guard.iter().position(|entry| {
            entry.smtc_title == title_to_save && entry.smtc_artists == artists_to_save // 检查是否已存在相同歌曲的缓存
        }) {
            log::info!(
                "[本地缓存] 找到现有缓存条目，将替换为新的歌词: {}",
                title_to_save
            );
            // 删除旧的缓存文件
            if let Some(old_filename) = index_guard
                .get(existing_idx)
                .map(|e| e.ttml_filename.clone())
            {
                let old_file_path = cache_dir.join(old_filename);
                if let Err(e) = std::fs::remove_file(&old_file_path) {
                    log::warn!("[本地缓存] 删除旧缓存文件 {:?} 失败: {}", old_file_path, e);
                }
            }
            index_guard[existing_idx] = new_entry.clone(); // 替换现有条目
        } else {
            index_guard.push(new_entry.clone()); // 添加新条目
        }
        drop(index_guard); // 释放索引锁

        // 将新条目追加到索引文件 (如果路径存在)
        if let Some(index_file_path) = &self.local_lyrics_cache_index_path {
            match OpenOptions::new()
                .append(true) // 以追加模式打开
                .create(true) // 如果文件不存在则创建
                .open(index_file_path)
            {
                Ok(file) => {
                    let mut writer = BufWriter::new(file); // 使用带缓冲的写入器
                    if let Ok(json_line) = serde_json::to_string(&new_entry) {
                        // 序列化为 JSON 字符串
                        if writeln!(writer, "{}", json_line).is_err() {
                            // 写入并换行
                            log::error!("[本地缓存] 写入索引条目到 {:?} 失败。", index_file_path);
                        }
                    }
                }
                Err(e) => log::error!(
                    "[本地缓存] 打开或创建本地索引文件 {:?} 失败: {}",
                    index_file_path,
                    e
                ),
            }
        }

        // 显示成功提示
        self.toasts.add(Toast {
            text: format!("歌词已保存: {}", title_to_save).into(),
            kind: ToastKind::Success,
            options: ToastOptions::default()
                .duration_in_seconds(3.0)
                .show_progress(true)
                .show_icon(true),
            style: Default::default(),
        });
    }

    /// 检查 AMLL 索引是否有更新。
    pub fn check_for_amll_index_update(&mut self) {
        let mut current_state_guard = self.amll_index_download_state.lock().unwrap();
        // 如果当前正在检查更新或下载中，则跳过
        if *current_state_guard == AmllIndexDownloadState::CheckingForUpdate
            || matches!(*current_state_guard, AmllIndexDownloadState::Downloading(_))
        {
            log::debug!("[UniLyricApp 检查更新] 已在检查更新或下载中，跳过。");
            return;
        }
        // 设置状态为正在检查
        *current_state_guard = AmllIndexDownloadState::CheckingForUpdate;
        drop(current_state_guard); // 在调用外部函数前释放锁

        // 触发 AMLL 索引更新检查
        amll_connector_manager::trigger_amll_index_update_check(
            self.http_client.clone(),
            Arc::clone(&self.amll_index_download_state),
            self.amll_index_cache_path.clone(),
            Arc::clone(&self.tokio_runtime),
        );
    }

    /// 触发 AMLL 索引的下载。
    /// `force_network_refresh`: 是否强制从网络刷新，即使本地已有或状态为成功。
    pub fn trigger_amll_index_download(&mut self, force_network_refresh: bool) {
        let mut current_state_guard = self.amll_index_download_state.lock().unwrap();

        let mut initial_head_candidate_for_async: Option<String> = None; // 用于传递给异步任务的初始 HEAD 候选
        let mut should_proceed_with_download = force_network_refresh; // 是否应该继续下载流程

        if !force_network_refresh {
            // 如果不是强制刷新，根据当前状态判断是否需要下载
            match &*current_state_guard {
                AmllIndexDownloadState::UpdateAvailable(head) => {
                    // 如果有可用更新
                    initial_head_candidate_for_async = Some(head.clone());
                    should_proceed_with_download = true;
                    log::debug!(
                        "[UniLyricApp 触发下载] 检测到更新 (HEAD: {}), 准备下载。",
                        head.chars().take(7).collect::<String>() // 只记录HEAD的前7个字符
                    );
                }
                AmllIndexDownloadState::Idle | AmllIndexDownloadState::Error(_) => {
                    // 如果当前是空闲或错误状态，也应该尝试下载
                    should_proceed_with_download = true;
                    log::debug!(
                        "[UniLyricApp 触发下载] 从 {:?} 状态触发下载 (非强制)。",
                        *current_state_guard
                    );
                }
                AmllIndexDownloadState::Success(loaded_head) => {
                    // 如果已成功加载，并且不是强制刷新
                    if let Some(ref cache_p) = self.amll_index_cache_path {
                        if !cache_p.exists() {
                            // 虽然状态是成功，但缓存文件不存在，这很奇怪，尝试重新下载
                            log::warn!(
                                "[UniLyricApp 触发下载] 状态为 Success({}) 但缓存文件不存在，将尝试下载。",
                                loaded_head.chars().take(7).collect::<String>()
                            );
                            should_proceed_with_download = true;
                            initial_head_candidate_for_async = Some(loaded_head.clone()); // 尝试下载这个已知的HEAD
                        } else {
                            // 状态成功且缓存存在，非强制刷新则不下载
                            log::debug!(
                                "[UniLyricApp 触发下载] 状态为 Success({}) 且非强制刷新，不执行下载。",
                                loaded_head.chars().take(7).collect::<String>()
                            );
                            // should_proceed_with_download 保持 false
                        }
                    } else {
                        // 没有缓存路径信息，但状态是 Success，这也阻止非强制下载
                        log::warn!("[UniLyricApp 触发下载] 状态为 Success 但无缓存路径，不下载。");
                    }
                }
                AmllIndexDownloadState::CheckingForUpdate
                | AmllIndexDownloadState::Downloading(_) => {
                    // 如果正在检查更新或下载中，则跳过
                    log::debug!("[UniLyricApp 触发下载] 已在检查更新或下载中，跳过。");
                    return;
                }
            }
        } else {
            // force_network_refresh 为 true
            log::trace!("[UniLyricApp 触发下载] 强制刷新，将下载最新版本。");
            // initial_head_candidate_for_async 保持 None，让异步任务获取最新 HEAD
        }

        // 如果最终判断不需要下载，则直接返回
        if !should_proceed_with_download {
            return;
        }

        // 更新状态为 Downloading
        // 异步任务内部会再次确认/获取最终的 HEAD 并可能再次更新 Downloading 状态
        *current_state_guard =
            AmllIndexDownloadState::Downloading(initial_head_candidate_for_async.clone());
        drop(current_state_guard); // 释放锁

        // 调用管理器中的函数执行异步下载
        let params = amll_connector_manager::AmllIndexDownloadParams {
            http_client: self.http_client.clone(),
            amll_db_repo_url_base: self.amll_db_repo_url_base.clone(),
            amll_index_data: Arc::clone(&self.amll_index),
            amll_index_download_state: Arc::clone(&self.amll_index_download_state),
            amll_index_cache_path: self.amll_index_cache_path.clone(),
            tokio_runtime: Arc::clone(&self.tokio_runtime),
        };

        amll_connector_manager::trigger_amll_index_download_async(
            params,
            force_network_refresh,
            initial_head_candidate_for_async,
        );    
    }

    /// 触发 AMLL 歌词的搜索和下载。
    /// `selected_entry_to_download`: 如果是 Some，则直接下载此条目；如果是 None，则根据 `amll_search_query` 进行搜索。
    pub fn trigger_amll_lyrics_search_and_download(
        &mut self,
        selected_entry_to_download: Option<AmllIndexEntry>,
    ) {
        // 1. 检查 AMLL 索引是否已成功加载
        let index_state_lock = self.amll_index_download_state.lock().unwrap();
        if !matches!(*index_state_lock, AmllIndexDownloadState::Success(_)) {
            warn!(
                "[UniLyricApp] AMLL TTML Database 索引文件尚未成功加载，无法搜索或下载。当前状态: {:?}",
                *index_state_lock
            );
            // 如果索引未加载/错误，并且是 Idle 或 Error 状态，则尝试检查更新
            if matches!(
                *index_state_lock,
                AmllIndexDownloadState::Idle | AmllIndexDownloadState::Error(_)
            ) {
                drop(index_state_lock); // 释放锁后才能调用 self 的其他方法
                self.check_for_amll_index_update(); // 先检查更新

                // 更新 TTML 下载状态为错误，提示用户
                let mut ttml_state_lock = self.amll_ttml_download_state.lock().unwrap();
                *ttml_state_lock = AmllTtmlDownloadState::Error(
                    "索引文件正在加载/检查更新，请稍后重试搜索。".to_string(),
                );
            } else {
                // 如果是 CheckingForUpdate 或 Downloading 状态，则不额外操作
                drop(index_state_lock);
            }
            return;
        }
        drop(index_state_lock); // 释放索引状态锁

        // 2. 准备参数
        // 如果是搜索操作 (selected_entry_to_download 为 None)
        let query_for_search = if selected_entry_to_download.is_none() {
            Some(self.amll_search_query.clone()) // 使用UI输入的搜索查询
        } else {
            None // 下载特定条目时不需要查询字符串
        };
        let field_for_search = if selected_entry_to_download.is_none() {
            Some(self.amll_selected_search_field.clone()) // 使用UI选择的搜索字段
        } else {
            None // 下载特定条目时不需要搜索字段
        };
        let index_data_for_search = if selected_entry_to_download.is_none() {
            Some(Arc::clone(&self.amll_index)) // 传递索引数据用于搜索
        } else {
            None // 下载特定条目时不需要完整索引数据 (条目本身已包含路径)
        };
        let search_results_for_search = if selected_entry_to_download.is_none() {
            Some(Arc::clone(&self.amll_search_results)) // 传递搜索结果的Arc用于更新
        } else {
            None // 下载特定条目时不直接更新搜索结果列表
        };

        let action = if let Some(entry) = selected_entry_to_download {
            // --- 情况1: 如果提供了要下载的特定条目 ---
            // 创建一个 Download 动作
            amll_connector_manager::AmllLyricsAction::Download(entry)
        } else if let (Some(query), Some(field), Some(index_data), Some(search_results)) = (
            query_for_search,
            field_for_search,
            index_data_for_search,
            search_results_for_search,
        ) {
            // --- 情况2: 如果提供了所有搜索所需的参数 ---
            // 创建一个 Search 动作
            amll_connector_manager::AmllLyricsAction::Search {
                query,
                field,
                index_data,
                search_results,
            }
        } else {
            // 如果参数不足以执行任何操作，则直接返回或记录警告
            log::error!("[UnilyricApp] handle_amll_lyrics_search_or_download_async 参数不足，无法确定操作。");
            return;
        };

        // 然后，使用创建好的 action 来调用新签名的函数
        amll_connector_manager::handle_amll_lyrics_search_or_download_async(
            self.http_client.clone(),
            self.amll_db_repo_url_base.clone(),
            Arc::clone(&self.amll_ttml_download_state),
            Arc::clone(&self.tokio_runtime),
            action, // 将构建好的 action 作为最后一个参数传入
        );
    }

    /// 处理 AMLL TTML 歌词下载完成的逻辑。
    pub fn handle_amll_ttml_download_completion(&mut self) {
        let mut fetched_lyrics_to_process: Option<FetchedAmllTtmlLyrics> = None; // 存储成功获取的歌词数据
        let mut error_to_report: Option<String> = None; // 存储错误信息
        let mut should_close_window_and_reset_state = false; // 是否应关闭下载窗口并重置状态

        // 检查下载状态
        {
            let mut download_status_locked = self.amll_ttml_download_state.lock().unwrap();
            match &*download_status_locked {
                AmllTtmlDownloadState::Success(data) => {
                    // 下载成功
                    fetched_lyrics_to_process = Some(data.clone());
                    should_close_window_and_reset_state = true;
                }
                AmllTtmlDownloadState::Error(msg) => {
                    // 下载失败
                    error_to_report = Some(msg.clone());
                }
                _ => {} // Idle, Searching, Downloading 状态，不在此处处理
            }
            // 如果确定要关闭窗口并重置（通常是成功后）
            if should_close_window_and_reset_state {
                *download_status_locked = AmllTtmlDownloadState::Idle; // 重置下载状态为空闲
            }
        } // 下载状态锁释放

        // 处理获取到的歌词数据
        if let Some(fetched_data) = fetched_lyrics_to_process {
            // 清理可能存在的旧的次要歌词数据
            self.loaded_translation_lrc = None;
            self.loaded_romanization_lrc = None;
            self.pending_translation_lrc_from_download = None;
            self.pending_romanization_qrc_from_download = None;
            self.pending_romanization_lrc_from_download = None;

            // 调用核心处理函数处理平台获取的数据
            app_fetch_core::process_platform_lyrics_data(
                self,
                PlatformFetchedData::Amll(fetched_data), // 包装为 PlatformFetchedData 枚举
            );
        } else if let Some(err_msg) = error_to_report {
            // 如果有错误信息，记录日志
            log::error!("[Unilyric] AMLL TTML Database 下载错误: {}", err_msg);
            // 可以在这里添加UI提示，例如通过 self.toasts
        }

        // 如果需要关闭窗口并重置相关UI状态
        if should_close_window_and_reset_state {
            self.show_amll_download_window = false; // 关闭 AMLL 下载窗口
            self.amll_search_query.clear(); // 清空搜索查询
            let mut search_results_lock = self.amll_search_results.lock().unwrap();
            search_results_lock.clear(); // 清空搜索结果列表
        }
    }

    /// 触发QQ音乐歌词的下载流程。
    pub fn trigger_qqmusic_download(&mut self) {
        let query = self.qqmusic_query.trim().to_string(); // 获取并清理查询字符串
        if query.is_empty() {
            log::error!("[Unilyric] QQ音乐下载：请输入有效的搜索内容。");
            // 如果当前状态是下载中，也将其重置为空闲，以避免UI卡在下载状态。
            let mut download_status_locked = self.qq_download_state.lock().unwrap();
            if matches!(*download_status_locked, QqMusicDownloadState::Downloading) {
                *download_status_locked = QqMusicDownloadState::Idle;
            }
            return;
        }

        // 设置下载状态为“下载中”
        {
            let mut download_status_locked = self.qq_download_state.lock().unwrap();
            *download_status_locked = QqMusicDownloadState::Downloading;
        }

        // 克隆需要在新线程中使用的数据
        let state_clone = Arc::clone(&self.qq_download_state);
        let client_clone = self.http_client.clone(); // HTTP客户端通常是 Arc<Client>

        // 创建新线程执行异步下载任务
        std::thread::spawn(move || {
            // 为新线程创建 Tokio 运行时
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all() // 启用所有 Tokio 功能
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    log::error!("[Unilyric] QQ音乐下载：创建Tokio运行时失败: {}", e);
                    let mut status_lock = state_clone.lock().unwrap();
                    *status_lock =
                        QqMusicDownloadState::Error(format!("创建异步运行时失败: {}", e));
                    return;
                }
            };

            // 在 Tokio 运行时中执行异步代码块
            rt.block_on(async {
                log::info!("[Unilyric] QQ音乐下载：正在获取: '{}'", query);
                // 调用 QQ 音乐歌词获取器的下载函数
                match qq_lyrics_fetcher::qqlyricsfetcher::download_lyrics_by_query_first_match(
                    &client_clone, // 传递 HTTP 客户端引用
                    &query,
                )
                .await // 等待异步操作完成
                {
                    Ok(data) => {
                        // 下载成功
                        info!(
                            "[Unilyric] 下载成功： {} - {}",
                            data.song_name.as_deref().unwrap_or("未知歌名"),
                            data.artists_name.join("/")
                        );
                        let mut status_lock = state_clone.lock().unwrap();
                        *status_lock = QqMusicDownloadState::Success(data); // 更新状态为成功并附带数据
                    }
                    Err(e) => {
                        // 下载失败
                        log::error!("[Unilyric] QQ音乐歌词下载失败: {}", e);
                        let mut status_lock = state_clone.lock().unwrap();
                        *status_lock = QqMusicDownloadState::Error(e.to_string()); // 更新状态为错误并附带错误信息
                    }
                }
            });
        });
    }

    /// 触发网易云音乐歌词的下载流程。
    pub fn trigger_netease_download(&mut self) {
        let query = self.netease_query.trim().to_string(); // 获取并清理查询内容
        if query.is_empty() {
            log::error!("[Unilyric] 网易云音乐下载：查询内容为空，无法开始下载。");
            let mut ds_lock = self.netease_download_state.lock().unwrap();
            *ds_lock = NeteaseDownloadState::Idle; // 重置状态为空闲
            return;
        }

        // 克隆需要在新线程中使用的数据
        let download_state_clone = Arc::clone(&self.netease_download_state);
        let client_mutex_arc_clone = Arc::clone(&self.netease_client); // 网易云客户端是 Arc<Mutex<Option<NeteaseClient>>>

        // 根据客户端是否已初始化，设置初始下载状态
        {
            let mut ds_lock = download_state_clone.lock().unwrap();
            let client_guard = client_mutex_arc_clone.lock().unwrap(); // 获取客户端的锁
            if client_guard.is_none() {
                // 如果客户端未初始化
                *ds_lock = NeteaseDownloadState::InitializingClient; // 设置状态为正在初始化客户端
            } else {
                // 如果客户端已初始化
                *ds_lock = NeteaseDownloadState::Downloading; // 设置状态为下载中
            }
        } // 客户端锁和下载状态锁在此释放

        // 创建新线程执行异步下载任务
        std::thread::spawn(move || {
            // 为新线程创建 Tokio 运行时
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    log::error!("[Unilyric 网易云下载线程] 创建Tokio运行时失败: {}", e);
                    let mut status_lock = download_state_clone.lock().unwrap();
                    *status_lock =
                        NeteaseDownloadState::Error(format!("创建异步运行时失败: {}", e));
                    return;
                }
            };

            // 在 Tokio 运行时中执行异步代码块
            rt.block_on(async move {
                let maybe_client_instance: Option<netease_lyrics_fetcher::api::NeteaseClient>;
                // 确保客户端已初始化
                {
                    let mut client_option_guard = client_mutex_arc_clone.lock().unwrap(); // 获取客户端选项的锁
                    if client_option_guard.is_none() {
                        // 如果客户端实例不存在，则创建新的实例
                        match netease_lyrics_fetcher::api::NeteaseClient::new() {
                            Ok(new_client) => {
                                *client_option_guard = Some(new_client); // 存储新创建的客户端实例
                            }
                            Err(e) => {
                                // 客户端初始化失败
                                let mut status_lock = download_state_clone.lock().unwrap();
                                *status_lock =
                                    NeteaseDownloadState::Error(format!("客户端初始化失败: {}", e));
                                return;
                            }
                        }
                    }
                    // 克隆客户端实例以在异步块中使用 (Option<NeteaseClient> 本身是 Clone 的)
                    maybe_client_instance = (*client_option_guard).clone();
                } // 客户端选项锁在此释放

                if let Some(netease_api_client) = maybe_client_instance {
                    // 如果客户端实例成功获取或创建
                    // 再次检查并设置下载状态为Downloading (如果之前是InitializingClient)
                    {
                        let mut ds_lock = download_state_clone.lock().unwrap();
                        if matches!(*ds_lock, NeteaseDownloadState::InitializingClient) {
                            *ds_lock = NeteaseDownloadState::Downloading; // 更新状态为下载中
                        }
                    } // 下载状态锁在此释放

                    // 调用网易云歌词获取器的搜索和下载函数
                    match netease_lyrics_fetcher::search_and_fetch_first_netease_lyrics(
                        &netease_api_client, // 传递客户端引用
                        &query,
                    )
                    .await // 等待异步操作完成
                    {
                        Ok(data) => {
                            // 下载成功
                            log::info!(
                                "[Unilyric] 网易云音乐下载成功：已获取 {} - {}",
                                data.song_name.as_deref().unwrap_or("未知歌名"),
                                data.artists_name.join("/")
                            );
                            let mut status_lock = download_state_clone.lock().unwrap();
                            *status_lock = NeteaseDownloadState::Success(data); // 更新状态为成功并附带数据
                        }
                        Err(e) => {
                            // 下载失败
                            log::error!("[Unilyric] 网易云歌词下载失败: {}", e);
                            let mut status_lock = download_state_clone.lock().unwrap();
                            *status_lock = NeteaseDownloadState::Error(e.to_string()); // 更新状态为错误并附带错误信息
                        }
                    }
                } else {
                    // 理论上不应该发生，因为上面已经确保客户端被创建
                    log::error!("[Unilyric] 网易云下载：获取客户端实例时发生意外错误。");
                    let mut status_lock = download_state_clone.lock().unwrap();
                    *status_lock = NeteaseDownloadState::Error("客户端创建失败".to_string());
                }
            });
        });
    }

    /// 处理QQ音乐歌词下载完成后的逻辑。
    /// 包括清理旧数据、设置新歌词内容、处理元数据、暂存次要歌词，并触发转换。
    pub fn handle_qq_download_completion(&mut self) {
        let mut fetched_lyrics_to_process: Option<
            crate::qq_lyrics_fetcher::qqlyricsfetcher::FetchedQqLyrics, // QQ音乐获取的数据类型
        > = None;
        let mut error_to_report: Option<String> = None; // 存储错误信息
        let mut should_close_window = false; // 是否应关闭下载窗口

        // 关键：检查下载状态，获取数据或错误信息
        {
            // 锁的作用域开始
            let mut download_status_locked = self.qq_download_state.lock().unwrap(); // 获取QQ下载状态的锁

            match &*download_status_locked {
                QqMusicDownloadState::Success(data) => {
                    // 下载成功
                    fetched_lyrics_to_process = Some(data.clone()); // 克隆获取到的数据
                    should_close_window = true; // 成功后通常关闭窗口
                    *download_status_locked = QqMusicDownloadState::Idle; // 处理后重置状态为空闲
                }
                QqMusicDownloadState::Error(msg) => {
                    // 下载失败
                    error_to_report = Some(msg.clone()); // 克隆错误信息
                    should_close_window = true; // 失败后也可能关闭窗口（取决于UI设计）
                    *download_status_locked = QqMusicDownloadState::Idle; // 处理后重置状态为空闲
                }
                QqMusicDownloadState::Downloading => {} // 仍在下载中，不处理
                QqMusicDownloadState::Idle => {}        // 空闲状态，不处理
            }
        } // QQ下载状态锁在此释放

        // 现在根据提取的结果进行处理
        if let Some(fetched_data) = fetched_lyrics_to_process {
            // 如果成功获取到数据
            app_fetch_core::process_platform_lyrics_data(
                self,
                PlatformFetchedData::Qq(fetched_data), // 将QQ获取的数据包装并传递给核心处理函数
            );
        } else if let Some(err_msg) = error_to_report {
            // 如果有错误信息
            log::error!(
                "[UniLyricApp] QQ音乐下载失败 (在 handle_completion 中报告): {}",
                err_msg
            );
            // 可以在此添加UI提示，例如 self.toasts.add(...)
        }

        // 如果需要关闭下载窗口
        if should_close_window {
            self.show_qqmusic_download_window = false; // 控制QQ音乐下载窗口的显示状态
        }
    }

    /// 处理酷狗音乐KRC歌词下载完成后的逻辑。
    pub fn handle_kugou_download_completion(&mut self) {
        let mut fetched_krc_to_process: Option<crate::kugou_lyrics_fetcher::FetchedKrcLyrics> =
            None; // 存储成功获取的KRC歌词数据
        let mut error_to_report: Option<String> = None; // 存储错误信息
        let mut should_close_window = false; // 标志，用于决定是否关闭下载窗口

        // 检查下载状态
        {
            let mut download_status_locked = self.kugou_download_state.lock().unwrap(); // 获取酷狗下载状态的锁
            match &*download_status_locked {
                KrcDownloadState::Success(data) => {
                    // 下载成功
                    fetched_krc_to_process = Some(data.clone()); // 克隆获取到的数据
                    should_close_window = true; // 成功后通常关闭窗口
                }
                KrcDownloadState::Error(msg) => {
                    // 下载失败
                    error_to_report = Some(msg.clone()); // 克隆错误信息
                    should_close_window = true; // 失败后也可能关闭窗口
                }
                _ => {} // Downloading 或 Idle 状态，不在此处处理
            }
            // 如果确定要关闭窗口（成功或失败后）
            if should_close_window {
                *download_status_locked = KrcDownloadState::Idle; // 重置状态为空闲
            }
        } // 酷狗下载状态锁在此释放

        // 处理获取到的歌词数据
        if let Some(fetched_data) = fetched_krc_to_process {
            app_fetch_core::process_platform_lyrics_data(
                self,
                PlatformFetchedData::Kugou(fetched_data), // 将酷狗获取的数据包装并传递给核心处理函数
            );
        } else if let Some(err_msg) = error_to_report {
            // 如果有错误信息
            log::error!("[Unilyric] 酷狗歌词下载失败: {}", err_msg);
            // 可以在此添加UI提示
        }

        // 如果需要关闭下载窗口
        if should_close_window {
            self.show_kugou_download_window = false; // 控制酷狗下载窗口的显示状态
        }
    }

    /// 处理网易云音乐歌词下载完成后的逻辑。
    pub fn handle_netease_download_completion(&mut self) {
        let mut fetched_data_to_process: Option<
            crate::netease_lyrics_fetcher::FetchedNeteaseLyrics, // 网易云获取的数据类型
        > = None;
        let mut error_to_report: Option<String> = None; // 存储错误信息
        let mut should_close_window = false; // 标志，用于决定是否关闭下载窗口

        // 检查下载状态
        {
            let mut download_status_locked = self.netease_download_state.lock().unwrap(); // 获取网易云下载状态的锁
            match &*download_status_locked {
                NeteaseDownloadState::Success(data) => {
                    // 下载成功
                    fetched_data_to_process = Some(data.clone()); // 克隆获取到的数据
                    should_close_window = true; // 成功后通常关闭窗口
                }
                NeteaseDownloadState::Error(msg) => {
                    // 下载失败
                    error_to_report = Some(msg.clone()); // 克隆错误信息
                    should_close_window = true; // 失败后也可能关闭窗口
                }
                _ => {} // InitializingClient, Downloading 或 Idle 状态，不在此处处理
            }
            // 如果确定要关闭窗口（成功或失败后）
            if should_close_window {
                *download_status_locked = NeteaseDownloadState::Idle; // 重置状态为空闲
            }
        } // 网易云下载状态锁在此释放

        // 处理获取到的歌词数据
        if let Some(fetched_data) = fetched_data_to_process {
            app_fetch_core::process_platform_lyrics_data(
                self,
                PlatformFetchedData::Netease(fetched_data), // 将网易云获取的数据包装并传递给核心处理函数
            );
        } else if let Some(err_msg) = error_to_report {
            // 如果有错误信息
            log::error!("[Unilyric] 网易云音乐歌词下载失败: {}", err_msg);
            // 可以在此添加UI提示
        }

        // 如果需要关闭下载窗口
        if should_close_window {
            self.show_netease_download_window = false; // 控制网易云下载窗口的显示状态
            log::info!("[Unilyric] 网易云下载窗口已关闭 (因下载完成或错误)。");
        }
    }

    /// 清理所有派生数据，例如输出文本、解析后的段落、标记等。
    /// 通常在加载新输入前或清除所有数据时调用。
    pub fn clear_derived_data(&mut self) {
        log::info!("[Unilyric] 正在清理输出文本、已解析段落、标记等...");
        self.output_text.clear(); // 清空主输出框的文本
        self.display_translation_lrc_output.clear(); // 清空翻译LRC预览面板的文本
        self.display_romanization_lrc_output.clear(); // 清空罗马音LRC预览面板的文本
        self.parsed_ttml_paragraphs = None; // 清除已解析的TTML段落数据
        self.current_markers.clear(); // 清除当前歌词中的标记（如乐器段落）
        self.source_is_line_timed = false; // 重置源歌词是否为逐行定时的标志
        self.current_raw_ttml_from_input = None; // 清除从输入缓存的原始TTML文本

        // 注意： self.metadata_store 和 self.editable_metadata 不在此处清理，
        // 它们有独立的管理逻辑，尤其是在 clear_all_data 中会处理。

        // 如果 WebSocket 服务器已启用，发送一个空的歌词更新
        if self.websocket_server_enabled {
            self.send_lyrics_update_to_websocket(); // output_text 此时为空
        }
    }

    /// 清理应用中的所有数据，包括输入、输出、已解析内容、元数据（除用户固定的）、待处理下载等。
    /// 目的是将应用恢复到一个相对干净的状态，准备加载新内容。
    pub fn clear_all_data(&mut self) {
        log::info!("[UniLyricApp] 正在清理所有数据...");

        // 1. 清空主输入框文本
        self.input_text.clear();

        // 2. 清理所有派生数据 (输出文本, 已解析的TTML段落, 标记等)
        self.clear_derived_data(); // 此函数会清理 output_text, parsed_ttml_paragraphs, current_markers 等

        // 3. 清空手动加载/编辑的翻译LRC及其UI显示字符串
        self.loaded_translation_lrc = None;
        self.display_translation_lrc_output.clear();

        // 4. 清空手动加载/编辑的罗马音LRC及其UI显示字符串
        self.loaded_romanization_lrc = None;
        self.display_romanization_lrc_output.clear();

        // 5. 清空所有待处理的、可能来自上一次下载会话的歌词片段和平台元数据
        self.pending_translation_lrc_from_download = None; // 待处理的下载翻译LRC
        self.pending_romanization_qrc_from_download = None; // 待处理的下载罗马音QRC
        self.pending_romanization_lrc_from_download = None; // 待处理的下载罗马音LRC
        self.pending_krc_translation_lines = None; // KRC内嵌翻译行
        self.session_platform_metadata.clear(); // 从下载平台获取的元数据
        self.direct_netease_main_lrc_content = None; // 网易云直接获取的主LRC内容（特殊情况）

        // 6. 重置元数据来源标记 (通常在加载新数据时会重新设置)
        self.metadata_source_is_download = false; // 标记元数据是否来自下载

        // 7. 元数据存储处理：
        // 清空当前的元数据存储，然后从应用设置中重新加载用户标记为“固定”的元数据。
        // 这样做是为了保留用户希望持久化的元数据项，即使清除了当前文件的所有内容。
        {
            let mut store = self.metadata_store.lock().unwrap(); // 获取元数据存储的锁
            store.clear(); // 完全清空内部的 MetadataStore

            // 从 app_settings.pinned_metadata 重新加载用户固定的元数据
            let app_settings_locked = self.app_settings.lock().unwrap(); // 获取应用设置的锁
            for (display_key_from_settings, values_vec_from_settings) in
                &app_settings_locked.pinned_metadata
            // 遍历设置中固定的元数据
            {
                // 检查这个从设置中读取的固定项的键，是否确实也存在于 self.persistent_canonical_keys 中
                // (即用户当前仍然希望固定这种类型的元数据)
                let canonical_key_to_check = match display_key_from_settings
                    .trim()
                    .parse::<CanonicalMetadataKey>() // 尝试将显示键解析为规范键
                {
                    Ok(ck) => ck, // 解析成功
                    Err(_) => {
                        // 解析失败，视为自定义键
                        CanonicalMetadataKey::Custom(display_key_from_settings.trim().to_string())
                    }
                };

                // 如果该类型的元数据当前仍被用户标记为固定
                if self
                    .persistent_canonical_keys
                    .contains(&canonical_key_to_check)
                {
                    // 将固定值添加回元数据存储
                    for v_str in values_vec_from_settings {
                        if let Err(e) = store.add(display_key_from_settings, v_str.clone()) {
                            log::warn!(
                                "[UniLyricApp clear_all_data] 从设置重载固定元数据 '{}' (值: '{}') 到Store失败: {}",
                                display_key_from_settings,
                                v_str,
                                e
                            );
                        }
                    }
                }
            }
        } // MetadataStore 和 AppSettings 的锁在此释放

        // 8. 根据更新后的（仅包含固定项的）MetadataStore 重建UI的可编辑元数据列表
        self.rebuild_editable_metadata_from_store();

        log::info!(
            "[UniLyricApp clear_all_data] 所有当前歌词数据已清理完毕。固定的元数据已重新加载。"
        );
    }

    /// 解析输入框中的文本到内部的中间数据结构 (`ParsedSourceData`)。
    /// 这个中间数据结构通常包含TTML格式的段落列表和各种元数据。
    ///
    /// # 返回
    /// `Result<ParsedSourceData, ConvertError>` - 成功则返回解析后的数据，失败则返回转换错误。
    fn parse_input_to_intermediate_data(&self) -> Result<ParsedSourceData, ConvertError> {
        // 如果输入文本去除首尾空格后为空，则直接返回默认的（空的）ParsedSourceData
        if self.input_text.trim().is_empty() {
            log::warn!("[Unilyric 解析输入] 输入文本为空，返回默认的空 ParsedSourceData。");
            return Ok(Default::default()); // 返回一个空的 ParsedSourceData
        }

        // 根据当前选择的源格式 (self.source_format) 调用相应的解析逻辑
        match self.source_format {
            LyricFormat::Ass => {
                // 处理ASS格式
                // 从字符串加载并处理ASS数据
                let ass_data: ProcessedAssData =
                    ass_parser::load_and_process_ass_from_string(&self.input_text)?;
                // 将处理后的ASS数据生成为中间TTML字符串
                // true 表示生成用于内部处理的TTML，可能包含特殊标记
                let internal_ttml_str =
                    ttml_generator::generate_intermediate_ttml_from_ass(&ass_data, true)?;
                // 解析这个内部TTML字符串
                let (
                    paragraphs,                    // TTML段落
                    _ttml_derived_meta,            // 从TTML派生的元数据 (此处未使用)
                    is_line_timed_val,             // 是否为逐行定时
                    detected_formatted_ttml,       // 是否检测到格式化的TTML (如Apple Music风格)
                    _detected_ass_ttml_trans_lang, // 从ASS转换的TTML中检测到的翻译语言 (此处未使用)
                ) = ttml_parser::parse_ttml_from_string(&internal_ttml_str)?;
                // 构建 ParsedSourceData
                Ok(ParsedSourceData {
                    paragraphs,
                    language_code: ass_data.language_code.clone(), // ASS文件头中的语言代码
                    songwriters: ass_data.songwriters.clone(),     // ASS文件头中的词曲作者
                    agent_names: ass_data.agent_names.clone(),     // ASS文件头中的角色名
                    apple_music_id: ass_data.apple_music_id.clone(), // ASS文件头中的Apple Music ID
                    general_metadata: ass_data.metadata.clone(),   // 其他通用元数据
                    markers: ass_data.markers.clone(),             // ASS中的标记（如乐器段）
                    is_line_timed_source: is_line_timed_val,       // 标记源是否为逐行定时
                    raw_ttml_from_input: Some(internal_ttml_str),  // 存储转换后的内部TTML
                    detected_formatted_input: Some(detected_formatted_ttml), // 标记是否检测到格式化输入
                    _source_translation_language: ass_data.detected_translation_language.clone(), // ASS中检测到的翻译语言
                    ..Default::default() // 其他字段使用默认值
                })
            }
            LyricFormat::Ttml => {
                // 处理TTML格式
                // 直接从输入文本解析TTML
                let (
                    paragraphs,
                    meta, // 从TTML中直接解析出的元数据
                    is_line_timed_val,
                    detected_formatted,
                    detected_ttml_trans_lang, // TTML中检测到的翻译语言
                ) = ttml_parser::parse_ttml_from_string(&self.input_text)?;
                let mut psd = ParsedSourceData {
                    paragraphs,
                    is_line_timed_source: is_line_timed_val,
                    raw_ttml_from_input: Some(self.input_text.clone()), // 缓存原始TTML输入
                    detected_formatted_input: Some(detected_formatted),
                    general_metadata: meta, // 应用从TTML解析出的元数据
                    _source_translation_language: detected_ttml_trans_lang, // 使用从 TTML 解析器直接获取的值
                    ..Default::default()
                };
                // 从 general_metadata 中提取特定类型的元数据到 ParsedSourceData 的专用字段
                let mut remaining_general_meta = Vec::new();
                for m in &psd.general_metadata {
                    match m.key.parse::<CanonicalMetadataKey>() {
                        Ok(CanonicalMetadataKey::Language) => {
                            psd.language_code = Some(m.value.clone())
                        }
                        Ok(CanonicalMetadataKey::AppleMusicId) => {
                            psd.apple_music_id = m.value.clone()
                        }
                        Ok(CanonicalMetadataKey::Songwriter) => {
                            psd.songwriters.push(m.value.clone())
                        }
                        Ok(_) => {
                            // 其他标准键，但未在此处特定处理的，保留在通用元数据中
                            remaining_general_meta.push(m.clone());
                        }
                        Err(_) => {
                            // 自定义键或无法解析的键，保留在通用元数据中
                            remaining_general_meta.push(m.clone());
                            log::trace!(
                                // 使用 trace 级别，因为这通常是预期的行为
                                "[Unilyric 解析输入] 元数据键 '{}' 无法解析为标准键，将保留在通用元数据中。",
                                m.key
                            );
                        }
                    }
                }
                psd.general_metadata = remaining_general_meta; // 更新通用元数据列表
                psd.songwriters.sort_unstable(); // 对词曲作者列表排序
                psd.songwriters.dedup(); // 去除重复的词曲作者
                Ok(psd)
            }
            LyricFormat::Json => {
                // 处理JSON格式 (通常是Apple Music的JSON)
                // 从字符串加载JSON数据
                let bundle = json_parser::load_from_string(&self.input_text)?;
                // JSON解析器内部已将数据转换为类似 ParsedSourceData 的结构
                Ok(ParsedSourceData {
                    paragraphs: bundle.paragraphs,
                    language_code: bundle.language_code,
                    songwriters: bundle.songwriters,
                    agent_names: bundle.agent_names,
                    apple_music_id: bundle.apple_music_id,
                    general_metadata: bundle.general_metadata,
                    is_line_timed_source: bundle.is_line_timed,
                    raw_ttml_from_input: Some(bundle.raw_ttml_string), // JSON中内嵌的TTML字符串
                    detected_formatted_input: Some(bundle.detected_formatted_ttml),
                    ..Default::default()
                })
            }
            LyricFormat::Krc => {
                // 处理KRC格式 (酷狗)
                // 从字符串加载KRC数据
                let (krc_lines, mut krc_meta_from_parser) =
                    krc_parser::load_krc_from_string(&self.input_text)?;
                let mut _krc_internal_translation_base64: Option<String> = None; // 用于存储KRC内嵌翻译的Base64 (暂未使用)
                // 移除特殊的内部翻译元数据项，并将其值存储起来
                krc_meta_from_parser.retain(|item| {
                    if item.key == "KrcInternalTranslation" {
                        // 自定义的键名，用于标记内部翻译数据
                        _krc_internal_translation_base64 = Some(item.value.clone());
                        false // 从元数据列表中移除此项
                    } else {
                        true // 保留其他元数据项
                    }
                });
                // 将KRC行和处理后的元数据转换为TTML段落和元数据
                // KRC和QRC的逐字歌词结构相似，可以共用转换逻辑
                let (paragraphs, meta_from_converter) =
                    qrc_to_ttml_data::convert_qrc_to_ttml_data(&krc_lines, krc_meta_from_parser)?;
                let mut psd = ParsedSourceData {
                    paragraphs,
                    general_metadata: meta_from_converter,
                    is_line_timed_source: false, // KRC是逐字歌词，不是逐行
                    ..Default::default()
                };
                // 从转换后的元数据中提取特定类型
                let mut final_general_meta = Vec::new();
                for item in psd.general_metadata.iter().cloned() {
                    match item.key.parse::<CanonicalMetadataKey>() {
                        Ok(CanonicalMetadataKey::Language) => psd.language_code = Some(item.value),
                        Ok(CanonicalMetadataKey::Songwriter) => psd.songwriters.push(item.value),
                        _ => final_general_meta.push(item), // 其他或无法解析的保留在通用元数据
                    }
                }
                psd.general_metadata = final_general_meta;
                psd.songwriters.sort_unstable();
                psd.songwriters.dedup();
                Ok(psd)
            }
            LyricFormat::Qrc => {
                // 处理QRC格式 (QQ音乐)
                // 从字符串加载QRC数据
                let (qrc_lines, qrc_meta_from_parser) =
                    qrc_parser::load_qrc_from_string(&self.input_text)?;

                // 将QRC行和元数据转换为TTML段落和元数据
                let (paragraphs, meta_from_converter) =
                    qrc_to_ttml_data::convert_qrc_to_ttml_data(&qrc_lines, qrc_meta_from_parser)?;

                let mut psd = ParsedSourceData {
                    paragraphs,
                    general_metadata: meta_from_converter,
                    is_line_timed_source: false, // QRC是逐字歌词
                    ..Default::default()
                };
                // 从转换后的元数据中提取特定类型
                let mut final_general_meta = Vec::new();
                for item in psd.general_metadata.iter().cloned() {
                    match item.key.parse::<CanonicalMetadataKey>() {
                        Ok(CanonicalMetadataKey::Language) => psd.language_code = Some(item.value),
                        Ok(CanonicalMetadataKey::Songwriter) => psd.songwriters.push(item.value),
                        _ => final_general_meta.push(item),
                    }
                }
                psd.general_metadata = final_general_meta;
                psd.songwriters.sort_unstable();
                psd.songwriters.dedup();
                Ok(psd)
            }
            LyricFormat::Lys | LyricFormat::Spl | LyricFormat::Yrc => {
                // 处理 LYS (Sonymusic LYRICA), SPL (Smalyrics), YRC (网易云逐字)
                // SPL 格式的特殊处理逻辑
                if self.source_format == LyricFormat::Spl {
                    let (spl_blocks_from_parser, _spl_meta) = // SPL通常无元数据
                        spl_parser::load_spl_from_string(&self.input_text)?;

                    if spl_blocks_from_parser.is_empty() {
                        log::info!("[UniLyric 解析输入] SPL解析器未返回任何歌词数据。");
                        return Ok(Default::default()); // 返回空数据
                    }

                    let mut initial_ttml_paragraphs: Vec<TtmlParagraph> = Vec::new();

                    // 遍历SPL解析出的每个歌词块
                    for (block_idx, spl_block) in spl_blocks_from_parser.iter().enumerate() {
                        // 获取当前块的主歌词起始时间 (通常是第一个时间戳)
                        let primary_start_time_for_block =
                            spl_block.start_times_ms.first().cloned().unwrap_or(0);
                        // 计算块的实际结束时间 (优先使用显式结束时间，否则尝试用下一块开始时间或默认时长)
                        let block_actual_end_ms = match spl_block.explicit_block_end_ms {
                            Some(explicit_end) => explicit_end,
                            None => {
                                if block_idx + 1 < spl_blocks_from_parser.len() {
                                    // 如果有下一块
                                    spl_blocks_from_parser[block_idx + 1]
                                        .start_times_ms
                                        .first()
                                        .cloned()
                                        .unwrap_or(primary_start_time_for_block + 3000) // 默认下一块开始，或当前块+3秒
                                } else {
                                    // 如果是最后一块
                                    primary_start_time_for_block + 3000 // 默认当前块+3秒
                                }
                            }
                        };
                        // 解析SPL块中的主文本（带内联时间戳）为音节列表 (LysSyllable)
                        let main_syllables_lys: Vec<LysSyllable> =
                            match spl_parser::parse_spl_main_text_to_syllables(
                                &spl_block.main_text_with_inline_ts, // 主文本内容
                                primary_start_time_for_block,        // 块起始时间
                                block_actual_end_ms,                 // 块结束时间
                                block_idx + 1,                       // 块索引 (用于日志)
                            ) {
                                Ok(syls) => syls,
                                Err(e) => {
                                    log::error!(
                                        "[UniLyric 解析输入] 解析块 #{} ('{}') 的主文本音节失败: {}",
                                        block_idx + 1, // 日志中块索引从1开始
                                        spl_block.main_text_with_inline_ts,
                                        e
                                    );
                                    continue; // 跳过此块
                                }
                            };
                        // 将 LysSyllable 转换为 TtmlSyllable
                        let processed_main_syllables: Vec<TtmlSyllable> =
                            crate::utils::process_parsed_syllables_to_ttml(
                                &main_syllables_lys,
                                "SPL", // "SPL" 作为来源标记
                            );
                        // 获取块中的翻译文本 (可能多行，用 / 连接)
                        let translation_string_from_block: Option<String> =
                            if !spl_block.all_translation_lines.is_empty() {
                                Some(spl_block.all_translation_lines.join("/"))
                            } else {
                                None
                            };
                        let translation_tuple = translation_string_from_block.map(|t| (t, None)); // (文本, 语言代码=None)

                        // SPL块可能有多个起始时间 (start_times_ms)，每个时间点生成一个TTML段落
                        for &line_start_ms_for_para in &spl_block.start_times_ms {
                            // 计算该TTML段落的结束时间
                            let p_end_ms_for_para: u64 = if let Some(last_syl) =
                                processed_main_syllables.last()
                            {
                                let end_based_on_syl = last_syl.end_ms.max(line_start_ms_for_para); // 基于最后一个音节结束时间
                                end_based_on_syl.min(block_actual_end_ms) // 不超过块的实际结束时间
                            } else if translation_tuple.is_some() {
                                // 如果没有音节但有翻译
                                block_actual_end_ms // 段落结束时间为块结束时间
                            } else {
                                // 如果既无音节也无翻译
                                line_start_ms_for_para // 段落结束时间等于开始时间 (空行)
                            };

                            // 确保段落结束时间不早于开始时间
                            let final_p_end_ms_for_para =
                                if p_end_ms_for_para < line_start_ms_for_para {
                                    line_start_ms_for_para
                                } else {
                                    p_end_ms_for_para
                                };

                            // 只有当主歌词有内容或翻译有内容时，才创建TTML段落
                            let main_line_has_content = !processed_main_syllables.is_empty()
                                || !spl_block.main_text_with_inline_ts.trim().is_empty();
                            if main_line_has_content || translation_tuple.is_some() {
                                initial_ttml_paragraphs.push(TtmlParagraph {
                                    p_start_ms: line_start_ms_for_para,
                                    p_end_ms: final_p_end_ms_for_para,
                                    main_syllables: processed_main_syllables.clone(), // 克隆音节列表
                                    translation: translation_tuple.clone(), // 克隆翻译元组
                                    agent: "v1".to_string(), // SPL不支持演唱者信息，默认为v1
                                    ..Default::default()
                                });
                            } else {
                                log::trace!(
                                    // 使用 trace 级别记录跳过空段落
                                    "[UniLyric 解析输入] 跳过创建空的TTML段落，块起始时间: {:?}，主文本: '{}'",
                                    spl_block.start_times_ms,
                                    spl_block.main_text_with_inline_ts
                                );
                            }
                        }
                    } // 遍历SPL块结束

                    // SPL 后处理：合并在相同起始时间生成的多个TTML段落的翻译
                    let mut final_ttml_paragraphs: Vec<TtmlParagraph> = Vec::new();
                    if !initial_ttml_paragraphs.is_empty() {
                        let mut temp_iter = initial_ttml_paragraphs.into_iter().peekable(); // 使用可偷窥的迭代器
                        while let Some(mut current_para) = temp_iter.next() {
                            let mut collected_additional_translations_for_current_para: Vec<
                                String,
                            > = Vec::new();
                            // 查看后续是否有相同起始时间的段落
                            while let Some(next_para_peek) = temp_iter.peek() {
                                if next_para_peek.p_start_ms == current_para.p_start_ms {
                                    // 如果起始时间相同
                                    let next_para = temp_iter.next().unwrap(); // 取出这个段落
                                    // 如果这个后续段落的主音节部分有文本，也作为翻译的一部分
                                    if !next_para.main_syllables.is_empty() {
                                        let trans_text_from_next_main = next_para
                                            .main_syllables
                                            .iter()
                                            .map(|s| {
                                                s.text.clone()
                                                    + if s.ends_with_space { " " } else { "" }
                                            })
                                            .collect::<String>()
                                            .trim()
                                            .to_string();
                                        if !trans_text_from_next_main.is_empty() {
                                            collected_additional_translations_for_current_para
                                                .push(trans_text_from_next_main);
                                        }
                                    }
                                    // 如果这个后续段落本身也有翻译，也加入
                                    if let Some((next_trans_text, _)) = next_para.translation {
                                        if !next_trans_text.is_empty() {
                                            collected_additional_translations_for_current_para
                                                .push(next_trans_text);
                                        }
                                    }
                                } else {
                                    break; // 起始时间不同，停止合并
                                }
                            }
                            // 将收集到的额外翻译合并到当前段落的翻译中
                            if !collected_additional_translations_for_current_para.is_empty() {
                                let combined_additional_trans =
                                    collected_additional_translations_for_current_para.join("/");
                                if let Some((ref mut existing_trans, _)) = current_para.translation
                                {
                                    if !existing_trans.is_empty() {
                                        existing_trans.push('/');
                                        existing_trans.push_str(&combined_additional_trans);
                                    } else {
                                        *existing_trans = combined_additional_trans;
                                    }
                                } else {
                                    current_para.translation =
                                        Some((combined_additional_trans, None));
                                }
                            }
                            final_ttml_paragraphs.push(current_para); // 添加处理后的段落到最终列表
                        }
                    }
                    log::info!(
                        "[Unilyric 解析输入 SPL] SPL转换完成，生成 {} 个TTML段落。",
                        final_ttml_paragraphs.len()
                    );
                    // 判断SPL是否为逐行格式 (如果每个段落最多只有一个音节，或者没有音节但有翻译)
                    let is_spl_line_timed = final_ttml_paragraphs.iter().all(|p| {
                        p.main_syllables.len() <= 1
                            || (p.main_syllables.is_empty() && p.translation.is_some())
                    });
                    return Ok(ParsedSourceData {
                        paragraphs: final_ttml_paragraphs,
                        general_metadata: Vec::new(), // SPL不支持元数据
                        is_line_timed_source: is_spl_line_timed,
                        ..Default::default()
                    }); // 注意这里是 return，因为SPL的处理已完成
                } // SPL 的 if 结束

                // LYS 和 YRC 的通用处理 (如果不是 SPL)
                let (paragraphs, general_metadata_from_parser, is_line_timed) =
                    match self.source_format {
                        LyricFormat::Lys => {
                            // 处理LYS格式
                            let (lys_lines, lys_meta) =
                                lys_parser::load_lys_from_string(&self.input_text)?;
                            let (ps, _meta_from_lys_converter) = // LYS转换器返回的元数据通常为空或已包含在lys_meta中
                                lys_to_ttml_data::convert_lys_to_ttml_data(&lys_lines)?;
                            (ps, lys_meta, false) // LYS是逐字格式
                        }
                        LyricFormat::Yrc => {
                            // 处理YRC格式 (网易云逐字)
                            let (yrc_lines, yrc_meta) =
                                yrc_parser::load_yrc_from_string(&self.input_text)?;
                            let (ps, _meta_from_yrc_converter) = // YRC转换器元数据处理
                                yrc_to_ttml_data::convert_yrc_to_ttml_data(
                                    &yrc_lines,
                                    yrc_meta.clone(), // YRC元数据需要传递给转换器
                                )?;
                            (ps, yrc_meta, false) // YRC是逐字格式
                        }
                        _ => unreachable!(), // 因为 SPL 已被上面处理，不应到达此分支
                    };
                let mut psd = ParsedSourceData {
                    paragraphs,
                    general_metadata: general_metadata_from_parser,
                    is_line_timed_source: is_line_timed,
                    ..Default::default()
                };
                // 从元数据中提取特定类型
                let mut final_general_meta = Vec::new();
                for item in psd.general_metadata.iter().cloned() {
                    match item.key.parse::<CanonicalMetadataKey>() {
                        Ok(CanonicalMetadataKey::Language) => psd.language_code = Some(item.value),
                        Ok(CanonicalMetadataKey::Songwriter) => psd.songwriters.push(item.value),
                        _ => final_general_meta.push(item),
                    }
                }
                psd.general_metadata = final_general_meta;
                psd.songwriters.sort_unstable();
                psd.songwriters.dedup();
                Ok(psd)
            }
            LyricFormat::Lyl => {
                // 处理LYL格式 (Lyricify Lines)
                // 解析Lyricify行
                let parsed_lines = lyricify_lines_parser::parse_lyricify_lines(&self.input_text)?;
                // 将解析后的行转换为TTML段落和元数据
                let (paragraphs, metadata) =
                    lyricify_lines_to_ttml_data::convert_lyricify_to_ttml_data(&parsed_lines)?;
                Ok(ParsedSourceData {
                    paragraphs,
                    general_metadata: metadata,
                    is_line_timed_source: true, // LYL是逐行格式
                    ..Default::default()
                })
            }
            LyricFormat::Lqe => {
                // 处理LQE格式 (Lyrics Station / Lyrics Quick Editor)
                // 从字符串加载LQE数据
                let lqe_parsed_data = crate::lqe_parser::load_lqe_from_string(&self.input_text)?;
                // 将LQE解析数据转换为内部的 ParsedSourceData 结构
                let mut intermediate_result =
                    crate::lqe_to_ttml_data::convert_lqe_to_intermediate_data(&lqe_parsed_data)?;
                // 从通用元数据中提取特定类型 (例如词曲作者)
                let mut final_general_meta_lqe: Vec<AssMetadata> = Vec::new();
                for meta_item in intermediate_result.general_metadata.iter().cloned() {
                    match meta_item.key.parse::<CanonicalMetadataKey>() {
                        Ok(CanonicalMetadataKey::Songwriter) => {
                            intermediate_result.songwriters.push(meta_item.value)
                        }
                        _ => final_general_meta_lqe.push(meta_item), // 其他保留
                    }
                }
                intermediate_result.general_metadata = final_general_meta_lqe;
                intermediate_result.songwriters.sort_unstable();
                intermediate_result.songwriters.dedup();
                Ok(intermediate_result)
            }
            LyricFormat::Lrc => {
                // 处理LRC格式
                // 解析LRC文本，获取显示行、双语翻译（如果存在）和元数据
                let (display_lrc_lines, bilingual_translations, lrc_meta) =
                    lrc_parser::parse_lrc_text_to_lines(&self.input_text)?;

                // 过滤出有效的 LrcLine (已解析的行) 用于创建 TtmlParagraph
                let mut valid_lrc_lines: Vec<LrcLine> = display_lrc_lines
                    .into_iter() // display_lrc_lines 现在只包含主歌词行或原始行
                    .filter_map(|display_line| match display_line {
                        DisplayLrcLine::Parsed(lrc_line) => Some(lrc_line), // 保留已解析的行
                        DisplayLrcLine::Raw { .. } => None, // 丢弃原始未解析的行 (通常是元数据或格式错误的行)
                    })
                    .collect();

                valid_lrc_lines.sort_by_key(|line| line.timestamp_ms); // 按时间戳排序

                let mut paragraphs: Vec<TtmlParagraph> = Vec::with_capacity(valid_lrc_lines.len());

                // 遍历有效的LRC行，创建TTML段落
                for (i, current_lrc_line) in valid_lrc_lines.iter().enumerate() {
                    let p_start_ms = current_lrc_line.timestamp_ms; // 段落开始时间
                    // 计算段落结束时间：下一行开始时间，或当前行开始+默认时长
                    let p_end_ms: u64 = if i + 1 < valid_lrc_lines.len() {
                        let next_line_start_ms = valid_lrc_lines[i + 1].timestamp_ms;
                        if next_line_start_ms > p_start_ms {
                            next_line_start_ms // 正常情况，使用下一行开始时间
                        } else {
                            // 时间戳相同或乱序，给一个默认时长
                            p_start_ms.saturating_add(5000) // 默认5秒
                        }
                    } else {
                        // 最后一行，给一个较长的默认时长 (或根据歌曲总时长调整)
                        p_start_ms.saturating_add(60000) // 默认60秒，可调整
                    };

                    // 创建TTML段落，LRC的每行文本作为一个单独的音节
                    paragraphs.push(TtmlParagraph {
                        p_start_ms,
                        p_end_ms,
                        main_syllables: vec![TtmlSyllable {
                            text: current_lrc_line.text.clone(),
                            start_ms: p_start_ms,   // 音节开始时间同段落开始时间
                            end_ms: p_end_ms,       // 音节结束时间同段落结束时间
                            ends_with_space: false, // LRC行通常不包含尾随空格信息
                        }],
                        agent: "v1".to_string(), // LRC无演唱者信息，默认为v1
                        ..Default::default()
                    });
                }

                let mut psd = ParsedSourceData {
                    paragraphs,
                    general_metadata: lrc_meta, // 应用从LRC解析的元数据
                    is_line_timed_source: true, // LRC 是逐行时间
                    // 存储从双语LRC中提取的翻译行
                    bilingual_extracted_translations: if bilingual_translations.is_empty() {
                        None
                    } else {
                        Some(bilingual_translations)
                    },
                    ..Default::default()
                };
                // 从通用元数据中提取特定类型 (如语言，作者等)
                let mut final_general_meta = Vec::new();
                for item in psd.general_metadata.iter().cloned() {
                    match item.key.parse::<crate::types::CanonicalMetadataKey>() {
                        Ok(crate::types::CanonicalMetadataKey::Language) => {
                            psd.language_code = Some(item.value)
                        }
                        // 特殊处理LRC中的 [by:作者] 标签，确保它被识别为作者
                        Ok(crate::types::CanonicalMetadataKey::Author)
                            if item.key.eq_ignore_ascii_case("by") =>
                        {
                            // 如果键是 "by"，即使已解析为 Author，也保留它，
                            // 或者根据需要将其值赋给 psd.songwriters (如果LRC的by通常指词曲作者)
                            // 当前逻辑是保留在通用元数据中，由后续的元数据处理逻辑统一处理
                            final_general_meta.push(item);
                        }
                        _ => final_general_meta.push(item), // 其他保留
                    }
                }
                psd.general_metadata = final_general_meta;
                Ok(psd)
            }
        }
    }

    /// 根据解析后的源数据 (`ParsedSourceData`) 更新应用的核心状态，
    /// 包括主歌词段落、标记、元数据存储等。
    /// 此方法在加载新文件或从网络下载歌词后被调用。
    /// 它处理新元数据与用户已固定的元数据之间的合并逻辑。
    pub fn update_app_state_from_parsed_data(&mut self, data: ParsedSourceData) {
        log::info!(
            "[Unilyric 更新应用状态] 开始更新。已解析 {} 个段落。",
            data.paragraphs.len()
        );

        // --- 歌词内容清理步骤 ---
        // 检查是否需要对当前来源的歌词进行清理 (例如移除广告、描述性文本)
        let should_strip_for_current_source =
            if let Some(ref fetch_source) = self.last_auto_fetch_source_for_stripping_check {
                // 只对特定在线来源进行清理
                matches!(
                    fetch_source,
                    AutoSearchSource::QqMusic | AutoSearchSource::Kugou | AutoSearchSource::Netease
                )
            } else {
                false // 如果没有记录来源，则不清理
            };

        let mut paragraphs_after_processing = data.paragraphs; // 初始化为解析后的段落

        // 如果需要清理且段落不为空
        if should_strip_for_current_source && !paragraphs_after_processing.is_empty() {
            let settings_guard = self.app_settings.lock().unwrap(); // 获取应用设置的锁
            // 总开关，控制所有类型的自动清理
            if settings_guard.enable_online_lyric_stripping {
                log::info!(
                    "[UniLyricApp 更新应用状态] 对来源 {:?} 的TTML段落执行自动清理。原始段落数: {}",
                    self.last_auto_fetch_source_for_stripping_check
                        .as_ref()
                        .map_or("未知", |s| s.display_name()), // 显示来源名称
                    paragraphs_after_processing.len()
                );

                // 调用行清理函数
                paragraphs_after_processing =
                    lyric_processor::line_stripper::strip_descriptive_metadata_blocks(
                        paragraphs_after_processing,
                        &settings_guard.stripping_keywords, // 清理关键词列表
                        settings_guard.stripping_keyword_case_sensitive, // 关键词是否区分大小写
                        settings_guard.enable_ttml_regex_stripping, // 是否启用正则清理
                        &settings_guard.ttml_stripping_regexes, // 正则表达式列表
                        settings_guard.ttml_regex_stripping_case_sensitive, // 正则是否区分大小写
                    );

                log::info!(
                    "[UniLyricApp 更新应用状态] TTML段落自动清理完成。段落数变为: {}",
                    paragraphs_after_processing.len()
                );
            } else {
                log::debug!(
                    "[UniLyricApp 更新应用状态] 自动清理功能总开关未启用或段落为空，跳过。"
                );
            }
            // settings_guard 在此被 drop，锁释放
        }
        // --- 清理步骤结束 ---

        // 1. 更新应用的核心歌词数据状态
        // 使用处理后（可能被清理过）的段落列表更新应用状态
        self.parsed_ttml_paragraphs = Some(paragraphs_after_processing); // 更新已解析的TTML段落
        self.current_markers = data.markers; // 更新当前标记
        self.source_is_line_timed = data.is_line_timed_source; // 更新源是否为逐行定时
        self.current_raw_ttml_from_input = data.raw_ttml_from_input; // 更新原始TTML输入缓存
        self.detected_formatted_ttml_source = data.detected_formatted_input.unwrap_or(false); // 更新是否检测到格式化TTML源

        // 2. 元数据处理核心逻辑
        {
            // MetadataStore 锁作用域开始
            let mut store = self.metadata_store.lock().unwrap(); // 获取元数据存储的锁
            store.clear(); // 2a. 清空当前的 MetadataStore，准备从新源和固定项重新填充

            // 2b. 添加来自新源的元数据
            // 首先添加会话/平台元数据 (通常来自网络下载，这些应该优先于文件内嵌的元数据，
            // 但当前 add 逻辑是追加，固定值逻辑会在后面覆盖)
            if !self.session_platform_metadata.is_empty() {
                for (key_str, value_str) in &self.session_platform_metadata {
                    // 忽略添加错误，因为某些键可能不符合规范，但仍尝试添加
                    if let Err(_e) = store.add(key_str, value_str.clone()) {
                        // log::trace!("[Unilyric 更新应用状态] 添加会话元数据 '{}' (值: '{}') 到Store时发生预期内的解析/添加问题: {}", key_str, value_str, _e);
                    }
                }
            }

            // 然后添加来自文件内嵌的通用元数据 (data.general_metadata)
            if !data.general_metadata.is_empty() {
                for meta_item in &data.general_metadata {
                    if let Err(_e) = store.add(&meta_item.key, meta_item.value.clone()) {
                        // log::trace!("[Unilyric 更新应用状态] 添加文件元数据 '{}' (值: '{}') 到Store时发生预期内的解析/添加问题: {}", meta_item.key, meta_item.value, _e);
                    }
                }
            }

            // 添加从 ParsedSourceData 特定字段提取的元数据
            if let Some(lang) = &data.language_code {
                let _ = store.add("language", lang.clone()); // "language" 会被规范化为 CanonicalMetadataKey::Language
            }
            if !data.songwriters.is_empty() {
                for sw in &data.songwriters {
                    let _ = store.add("songwriters", sw.clone()); // "songwriters" -> CanonicalMetadataKey::Songwriter (注意单复数)
                }
            }
            if !data.apple_music_id.is_empty() {
                let _ = store.add("appleMusicId", data.apple_music_id.clone()); // "appleMusicId" -> CanonicalMetadataKey::AppleMusicId
            }
            if !data.agent_names.is_empty() {
                for (agent_id, agent_name) in &data.agent_names {
                    // agent_id (如 "v1", "v2") 通常是唯一的，但如果解析出多个同名agent，add会保留它们
                    // 这些通常是自定义键 CanonicalMetadataKey::Custom(agent_id)
                    let _ = store.add(agent_id, agent_name.clone());
                }
            }

            // 2c. 应用/覆盖用户通过UI标记为“固定”并已保存到设置的元数据值
            //     `self.app_settings.pinned_metadata` 存储的是上次保存到INI的固定项（显示键->值列表）。
            //     `self.persistent_canonical_keys` 存储的是用户当前在UI上希望固定的元数据类型的规范化键。
            let settings_pinned_map = self.app_settings.lock().unwrap().pinned_metadata.clone(); // 获取设置中固定的元数据
            if !settings_pinned_map.is_empty() {
                log::info!(
                    "[Unilyric 更新应用状态] 应用 {} 个来自设置的固定元数据键。",
                    settings_pinned_map.len()
                );
            }

            for (pinned_display_key, pinned_values_vec) in settings_pinned_map {
                // pinned_values_vec 是 Vec<String>，表示一个固定键可能有多个固定值
                // 将设置中存储的显示键解析为其规范化形式
                let canonical_key_of_pinned_item = match pinned_display_key
                    .trim()
                    .parse::<CanonicalMetadataKey>()
                {
                    Ok(ck) => ck,
                    Err(_) => CanonicalMetadataKey::Custom(pinned_display_key.trim().to_string()),
                };

                // 关键检查：只有当这个规范化的键确实在 self.persistent_canonical_keys 集合中
                // (即用户当前仍然希望固定这种类型的元数据)，才应用设置中的固定值。
                if self
                    .persistent_canonical_keys
                    .contains(&canonical_key_of_pinned_item)
                {
                    // 从Store中移除由新文件/下载加载的、与此固定键对应的所有值
                    // 这样可以确保固定值覆盖来自源的值
                    store.remove(&canonical_key_of_pinned_item);
                    // 使用设置中存储的显示键和值（可能是多个）重新添加到Store。
                    // store.add 会再次将其键名规范化。
                    for pinned_value_str in pinned_values_vec {
                        // 遍历该固定键的所有固定值
                        if let Err(e) = store.add(&pinned_display_key, pinned_value_str.clone()) {
                            log::warn!(
                                "[Unilyric 更新应用状态] 应用设置中的固定元数据 '{}' (值: '{}') 到Store失败: {}",
                                pinned_display_key,
                                pinned_value_str,
                                e
                            );
                        }
                    }
                }
            }

            // 2d. 显式移除 KRC 内部语言 Base64 值 (如果它因任何原因进入了Store)
            //     这一步作为最后防线，确保它不会出现在最终的元数据列表中。
            if let Ok(key_to_remove) = "KrcInternalTranslation".parse::<CanonicalMetadataKey>() {
                store.remove(&key_to_remove);
            }

            // 2e. 对元数据存储进行去重，确保每个键下的值的唯一性
            //     去重操作会保留每个键下唯一的、非空的值。
            //     如果一个键之前通过多次 add 累积了相同的值，去重后只会保留一个。
            store.deduplicate_values();
        } // MetadataStore 锁释放

        // 3. 处理从双语LRC主输入中提取的翻译
        if let Some(bilingual_translations) = data.bilingual_extracted_translations {
            if !bilingual_translations.is_empty() {
                log::info!(
                    "[Unilyric 更新应用状态] 应用从双语LRC主输入中提取的 {} 行翻译。",
                    bilingual_translations.len()
                );
                // 1. 将 Vec<LrcLine> 转换为 Vec<DisplayLrcLine> (用于存储和UI)
                let display_translations: Vec<DisplayLrcLine> = bilingual_translations
                    .iter()
                    .map(|lrc_line| DisplayLrcLine::Parsed(lrc_line.clone()))
                    .collect();

                // 2. 更新 loaded_translation_lrc (应用内部存储的翻译LRC行)
                self.loaded_translation_lrc = Some(display_translations);

                // 3. 更新 display_translation_lrc_output (用于UI预览的LRC文本字符串)
                //    需要重新生成LRC文本，包括可能的元数据头部
                let mut temp_translation_lrc_output = String::new();
                //    首先尝试从元数据存储中获取翻译相关的头部信息
                let header;
                {
                    // 限制锁的范围
                    let store_guard = self.metadata_store.lock().unwrap();
                    header = self.generate_specific_lrc_header_from_store(
                        LrcContentType::Translation, // 指定为翻译类型
                        &store_guard,
                    );
                } // 元数据存储锁释放
                temp_translation_lrc_output.push_str(&header); // 添加头部

                // 添加LRC行
                for lrc_line in bilingual_translations {
                    let time_str = crate::utils::format_lrc_time_ms(lrc_line.timestamp_ms); // 格式化时间戳
                    if let Err(e) =
                        writeln!(temp_translation_lrc_output, "{}{}", time_str, lrc_line.text)
                    // 写入行
                    {
                        log::error!(
                            "[Unilyric 更新应用状态] 写入双语翻译LRC行到字符串失败: {}",
                            e
                        );
                    }
                }
                // 更新UI预览字符串，确保末尾有换行符（如果非空）
                self.display_translation_lrc_output =
                    if temp_translation_lrc_output.trim().is_empty() {
                        String::new() // 如果内容为空，则设置为空字符串
                    } else {
                        temp_translation_lrc_output
                            .trim_end_matches('\n') // 移除可能的多余尾部换行
                            .to_string()
                            + "\n" // 添加一个尾部换行
                    };
                log::trace!("[Unilyric 更新应用状态] 双语翻译LRC面板内容已更新。");
            }
        }

        // 4. 根据更新后的 MetadataStore 重建UI的可编辑元数据列表
        self.rebuild_editable_metadata_from_store();
    }

    /// 处理歌词转换的核心函数。
    /// 1. 解析输入文本到中间数据结构 (`ParsedSourceData`)。
    /// 2. 更新应用状态 (元数据、标记等)。
    /// 3. 合并KRC内嵌翻译、网络下载的次要歌词、手动加载的LRC。
    /// 4. 生成目标格式的输出文本。
    pub fn handle_convert(&mut self) {
        log::info!(
            "[Unilyric 处理转换] 开始转换流程。输入文本是否为空: {}. 翻译LRC是否加载: {}, 罗马音LRC是否加载: {}",
            self.input_text.is_empty(),
            self.loaded_translation_lrc.is_some(),
            self.loaded_romanization_lrc.is_some()
        );
        self.conversion_in_progress = true; // 标记转换正在进行
        self.new_trigger_log_exists = false; // 重置新触发日志标志

        let mut parsed_data_for_update: ParsedSourceData = Default::default(); // 初始化空的解析数据

        // 步骤 1: 解析输入文本 (如果存在)
        if !self.input_text.trim().is_empty() {
            // 如果输入文本不为空
            match self.parse_input_to_intermediate_data() {
                Ok(parsed_data_bundle) => {
                    // 解析成功
                    parsed_data_for_update = parsed_data_bundle;
                }
                Err(e) => {
                    // 解析失败
                    log::error!(
                        "[Unilyric 处理转换] 解析源数据失败: {}. 仍将尝试应用元数据和已加载LRC。",
                        e
                    );
                    // 清理可能存在的旧解析结果
                    self.parsed_ttml_paragraphs = None;
                    self.current_markers.clear();
                    self.source_is_line_timed = false;
                    self.current_raw_ttml_from_input = None;
                    // 显示错误提示给用户
                    self.toasts.add(egui_toast::Toast {
                        text: format!("主歌词解析失败: {}", e).into(),
                        kind: egui_toast::ToastKind::Error,
                        options: egui_toast::ToastOptions::default()
                            .duration_in_seconds(3.0)
                            .show_icon(true),
                        style: Default::default(),
                    });
                    // parsed_data_for_update 保持默认空值
                }
            }
        } else if self.parsed_ttml_paragraphs.is_some() && self.input_text.trim().is_empty() {
            // 如果输入文本为空，但之前有解析过的段落 (例如用户清空了输入框)
            log::info!("[Unilyric 处理转换] 输入文本为空，清除主歌词段落。");
            self.parsed_ttml_paragraphs = None; // 清除段落
            self.current_markers.clear(); // 清除标记
            self.output_text.clear(); // 清空输出文本
            if self.websocket_server_enabled {
                self.send_lyrics_update_to_websocket(); // 发送空的歌词更新
            }
            // parsed_data_for_update 保持默认空值，因为没有新的输入来更新状态
        }

        // 步骤 2: 更新应用状态 (元数据、标记等)
        // 即使主歌词解析失败 (parsed_data_for_update 为空)，此步骤也会执行，
        // 以便处理固定的元数据和可能已加载的次要LRC。
        self.update_app_state_from_parsed_data(parsed_data_for_update);

        // 步骤 3: 合并KRC内嵌翻译 (如果存在)
        if let Some(trans_lines) = self.pending_krc_translation_lines.take() {
            // 取出待处理的KRC翻译行
            if let Some(ref mut paragraphs) = self.parsed_ttml_paragraphs {
                // 如果主歌词段落存在
                if !paragraphs.is_empty() && !trans_lines.is_empty() {
                    log::info!(
                        "[Unilyric 处理转换] 应用KRC内嵌翻译 ({} 行)",
                        trans_lines.len()
                    );
                    // 将KRC翻译行合并到对应的TTML段落中
                    for (i, para_line) in paragraphs.iter_mut().enumerate() {
                        if let Some(trans_text) = trans_lines.get(i) {
                            let text_to_use = if trans_text == "//" {
                                // KRC中 "//" 表示空翻译
                                ""
                            } else {
                                trans_text.as_str()
                            };
                            // 只有当段落没有翻译，或者翻译为空时，才应用KRC翻译
                            if para_line.translation.is_none()
                                || para_line
                                    .translation
                                    .as_ref()
                                    .is_some_and(|(t, _)| t.is_empty())
                            {
                                para_line.translation = Some((text_to_use.to_string(), None)); // 语言代码未知
                            }
                        }
                    }
                }
            } else {
                // 如果主歌词段落不存在 (例如解析失败)，则将KRC翻译行放回，等待下次处理
                self.pending_krc_translation_lines = Some(trans_lines);
            }
        }

        // 步骤 4: 合并从网络下载的次要歌词 (翻译LRC, 罗马音QRC/LRC)
        let pending_trans_lrc = self.pending_translation_lrc_from_download.take();
        let pending_roma_qrc = self.pending_romanization_qrc_from_download.take();
        let pending_roma_lrc = self.pending_romanization_lrc_from_download.take();
        let pending_krc_lines = self.pending_krc_translation_lines.take(); // 再次检查，以防上面未处理

        let (ind_trans_lrc, ind_roma_lrc) = {
            // ind_ 表示 independent, 即独立生成的LRC
            let metadata_store_guard = self.metadata_store.lock().unwrap(); // 获取元数据存储锁

            let pending_lyrics_data = lyrics_merger::PendingSecondaryLyrics {
                translation_lrc: pending_trans_lrc,
                romanization_qrc: pending_roma_qrc,
                romanization_lrc: pending_roma_lrc,
                krc_translation_lines: pending_krc_lines, // 传递KRC内嵌翻译（如果还存在）
            };

            // 调用歌词合并逻辑
            lyrics_merger::merge_downloaded_secondary_lyrics(
                &mut self.parsed_ttml_paragraphs, // 主歌词段落 (可变引用)
                pending_lyrics_data,              // 待合并的次要歌词数据
                &self.session_platform_metadata,  // 平台元数据 (用于辅助合并)
                &metadata_store_guard,            // 元数据存储 (用于获取语言等信息)
                self.source_format,               // 源格式 (用于判断是否需要特殊处理)
            )
        }; // 元数据存储锁在此释放

        // 如果合并后生成了独立的翻译LRC行，并且当前没有手动加载的翻译LRC，则应用它
        if let Some(lines) = ind_trans_lrc {
            if self.loaded_translation_lrc.is_none() {
                self.loaded_translation_lrc = Some(lines);
            }
        }
        // 如果合并后生成了独立的罗马音LRC行，并且当前没有手动加载的罗马音LRC，则应用它
        if let Some(lines) = ind_roma_lrc {
            if self.loaded_romanization_lrc.is_none() {
                self.loaded_romanization_lrc = Some(lines);
            }
        }

        // 步骤 5: 如果主歌词段落存在，且翻译/罗马音LRC未手动加载，则尝试从主歌词段落生成它们
        if self.parsed_ttml_paragraphs.is_some() {
            let paragraphs_ref = self.parsed_ttml_paragraphs.as_ref().unwrap(); // 获取段落的不可变引用
            if !paragraphs_ref.is_empty() {
                let store_for_header_gen_guard = self.metadata_store.lock().unwrap(); // 获取元数据存储锁 (用于生成LRC头部)

                // 生成翻译LRC (如果未手动加载)
                if self.loaded_translation_lrc.is_none() {
                    let header = self.generate_specific_lrc_header_from_store(
                        LrcContentType::Translation, // 指定为翻译类型
                        &store_for_header_gen_guard,
                    );
                    match crate::lrc_generator::generate_lrc_from_paragraphs(
                        paragraphs_ref,
                        LrcContentType::Translation, // 从段落的翻译部分生成
                    ) {
                        Ok(lrc_text_body) => {
                            // 只有当生成的LRC体或头部非空时才更新
                            if !lrc_text_body.trim().is_empty() || !header.trim().is_empty() {
                                let full_lrc_content = header + &lrc_text_body;
                                self.display_translation_lrc_output =
                                    if full_lrc_content.trim().is_empty() {
                                        String::new()
                                    } else {
                                        full_lrc_content.trim_end_matches('\n').to_string() + "\n"
                                    };
                                // 尝试解析刚生成的LRC，以填充 loaded_translation_lrc
                                match crate::lrc_parser::parse_lrc_text_to_lines(
                                    &self.display_translation_lrc_output,
                                ) {
                                    Ok((display_lines, _bilingual_translations, _meta)) => {
                                        self.loaded_translation_lrc = Some(display_lines);
                                    }
                                    Err(e) => {
                                        log::warn!(
                                            "[HandleConvert] 解析自动生成的翻译LRC失败: {}",
                                            e
                                        );
                                        self.loaded_translation_lrc = None;
                                        self.display_translation_lrc_output.clear(); // 解析失败则清空
                                    }
                                }
                            } else {
                                // 如果生成内容为空，则清空
                                self.display_translation_lrc_output.clear();
                                self.loaded_translation_lrc = None;
                            }
                        }
                        Err(e) => {
                            log::error!("[HandleConvert] 生成翻译LRC失败: {}", e);
                            self.display_translation_lrc_output.clear();
                            self.loaded_translation_lrc = None;
                        }
                    }
                }

                // 生成罗马音LRC (如果未手动加载)
                if self.loaded_romanization_lrc.is_none() {
                    let header = self.generate_specific_lrc_header_from_store(
                        LrcContentType::Romanization, // 指定为罗马音类型
                        &store_for_header_gen_guard,
                    );
                    match crate::lrc_generator::generate_lrc_from_paragraphs(
                        paragraphs_ref,
                        LrcContentType::Romanization, // 从段落的罗马音部分生成 (如果TTML支持)
                                                      // 注意：当前TTML结构可能没有直接的罗马音字段，
                                                      // 此处可能需要调整逻辑，或依赖于翻译字段被用作罗马音的情况。
                                                      // 假设 generate_lrc_from_paragraphs 能处理这种情况。
                    ) {
                        Ok(lrc_text_body) => {
                            if !lrc_text_body.trim().is_empty() || !header.trim().is_empty() {
                                let full_lrc_content = header + &lrc_text_body;
                                self.display_romanization_lrc_output =
                                    if full_lrc_content.trim().is_empty() {
                                        String::new()
                                    } else {
                                        full_lrc_content.trim_end_matches('\n').to_string() + "\n"
                                    };
                                match crate::lrc_parser::parse_lrc_text_to_lines(
                                    &self.display_romanization_lrc_output,
                                ) {
                                    Ok((display_lines, _bilingual_translations, _meta)) => {
                                        self.loaded_romanization_lrc = Some(display_lines);
                                    }
                                    Err(e) => {
                                        log::warn!(
                                            "[HandleConvert] 解析自动生成的罗马音LRC失败: {}",
                                            e
                                        );
                                        self.loaded_romanization_lrc = None;
                                        self.display_romanization_lrc_output.clear();
                                    }
                                }
                            } else {
                                self.display_romanization_lrc_output.clear();
                                self.loaded_romanization_lrc = None;
                            }
                        }
                        Err(e) => {
                            log::error!("[HandleConvert] 生成罗马音LRC失败: {}", e);
                            self.display_romanization_lrc_output.clear();
                            self.loaded_romanization_lrc = None;
                        }
                    }
                }
            } // 段落非空检查结束
        } // 主歌词段落存在检查结束

        // 步骤 6: 将手动加载的翻译LRC和罗马音LRC（如果存在）合并回主歌词段落
        // 这一步确保即使主歌词段落中已有翻译/罗马音，手动加载的也会覆盖或补充它们。
        {
            let metadata_store_guard = self.metadata_store.lock().unwrap(); // 获取元数据存储锁
            lyrics_merger::merge_manually_loaded_lrc_into_paragraphs(
                &mut self.parsed_ttml_paragraphs,      // 主歌词段落 (可变引用)
                self.loaded_translation_lrc.as_ref(),  // 手动加载的翻译LRC行
                self.loaded_romanization_lrc.as_ref(), // 手动加载的罗马音LRC行
                &metadata_store_guard,                 // 元数据存储
            );
        } // 元数据存储锁在此释放

        // 步骤 7: 生成最终的目标格式输出文本
        self.generate_target_format_output();

        // 步骤 8: 如果WebSocket服务器启用，发送更新后的歌词
        if self.websocket_server_enabled {
            self.send_lyrics_update_to_websocket();
        }

        self.conversion_in_progress = false; // 标记转换结束
        log::info!("[Unilyric 处理转换] 转换流程执行完毕。");
    }

    /// 判断给定的歌词格式是否为逐行格式。
    /// (这是一个静态辅助函数，虽然定义在 impl UniLyricApp 中，但可以移到更合适的地方，如 types.rs 或 utils.rs)
    pub fn source_format_is_line_timed(format: LyricFormat) -> bool {
        matches!(format, LyricFormat::Lrc | LyricFormat::Lyl) // LRC 和 LYL 是典型的逐行格式
    }

    /// 触发酷狗音乐KRC歌词的下载流程。
    pub fn trigger_kugou_download(&mut self) {
        let query = self.kugou_query.trim().to_string(); // 获取并清理查询字符串
        if query.is_empty() {
            log::error!("[Unilyric] 酷狗音乐下载：请输入有效的搜索内容。");
            // 如果当前状态是下载中，也将其重置为空闲
            let mut download_status_locked = self.kugou_download_state.lock().unwrap();
            if matches!(*download_status_locked, KrcDownloadState::Downloading) {
                *download_status_locked = KrcDownloadState::Idle;
            }
            return;
        }

        // 设置下载状态为“下载中”
        {
            let mut download_status_locked = self.kugou_download_state.lock().unwrap();
            *download_status_locked = KrcDownloadState::Downloading;
        }

        // 克隆需要在新线程中使用的数据
        let state_clone = Arc::clone(&self.kugou_download_state);
        let client_clone = self.http_client.clone();

        // 创建新线程执行异步下载任务
        std::thread::spawn(move || {
            // 为新线程创建 Tokio 运行时
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    log::error!("[Unilyric 酷狗下载线程] 创建Tokio运行时失败: {}", e);
                    let mut status_lock = state_clone.lock().unwrap();
                    *status_lock = KrcDownloadState::Error(format!("创建异步运行时失败: {}", e));
                    return;
                }
            };

            // 在 Tokio 运行时中执行异步代码块
            rt.block_on(async {
                log::info!("[Unilyric 酷狗下载线程] 正在获取 '{}' 的KRC歌词...", query);
                // 调用酷狗歌词获取器的下载函数
                match kugou_lyrics_fetcher::fetch_lyrics_for_song_async(&client_clone, &query).await
                {
                    Ok(fetched_data) => {
                        // 下载成功
                        log::info!(
                            "[Unilyric] 酷狗音乐下载成功：已获取 {} - {}",
                            fetched_data.song_name.as_deref().unwrap_or("未知歌名"),
                            fetched_data.artists_name.join("/")
                        );
                        let mut status_lock = state_clone.lock().unwrap();
                        *status_lock = KrcDownloadState::Success(fetched_data); // 更新状态为成功
                    }
                    Err(e) => {
                        // 下载失败
                        let error_message = format!("下载失败: {}", e);
                        log::error!("[Unilyric] 酷狗歌词下载失败: {}", error_message);
                        let mut status_lock = state_clone.lock().unwrap();
                        *status_lock = KrcDownloadState::Error(error_message); // 更新状态为错误
                    }
                }
            });
        });
    }

    /// 为特定类型的LRC内容（翻译或罗马音）生成LRC文件头部元数据字符串。
    /// 例如 `[ti:歌名]\n[ar:歌手]\n[language:zh]\n`
    ///
    /// # 参数
    /// * `content_type` - `LrcContentType`，指示是为翻译还是罗马音生成头部。
    /// * `store` - `MetadataStore` 的引用，用于获取元数据。
    ///
    /// # 返回
    /// `String` - 生成的LRC头部字符串，每条元数据占一行。
    pub fn generate_specific_lrc_header_from_store(
        &self, // self 在此函数中可能未使用，但保留以与类方法一致，或将来可能使用 self 的配置
        content_type: LrcContentType,
        store: &MetadataStore, // 从外部传入元数据存储的引用
    ) -> String {
        let mut header = String::new(); // 初始化空的头部字符串
        let mut lang_to_use: Option<String> = None; // 用于存储要写入的语言代码

        // 根据内容类型确定语言代码的来源
        if content_type == LrcContentType::Translation {
            // 如果是翻译LRC
            // 优先从TTML段落的翻译部分获取语言代码
            if let Some(paragraphs) = &self.parsed_ttml_paragraphs {
                for p in paragraphs {
                    if let Some((_text, Some(lang_code))) = &p.translation {
                        if !lang_code.is_empty() {
                            lang_to_use = Some(lang_code.clone());
                            break; // 找到第一个非空语言代码即停止
                        }
                    }
                }
            }
            // 如果TTML段落中没有，则尝试从元数据存储中获取 "translation_language"
            if lang_to_use.is_none() {
                lang_to_use = store
                    .get_single_value_by_str("translation_language") // 自定义键
                    .cloned();
            }
        }
        // 如果上述都未找到，或者不是翻译类型，则尝试获取通用的 "language" 元数据
        if lang_to_use.is_none() {
            lang_to_use = store
                .get_single_value(&crate::types::CanonicalMetadataKey::Language) // 标准语言键
                .cloned();
        }

        // 如果获取到了语言代码，则写入LRC头部
        if let Some(lang) = lang_to_use {
            if !lang.is_empty() {
                let _ = writeln!(header, "[language:{}]", lang.trim()); // 写入 [language:xx]
            }
        }

        // 定义LRC标准标签与规范元数据键的映射关系
        let lrc_tags_map = [
            (CanonicalMetadataKey::Title, "ti"),      // 标题
            (CanonicalMetadataKey::Artist, "ar"),     // 艺术家
            (CanonicalMetadataKey::Album, "al"),      // 专辑
            (CanonicalMetadataKey::Author, "by"),     // LRC文件创建者/歌词作者
            (CanonicalMetadataKey::Offset, "offset"), // 时间偏移
            (CanonicalMetadataKey::Length, "length"), // 歌曲长度 (较少使用)
            (CanonicalMetadataKey::Editor, "re"),     // LRC编辑器
            (CanonicalMetadataKey::Version, "ve"),    // LRC版本
        ];

        // 遍历映射关系，从元数据存储中获取值并写入头部
        for (canonical_key, lrc_tag_name) in lrc_tags_map.iter() {
            if let Some(values_vec) = store.get_multiple_values(canonical_key) {
                // 获取该键的所有值
                if !values_vec.is_empty() {
                    // 将多个值用 "/" 连接 (LRC标准通常只支持单个值，但这里做兼容处理)
                    let combined_value = values_vec
                        .iter()
                        .map(|s| s.trim()) // 去除首尾空格
                        .filter(|s| !s.is_empty()) // 过滤空值
                        .collect::<Vec<&str>>()
                        .join("/");
                    if !combined_value.is_empty() && writeln!(header, "[{}:{}]", lrc_tag_name, combined_value).is_err() {
                        log::error!("[生成LRC头部] 写入 {} 标签失败。", lrc_tag_name);
                    }
                }
            }
        }
        header // 返回生成的头部字符串
    }

    /// 根据当前的应用状态（已解析的TTML段落、元数据等）生成目标格式的歌词文本，
    /// 并更新主输出框和相关的LRC预览面板。
    pub fn generate_target_format_output(&mut self) {
        // 根据选择的目标格式，调用相应的生成函数
        let result: Result<String, ConvertError> = match self.target_format {
            LyricFormat::Ttml => {
                // 生成 TTML 格式
                if self.source_format == LyricFormat::Lrc {
                    // 如果源是LRC，则生成逐行定时的TTML
                    let paragraphs_to_use: Vec<TtmlParagraph>;
                    if let Some(ref paras_vec) = self.parsed_ttml_paragraphs {
                        paragraphs_to_use = paras_vec.clone();
                    } else {
                        paragraphs_to_use = Vec::new(); // 如果没有解析段落，则使用空Vec
                    }
                    let store_guard = self.metadata_store.lock().unwrap(); // 获取元数据存储锁
                    crate::ttml_generator::generate_line_timed_ttml_from_paragraphs(
                        &paragraphs_to_use,
                        &store_guard,
                    )
                } else {
                    // 其他源格式，生成标准TTML (可能是逐字或逐行，取决于 source_is_line_timed)
                    let paragraphs_for_gen_local: Vec<TtmlParagraph>;
                    if let Some(ref paras_vec) = self.parsed_ttml_paragraphs {
                        paragraphs_for_gen_local = paras_vec.clone();
                    } else {
                        paragraphs_for_gen_local = Vec::new();
                    };
                    let store_guard = self.metadata_store.lock().unwrap();
                    crate::ttml_generator::generate_ttml_from_paragraphs(
                        &paragraphs_for_gen_local,
                        &store_guard,
                        if self.source_is_line_timed {
                            // 根据源是否逐行定时决定TTML的timingMode
                            "Line"
                        } else {
                            "Word"
                        },
                        // 是否使用Apple Music格式化TTML的检测结果
                        Some(
                            self.detected_formatted_ttml_source
                                && (self.source_format == LyricFormat::Ttml // 源是TTML
                                    || self.source_format == LyricFormat::Json), // 或源是JSON (通常内含TTML)
                        ),
                        false, // false 表示不是为Apple Music JSON内嵌TTML生成
                    )
                }
            }
            LyricFormat::Lrc => {
                // 生成 LRC 格式
                let paragraphs_for_gen_local: Vec<TtmlParagraph>;
                if let Some(ref paras_vec) = self.parsed_ttml_paragraphs {
                    paragraphs_for_gen_local = paras_vec.clone();
                } else {
                    paragraphs_for_gen_local = Vec::new();
                };
                let store_guard = self.metadata_store.lock().unwrap();
                crate::lrc_generator::generate_main_lrc_from_paragraphs(
                    // 从主歌词部分生成LRC
                    &paragraphs_for_gen_local,
                    &store_guard,
                )
            }
            LyricFormat::Ass => {
                // 生成 ASS 格式
                let paragraphs_for_gen_local: Vec<TtmlParagraph>;
                if let Some(ref paras_vec) = self.parsed_ttml_paragraphs {
                    paragraphs_for_gen_local = paras_vec.clone();
                } else {
                    paragraphs_for_gen_local = Vec::new();
                };
                let store_guard = self.metadata_store.lock().unwrap();
                crate::ass_generator::generate_ass(paragraphs_for_gen_local.to_vec(), &store_guard)
            }
            LyricFormat::Json => {
                // 生成 Apple Music JSON 格式
                let paragraphs_for_gen_local: Vec<TtmlParagraph>;
                if let Some(ref paras_vec) = self.parsed_ttml_paragraphs {
                    paragraphs_for_gen_local = paras_vec.clone();
                } else {
                    paragraphs_for_gen_local = Vec::new();
                };
                let store_guard = self.metadata_store.lock().unwrap();
                let output_timing_mode_for_json_ttml = if self.source_is_line_timed {
                    "Line"
                } else {
                    "Word"
                };
                // 先生成内嵌的TTML字符串
                crate::ttml_generator::generate_ttml_from_paragraphs(
                    &paragraphs_for_gen_local,
                    &store_guard,
                    output_timing_mode_for_json_ttml,
                    Some(
                        // 是否使用Apple Music格式化TTML的检测结果
                        self.detected_formatted_ttml_source
                            && (self.source_format == LyricFormat::Ttml
                                || self.source_format == LyricFormat::Json),
                    ),
                    true, // true 表示为Apple Music JSON内嵌TTML生成 (可能有特殊处理)
                )
                .and_then(|ttml_json_content| {
                    // 如果TTML生成成功
                    // 获取Apple Music ID
                    let apple_music_id_from_store = store_guard
                        .get_single_value(&CanonicalMetadataKey::AppleMusicId)
                        .cloned()
                        .unwrap_or_else(|| "unknown_id".to_string()); // 如果没有则使用默认ID
                    // 构建Apple Music JSON结构
                    let play_params = crate::types::AppleMusicPlayParams {
                        id: apple_music_id_from_store.clone(),
                        kind: "lyric".to_string(), // 通常是 "lyric" 或 "song"
                        catalog_id: apple_music_id_from_store.clone(), // 通常与id相同
                        display_type: 2,           // 某种显示类型标记
                    };
                    let attributes = crate::types::AppleMusicAttributes {
                        ttml: ttml_json_content, // 内嵌的TTML内容
                        play_params,
                    };
                    let data_object = crate::types::AppleMusicDataObject {
                        id: apple_music_id_from_store,
                        data_type: "syllable-lyrics".to_string(), // 数据类型
                        attributes,
                    };
                    let root = crate::types::AppleMusicRoot {
                        data: vec![data_object], // 包含单个数据对象的数组
                    };
                    // 序列化为JSON字符串
                    serde_json::to_string(&root).map_err(ConvertError::JsonParse)
                })
            }
            LyricFormat::Lys => {
                // 生成 LYS 格式
                let paragraphs_for_gen_local: Vec<TtmlParagraph>;
                if let Some(ref paras_vec) = self.parsed_ttml_paragraphs {
                    paragraphs_for_gen_local = paras_vec.clone();
                } else {
                    paragraphs_for_gen_local = Vec::new();
                };
                let store_guard = self.metadata_store.lock().unwrap();
                crate::lys_generator::generate_lys_from_ttml_data(
                    &paragraphs_for_gen_local,
                    &store_guard,
                    true, // true 表示包含时间戳 (LYS标准格式)
                )
            }
            LyricFormat::Qrc => {
                // 生成 QRC 格式
                let paragraphs_for_gen_local: Vec<TtmlParagraph>;
                if let Some(ref paras_vec) = self.parsed_ttml_paragraphs {
                    paragraphs_for_gen_local = paras_vec.clone();
                } else {
                    paragraphs_for_gen_local = Vec::new();
                };
                let store_guard = self.metadata_store.lock().unwrap();
                crate::qrc_generator::generate_qrc_from_ttml_data(
                    &paragraphs_for_gen_local,
                    &store_guard,
                )
            }
            LyricFormat::Yrc => {
                // 生成 YRC 格式
                let paragraphs_for_gen_local: Vec<TtmlParagraph>;
                if let Some(ref paras_vec) = self.parsed_ttml_paragraphs {
                    paragraphs_for_gen_local = paras_vec.clone();
                } else {
                    paragraphs_for_gen_local = Vec::new();
                };
                let store_guard = self.metadata_store.lock().unwrap();
                crate::yrc_generator::generate_yrc_from_ttml_data(
                    &paragraphs_for_gen_local,
                    &store_guard,
                )
            }
            LyricFormat::Lyl => {
                // 生成 LYL (Lyricify Lines) 格式
                let paragraphs_for_gen_local: Vec<TtmlParagraph>;
                if let Some(ref paras_vec) = self.parsed_ttml_paragraphs {
                    paragraphs_for_gen_local = paras_vec.clone();
                } else {
                    paragraphs_for_gen_local = Vec::new();
                };
                // LYL格式通常不依赖外部元数据存储，直接从TTML段落转换
                crate::lyricify_lines_generator::generate_from_ttml_data(&paragraphs_for_gen_local)
            }
            LyricFormat::Spl => {
                // 生成 SPL (Smalyrics) 格式
                let paragraphs_for_gen_local: Vec<TtmlParagraph>;
                if let Some(ref paras_vec) = self.parsed_ttml_paragraphs {
                    paragraphs_for_gen_local = paras_vec.clone();
                } else {
                    paragraphs_for_gen_local = Vec::new();
                };
                let store_guard = self.metadata_store.lock().unwrap();
                crate::spl_generator::generate_spl_from_ttml_data(
                    &paragraphs_for_gen_local,
                    &store_guard,
                )
            }
            LyricFormat::Lqe => {
                // 生成 LQE 格式
                let paragraphs_for_gen_local: Vec<TtmlParagraph>;
                if let Some(ref paras_vec) = self.parsed_ttml_paragraphs {
                    paragraphs_for_gen_local = paras_vec.clone();
                } else {
                    paragraphs_for_gen_local = Vec::new();
                };
                let store_guard = self.metadata_store.lock().unwrap();
                // 为LQE生成器准备 ParsedSourceData 结构，填充所需信息
                let source_data_for_lqe_gen = crate::types::ParsedSourceData {
                    paragraphs: paragraphs_for_gen_local.to_vec(),
                    language_code: store_guard // 从元数据存储获取语言代码
                        .get_single_value(&crate::types::CanonicalMetadataKey::Language)
                        .cloned(),
                    songwriters: store_guard // 获取词曲作者
                        .get_multiple_values(&crate::types::CanonicalMetadataKey::Songwriter)
                        .cloned()
                        .unwrap_or_default(),
                    agent_names: self // 获取角色名 (通常来自ASS或平台元数据)
                        .session_platform_metadata
                        .iter()
                        .filter(|(k, _)| { // 过滤出符合角色ID格式的键 (如 v1, v1.01)
                            k.starts_with('v')
                                && (k.len() == 2 || k.len() == 5) // 长度为2 (v1) 或 5 (v1.01)
                                && k[1..].chars().all(|c| c.is_numeric() || c == '.') // 后续字符为数字或点
                        })
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    apple_music_id: store_guard // 获取Apple Music ID
                        .get_single_value(&crate::types::CanonicalMetadataKey::AppleMusicId)
                        .cloned()
                        .unwrap_or_default(),
                    markers: self.current_markers.clone(), // 当前标记
                    is_line_timed_source: self.source_is_line_timed, // 源是否逐行定时
                    raw_ttml_from_input: self.current_raw_ttml_from_input.clone(), // 原始TTML输入
                    detected_formatted_input: Some(self.detected_formatted_ttml_source), // 是否检测到格式化输入
                    // LQE特定字段，用于存储翻译和罗马音的LRC内容及其语言
                    lqe_extracted_translation_lrc_content: self
                        .loaded_translation_lrc // 如果加载了翻译LRC
                        .as_ref()
                        .map(|_| self.display_translation_lrc_output.clone()), // 则使用其UI显示文本
                    lqe_translation_language: store_guard // 翻译语言
                        .get_single_value_by_str("translation_language")
                        .cloned(),
                    lqe_extracted_romanization_lrc_content: self
                        .loaded_romanization_lrc // 如果加载了罗马音LRC
                        .as_ref()
                        .map(|_| self.display_romanization_lrc_output.clone()), // 则使用其UI显示文本
                    lqe_romanization_language: store_guard // 罗马音语言
                        .get_single_value_by_str("romanization_language")
                        .cloned(),
                    lqe_main_lyrics_as_lrc: self.source_format == LyricFormat::Lrc, // 主歌词源是否为LRC
                    lqe_direct_main_lrc_content: if self.source_format == LyricFormat::Lrc { // 如果主歌词源是LRC
                        self.direct_netease_main_lrc_content.clone() // 使用直接获取的LRC内容 (特殊情况)
                    } else {
                        None
                    },
                    ..Default::default() // 其他字段使用默认值
                };
                crate::lqe_generator::generate_lqe_from_intermediate_data(
                    &source_data_for_lqe_gen,
                    &store_guard,
                )
            }
            LyricFormat::Krc => {
                // 生成 KRC 格式
                let paragraphs_for_gen_local: Vec<TtmlParagraph>;
                if let Some(ref paras_vec) = self.parsed_ttml_paragraphs {
                    paragraphs_for_gen_local = paras_vec.clone();
                } else {
                    paragraphs_for_gen_local = Vec::new();
                };
                let store_guard = self.metadata_store.lock().unwrap();
                crate::krc_generator::generate_krc_from_ttml_data(
                    &paragraphs_for_gen_local,
                    &store_guard,
                )
            }
        };

        // 处理生成结果
        match result {
            Ok(text) => {
                // 生成成功，更新输出文本框
                self.output_text = text;
            }
            Err(e) => {
                // 生成失败
                log::error!(
                    "[Unilyric 生成目标输出] 生成目标格式 {:?} 失败: {}",
                    self.target_format.to_string(),
                    e
                );
                self.output_text.clear(); // 清空输出文本框
                // 显示错误提示给用户
                self.toasts.add(egui_toast::Toast {
                    text: format!("生成 {} 失败: {}", self.target_format, e).into(),
                    kind: egui_toast::ToastKind::Error,
                    options: egui_toast::ToastOptions::default()
                        .duration_in_seconds(3.0)
                        .show_icon(true),
                    style: Default::default(),
                });
            }
        }
    }

    /// 启动进度模拟定时器（如果需要）。
    /// 定时器用于在连接到 AMLL Player 且媒体正在播放时，模拟播放进度并发送时间更新。
    pub fn start_progress_timer_if_needed(&mut self) {
        // 获取连接器是否启用
        let connector_enabled = self.media_connector_config.lock().unwrap().enabled;
        // 检查 WebSocket 是否已连接
        let ws_connected =
            *self.media_connector_status.lock().unwrap() == WebsocketStatus::已连接;

        // 使用 self.is_currently_playing_sensed_by_smtc，这个状态由 SMTC 事件直接更新，更可靠
        let is_playing_sensed_by_smtc = self.is_currently_playing_sensed_by_smtc;

        // 定时器应该只在连接器启用、WebSocket已连接、并且媒体正在播放时运行
        if connector_enabled && ws_connected && is_playing_sensed_by_smtc {
            // 检查定时器是否未运行或已结束
            if self
                .progress_timer_join_handle
                .as_ref()
                .is_none_or(|h| h.is_finished())
            // .is_none_or() 是 nightly API，稳定版可用 .map_or(true, |h| h.is_finished())
            {
                log::trace!(
                    "[UniLyric] 启动进度定时器任务 (条件满足: 连接器启用={}, WebSocket连接={}, 播放中={}).",
                    connector_enabled,
                    ws_connected,
                    is_playing_sensed_by_smtc
                );
                // 确保在启动新定时器前，任何旧的定时器都已停止
                self.stop_progress_timer(); // 发送停止信号给可能存在的旧定时器

                let interval = self.progress_simulation_interval; // 获取模拟间隔
                let media_info_arc = Arc::clone(&self.current_media_info); // 当前媒体信息

                // 确保媒体连接器命令发送端存在
                let Some(cmd_tx) = self.media_connector_command_tx.clone() else {
                    log::warn!(
                        "[UniLyric] 无法启动进度定时器：媒体连接器命令发送端 (media_connector_command_tx) 为 None。"
                    );
                    return;
                };

                let connector_config_arc = Arc::clone(&self.media_connector_config); // 连接器配置
                let tokio_runtime_arc = Arc::clone(&self.tokio_runtime); // Tokio 运行时

                // 创建用于关闭定时器任务的 oneshot channel
                let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
                self.progress_timer_shutdown_tx = Some(shutdown_tx); // 保存发送端

                // 克隆用于从定时器任务接收更新的发送端 (如果定时器任务需要向主应用发送更新)
                let update_tx_clone = self.media_connector_update_tx_for_worker.clone();

                // 在 Tokio 运行时中启动定时器任务
                let handle = tokio_runtime_arc.spawn(run_progress_timer_task(
                    interval,
                    media_info_arc,
                    cmd_tx,
                    connector_config_arc,
                    shutdown_rx, // 传递关闭信号的接收端
                    update_tx_clone,
                ));
                self.progress_timer_join_handle = Some(handle); // 保存任务句柄
            } else {
                // log::trace!("[UniLyric] 进度定时器任务已在运行，无需重复启动。");
            }
        } else {
            // 如果条件不满足（例如 WebSocket 未连接或未播放），则确保定时器停止
            // log::trace!(
            //     "[UniLyric] 进度定时器启动条件未满足 (连接器启用={}, WebSocket连接={}, 播放中={})，将停止定时器。",
            //     connector_enabled, ws_connected, is_playing_sensed_by_smtc
            // );
            self.stop_progress_timer(); // 确保定时器已停止
        }
    }

    /// 停止进度模拟定时器。
    pub fn stop_progress_timer(&mut self) {
        // 如果存在关闭信号发送端
        if let Some(tx) = self.progress_timer_shutdown_tx.take() {
            // 发送关闭信号
            if tx.send(()).is_err() {
                // 发送失败，可能任务已经自行退出
                log::warn!("[UniLyric] 发送关闭信号给进度定时器失败 (可能已退出)。");
            } else {
                log::trace!("[UniLyric] 已发送关闭信号给进度定时器。");
            }
        }
        // Join handle 会在任务结束时自动清理，或者可以在应用关闭时显式 join (如果需要等待任务完成)
        // 当前设计是发送信号后不等待，让任务自行结束。
        if self.progress_timer_join_handle.is_some() {
            // log::trace!("[UniLyric] 进度定时器任务句柄存在，将在任务结束时自动清理。");
            // 如果需要立即等待任务结束，可以取消注释下面的代码，但这可能阻塞UI线程
            // if let Some(handle) = self.progress_timer_join_handle.take() {
            //     tokio_runtime_arc.block_on(async { handle.await }).ok();
            // }
        }
    }

    /// 从存储的已处理歌词结果中加载歌词。
    /// 通常用于从自动搜索结果中选择一个来源并应用其歌词。
    ///
    /// # 参数
    /// * `stored_data` - `ProcessedLyricsSourceData` 结构，包含已处理的歌词和元数据。
    /// * `original_source_enum` - `AutoSearchSource` 枚举，指示此数据的原始来源。
    pub(crate) fn load_lyrics_from_stored_result(
        &mut self,
        stored_data: ProcessedLyricsSourceData,
        original_source_enum: AutoSearchSource,
    ) {
        log::info!(
            "[UniLyric] 从 {:?} 加载存储的歌词结果: 歌曲名 '{:?}', 艺术家 '{:?}'",
            original_source_enum.display_name(), // 显示来源名称
            stored_data // 从平台元数据中获取歌曲名和艺术家
                .platform_metadata
                .get("musicName") // 假设平台元数据中有 "musicName" 键
                .cloned()
                .unwrap_or_default(),
            stored_data
                .platform_metadata
                .get("artist") // 假设平台元数据中有 "artist" 键
                .cloned()
                .unwrap_or_default()
        );

        // 1. 记录当前来源，用于可能的后续处理（如歌词清理）
        self.last_auto_fetch_source_for_stripping_check = Some(original_source_enum);

        // 2. 设置应用状态以准备转换
        self.last_auto_fetch_source_format = Some(stored_data.format); // 记录原始获取格式，用于本地缓存保存
        self.clear_all_data(); // 清理旧的歌词数据和部分元数据（保留固定的元数据）

        self.input_text = stored_data.main_lyrics; // 使用存储的原始主歌词作为输入
        self.source_format = stored_data.format; // 设置源格式

        // 暂存从网络获取的次要歌词信息 (如果 stored_data 中有这些字段)
        self.pending_translation_lrc_from_download = stored_data.translation_lrc;
        self.pending_romanization_qrc_from_download = stored_data.romanization_qrc;
        self.pending_romanization_lrc_from_download = stored_data.romanization_lrc;
        self.pending_krc_translation_lines = stored_data.krc_translation_lines;
        self.session_platform_metadata = stored_data.platform_metadata; // 应用平台元数据
        self.metadata_source_is_download = true; // 标记元数据主要来自下载（或等效的已处理源）

        // 清理手动加载的LRC，因为我们将使用来自存储结果的（可能包含的）次要歌词
        self.loaded_translation_lrc = None;
        self.loaded_romanization_lrc = None;

        // 3. 执行核心转换逻辑
        self.handle_convert();

        // 4. 清理暂存的来源标记，以防影响后续非自动下载的操作
        self.last_auto_fetch_source_for_stripping_check = None;

        // 5. 发送TTML到AMLL Player (如果已连接且output_text非空)
        if self.media_connector_config.lock().unwrap().enabled {
            // 如果媒体连接器启用
            if let Some(tx) = &self.media_connector_command_tx {
                // 如果命令发送端存在
                if !self.output_text.is_empty() {
                    // 如果输出文本非空 (即已生成目标格式歌词)
                    log::info!(
                        "[UniLyricApp 加载存储结果] 发送从 {:?} 加载并处理后的 TTML (长度: {}) 到播放器。",
                        original_source_enum.display_name(),
                        self.output_text.len()
                    );
                    let ttml_body = ProtocolBody::SetLyricFromTTML {
                        // 构建协议体
                        data: self.output_text.as_str().into(), // 歌词数据
                    };
                    if tx // 发送命令
                        .send(ConnectorCommand::SendProtocolBody(ttml_body))
                        .is_err()
                    {
                        log::error!("[UniLyricApp 加载存储结果] 发送 TTML 失败。");
                    }
                } else {
                    log::warn!(
                        "[UniLyricApp 加载存储结果] 处理后输出为空，不发送TTML。来源: {:?}",
                        original_source_enum
                    );
                }
            }
        }

        // 6. 更新UI上对应源的搜索状态为成功
        //    这有助于用户了解哪个来源的歌词被成功加载了。
        let status_arc_to_update = match original_source_enum {
            AutoSearchSource::QqMusic => &self.qqmusic_auto_search_status,
            AutoSearchSource::Kugou => &self.kugou_auto_search_status,
            AutoSearchSource::Netease => &self.netease_auto_search_status,
            AutoSearchSource::AmllDb => &self.amll_db_auto_search_status,
            AutoSearchSource::LocalCache => &self.local_cache_auto_search_status,
        };
        // 只有当源不是本地缓存时，才强制更新状态为成功，
        // 因为本地缓存加载是不同的流程，其状态可能已通过其他方式管理。
        if original_source_enum != AutoSearchSource::LocalCache {
            *status_arc_to_update.lock().unwrap() = AutoSearchStatus::Success(self.source_format);
        }

        // 7. 标记UI已被用户主动填充内容
        self.current_auto_search_ui_populated = true;
        // 如果设置了“非总是搜索所有源”，则将其他源标记为未尝试（因为用户已主动加载了一个）
        if !self.app_settings.lock().unwrap().always_search_all_sources {
            app_fetch_core::set_other_sources_not_attempted(self, original_source_enum);
        }

        // 8. 显示成功提示
        self.toasts.add(egui_toast::Toast {
            text: format!(
                "已加载并处理来自 {} 的歌词",
                original_source_enum.display_name()
            )
            .into(),
            kind: egui_toast::ToastKind::Info,
            options: egui_toast::ToastOptions::default()
                .duration_in_seconds(2.5)
                .show_icon(true),
            ..Default::default()
        });
    }

    /// 处理用户选择新的 SMTC (System Media Transport Controls) 会话的逻辑。
    /// SMTC 会话通常对应一个正在播放媒体的应用。
    ///
    /// # 参数
    /// * `session_id_to_select` - 用户选择的会话 ID (通常是 SourceAppUserModelId)。
    ///   如果为 None，表示用户可能选择了“自动选择”或清除了特定选择。
    pub fn select_new_smtc_session(&mut self, session_id_to_select: Option<String>) {
        let mut current_selected_guard = self.selected_smtc_session_id.lock().unwrap(); // 获取当前选定会话ID的锁

        // 如果用户尝试选择当前已经选中的会话，则不执行任何操作
        if *current_selected_guard == session_id_to_select {
            log::trace!(
                "[UniLyricApp] 用户尝试选择当前已选中的 SMTC 会话 ({:?})，不执行操作。",
                session_id_to_select
            );
            return;
        }

        log::info!("[UniLyricApp] 切换 SMTC 会话到: {:?}", session_id_to_select);
        *current_selected_guard = session_id_to_select.clone(); // 更新当前选定的会话ID
        drop(current_selected_guard); // 释放锁

        // 保存用户选择到应用设置，以便下次启动时恢复
        if let Ok(mut settings) = self.app_settings.lock() {
            settings.last_selected_smtc_session_id = session_id_to_select.clone();
            if settings.save().is_err() {
                log::error!("[UniLyricApp] 保存用户选择的 SMTC 会话 ID 到设置失败。");
            }
        }

        // 向媒体连接器工作线程发送命令，通知其切换到新的SMTC会话
        if let Some(ref tx) = self.media_connector_command_tx {
            let command_payload = session_id_to_select.unwrap_or_default(); // 如果是None，则发送空字符串（表示自动）
            if tx
                .send(ConnectorCommand::SelectSmtcSession(command_payload.clone()))
                .is_err()
            {
                log::error!(
                    "[UniLyricApp] 发送 SelectSmtcSession 命令 (ID: {}) 失败。",
                    command_payload
                );
            } else {
                log::debug!(
                    "[UniLyricApp] 已发送 SelectSmtcSession 命令 (ID: {}) 给 worker。",
                    command_payload
                );
            }
        } else {
            log::warn!("[UniLyricApp] media_connector_command_tx 不可用，无法发送会话选择命令。");
        }
    }

    /// 处理 SMTC 更新，并将其信息发送到 WebSocket 服务器（如果启用）。
    /// 当 SMTC 报告新的播放信息（如歌曲标题、艺术家变化）时调用此方法。
    ///
    /// # 参数
    /// * `new_smtc_info` - `crate::amll_connector::NowPlayingInfo` 结构，包含新的播放信息。
    pub fn process_smtc_update_for_websocket(
        &mut self,
        new_smtc_info: &crate::amll_connector::NowPlayingInfo,
    ) {
        // 如果 WebSocket 服务器命令发送端存在
        if let Some(tx) = &self.websocket_server_command_tx {
            let title = new_smtc_info.title.clone(); // 获取歌曲标题
            let artist = new_smtc_info.artist.clone(); // 获取艺术家
            let ttml_lyrics = if !self.output_text.is_empty() {
                // 仅当有歌词（已处理并生成到 output_text）时发送
                Some(self.output_text.clone())
            } else {
                None // 如果没有歌词，则不发送
            };

            // 构建播放信息负载
            let playback_info_payload = PlaybackInfoPayload {
                title,
                artist,
                ttml_lyrics,
            };
            // 尝试发送播放信息到 WebSocket 服务器
            if let Err(e) = tx.try_send(ServerCommand::BroadcastPlaybackInfo(playback_info_payload))
            {
                // 使用 try_send 避免阻塞，如果通道已满或关闭则记录警告
                warn!(
                    "[UniLyricApp] 发送 PlaybackInfo 到 WebSocket 服务器失败 (通道可能已满或关闭): {}",
                    e
                );
            } else {
                // log::trace!("[UniLyricApp] 已发送 PlaybackInfo 到 WebSocket 服务器。"); // 成功发送，可选日志
            }
        }
    }

    /// 发送当前歌词（作为 PlaybackInfo 的一部分）到 WebSocket 服务器。
    /// 当歌词内容发生变化时（例如，用户编辑、加载新歌词、转换格式后）调用此方法。
    fn send_lyrics_update_to_websocket(&mut self) {
        // 如果 WebSocket 服务器命令发送端存在
        if let Some(tx) = &self.websocket_server_command_tx {
            // 我们需要从当前状态构建 PlaybackInfoPayload，因为协议要求 title 和 artist 也一起发送
            let current_title;
            let current_artist;
            {
                // 限制 media_info_guard 的作用域
                let media_info_guard = self.current_media_info.try_lock(); // 使用 try_lock 避免阻塞UI线程
                if let Ok(guard) = media_info_guard {
                    if let Some(info) = &*guard {
                        // 如果成功获取锁且有媒体信息
                        current_title = info.title.clone();
                        current_artist = info.artist.clone();
                    } else {
                        // 无媒体信息
                        current_title = None;
                        current_artist = None;
                    }
                } else {
                    // 获取锁失败
                    current_title = None;
                    current_artist = None;
                }
            } // media_info_guard 锁在此释放

            // 构建播放信息负载，包含当前歌词
            let playback_info_payload = PlaybackInfoPayload {
                title: current_title,
                artist: current_artist,
                ttml_lyrics: if self.output_text.is_empty() {
                    None
                } else {
                    Some(self.output_text.clone())
                }, // 如果歌词为空，则发送None
            };

            // 尝试发送播放信息（包含歌词更新）
            if let Err(e) = tx.try_send(ServerCommand::BroadcastPlaybackInfo(playback_info_payload))
            {
                warn!(
                    "[UniLyricApp] 发送歌词更新 (作为PlaybackInfo) 到 WebSocket 服务器失败: {}",
                    e
                );
            } else {
                // log::trace!("[UniLyricApp] 已发送歌词更新 (作为PlaybackInfo) 到 WebSocket 服务器。");
            }
        }
    }

    /// 发送当前播放时间更新到 WebSocket 服务器。
    /// 当 SMTC 报告播放时间变化，或者应用内部模拟播放进度时调用。
    ///
    /// # 参数
    /// * `current_time_ms` - 当前播放时间，单位毫秒。
    pub fn send_time_update_to_websocket(&self, current_time_ms: u64) {
        // 如果 WebSocket 服务器命令发送端存在
        if let Some(tx) = &self.websocket_server_command_tx {
            // 构建时间更新负载，时间单位转换为秒 (浮点数)
            let time_update_payload = TimeUpdatePayload {
                current_time_seconds: current_time_ms as f64 / 1000.0,
            };
            // 尝试发送时间更新
            if let Err(e) = tx.try_send(ServerCommand::BroadcastTimeUpdate(time_update_payload)) {
                warn!(
                    "[UniLyricApp] 发送 TimeUpdate 到 WebSocket 服务器失败 (通道可能已满或关闭): {}",
                    e
                );
            } else {
                // log::trace!("[UniLyricApp] 已发送 TimeUpdate ({:.3}s) 到 WebSocket 服务器。", current_time_ms as f64 / 1000.0);
            }
        }
    }

    /// 触发将当前TTML歌词上传到 TTML DB (通过 dpaste.org 和 GitHub Issue)。
    pub fn trigger_ttml_db_upload(&mut self) {
        // --- 阶段 A: 预检查 ---
        // 防止重复点击
        if self.ttml_db_upload_in_progress {
            log::warn!("[TTML数据库上传] 操作已在进行中，防止重复点击。");
            return;
        }
        // 检查是否有TTML输出且格式正确
        if self.output_text.is_empty() || self.target_format != LyricFormat::Ttml {
            log::error!("[TTML数据库上传] 没有TTML输出或格式不正确。");
            let error_msg = "错误：无TTML歌词内容，或当前输出非TTML格式。".to_string();
            // 发送准备错误消息给UI线程
            if self
                .ttml_db_upload_action_tx
                .send(TtmlDbUploadUserAction::PreparationError(error_msg.clone()))
                .is_err()
            {
                log::error!("[TTML数据库上传] 发送PreparationError消息失败 (无TTML)。");
            } else {
                // 同时显示一个toast提示
                self.toasts.add(Toast {
                    text: error_msg.into(),
                    kind: ToastKind::Error,
                    options: ToastOptions::default()
                        .duration_in_seconds(4.0)
                        .show_icon(true),
                    style: Default::default(),
                });
            }
            return;
        }
        // 检查元数据：艺术家和歌曲标题是否存在
        let artists_vec_opt: Option<Vec<String>>;
        let titles_vec_opt: Option<Vec<String>>;
        {
            // 限制元数据存储锁的范围
            let store_guard = self.metadata_store.lock().unwrap();
            artists_vec_opt = store_guard
                .get_multiple_values(&CanonicalMetadataKey::Artist)
                .cloned();
            titles_vec_opt = store_guard
                .get_multiple_values(&CanonicalMetadataKey::Title)
                .cloned();
        }
        let artists_exist = artists_vec_opt
            .as_ref()
            .is_some_and(|v| !v.is_empty() && v.iter().any(|s| !s.trim().is_empty()));
        let titles_exist = titles_vec_opt
            .as_ref()
            .is_some_and(|v| !v.is_empty() && v.iter().any(|s| !s.trim().is_empty()));

        if !artists_exist || !titles_exist {
            log::error!("[TTML数据库上传] 缺少艺术家或歌曲标题元数据。");
            let error_msg = "错误：上传前请确保歌词包含艺术家和歌曲标题元数据。".to_string();
            if self
                .ttml_db_upload_action_tx
                .send(TtmlDbUploadUserAction::PreparationError(error_msg.clone()))
                .is_err()
            {
                log::error!("[TTML数据库上传] 发送PreparationError消息失败 (元数据缺失)。");
            } else {
                self.toasts.add(Toast {
                    text: error_msg.into(),
                    kind: ToastKind::Error,
                    options: ToastOptions::default()
                        .duration_in_seconds(4.0)
                        .show_icon(true),
                    style: Default::default(),
                });
            }
            return;
        }
        // 准备用于GitHub Issue的艺术家和标题字符串
        let artist_str_for_meta = artists_vec_opt.unwrap_or_default().join("/");
        let title_str_for_meta = titles_vec_opt.unwrap_or_default().join("/");

        // --- 阶段 B: 设置状态并准备异步任务 ---
        self.ttml_db_upload_in_progress = true; // 标记上传正在进行
        self.ttml_db_last_paste_url = None; // 清除上一次的 dpaste URL
        // 清空可能残留的旧消息
        while self.ttml_db_upload_action_rx.try_recv().is_ok() {}

        // 克隆需要在异步任务中使用的数据
        let action_sender_clone = self.ttml_db_upload_action_tx.clone(); // 用于从异步任务发送消息回UI
        let ttml_content_to_upload = self.output_text.clone(); // 要上传的TTML内容
        let http_client_for_async = self.http_client.clone(); // HTTP客户端
        let tokio_runtime_arc = self.tokio_runtime.clone(); // Tokio运行时

        // TTML DB GitHub仓库信息
        let ttml_db_repo_owner = "Steve-xmh".to_string();
        let ttml_db_repo_name = "amll-ttml-db".to_string();

        let artist_for_async = artist_str_for_meta.clone();
        let title_for_async = title_str_for_meta.clone();

        // 发送初始“进行中”消息
        if action_sender_clone
            .send(TtmlDbUploadUserAction::InProgressUpdate(
                "正在上传TTML到dpaste.org...".to_string(),
            ))
            .is_err()
        {
            log::error!("[TTML数据库上传] 启动时发送InProgressUpdate消息失败。");
            self.ttml_db_upload_in_progress = false; // 重置状态
            return;
        }

        // --- 阶段 C: 执行异步任务 ---
        tokio_runtime_arc.spawn(async move { // 在Tokio运行时中执行异步块
            let dpaste_api_url = "https://dpaste.org/api/"; // dpaste.org API URL

            // 构建 multipart/form-data 请求体
            let form = reqwest::multipart::Form::new()
                .text("content", ttml_content_to_upload) // TTML内容
                .text("lexer", "xml")                   // 指定语法高亮为 XML
                .text("format", "url")                  // 要求返回纯文本的 URL
                .text("expires", "604800");             // 设置过期时间为7天 (604800秒)

            log::debug!(
                "[TTML数据库上传 异步任务] 开始上传到 dpaste.org (URL: {}).",
                dpaste_api_url
            );

            // 执行 dpaste.org 上传
            let dpaste_upload_result: Result<String, String> = async {
                let response = http_client_for_async
                    .post(dpaste_api_url)
                    .header("User-Agent", "UniLyricApp/0.1.0") // 设置User-Agent
                    .multipart(form) // 设置请求体
                    .send()
                    .await; // 发送请求并等待响应

                match response {
                    Ok(res) => {
                        // 请求成功发送
                        let status_code = res.status();
                        log::debug!(
                            "[TTML数据库上传 异步任务] dpaste.org API 响应状态码: {}",
                            status_code
                        );

                        if status_code == reqwest::StatusCode::OK { // 如果HTTP状态码为200 OK
                            match res.text().await { // 读取响应体文本
                                Ok(body_text) => {
                                    let base_url = body_text.trim().to_string(); // 去除首尾空格
                                    // 验证返回的是否为有效的URL
                                    if base_url.starts_with("http://") || base_url.starts_with("https://") {
                                        // 构建指向 raw 内容的 URL
                                        let raw_paste_url = if base_url.ends_with('/') {
                                            format!("{}raw", base_url)
                                        } else {
                                            format!("{}/raw", base_url)
                                        };
                                        log::info!(
                                            "[TTML数据库上传 异步任务] dpaste.org 上传成功，基础链接: {}, Raw链接: {}",
                                            base_url,
                                            raw_paste_url
                                        );
                                        Ok(raw_paste_url) // 返回 raw 内容的 URL
                                    } else {
                                        log::error!(
                                            "[TTML数据库上传 异步任务] dpaste.org API 响应成功但 Body 不是有效的 URL: {}",
                                            base_url.chars().take(100).collect::<String>() // 记录部分响应体
                                        );
                                        Err("dpaste.org API 响应成功但 Body 不是有效的 URL".to_string())
                                    }
                                }
                                Err(e) => {
                                    log::error!(
                                        "[TTML数据库上传 异步任务] 读取 dpaste.org API 成功响应的 Body 失败: {}",
                                        e
                                    );
                                    Err(format!("读取 dpaste.org API 成功响应的 Body 失败: {}", e))
                                }
                            }
                        } else { // HTTP状态码非200
                            let error_text = res
                                .text()
                                .await
                                .unwrap_or_else(|_| "获取 dpaste.org API 错误详情失败".to_string());
                            log::error!(
                                "[TTML数据库上传 异步任务] dpaste.org API 请求失败 (状态码 {}): {}",
                                status_code,
                                error_text.chars().take(250).collect::<String>() // 记录部分错误文本
                            );
                            Err(format!(
                                "dpaste.org API 请求失败 (状态码 {}): {}",
                                status_code,
                                error_text.chars().take(100).collect::<String>()
                            ))
                        }
                    }
                    Err(e) => { // 网络请求本身失败
                        log::error!(
                            "[TTML数据库上传 异步任务] 发送 dpaste.org API 请求网络错误: {}",
                            e
                        );
                        Err(format!("发送 dpaste.org API 请求网络错误: {}", e))
                    }
                }
            }
            .await; // dpaste 上传的 async 块结束

            // 处理 dpaste 上传结果
            match dpaste_upload_result {
                Ok(paste_url_from_api) => {
                    // dpaste 上传成功，准备 GitHub Issue URL
                    let issue_title_prefix = "[歌词提交] ";
                    let issue_title_content = format!("{} - {}", artist_for_async, title_for_async);
                    let final_issue_title_str = format!("{}{}", issue_title_prefix, issue_title_content);
                    let issue_title_encoded = urlencoding::encode(&final_issue_title_str).into_owned(); // URL编码标题

                    let labels_str = "歌词提交"; // GitHub Issue 标签
                    let labels_encoded = urlencoding::encode(labels_str).into_owned();

                    let assignees_str = "Steve-xmh"; // GitHub Issue 指派人
                    let assignees_encoded = urlencoding::encode(assignees_str).into_owned();

                    // 构建预填表单的 GitHub Issue URL
                    let github_issue_url_to_open = format!(
                        "https://github.com/{}/{}/issues/new?template=submit-lyric.yml&title={}&labels={}&assignees={}",
                        ttml_db_repo_owner,
                        ttml_db_repo_name,
                        issue_title_encoded,
                        labels_encoded,
                        assignees_encoded
                        // 注意：dpaste_url 需要通过 issue template 的 body 参数传递，
                        // 或者让用户手动粘贴。这里只生成打开 issue 页面的链接。
                        // 如果模板支持通过URL参数预填body，可以添加 &body=...paste_url_from_api...
                    );

                    // 发送成功消息给UI线程
                    if action_sender_clone.send(TtmlDbUploadUserAction::PasteReadyAndCopied {
                        paste_url: paste_url_from_api, // dpaste 的 raw URL
                        github_issue_url_to_open,     // GitHub Issue URL
                    }).is_err() {
                        log::error!("[TTML数据库上传 异步任务] 发送PasteReadyAndCopied消息失败。");
                    }
                }
                Err(e_msg) => {
                    // dpaste 上传失败
                    log::error!("[TTML数据库上传 异步任务] dpaste.org 上传流程失败: {}", e_msg);
                    // 发送错误消息给UI线程
                    if action_sender_clone.send(TtmlDbUploadUserAction::Error(format!("dpaste.org上传失败: {}", e_msg))).is_err() {
                         log::error!("[TTML数据库上传 异步任务] 发送Error消息失败。");
                    }
                }
            }
        }); // Tokio spawn 结束
    }
}

// 实现 eframe::App trait，用于定义应用的行为和UI更新逻辑
impl eframe::App for UniLyricApp {
    /// 每帧更新时调用此方法。
    ///
    /// # 参数
    /// * `ctx` - `egui::Context`，用于UI绘制和交互。
    /// * `_frame` - `eframe::Frame`，应用窗口的引用 (此处未使用，故用 `_` 开头)。
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 1. 处理窗口关闭请求
        // 如果用户请求关闭窗口 (例如点击关闭按钮) 且尚未启动关闭流程
        if ctx.input(|i| i.viewport().close_requested()) && !self.shutdown_initiated {
            self.shutdown_initiated = true; // 标记关闭流程已启动，防止重复执行
            log::trace!("[UniLyricApp 更新循环] 检测到窗口关闭请求。正在启动关闭序列...");

            // 停止进度模拟定时器
            self.stop_progress_timer();
            log::trace!(
                "[UniLyricApp 更新循环] UniLyricApp 内的 SMTC 进度模拟定时器已通过窗口关闭请求停止。"
            );

            // 向 AMLL Connector Worker 发送关闭命令
            if let Some(tx) = &self.media_connector_command_tx {
                log::trace!(
                    "[UniLyricApp 更新循环] 正在向 AMLL Connector Worker 发送 Shutdown 命令..."
                );
                if tx
                    .send(crate::amll_connector::ConnectorCommand::Shutdown)
                    .is_err()
                {
                    log::warn!(
                        "[UniLyricApp 更新循环] 向 AMLL Connector Worker 发送 Shutdown 命令失败 (通道可能已关闭)。"
                    );
                } else {
                    log::trace!(
                        "[UniLyricApp 更新循环] Shutdown 命令已成功发送给 AMLL Connector Worker。"
                    );
                }
            } else {
                log::warn!(
                    "[UniLyricApp 更新循环] media_connector_command_tx 为 None，无法向 Worker 发送 Shutdown 命令。"
                );
            }

            // 向 WebSocket 服务器任务发送关闭命令
            if let Some(ws_tx) = self.websocket_server_command_tx.take() {
                // 使用 take() 获取所有权
                log::trace!(
                    "[UniLyricApp 更新循环] 正在向 WebSocket 服务器任务发送 Shutdown 命令..."
                );
                let rt = std::sync::Arc::clone(&self.tokio_runtime); // 克隆 Tokio 运行时 Arc
                rt.spawn(async move { // 在 Tokio 运行时中异步发送关闭命令
                    if ws_tx.send(crate::websocket_server::ServerCommand::Shutdown).await.is_err() {
                        log::warn!("[UniLyricApp 更新循环] 向 WebSocket 服务器任务发送 Shutdown 命令失败 (通道可能已关闭)。");
                    } else {
                        log::trace!("[UniLyricApp 更新循环] Shutdown 命令已成功发送给 WebSocket 服务器任务。");
                    }
                });
            } else {
                log::warn!(
                    "[UniLyricApp 更新循环] websocket_server_command_tx 已为 None，无法向 WebSocket 服务器发送 Shutdown 命令。"
                );
            }
            log::trace!("[UniLyricApp 更新循环] 关闭信号已发送。等待 eframe 关闭应用。");
            // eframe 会在下一帧处理实际的窗口关闭
        }

        // 2. 处理来自其他线程的日志消息 (通过通道接收)
        app_update::process_log_messages(self);

        // 3. 处理各种下载完成事件 (例如 QQ音乐、酷狗、网易云、AMLL TTML)
        // 这些函数会检查对应的下载状态，并在成功或失败时处理数据或错误
        app_update::handle_qq_download_completion_logic(self);
        app_update::handle_kugou_download_completion_logic(self);
        app_update::handle_netease_download_completion_logic(self);
        app_update::handle_amll_ttml_download_completion_logic(self);

        // 4. 处理来自 AMLL Connector worker 的更新 (例如 SMTC 状态、播放信息)
        app_update::process_connector_updates(self);

        // 5. 处理自动获取歌词的结果 (当所有自动搜索源完成后)
        app_update::handle_auto_fetch_results(self);

        // 6. 请求UI重绘
        // 设置一个默认的重绘延迟，如果媒体连接器启用，则使用较短的延迟以更频繁地更新UI
        let mut desired_repaint_delay = Duration::from_millis(1000); // 默认1秒
        if self.media_connector_config.lock().unwrap().enabled {
            desired_repaint_delay = desired_repaint_delay.min(Duration::from_millis(500)); // 连接器启用时，0.5秒
        }
        ctx.request_repaint_after(desired_repaint_delay); // 请求在指定延迟后重绘

        // 7. 绘制所有UI面板和模态窗口
        app_update::draw_ui_elements(self, ctx);

        // 8. 处理文件拖放事件
        app_update::handle_file_drops(self, ctx);

        // 9. 处理 TTML DB 上传相关的用户操作和状态更新
        app_update::handle_ttml_db_upload_actions(self);

        // 10. 显示 toast 通知
        self.toasts.show(ctx);
    }
}
