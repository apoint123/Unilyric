//! 元数据处理器。

use std::{
    collections::{HashMap, HashSet},
    fmt::Write as FmtWrite,
};

use crate::{
    Agent, AgentStore, AgentType, CanonicalMetadataKey, ParseCanonicalMetadataKeyError,
    ParsedSourceData,
};

/// 一个用于存储、管理和规范化歌词元数据的中央容器。
#[derive(Debug, Clone, Default)]
pub struct MetadataStore {
    /// 存储所有元数据。键是规范化的枚举，值是该元数据的所有值列表。
    data: HashMap<CanonicalMetadataKey, Vec<String>>,
}

impl MetadataStore {
    /// 创建一个新的、空的 `MetadataStore` 实例。
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    /// 添加一个元数据键值对。
    ///
    /// 此函数会尝试将传入的字符串键 `key_str` 解析为 `CanonicalMetadataKey`。
    /// 如果无法解析，则将该键视为 `CanonicalMetadataKey::Custom`。
    ///
    /// # 参数
    /// * `key_str` - 原始的元数据键名，例如 "ti", "artist"。
    /// * `value` - 该键对应的值。
    ///
    /// # 返回
    /// 如果键的解析过程出现问题（虽然当前总会成功），则返回错误。
    pub fn add(
        &mut self,
        key_str: &str,
        value: &str,
    ) -> Result<(), ParseCanonicalMetadataKeyError> {
        let trimmed_value = value.trim();
        if trimmed_value.is_empty() {
            return Ok(());
        }

        let canonical_key = key_str
            .parse::<CanonicalMetadataKey>()
            // 如果解析失败，则将其视为一个自定义键
            .unwrap_or_else(|_| CanonicalMetadataKey::Custom(key_str.to_string()));

        self.data
            .entry(canonical_key)
            .or_default()
            .push(trimmed_value.to_string());

        Ok(())
    }

    /// 设置或覆盖一个单值元数据标签。
    ///
    /// 此函数会先清除指定标签的所有现有值，然后再添加新的单个值。
    /// 可用于从 API 等权威来源设置元数据（如标题、专辑）。
    ///
    /// # 参数
    /// * `key_str` - 原始的元数据键名，例如 "title", "artist"。
    /// * `value` - 要设置的新值。
    pub fn set_single(&mut self, key_str: &str, value: &str) {
        let canonical_key = key_str
            .parse::<CanonicalMetadataKey>()
            .unwrap_or_else(|_| CanonicalMetadataKey::Custom(key_str.to_string()));

        // `insert` 方法会直接覆盖掉指定键的旧值
        self.data
            .insert(canonical_key, vec![value.trim().to_string()]);
    }

    /// 设置或覆盖一个多值元数据标签。
    ///
    /// 类似于 `set_single`，但接受一个值的向量，用于艺术家等可能有多值的场景。
    ///
    /// # 参数
    /// * `key_str` - 原始的元数据键名，例如 "title", "artist"。
    /// * `values` - 要设置的新值列表。
    pub fn set_multiple(&mut self, key_str: &str, values: Vec<String>) {
        let canonical_key = key_str
            .parse::<CanonicalMetadataKey>()
            .unwrap_or_else(|_| CanonicalMetadataKey::Custom(key_str.to_string()));

        let cleaned_values = values.into_iter().map(|v| v.trim().to_string()).collect();

        self.data.insert(canonical_key, cleaned_values);
    }

    /// 获取指定元数据键的单个值。
    ///
    /// 如果一个键有多个值，此方法只返回第一个。
    #[must_use]
    pub fn get_single_value(&self, key: &CanonicalMetadataKey) -> Option<&String> {
        self.data.get(key).and_then(|values| values.first())
    }

    /// 获取指定元数据键的所有值。
    #[must_use]
    pub fn get_multiple_values(&self, key: &CanonicalMetadataKey) -> Option<&Vec<String>> {
        self.data.get(key)
    }

    /// 获取对所有元数据的不可变引用。
    #[must_use]
    pub fn get_all_data(&self) -> &HashMap<CanonicalMetadataKey, Vec<String>> {
        &self.data
    }

    /// 清空存储中的所有元数据。
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// 根据自定义的字符串键获取多个元数据值。
    ///
    /// # 参数
    /// * `key` - 用于查找的字符串键。
    ///
    /// # 返回
    /// * `Option<&Vec<String>>` - 如果找到，则返回对应的值切片引用。
    #[must_use]
    pub fn get_multiple_values_by_key(&self, key: &str) -> Option<&Vec<String>> {
        let canonical_key = key
            .parse::<CanonicalMetadataKey>()
            .unwrap_or_else(|_| CanonicalMetadataKey::Custom(key.to_string()));

        self.data.get(&canonical_key)
    }

    /// 对所有存储的元数据值进行清理和去重。
    ///
    /// 包括：
    /// 1. 移除每个值首尾的空白字符。
    /// 2. 移除完全为空的元数据条目。
    /// 3. 对每个键的值列表进行排序和去重。
    pub fn deduplicate_values(&mut self) {
        let mut keys_to_remove: Vec<CanonicalMetadataKey> = Vec::new();
        for (key, values) in &mut self.data {
            for v in values.iter_mut() {
                *v = v.trim().to_string();
            }
            values.retain(|v| !v.is_empty());

            if values.is_empty() {
                keys_to_remove.push(key.clone());
                continue;
            }

            values.sort_unstable();
            values.dedup();
        }

        // 移除所有值都为空的键
        for key in keys_to_remove {
            self.data.remove(&key);
        }
    }

    /// 移除一个元数据键及其所有关联的值。
    ///
    /// # 参数
    /// * `key_str` - 原始的元数据键名，例如 "ti", "artist"。
    pub fn remove(&mut self, key_str: &str) {
        let canonical_key = key_str
            .parse::<CanonicalMetadataKey>()
            .unwrap_or_else(|_| CanonicalMetadataKey::Custom(key_str.to_string()));

        self.data.remove(&canonical_key);
    }

    /// 生成通用的LRC元数据头部字符串。
    ///
    /// 包括：
    /// - ti
    /// - ar
    /// - al
    /// - by
    /// - language
    /// - offset
    ///
    /// 有多个值的，使用 "/" 连接（offset 除外）。
    #[must_use]
    pub fn generate_lrc_header(&self) -> String {
        let mut output = String::new();
        let mut written_keys: HashSet<&CanonicalMetadataKey> = HashSet::new();

        // 定义LRC标签和对应的CanonicalMetadataKey的映射
        let lrc_tags_to_write: Vec<(CanonicalMetadataKey, &str)> = vec![
            (CanonicalMetadataKey::Title, "ti"),
            (CanonicalMetadataKey::Artist, "ar"),
            (CanonicalMetadataKey::Album, "al"),
            (CanonicalMetadataKey::TtmlAuthorGithubLogin, "by"),
            (CanonicalMetadataKey::Language, "language"),
            (CanonicalMetadataKey::Offset, "offset"),
        ];

        for (key_type, lrc_tag_name) in &lrc_tags_to_write {
            if let Some(values) = self.data.get(key_type) {
                if values.is_empty() {
                    continue;
                }

                // 对所有非 offset 的键，都用 "/" 连接多个值
                let value_to_write = if key_type != &CanonicalMetadataKey::Offset {
                    values.join("/")
                } else if let Some(first_value) = values.first() {
                    first_value.clone()
                } else {
                    continue;
                };

                if !value_to_write.trim().is_empty() || *lrc_tag_name == "offset" {
                    let _ = writeln!(output, "[{}:{}]", lrc_tag_name, value_to_write.trim());
                    written_keys.insert(key_type);
                }
            }
        }
        output
    }

    /// 从元数据中构建一个 `AgentStore`。
    #[must_use]
    pub fn to_agent_store(&self) -> AgentStore {
        let mut store = AgentStore::default();

        if let Some(agent_definitions) = self.get_multiple_values_by_key("agent") {
            for def_string in agent_definitions {
                let (id, parsed_name) = match def_string.split_once('=') {
                    Some((id, name)) => (id.to_string(), Some(name.to_string())),
                    None => (def_string.clone(), None),
                };

                let is_chorus = id == "v1000"
                    || parsed_name.as_deref() == Some("合")
                    || parsed_name.as_deref() == Some("合唱");

                let final_name = if is_chorus { None } else { parsed_name };
                let agent_type = if is_chorus {
                    AgentType::Group
                } else {
                    AgentType::Person
                };

                let agent = Agent {
                    id: id.clone(),
                    name: final_name,
                    agent_type,
                };
                store.agents_by_id.insert(id, agent);
            }
        }
        store
    }

    /// 从一个原始的、未规范化的元数据 `HashMap` 中加载数据。
    ///
    /// 这个方法通常在解析完源文件后调用，用于将解析器产出的原始元数据
    /// (`HashMap<String, Vec<String>>`) 填入 `MetadataStore`，
    /// 在这个过程中会通过调用 `add` 方法来完成键的规范化和值的清理。
    ///
    /// # 参数
    ///
    /// * `raw_metadata` - 一个包含原始键值对的 `HashMap` 的引用。
    pub fn load_from_raw(&mut self, raw_metadata: &HashMap<String, Vec<String>>) {
        for (key, values) in raw_metadata {
            for value in values {
                let _ = self.add(key, &value.clone());
            }
        }
    }

    /// 将存储的元数据转换为一个可序列化（例如，转换为 JSON）的 `HashMap`。
    ///
    /// 此方法只包含被认为是“公共”的元数据项，用于输出。
    /// “公共”的定义由 `CanonicalMetadataKey::is_public()` 方法决定。
    /// 用于避免输出不适合展示的内部元数据。
    ///
    /// # 返回
    ///
    /// 返回一个新的 `HashMap<String, Vec<String>>`，其中键是元数据键的字符串表示。
    #[must_use]
    pub fn to_serializable_map(&self) -> HashMap<String, Vec<String>> {
        self.data
            .iter()
            .filter(|(key, _values)| key.is_public())
            .map(|(key, values)| (key.to_string(), values.clone()))
            .collect()
    }
}

/// 实现从 `ParsedSourceData` 到 `MetadataStore` 的转换
impl From<&ParsedSourceData> for MetadataStore {
    /// 从一个 `ParsedSourceData` 引用创建一个 `MetadataStore`。
    ///
    /// 用于在 API 调用中方便地将解析结果转换为元数据存储。
    fn from(parsed_data: &ParsedSourceData) -> Self {
        let mut store = MetadataStore::new();
        for (key, values) in &parsed_data.raw_metadata {
            for value in values {
                // 忽略错误，因为我们只是在构建一个临时的 store
                let _ = store.add(key, value);
            }
        }
        store.deduplicate_values();
        store
    }
}
