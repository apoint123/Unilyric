//! # TTML 生成器 - Head 处理模块
//!
//! 该模块负责生成 TTML 文件的 `<head>` 部分，包括元数据、演唱者信息和
//! Apple 格式特有的逐字和逐行辅助轨道。

use std::collections::HashMap;

use crate::utils::normalize_text_whitespace;
use lyrics_helper_core::{
    Agent, AgentStore, AgentType, CanonicalMetadataKey, ConvertError, LyricLine, LyricTrack,
    MetadataStore, TrackMetadataKey, TtmlGenerationOptions,
};
use quick_xml::{Writer, events::BytesText};

use super::utils::format_ttml_time;

#[allow(clippy::too_many_lines)]
pub(super) fn write_ttml_head<W: std::io::Write>(
    writer: &mut Writer<W>,
    metadata_store: &MetadataStore,
    lines: &[LyricLine],
    agent_store: &AgentStore,
    options: &TtmlGenerationOptions,
) -> Result<(), ConvertError> {
    writer
        .create_element("head")
        .write_inner_content(|writer| {
            writer
                .create_element("metadata")
                .write_inner_content(|writer| {
                    let mut sorted_agents: Vec<_> = agent_store.all_agents().cloned().collect();

                    if sorted_agents.is_empty() && !lines.is_empty() {
                        // 如果没有 agent 但有歌词行，创建一个默认的
                        sorted_agents.push(Agent {
                            id: "v1".to_string(),
                            name: None,
                            agent_type: AgentType::Person,
                        });
                    }

                    sorted_agents.sort_by(|a, b| a.id.cmp(&b.id));

                    for agent in sorted_agents {
                        let type_str = match agent.agent_type {
                            AgentType::Person => "person",
                            AgentType::Group => "group",
                            AgentType::Other => "other",
                        };

                        let agent_element = writer
                            .create_element("ttm:agent")
                            .with_attribute(("type", type_str))
                            .with_attribute(("xml:id", agent.id.as_str()));

                        if let Some(name) = &agent.name {
                            agent_element.write_inner_content(|writer| {
                                writer
                                    .create_element("ttm:name")
                                    .with_attribute(("type", "full"))
                                    .write_text_content(BytesText::new(name))?;
                                Ok(())
                            })?;
                        } else {
                            agent_element.write_empty()?;
                        }
                    }

                    if options.use_apple_format_rules {
                        let mut translations_by_lang: HashMap<
                            Option<String>,
                            Vec<(String, String)>,
                        > = HashMap::new();

                        for (i, line) in lines.iter().enumerate() {
                            let p_key = format!("L{}", i + 1);
                            for at in &line.tracks {
                                for track in &at.translations {
                                    let all_syllables: Vec<_> =
                                        track.words.iter().flat_map(|w| &w.syllables).collect();
                                    let is_timed =
                                        all_syllables.iter().any(|s| s.end_ms > s.start_ms);

                                    if !is_timed || all_syllables.len() <= 1 {
                                        let lang = track
                                            .metadata
                                            .get(&TrackMetadataKey::Language)
                                            .cloned();
                                        let full_text = all_syllables
                                            .iter()
                                            .map(|s| s.text.clone())
                                            .collect::<Vec<_>>()
                                            .join(" ");

                                        if !full_text.trim().is_empty() {
                                            translations_by_lang.entry(lang).or_default().push((
                                                p_key.clone(),
                                                normalize_text_whitespace(&full_text),
                                            ));
                                        }
                                    }
                                }
                            }
                        }

                        // 检查是否有任何内容来证明创建 <iTunesMetadata> 块是合理的
                        let has_timed_translations = lines.iter().any(|l| {
                            l.tracks.iter().any(|at| {
                                at.translations
                                    .iter()
                                    .any(|t| t.words.iter().flat_map(|w| &w.syllables).count() > 1)
                            })
                        });
                        let has_timed_romanizations = lines
                            .iter()
                            .any(|l| l.tracks.iter().any(|at| !at.romanizations.is_empty()));

                        if !translations_by_lang.is_empty()
                            || has_timed_translations
                            || has_timed_romanizations
                        {
                            writer
                                .create_element("iTunesMetadata")
                                .with_attribute((
                                    "xmlns",
                                    "http://music.apple.com/lyric-ttml-internal",
                                ))
                                .write_inner_content(|writer| {
                                    if !translations_by_lang.is_empty() {
                                        writer.create_element("translations").write_inner_content(
                                            |writer| {
                                                for (lang, entries) in translations_by_lang {
                                                    let mut trans_builder = writer
                                                        .create_element("translation")
                                                        .with_attribute(("type", "subtitle"));
                                                    if let Some(lang_code) =
                                                        lang.as_ref().filter(|s| !s.is_empty())
                                                    {
                                                        trans_builder = trans_builder
                                                            .with_attribute((
                                                                "xml:lang",
                                                                lang_code.as_str(),
                                                            ));
                                                    }
                                                    trans_builder.write_inner_content(
                                                        |writer| {
                                                            for (key, text) in entries {
                                                                writer
                                                                    .create_element("text")
                                                                    .with_attribute((
                                                                        "for",
                                                                        key.as_str(),
                                                                    ))
                                                                    .write_text_content(
                                                                        BytesText::new(&text),
                                                                    )?;
                                                            }
                                                            Ok(())
                                                        },
                                                    )?;
                                                }
                                                Ok(())
                                            },
                                        )?;
                                    }

                                    let to_io_err = |e: ConvertError| std::io::Error::other(e);

                                    write_timed_tracks_to_head(
                                        writer,
                                        lines,
                                        1,
                                        "translation",
                                        "translations",
                                        "translation",
                                    )
                                    .map_err(to_io_err)?;

                                    write_timed_tracks_to_head(
                                        writer,
                                        lines,
                                        1,
                                        "romanization",
                                        "transliterations",
                                        "transliteration",
                                    )
                                    .map_err(to_io_err)?;

                                    Ok(())
                                })?;
                        }
                    }

                    let valid_songwriters: Vec<&String> = metadata_store
                        .get_multiple_values(&CanonicalMetadataKey::Songwriter)
                        .map(|vec| vec.iter().filter(|s| !s.trim().is_empty()).collect())
                        .unwrap_or_default();

                    if !valid_songwriters.is_empty() {
                        writer
                            .create_element("iTunesMetadata")
                            .with_attribute(("xmlns", "http://music.apple.com/lyric-ttml-internal"))
                            .write_inner_content(|writer| {
                                writer.create_element("songwriters").write_inner_content(
                                    |writer| {
                                        for sw_name in &valid_songwriters {
                                            writer
                                                .create_element("songwriter")
                                                .write_text_content(BytesText::new(
                                                    sw_name.trim(),
                                                ))?;
                                        }
                                        Ok(())
                                    },
                                )?;
                                Ok(())
                            })?;
                    }

                    let keys_map = [
                        (CanonicalMetadataKey::Title, "musicName"),
                        (CanonicalMetadataKey::Artist, "artists"),
                        (CanonicalMetadataKey::Album, "album"),
                        (CanonicalMetadataKey::Isrc, "isrc"),
                        (CanonicalMetadataKey::AppleMusicId, "appleMusicId"),
                        (CanonicalMetadataKey::NcmMusicId, "ncmMusicId"),
                        (CanonicalMetadataKey::SpotifyId, "spotifyId"),
                        (CanonicalMetadataKey::QqMusicId, "qqMusicId"),
                        (CanonicalMetadataKey::TtmlAuthorGithub, "ttmlAuthorGithub"),
                        (
                            CanonicalMetadataKey::TtmlAuthorGithubLogin,
                            "ttmlAuthorGithubLogin",
                        ),
                    ];

                    let mut written_keys = std::collections::HashSet::new();

                    for (key, amll_key_name) in &keys_map {
                        if let Some(values) = metadata_store.get_multiple_values(key) {
                            for value_str in values {
                                if !value_str.trim().is_empty() {
                                    writer
                                        .create_element("amll:meta")
                                        .with_attribute(("key", *amll_key_name))
                                        .with_attribute(("value", value_str.trim()))
                                        .write_empty()?;
                                }
                            }
                            written_keys.insert(key);
                        }
                    }

                    let mut custom_metadata = Vec::new();
                    for (key, values) in metadata_store.get_all_data() {
                        if !written_keys.contains(key)
                            && let CanonicalMetadataKey::Custom(s) = key
                        {
                            if s.eq_ignore_ascii_case("agent")
                                || s.eq_ignore_ascii_case("xml:lang_root")
                            {
                                continue;
                            }
                            custom_metadata.push((s.as_str(), values));
                        }
                    }

                    custom_metadata.sort_unstable_by_key(|(k, _)| *k);

                    for (key_name, values) in custom_metadata {
                        for value_str in values {
                            if !value_str.trim().is_empty() {
                                writer
                                    .create_element("amll:meta")
                                    .with_attribute(("key", key_name))
                                    .with_attribute(("value", value_str.trim()))
                                    .write_empty()?;
                            }
                        }
                    }
                    Ok(())
                })?;
            Ok(())
        })?;
    Ok(())
}

fn write_timed_tracks_to_head<W: std::io::Write>(
    writer: &mut Writer<W>,
    lines: &[LyricLine],
    p_key_counter_base: i32,
    track_kind: &str, // "translation" 或 "romanization"
    container_tag_name: &str,
    item_tag_name: &str,
) -> Result<(), ConvertError> {
    // 按语言对轨道进行分组
    let mut grouped_by_lang: HashMap<Option<String>, Vec<(i32, &LyricTrack)>> = HashMap::new();

    for (line_idx, line) in lines.iter().enumerate() {
        for annotated_track in &line.tracks {
            let tracks_to_check = match track_kind {
                "translation" => &annotated_track.translations,
                "romanization" => &annotated_track.romanizations,
                _ => continue,
            };

            for track in tracks_to_check {
                if track
                    .words
                    .iter()
                    .any(|w| w.syllables.iter().any(|s| s.end_ms > s.start_ms))
                {
                    let lang = track.metadata.get(&TrackMetadataKey::Language).cloned();
                    let line_key = line_idx.try_into().unwrap_or(i32::MAX - p_key_counter_base)
                        + p_key_counter_base;
                    grouped_by_lang
                        .entry(lang)
                        .or_default()
                        .push((line_key, track));
                }
            }
        }
    }

    if grouped_by_lang.is_empty() {
        return Ok(());
    }

    writer
        .create_element(container_tag_name)
        .write_inner_content(|writer| {
            let mut sorted_groups: Vec<_> = grouped_by_lang.into_iter().collect();
            sorted_groups.sort_by_key(|(lang, _)| lang.clone());

            for (lang, entries) in sorted_groups {
                let mut item_builder = writer.create_element(item_tag_name);
                if let Some(lang_code) = lang.as_ref().filter(|s| !s.is_empty()) {
                    item_builder = item_builder.with_attribute(("xml:lang", lang_code.as_str()));
                }

                item_builder.write_inner_content(|writer| {
                    for (line_idx, track) in entries {
                        writer
                            .create_element("text")
                            .with_attribute(("for", format!("L{line_idx}").as_str()))
                            .write_inner_content(|writer| {
                                for word in &track.words {
                                    for syl in &word.syllables {
                                        writer
                                            .create_element("span")
                                            .with_attribute((
                                                "begin",
                                                format_ttml_time(syl.start_ms).as_str(),
                                            ))
                                            .with_attribute((
                                                "end",
                                                format_ttml_time(syl.end_ms).as_str(),
                                            ))
                                            .write_text_content(BytesText::new(&syl.text))?;
                                    }
                                }
                                Ok(())
                            })?;
                    }
                    Ok(())
                })?;
            }
            Ok(())
        })?;

    Ok(())
}
