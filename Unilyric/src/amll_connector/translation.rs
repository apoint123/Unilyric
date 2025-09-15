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
    is_instrumental: bool,
) -> (Vec<protocol::LyricWord>, String, String) {
    let roman_syllables: Vec<_> = romanizations
        .first()
        .map(|track| {
            track
                .words
                .iter()
                .flat_map(|w| &w.syllables)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut roman_groups: Vec<Vec<String>> = vec![Vec::new(); syllables.len()];

    if !roman_syllables.is_empty() && !syllables.is_empty() {
        for roman_syl in &roman_syllables {
            let mut best_match_index = None;
            let mut max_overlap: i64 = 0;

            for (i, main_syl) in syllables.iter().enumerate() {
                let overlap = std::cmp::min(main_syl.end_ms, roman_syl.end_ms) as i64
                    - std::cmp::max(main_syl.start_ms, roman_syl.start_ms) as i64;

                if overlap > max_overlap {
                    max_overlap = overlap;
                    best_match_index = Some(i);
                }
            }

            if let Some(index) = best_match_index {
                roman_groups[index].push(roman_syl.text.clone());
            }
        }
    }

    let words = syllables
        .iter()
        .enumerate()
        .map(|(i, syllable)| {
            let word_text = if syllable.ends_with_space {
                format!("{} ", syllable.text)
            } else {
                syllable.text.clone()
            };

            let end_time = if is_instrumental {
                // 应对纯音乐提示文本
                syllable.start_ms + 3_600_000 // 1 h
            } else {
                syllable.end_ms
            };

            let roman_word_text = roman_groups[i].join("");

            protocol::LyricWord {
                start_time: syllable.start_ms,
                end_time,
                word: NullString(word_text),
                roman_word: NullString(roman_word_text),
            }
        })
        .collect();

    let mut translation = translations
        .iter()
        .find(|t| {
            t.metadata
                .get(&helper_types::TrackMetadataKey::Language)
                .is_some_and(|lang| lang.eq_ignore_ascii_case(PREFERRED_TRANSLATION_LANG))
        })
        .or_else(|| translations.first())
        .map_or(String::new(), get_track_text);

    if translation == "//" {
        translation = String::new();
    }

    let romanization = String::new();

    (words, translation, romanization)
}

pub(super) fn convert_to_protocol_lyrics(
    source_data: &helper_types::ParsedSourceData,
) -> Vec<protocol::LyricLine> {
    let is_instrumental = if source_data.lines.len() == 1 {
        source_data
            .lines
            .first()
            .and_then(|line| {
                line.tracks
                    .iter()
                    .find(|t| t.content_type == helper_types::ContentType::Main)
            })
            .is_some_and(|main_track| {
                main_track
                    .content
                    .words
                    .iter()
                    .flat_map(|w| &w.syllables)
                    .count()
                    == 1
            })
    } else {
        false
    };

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
                .and_then(|main_track| {
                    let main_syllables: Vec<_> = main_track
                        .content
                        .words
                        .iter()
                        .flat_map(|w| &w.syllables)
                        .cloned()
                        .collect();

                    if main_syllables.is_empty() {
                        return None;
                    }

                    let (words, translated_lyric, roman_lyric) = extract_line_components(
                        &main_syllables,
                        &main_track.translations,
                        &main_track.romanizations,
                        is_instrumental,
                    );

                    let start_time = words
                        .iter()
                        .map(|s| s.start_time)
                        .min()
                        .unwrap_or(helper_line.start_ms);
                    let end_time = words
                        .iter()
                        .map(|s| s.end_time)
                        .max()
                        .unwrap_or(helper_line.end_ms);

                    Some(protocol::LyricLine {
                        start_time,
                        end_time,
                        words,
                        translated_lyric: NullString(translated_lyric),
                        roman_lyric: NullString(roman_lyric),
                        is_bg: false,
                        is_duet: current_line_is_duet,
                    })
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
                        false,
                    );

                    let start_time = bg_words
                        .iter()
                        .map(|s| s.start_time)
                        .min()
                        .unwrap_or(helper_line.start_ms);
                    let end_time = bg_words
                        .iter()
                        .map(|s| s.end_time)
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

#[cfg(test)]
mod tests {
    use super::*;
    use lyrics_helper_rs::converter::parsers::qrc_parser;

    #[test]
    fn test_romanization_alignment() {
        let main_qrc_content = include_str!("../../test_data/main.qrc");
        let roma_qrc_content = include_str!("../../test_data/roma.qrc");

        let mut main_data = qrc_parser::parse_qrc(main_qrc_content).unwrap();
        let roma_data = qrc_parser::parse_qrc(roma_qrc_content).unwrap();

        const TIMESTAMP_TOLERANCE_MS: i64 = 30;
        let mut matched_roma_indices = vec![false; roma_data.lines.len()];

        for main_line in main_data.lines.iter_mut() {
            let best_match = roma_data
                .lines
                .iter()
                .enumerate()
                .filter(|(i, _)| !matched_roma_indices[*i])
                .min_by_key(|(_, roma_line)| {
                    (roma_line.start_ms as i64 - main_line.start_ms as i64).abs()
                });

            if let Some((roma_index, roma_line)) = best_match
                && (roma_line.start_ms as i64 - main_line.start_ms as i64).abs()
                    <= TIMESTAMP_TOLERANCE_MS
                && let Some(main_annotated_track) = main_line
                    .tracks
                    .iter_mut()
                    .find(|t| t.content_type == helper_types::ContentType::Main)
                && let Some(roma_annotated_track) = roma_line
                    .tracks
                    .iter()
                    .find(|t| t.content_type == helper_types::ContentType::Main)
            {
                let has_syllables = roma_annotated_track
                    .content
                    .words
                    .iter()
                    .any(|w| !w.syllables.is_empty());
                if has_syllables {
                    main_annotated_track
                        .romanizations
                        .push(roma_annotated_track.content.clone());
                    matched_roma_indices[roma_index] = true;
                }
            }
        }
        for (line_idx, line) in main_data.lines.iter().enumerate() {
            println!(
                "\n--- Line {} ({}ms - {}ms) ---",
                line_idx, line.start_ms, line.end_ms
            );

            if let Some(main_track) = line.main_track() {
                let main_syllables: Vec<_> = main_track
                    .content
                    .words
                    .iter()
                    .flat_map(|w| &w.syllables)
                    .cloned()
                    .collect();

                if main_syllables.is_empty() {
                    println!("  (No lyrical content in main track)");
                    continue;
                }

                let (protocol_words, _, _) = extract_line_components(
                    &main_syllables,
                    &main_track.translations,
                    &main_track.romanizations,
                    false,
                );

                assert_eq!(main_syllables.len(), protocol_words.len());

                // for (main_syl, protocol_word) in main_syllables.iter().zip(protocol_words.iter()) {
                //     println!(
                //         "  Main: '{}' ({} - {}) -> Romanization: '{}'",
                //         main_syl.text,
                //         main_syl.start_ms,
                //         main_syl.end_ms,
                //         protocol_word.roman_word.0
                //     );
                // }
            } else {
                println!("  (No main track in this line)");
            }
        }
    }
}
