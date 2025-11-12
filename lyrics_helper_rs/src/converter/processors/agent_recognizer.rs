//! 对唱识别器。

use regex::Regex;
use std::{borrow::Cow, collections::HashMap, sync::LazyLock};

use lyrics_helper_core::{Agent, AgentType, ContentType, LyricLine, ParsedSourceData};

/// 正则表达式，用于匹配行首的演唱者标记。
/// 支持全角/半角括号和冒号，以及无括号的情况。
/// 捕获组 1: 半角括号内的内容
/// 捕获组 2: 全角括号内的内容
/// 捕获组 3: 无括号的内容
static AGENT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:\((.+?)\)|（(.+?)）|([^\s:()（）]+))\s*[:：]\s*").unwrap()
});

/// 接收一个 `ParsedSourceData`，识别其中的演唱者，并直接修改它。
pub fn recognize_agents(data: &mut ParsedSourceData) {
    let original_lines = std::mem::take(&mut data.lines);
    let mut processed_lines = Vec::with_capacity(original_lines.len());
    let mut current_agent_id: Option<String> = None;

    let mut name_to_id_map: HashMap<String, String> = data
        .agents
        .all_agents()
        .filter_map(|agent| {
            agent
                .name
                .as_ref()
                .map(|name| (name.clone(), agent.id.clone()))
        })
        .collect();
    let mut next_agent_id_num = data.agents.agents_by_id.len() + 1;

    for mut line in original_lines {
        let full_text: String = get_text_from_main_track(&line).to_string();

        if let Some(captures) = AGENT_REGEX.captures(&full_text) {
            // 从多个捕获组中提取演唱者名称
            let agent_name = (1..=3)
                .find_map(|i| captures.get(i))
                .map(|m| m.as_str().trim().to_string());

            if let (Some(name), Some(full_match_capture)) = (agent_name, captures.get(0)) {
                let full_match_str = full_match_capture.as_str();

                let agent_id = name_to_id_map
                    .entry(name.clone())
                    .or_insert_with(|| {
                        let new_id = format!("v{next_agent_id_num}");
                        next_agent_id_num += 1;

                        let new_agent = Agent {
                            id: new_id.clone(),
                            name: Some(name),
                            agent_type: AgentType::Person,
                        };
                        data.agents.agents_by_id.insert(new_id.clone(), new_agent);

                        new_id
                    })
                    .clone();

                if let Some(remaining_text) = full_text.strip_prefix(full_match_str) {
                    if remaining_text.trim().is_empty() {
                        // 块模式: 如果标记后面没有文本，说明这只是一个标记行，用于标记后面行的演唱者
                        // 更新当前演唱者，并跳过此行
                        current_agent_id = Some(agent_id);
                        continue;
                    }
                    // 行模式: 标记和歌词在同一行。
                    line.agent = Some(agent_id.clone());
                    current_agent_id = Some(agent_id); // 更新当前演唱者以备后续行继承
                    clean_text_in_main_track(&mut line, full_match_str);
                }
            } else {
                // 正则匹配成功，但未能提取出有效的演唱者名称（理论上不太可能发生）
                line.agent.clone_from(&current_agent_id);
            }
        } else {
            // 整行都不匹配演唱者标记的格式
            if line.agent.is_some() {
                current_agent_id.clone_from(&line.agent);
            } else {
                line.agent.clone_from(&current_agent_id);
            }
        }

        processed_lines.push(line);
    }

    data.lines = processed_lines;
}

/// 辅助函数：从 `LyricLine` 中获取用于匹配的纯文本。
fn get_text_from_main_track(line: &LyricLine) -> Cow<'_, str> {
    line.tracks
        .iter()
        .find(|at| at.content_type == ContentType::Main)
        .map_or(Cow::Borrowed(""), |main_annotated_track| {
            let collected_string: String = main_annotated_track
                .content
                .words
                .iter()
                .flat_map(|w| &w.syllables)
                .map(|s| s.text.as_str())
                .collect();
            Cow::Owned(collected_string)
        })
}

/// 辅助函数：从主轨道的文本部分移除演唱者标记前缀。
fn clean_text_in_main_track(line: &mut LyricLine, prefix_to_remove: &str) {
    if let Some(main_annotated_track) = line
        .tracks
        .iter_mut()
        .find(|at| at.content_type == ContentType::Main)
    {
        let main_content_track = &mut main_annotated_track.content;
        let mut len_to_remove = prefix_to_remove.chars().count();
        if len_to_remove == 0 {
            return;
        }

        for word in &mut main_content_track.words {
            if len_to_remove == 0 {
                break;
            }

            let mut syllables_to_drain = 0;
            for syllable in &word.syllables {
                let syllable_len = syllable.text.chars().count();
                if len_to_remove >= syllable_len {
                    len_to_remove -= syllable_len;
                    syllables_to_drain += 1;
                } else {
                    break;
                }
            }

            if syllables_to_drain > 0 {
                word.syllables.drain(0..syllables_to_drain);
            }

            if len_to_remove > 0
                && let Some(first_syllable) = word.syllables.get_mut(0)
            {
                let first_syl_len = first_syllable.text.chars().count();
                if len_to_remove < first_syl_len {
                    first_syllable.text = first_syllable.text.chars().skip(len_to_remove).collect();
                } else {
                    word.syllables.remove(0);
                }
                len_to_remove = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use lyrics_helper_core::{AnnotatedTrack, ContentType, LyricSyllable, LyricTrack, Word};

    fn new_line(text: &str) -> LyricLine {
        let content_track = LyricTrack {
            words: vec![Word {
                syllables: vec![LyricSyllable {
                    text: text.to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        LyricLine {
            tracks: vec![AnnotatedTrack {
                content_type: ContentType::Main,
                content: content_track,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn new_syllable_line(syllables: Vec<&str>) -> LyricLine {
        let content_track = LyricTrack {
            words: vec![Word {
                syllables: syllables
                    .into_iter()
                    .map(|s| LyricSyllable {
                        text: s.to_string(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        };
        LyricLine {
            tracks: vec![AnnotatedTrack {
                content_type: ContentType::Main,
                content: content_track,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn test_recognize_agents_inline_mode() {
        let mut data = ParsedSourceData {
            lines: vec![
                new_line("汪：摘一颗苹果"),
                new_line("等你看我从门前过"),
                new_line("BY2：像夏天的可乐"),
                new_line("像冬天的可可"),
            ],
            ..Default::default()
        };

        recognize_agents(&mut data);

        assert_eq!(data.lines.len(), 4);
        assert_eq!(data.lines[0].agent.as_deref(), Some("v1"));
        assert_eq!(get_text_from_main_track(&data.lines[0]), "摘一颗苹果");

        assert_eq!(data.lines[1].agent.as_deref(), Some("v1"), "应继承ID 'v1'");

        assert_eq!(data.lines[2].agent.as_deref(), Some("v2"));
        assert_eq!(get_text_from_main_track(&data.lines[2]), "像夏天的可乐");

        assert_eq!(data.lines[3].agent.as_deref(), Some("v2"), "应继承ID 'v2'");

        assert_eq!(data.agents.agents_by_id.len(), 2);
        let agent1 = data.agents.agents_by_id.get("v1").unwrap();
        assert_eq!(agent1.name.as_deref(), Some("汪"));
        let agent2 = data.agents.agents_by_id.get("v2").unwrap();
        assert_eq!(agent2.name.as_deref(), Some("BY2"));
    }

    #[test]
    fn test_recognize_agents_block_mode() {
        let mut data = ParsedSourceData {
            lines: vec![
                new_line("TwoP："),
                new_line("都说爱情要慢慢来"),
                new_line("我的那个她却又慢半拍"),
                new_line("Stake:"),
                new_line("怕你跟不上我的节奏"),
            ],
            ..Default::default()
        };

        recognize_agents(&mut data);

        assert_eq!(data.lines.len(), 3, "纯标记行应被移除");

        assert_eq!(data.lines[0].agent.as_deref(), Some("v1"));
        assert_eq!(data.lines[1].agent.as_deref(), Some("v1"));
        assert_eq!(data.lines[2].agent.as_deref(), Some("v2"));

        assert_eq!(data.agents.agents_by_id.len(), 2);
        assert_eq!(
            data.agents.agents_by_id.get("v1").unwrap().name.as_deref(),
            Some("TwoP")
        );
        assert_eq!(
            data.agents.agents_by_id.get("v2").unwrap().name.as_deref(),
            Some("Stake")
        );
    }

    #[test]
    fn test_recognize_agents_mixed_and_complex() {
        let mut data = ParsedSourceData {
            lines: vec![
                new_line("（合）：合唱歌词"),
                new_line("第一句歌词"),
                new_syllable_line(vec!["TwoP", "："]),
                new_syllable_line(vec!["第", "二", "句", "逐", "字", "歌", "词"]),
                new_line("  Stake: 第三句行内歌词"),
                new_line("第四句继承Stake"),
            ],
            ..Default::default()
        };

        recognize_agents(&mut data);

        assert_eq!(data.lines.len(), 5);

        assert_eq!(data.lines[0].agent.as_deref(), Some("v1"));
        assert_eq!(get_text_from_main_track(&data.lines[0]), "合唱歌词");

        assert_eq!(data.lines[1].agent.as_deref(), Some("v1"));

        assert_eq!(data.lines[2].agent.as_deref(), Some("v2"));

        assert_eq!(data.lines[3].agent.as_deref(), Some("v3"));

        assert_eq!(data.lines[4].agent.as_deref(), Some("v3"));

        assert_eq!(data.agents.agents_by_id.len(), 3);
        assert_eq!(
            data.agents.agents_by_id.get("v1").unwrap().name.as_deref(),
            Some("合")
        );
        assert_eq!(
            data.agents.agents_by_id.get("v2").unwrap().name.as_deref(),
            Some("TwoP")
        );
        assert_eq!(
            data.agents.agents_by_id.get("v3").unwrap().name.as_deref(),
            Some("Stake")
        );
    }

    #[test]
    fn test_recognize_agents_no_agents() {
        let mut data = ParsedSourceData {
            lines: vec![new_line("这是一行普通歌词"), new_line("这是另一行普通歌词")],
            ..Default::default()
        };
        let original_data = data.clone();

        recognize_agents(&mut data);

        assert_eq!(data.lines.len(), 2);
        assert!(data.lines[0].agent.is_none());
        assert!(data.lines[1].agent.is_none());
        assert_eq!(data.agents.agents_by_id.len(), 0);
        assert_eq!(
            get_text_from_main_track(&data.lines[0]),
            get_text_from_main_track(&original_data.lines[0])
        );
    }
}
