use crate::amll_connector::{protocol, protocol_strings::NullString};
use lyrics_helper_rs::converter::types as helper_types;
use std::{collections::HashMap, iter};

const CHORUS_AGENT_ID: &str = "v1000";
const PREFERRED_TRANSLATION_LANG: &str = "zh-CN";

trait SectionLike {
    fn syllables(&self) -> &[helper_types::LyricSyllable];
    fn translations(&self) -> &[helper_types::TranslationEntry];
    fn romanizations(&self) -> &[helper_types::RomanizationEntry];
}

impl SectionLike for helper_types::LyricLine {
    fn syllables(&self) -> &[helper_types::LyricSyllable] {
        &self.main_syllables
    }
    fn translations(&self) -> &[helper_types::TranslationEntry] {
        &self.translations
    }
    fn romanizations(&self) -> &[helper_types::RomanizationEntry] {
        &self.romanizations
    }
}

impl SectionLike for helper_types::BackgroundSection {
    fn syllables(&self) -> &[helper_types::LyricSyllable] {
        &self.syllables
    }
    fn translations(&self) -> &[helper_types::TranslationEntry] {
        &self.translations
    }
    fn romanizations(&self) -> &[helper_types::RomanizationEntry] {
        &self.romanizations
    }
}

fn extract_line_components<T: SectionLike>(
    section: &T,
) -> (Vec<protocol::LyricWord>, String, String) {
    let words = section
        .syllables()
        .iter()
        .map(|syllable| {
            let word_text = if syllable.ends_with_space {
                format!("{} ", syllable.text)
            } else {
                syllable.text.clone()
            };

            protocol::LyricWord {
                start_time: syllable.start_ms,
                end_time: syllable.end_ms,
                word: NullString(word_text),
            }
        })
        .collect();

    let translation = section
        .translations()
        .iter()
        .find(|t| {
            t.lang
                .as_deref()
                .is_some_and(|lang| lang.eq_ignore_ascii_case(PREFERRED_TRANSLATION_LANG))
        })
        .or_else(|| section.translations().first())
        .map_or(String::new(), |t| t.text.clone());

    let romanization = section
        .romanizations()
        .first()
        .map_or(String::new(), |r| r.text.clone());

    (words, translation, romanization)
}

pub(super) fn convert_to_protocol_lyrics(
    source_data: &helper_types::ParsedSourceData,
) -> Vec<protocol::LyricLine> {
    let mut agent_duet_map: HashMap<String, bool> = HashMap::new();

    source_data
        .lines
        .iter()
        .flat_map(|helper_line| {
            let current_line_is_duet = match helper_line.agent.as_deref() {
                None | Some(CHORUS_AGENT_ID) => false,
                Some(agent_id) => {
                    if let Some(is_duet) = agent_duet_map.get(agent_id) {
                        *is_duet
                    } else {
                        // 为新出现的 agent 交替分配 `false` 和 `true`。
                        // 上层已对歌词行进行排序，所以这里不需要排序。
                        let new_duet_status = !agent_duet_map.len().is_multiple_of(2);
                        agent_duet_map.insert(agent_id.to_string(), new_duet_status);
                        new_duet_status
                    }
                }
            };

            let (words, translated_lyric, roman_lyric) = extract_line_components(helper_line);
            let main_line_iter = iter::once(protocol::LyricLine {
                start_time: helper_line.start_ms,
                end_time: helper_line.end_ms,
                words,
                translated_lyric: NullString(translated_lyric),
                roman_lyric: NullString(roman_lyric),
                is_bg: false,
                is_duet: current_line_is_duet,
            });

            let background_line_iter = helper_line
                .background_section
                .as_ref()
                .map(|bg_section| {
                    let (bg_words, bg_translation, bg_romanization) =
                        extract_line_components(bg_section);
                    protocol::LyricLine {
                        start_time: bg_section.start_ms,
                        end_time: bg_section.end_ms,
                        words: bg_words,
                        translated_lyric: NullString(bg_translation),
                        roman_lyric: NullString(bg_romanization),
                        is_bg: true,
                        is_duet: current_line_is_duet,
                    }
                })
                .into_iter();

            main_line_iter.chain(background_line_iter)
        })
        .collect()
}
