use crate::amll_connector::{protocol, protocol_strings::NullString};
use lyrics_helper_core::converter::types as helper_types;
use std::collections::HashMap;

const CHORUS_AGENT_ID: &str = "v1000";
const PREFERRED_TRANSLATION_LANG: &str = "zh-CN";

fn get_track_text(track: &helper_types::LyricTrack) -> String {
    track
        .words
        .iter()
        .flat_map(|word| &word.syllables)
        .map(|syl| {
            if syl.ends_with_space {
                format!("{} ", syl.text)
            } else {
                syl.text.clone()
            }
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn extract_line_components(
    syllables: &[helper_types::LyricSyllable],
    translations: &[helper_types::LyricTrack],
    romanizations: &[helper_types::LyricTrack],
) -> (Vec<protocol::LyricWord>, String, String) {
    let words = syllables
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

    let translation = translations
        .iter()
        .find(|t| {
            t.metadata
                .get(&helper_types::TrackMetadataKey::Language)
                .is_some_and(|lang| lang.eq_ignore_ascii_case(PREFERRED_TRANSLATION_LANG))
        })
        .or_else(|| translations.first())
        .map_or(String::new(), get_track_text);

    let romanization = romanizations.first().map_or(String::new(), get_track_text);

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

            let main_annotated_track = helper_line
                .tracks
                .iter()
                .find(|t| t.content_type == helper_types::ContentType::Main);

            let main_line_iter = main_annotated_track
                .map(|main_track| {
                    let main_syllables: Vec<_> = main_track
                        .content
                        .words
                        .iter()
                        .flat_map(|w| &w.syllables)
                        .cloned()
                        .collect();

                    let (words, translated_lyric, roman_lyric) = extract_line_components(
                        &main_syllables,
                        &main_track.translations,
                        &main_track.romanizations,
                    );

                    protocol::LyricLine {
                        start_time: helper_line.start_ms,
                        end_time: helper_line.end_ms,
                        words,
                        translated_lyric: NullString(translated_lyric),
                        roman_lyric: NullString(roman_lyric),
                        is_bg: false,
                        is_duet: current_line_is_duet,
                    }
                })
                .into_iter();

            let background_annotated_track = helper_line
                .tracks
                .iter()
                .find(|t| t.content_type == helper_types::ContentType::Background);

            let background_line_iter = background_annotated_track
                .and_then(|bg_track| {
                    let bg_syllables: Vec<_> = bg_track
                        .content
                        .words
                        .iter()
                        .flat_map(|w| &w.syllables)
                        .cloned()
                        .collect();
                    if bg_syllables.is_empty() {
                        return None;
                    }

                    let (bg_words, bg_translation, bg_romanization) = extract_line_components(
                        &bg_syllables,
                        &bg_track.translations,
                        &bg_track.romanizations,
                    );

                    let start_time = bg_syllables
                        .iter()
                        .map(|s| s.start_ms)
                        .min()
                        .unwrap_or(helper_line.start_ms);
                    let end_time = bg_syllables
                        .iter()
                        .map(|s| s.end_ms)
                        .max()
                        .unwrap_or(helper_line.end_ms);

                    Some(protocol::LyricLine {
                        start_time,
                        end_time,
                        words: bg_words,
                        translated_lyric: NullString(bg_translation),
                        roman_lyric: NullString(bg_romanization),
                        is_bg: true,
                        is_duet: current_line_is_duet,
                    })
                })
                .into_iter();

            main_line_iter.chain(background_line_iter)
        })
        .collect()
}
