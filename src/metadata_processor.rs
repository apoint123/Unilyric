// 从 types.rs 导入 AssMetadata（用于从文件加载元数据时的中间结构）
// 和 CanonicalMetadataKey（元数据键的规范化枚举）以及 ParseCanonicalMetadataKeyError（解析键时的错误类型）
use crate::types::{CanonicalMetadataKey, ParseCanonicalMetadataKeyError};
// 导入标准库的 HashMap 用于存储规范化键到其值（或值列表）的映射
use std::collections::{HashMap, HashSet};
// 导入标准库的 fmt::Write trait，用于向字符串写入格式化文本
use std::fmt::Write as FmtWrite;
// 导入 quick_xml 库的相关类型，用于生成 TTML 元数据头部
use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use std::io::Cursor;
// FromStr trait 用于从字符串解析 CanonicalMetadataKey，它在 types.rs 中为 CanonicalMetadataKey 实现

/// `MetadataStore` 结构体用于存储和管理歌词的元数据。
/// 它使用 `CanonicalMetadataKey`作为键，将不同来源的元数据统一起来。
/// 值存储为 `Vec<String>` 以支持多值元数据项（例如多个艺术家）。
#[derive(Debug, Clone, Default)]
pub struct MetadataStore {
    data: HashMap<CanonicalMetadataKey, Vec<String>>, // 存储元数据，键是规范化的，值是字符串向量
    group1_output_order: Vec<CanonicalMetadataKey>, // 定义 Group 1 格式 (LRC, QRC等) 元数据输出的推荐顺序
}

impl MetadataStore {
    /// 创建一个新的、空的 `MetadataStore` 实例。
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
            // 初始化 Group 1 格式元数据的输出顺序
            // 这个顺序主要影响 LRC, QRC, KRC, YRC, LYS 等格式的头部元数据标签的排列
            group1_output_order: vec![
                CanonicalMetadataKey::Title,
                CanonicalMetadataKey::Artist,
                CanonicalMetadataKey::Album,
                CanonicalMetadataKey::Author, // 通常对应 [by:]
                CanonicalMetadataKey::Language,
                CanonicalMetadataKey::Offset,
                CanonicalMetadataKey::Length,
                CanonicalMetadataKey::Editor,  // 通常对应 [re:]
                CanonicalMetadataKey::Version, // 通常对应 [ve:]
                CanonicalMetadataKey::KrcInternalTranslation, // KRC 特有的 [language:] 标签
                CanonicalMetadataKey::Songwriter, // 作曲/作词者
                CanonicalMetadataKey::AppleMusicId,
                // 其他自定义的 CanonicalMetadataKey 如果有固定顺序需求，也可以在这里添加
            ],
        }
    }

    /// 返回 Group 1 格式元数据输出顺序的引用。
    pub fn get_group1_output_order(&self) -> &[CanonicalMetadataKey] {
        &self.group1_output_order
    }

    /// 返回一个迭代器，用于遍历存储中的所有元数据项（键和对应的值向量）。
    pub fn iter_all(&self) -> impl Iterator<Item = (&CanonicalMetadataKey, &Vec<String>)> {
        self.data.iter()
    }

    /// 检查元数据存储是否为空。
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// 添加一条元数据。如果键已存在，则将新值追加到该键的值列表中。
    ///
    /// # Arguments
    /// * `key_str` - 元数据的键（字符串形式，将被尝试解析为 `CanonicalMetadataKey`）。
    /// * `value` - 元数据的值。
    ///
    /// # Returns
    /// `Result<(), ParseCanonicalMetadataKeyError>` - 如果键解析成功则返回 Ok，否则返回解析错误。
    pub fn add(
        &mut self,
        key_str: &str,
        value: String,
    ) -> Result<(), ParseCanonicalMetadataKeyError> {
        let trimmed_value = value.trim(); // 去除值两端的空白
        // 即使值为空字符串，也尝试添加。是否使用空值由后续的生成逻辑决定。
        // if trimmed_value.is_empty() {
        //     return Ok(());
        // }

        // 尝试将字符串键解析为规范化的 CanonicalMetadataKey
        match key_str.parse::<CanonicalMetadataKey>() {
            Ok(canonical_key) => {
                self.data
                    .entry(canonical_key.clone())
                    .or_default()
                    .push(trimmed_value.to_string());
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// 清空存储中的所有元数据。
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// 获取指定规范化键的第一个值（如果存在）。
    /// 对于单值元数据项，这通常是期望的获取方式。
    pub fn get_single_value(&self, key: &CanonicalMetadataKey) -> Option<&String> {
        self.data.get(key).and_then(|values| values.first())
    }

    /// 通过字符串键获取第一个值。内部会先将字符串键解析为 `CanonicalMetadataKey`。
    pub fn get_single_value_by_str(&self, key_str: &str) -> Option<&String> {
        if let Ok(canonical_key) = key_str.parse::<CanonicalMetadataKey>() {
            self.get_single_value(&canonical_key)
        } else {
            None
        }
    }

    /// 获取指定规范化键的所有值（一个字符串向量的引用）。
    /// 用于处理可能有多值的元数据项（如多个艺术家）。
    pub fn get_multiple_values(&self, key: &CanonicalMetadataKey) -> Option<&Vec<String>> {
        self.data.get(key)
    }

    /// 移除指定规范化键及其所有关联值。
    pub fn remove(&mut self, key: &CanonicalMetadataKey) {
        self.data.remove(key);
    }

    /// 对存储中的所有值进行去重和清理。
    /// 1. Trim 每个值。
    /// 2. 移除 Trim 后变为空字符串的值。
    /// 3. 对每个键的值列表进行排序和去重，移除完全相同的字符串。
    /// 4. 如果一个键的所有值都被移除（列表变空），则移除该键本身。
    pub fn deduplicate_values(&mut self) {
        let mut keys_to_remove_if_all_values_became_empty: Vec<CanonicalMetadataKey> = Vec::new();

        for (key, values) in self.data.iter_mut() {
            if values.is_empty() {
                // 如果值列表已为空，标记此键以便后续移除
                keys_to_remove_if_all_values_became_empty.push(key.clone());
                continue;
            }

            // 1. Trim 所有值
            values.iter_mut().for_each(|v| *v = v.trim().to_string());

            // 2. 移除处理后为空的字符串
            values.retain(|v| !v.is_empty());

            if values.is_empty() {
                // 如果移除空字符串后列表变空，标记此键
                keys_to_remove_if_all_values_became_empty.push(key.clone());
                continue;
            }

            // 3. 排序并去重 (dedup 需要已排序的切片)
            values.sort_unstable(); // 使用不稳定排序，因为值的顺序通常不重要
            values.dedup(); // 移除连续的重复项
        }

        // 移除那些在处理后值列表变为空的键
        for key_to_remove in keys_to_remove_if_all_values_became_empty {
            self.data.remove(&key_to_remove);
        }
    }

    /// 内部辅助函数，用于生成基于标签的元数据字符串（例如 LRC, QRC 的头部）。
    ///
    /// # Arguments
    /// * `artist_separator` - 当一个键有多个值时（特别是艺术家），用于连接这些值的分隔符。
    /// * `ensure_offset_zero` - 是否在没有 offset 标签时强制添加 `[offset:0]`。
    ///
    /// # Returns
    /// `String` - 生成的元数据标签字符串，每行一个标签。
    fn _generate_generic_tag_metadata(
        &self,
        artist_separator: &str,
        ensure_offset_zero: bool,
    ) -> String {
        let mut output = String::new();
        let mut written_keys: std::collections::HashSet<&CanonicalMetadataKey> =
            std::collections::HashSet::new();

        // 1. 首先按照 `group1_output_order` 中定义的顺序处理元数据
        for key_type in self.get_group1_output_order() {
            // `get_group1_tag_name_for_lrc_qrc` 方法定义在 `types.rs` 的 `CanonicalMetadataKey` impl 中
            if let Some(tag_name) = key_type.get_group1_tag_name_for_lrc_qrc() {
                if let Some(values) = self.data.get(key_type) {
                    // 获取该规范化键的值列表
                    if !values.is_empty() {
                        // 将所有非空值用指定分隔符连接起来
                        let value_str = values
                            .iter()
                            .map(|s| s.trim()) // 先 trim 每个值
                            .filter(|s| !s.is_empty()) // 过滤掉 trim 后为空的值
                            .collect::<Vec<&str>>()
                            .join(artist_separator);
                        if !value_str.is_empty() {
                            // 如果连接后的值非空
                            let _ = writeln!(output, "[{}:{}]", tag_name, value_str); // 写入标签
                            written_keys.insert(key_type); // 标记此键已处理
                        }
                    }
                }
            }
        }

        // 2. 处理 `group1_output_order` 中未包含的其他键
        for (key_type, values) in self.iter_all() {
            if written_keys.contains(key_type) {
                // 跳过已按顺序处理的键
                continue;
            }
            if let Some(tag_name) = key_type.get_group1_tag_name_for_lrc_qrc() {
                // 获取对应的标签名
                if !values.is_empty() {
                    let value_str = values
                        .iter()
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<&str>>()
                        .join(artist_separator);
                    if !value_str.is_empty() {
                        let _ = writeln!(output, "[{}:{}]", tag_name, value_str);
                    }
                }
            }
        }

        // 3. 如果需要，确保输出包含 [offset:0]
        if ensure_offset_zero
            && self
                .data
                .get(&CanonicalMetadataKey::Offset) // 检查是否存在 Offset 键
                .is_none_or(|v| v.is_empty() || v.first().is_none_or(|s| s.trim().is_empty()))
        {
            // 或者其值为空
            if !output.contains("[offset:") {
                // 并且输出中还没有 offset 标签
                let _ = writeln!(output, "[offset:0]");
            }
        }
        output
    }

    /// 生成 LRC 格式的元数据头部字符串。
    pub fn generate_lrc_metadata_string(&self) -> String {
        self._generate_generic_tag_metadata("/", false)
    }

    /// 生成 QRC, KRC 格式通用的元数据头部字符串。
    pub fn generate_qrc_krc_metadata_string(&self) -> String {
        self._generate_generic_tag_metadata("/", false)
    }

    /// 生成 LYS 格式的元数据头部字符串。
    pub fn generate_lys_metadata_string(&self) -> String {
        self._generate_generic_tag_metadata("/", false)
    }

    /// 将存储的元数据写入 TTML 文件的 `<head><metadata>...</metadata></head>` 部分。
    ///
    /// # Arguments
    /// * `store` - 对 `MetadataStore` 的引用。
    /// * `writer` - 用于写入 XML 事件的 `quick_xml::Writer`。
    ///
    /// # Returns
    /// `Result<(), quick_xml::Error>` - 成功或失败。
    pub fn write_ttml_head_metadata(
        store: &MetadataStore,
        writer: &mut Writer<Cursor<&mut Vec<u8>>>,
        paragraph_agent_ids: &HashSet<String>,
    ) -> Result<(), quick_xml::Error> {
        writer.write_event(Event::Start(BytesStart::new("head")))?;
        writer.write_event(Event::Start(BytesStart::new("metadata")))?;

        let mut all_agent_ids_to_process = HashSet::<String>::new();

        // 1. 从 store 中收集 agent ID (通常是 Custom("v1"), Custom("v2") 等)
        for (canonical_key, _values) in store.iter_all() {
            if let CanonicalMetadataKey::Custom(key_str) = canonical_key {
                if key_str.starts_with('v')
                    && key_str.len() > 1
                    && key_str[1..].chars().all(char::is_numeric)
                {
                    all_agent_ids_to_process.insert(key_str.clone());
                }
            }
        }

        // 2. 添加从段落中收集到的 agent ID
        for agent_id_from_para in paragraph_agent_ids {
            all_agent_ids_to_process.insert(agent_id_from_para.clone());
        }

        // 为了输出顺序一致，对 agent ID 进行排序
        let mut sorted_agent_ids: Vec<String> = all_agent_ids_to_process.into_iter().collect();
        sorted_agent_ids.sort_unstable(); // 或者 sort() 如果需要稳定排序

        for agent_id_str in &sorted_agent_ids {
            // 尝试从 store 中获取该 agent_id 的名字
            let agent_name_from_store: Option<String> = agent_id_str
                .parse::<CanonicalMetadataKey>() // 这会尝试解析为 Custom(agent_id_str)
                .ok()
                .and_then(|ck| store.get_single_value(&ck).cloned());

            if let Some(name) = agent_name_from_store {
                if !name.trim().is_empty() {
                    // 如果有名字，则写入完整的 agent 标签
                    let mut agent_tag_xml = BytesStart::new("ttm:agent");
                    agent_tag_xml.push_attribute(("type", "person"));
                    agent_tag_xml.push_attribute(("xml:id", agent_id_str.as_str()));
                    writer.write_event(Event::Start(agent_tag_xml))?;

                    let mut name_tag_xml = BytesStart::new("ttm:name");
                    name_tag_xml.push_attribute(("type", "full"));
                    writer.write_event(Event::Start(name_tag_xml))?;
                    writer.write_event(Event::Text(BytesText::new(name.trim())))?;
                    writer.write_event(Event::End(BytesEnd::new("ttm:name")))?;

                    writer.write_event(Event::End(BytesEnd::new("ttm:agent")))?;
                } else {
                    // 如果 store 中有此 agent_id 但名字为空，则写入无名 agent 标签
                    let mut agent_tag_xml = BytesStart::new("ttm:agent");
                    agent_tag_xml.push_attribute(("type", "person"));
                    agent_tag_xml.push_attribute(("xml:id", agent_id_str.as_str()));
                    writer.write_event(Event::Start(agent_tag_xml))?;
                    writer.write_event(Event::End(BytesEnd::new("ttm:agent")))?;
                }
            } else {
                // 如果 store 中没有此 agent_id (意味着它只在段落中出现)，则写入无名 agent 标签
                let mut agent_tag_xml = BytesStart::new("ttm:agent");
                agent_tag_xml.push_attribute(("type", "person"));
                agent_tag_xml.push_attribute(("xml:id", agent_id_str.as_str()));
                writer.write_event(Event::Start(agent_tag_xml))?;
                writer.write_event(Event::End(BytesEnd::new("ttm:agent")))?;
            }
        }

        // 2. 处理 <iTunesMetadata> 中的 <songwriters>
        if let Some(songwriters_vec) = store.get_multiple_values(&CanonicalMetadataKey::Songwriter)
        {
            let valid_songwriters: Vec<&String> = songwriters_vec
                .iter()
                .filter(|s| !s.trim().is_empty())
                .collect();
            if !valid_songwriters.is_empty() {
                let mut itunes_meta_tag_xml = BytesStart::new("iTunesMetadata");
                itunes_meta_tag_xml
                    .push_attribute(("xmlns", "http://music.apple.com/lyric-ttml-internal"));
                writer.write_event(Event::Start(itunes_meta_tag_xml))?;
                writer.write_event(Event::Start(BytesStart::new("songwriters")))?;
                for sw in valid_songwriters {
                    writer.write_event(Event::Start(BytesStart::new("songwriter")))?;
                    writer.write_event(Event::Text(BytesText::new(sw.trim())))?;
                    writer.write_event(Event::End(BytesEnd::new("songwriter")))?;
                }
                writer.write_event(Event::End(BytesEnd::new("songwriters")))?;
                writer.write_event(Event::End(BytesEnd::new("iTunesMetadata")))?;
            }
        }

        // 3. 处理预定义的 <amll:meta> 标签
        let amll_keys_map: Vec<(&str, CanonicalMetadataKey)> = vec![
            ("album", CanonicalMetadataKey::Album),
            ("appleMusicId", CanonicalMetadataKey::AppleMusicId),
            ("artists", CanonicalMetadataKey::Artist),
            ("isrc", CanonicalMetadataKey::Custom("isrc".to_string())),
            ("musicName", CanonicalMetadataKey::Title),
            (
                "ncmMusicId",
                CanonicalMetadataKey::Custom("ncmMusicId".to_string()),
            ),
            (
                "qqMusicId",
                CanonicalMetadataKey::Custom("qqMusicId".to_string()),
            ),
            (
                "spotifyId",
                CanonicalMetadataKey::Custom("spotifyId".to_string()),
            ),
            (
                "ttmlAuthorGithub",
                CanonicalMetadataKey::Custom("ttmlAuthorGithub".to_string()),
            ),
            ("ttmlAuthorGithubLogin", CanonicalMetadataKey::Author),
        ];

        let mut handled_canonical_keys_for_amll: HashSet<&CanonicalMetadataKey> = HashSet::new();

        for (output_key_name, canonical_key_in_map) in amll_keys_map.iter() {
            if let Some(values_vec) = store.get_multiple_values(canonical_key_in_map) {
                for value in values_vec {
                    // 为每个值创建一个新的 amll:meta 标签
                    let trimmed_value = value.trim();
                    if !trimmed_value.is_empty() {
                        let mut amll_meta_tag_xml = BytesStart::new("amll:meta");
                        amll_meta_tag_xml.push_attribute(("key", *output_key_name));
                        amll_meta_tag_xml.push_attribute(("value", trimmed_value));
                        writer.write_event(Event::Start(amll_meta_tag_xml))?;
                        writer.write_event(Event::End(BytesEnd::new("amll:meta")))?;
                    }
                }
                handled_canonical_keys_for_amll.insert(canonical_key_in_map);
            }
        }

        // 处理所有其他的 Custom 类型的元数据 ---
        for (canonical_key_from_store, values_vec) in store.iter_all() {
            if !handled_canonical_keys_for_amll.contains(canonical_key_from_store) {
                match canonical_key_from_store {
                    CanonicalMetadataKey::Language | CanonicalMetadataKey::Songwriter => {
                        continue;
                    }
                    CanonicalMetadataKey::Custom(custom_key_str) => {
                        if ["v1", "v2", "v1000"].contains(&custom_key_str.as_str()) {
                            // 避免重复处理已作为 agent 的 custom key
                            if sorted_agent_ids.contains(custom_key_str) {
                                // 如果这个 custom key 已经被上面的 agent 逻辑处理了
                                continue;
                            }
                        }

                        let is_predefined_output_key_name = amll_keys_map
                            .iter()
                            .any(|(out_key, _)| *out_key == custom_key_str);
                        if !custom_key_str.is_empty() && !is_predefined_output_key_name {
                            for value in values_vec {
                                let trimmed_value = value.trim();
                                if !trimmed_value.is_empty() {
                                    let mut amll_meta_tag_xml = BytesStart::new("amll:meta");
                                    amll_meta_tag_xml
                                        .push_attribute(("key", custom_key_str.as_str()));
                                    amll_meta_tag_xml.push_attribute(("value", trimmed_value));
                                    writer.write_event(Event::Start(amll_meta_tag_xml))?;
                                    writer.write_event(Event::End(BytesEnd::new("amll:meta")))?;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        writer.write_event(Event::End(BytesEnd::new("metadata")))?;
        writer.write_event(Event::End(BytesEnd::new("head")))?;
        Ok(())
    }

    /// 生成 ASS Event Comment 格式的元数据行。
    /// 对于多值元数据项，每个值会生成一个单独的 Comment 行。
    ///
    /// # Arguments
    /// * `style_name` - 用于这些 Comment 行的 ASS 样式名称 (例如 "meta")。
    ///
    /// # Returns
    /// `String` - 生成的 ASS Comment 元数据行，每行一个。
    pub fn generate_ass_event_comment_metadata_lines(&self, style_name: &str) -> String {
        let mut comment_output = String::new(); // 初始化输出字符串
        // 定义哪些规范化键应该映射到 ASS Comment 中的特定键名
        let comment_keys_map = [
            (CanonicalMetadataKey::Artist, "artist"),
            (CanonicalMetadataKey::Album, "album"),
            (CanonicalMetadataKey::Songwriter, "songwriter"),
            (CanonicalMetadataKey::AppleMusicId, "appleMusicId"),
            (CanonicalMetadataKey::Author, "ttmlAuthorGithubLogin"), // 可以按需添加更多已知键的映射
        ];

        let mut handled_keys_for_comments = std::collections::HashSet::new(); // 跟踪已处理的键，避免重复

        // 1. 处理预定义映射的键
        for (key_type_in_map, comment_key_name_in_map) in comment_keys_map.iter() {
            if let Some(values) = self.data.get(key_type_in_map) {
                // 获取该键的所有值
                if !values.is_empty() {
                    // 为每个值创建一个新的 Comment 行
                    for value in values {
                        let trimmed_value = value.trim(); // 对每个值进行 trim
                        if !trimmed_value.is_empty() {
                            // 只处理非空值
                            // 写入 ASS Comment 行
                            let _ = writeln!(
                                comment_output,
                                "Comment: 0,0:00:00.00,0:00:00.00,{},,0,0,0,,{}: {}",
                                style_name, *comment_key_name_in_map, trimmed_value
                            );
                        }
                    }
                    handled_keys_for_comments.insert(key_type_in_map); // 标记此键已处理（即使它有多个值，键本身只处理一次）
                }
            }
        }

        // 2. 处理其余的 Custom 键 (或其他未在上面映射的已知键)
        for (key_type_from_store, values) in self.iter_all() {
            // 跳过已处理的键，以及通常在 [Script Info] 中处理的 Title 和 Author
            if handled_keys_for_comments.contains(key_type_from_store)
                || matches!(key_type_from_store, CanonicalMetadataKey::Title)
            {
                continue;
            }

            // 对于 Custom 类型，使用其内部字符串作为ASS注释的键名
            // 其他未明确映射的 CanonicalMetadataKey 类型，如果需要输出，需要确定其在ASS Comment中的键名
            let ass_comment_key_name = match key_type_from_store {
                CanonicalMetadataKey::Custom(custom_key_str) => custom_key_str.clone(),
                // 可以为其他未在 comment_keys_map 中明确列出的 CanonicalMetadataKey 提供默认的ASS键名
                // 例如: key_type_from_store.to_display_key()
                // 但要注意 to_display_key() 可能包含不适合做ASS键的字符，或与已有键冲突
                // 为简单起见，这里只处理 Custom，其他未映射的非Custom键将被忽略，除非在 comment_keys_map 中定义
                _ => continue,
            };

            if !values.is_empty() && !ass_comment_key_name.is_empty() {
                // 为每个值创建一个新的 Comment 行
                for value in values {
                    let trimmed_value = value.trim();
                    if !trimmed_value.is_empty() {
                        let _ = writeln!(
                            comment_output,
                            "Comment: 0,0:00:00.00,0:00:00.00,{},,0,0,0,,{}: {}",
                            style_name, ass_comment_key_name, trimmed_value
                        );
                    }
                }
            }
        }
        comment_output
    }
}
