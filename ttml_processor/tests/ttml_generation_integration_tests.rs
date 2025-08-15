use std::collections::HashMap;

use lyrics_helper_core::*;
use ttml_processor::generate_ttml;

#[test]
fn test_generate_line_timed_snapshot() {
    let lines = vec![
        LyricLineBuilder::default()
            .start_ms(1000)
            .end_ms(5000)
            .track(AnnotatedTrack {
                content_type: ContentType::Main,
                content: LyricTrack {
                    words: vec![Word {
                        syllables: vec![
                            LyricSyllableBuilder::default()
                                .text("这是一行歌词")
                                .build()
                                .unwrap(),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ..Default::default()
            })
            .build()
            .unwrap(),
        LyricLineBuilder::default()
            .start_ms(6000)
            .end_ms(10000)
            .track(AnnotatedTrack {
                content_type: ContentType::Main,
                content: LyricTrack {
                    words: vec![Word {
                        syllables: vec![
                            LyricSyllableBuilder::default()
                                .text("这是第二行歌词")
                                .build()
                                .unwrap(),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ..Default::default()
            })
            .build()
            .unwrap(),
    ];
    let mut metadata = MetadataStore::new();
    metadata.set_single(&CanonicalMetadataKey::Title.to_string(), "逐行歌曲");
    metadata.set_single(&CanonicalMetadataKey::Artist.to_string(), "测试艺术家");

    let options = TtmlGenerationOptionsBuilder::default()
        .timing_mode(TtmlTimingMode::Line)
        .format(true)
        .build()
        .unwrap();

    let agent_store = MetadataStore::to_agent_store(&metadata);
    let ttml_output = generate_ttml(&lines, &metadata, &agent_store, &options).unwrap();

    insta::assert_snapshot!(ttml_output);
}

#[test]
fn test_generate_word_timed_with_agents_snapshot() {
    let lines = vec![
        LyricLineBuilder::default()
            .agent(Some("v1".to_string()))
            .start_ms(5000)
            .end_ms(8200)
            .track(AnnotatedTrack {
                content_type: ContentType::Main,
                content: LyricTrack {
                    words: vec![Word {
                        syllables: vec![
                            LyricSyllableBuilder::default()
                                .text("I")
                                .start_ms(5000)
                                .end_ms(5500)
                                .ends_with_space(true)
                                .build()
                                .unwrap(),
                            LyricSyllableBuilder::default()
                                .text("sing")
                                .start_ms(5600)
                                .end_ms(6200)
                                .build()
                                .unwrap(),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ..Default::default()
            })
            .build()
            .unwrap(),
        LyricLineBuilder::default()
            .agent(Some("v1000".to_string()))
            .start_ms(9000)
            .end_ms(12000)
            .track(AnnotatedTrack {
                content_type: ContentType::Main,
                content: LyricTrack {
                    words: vec![Word {
                        syllables: vec![
                            LyricSyllableBuilder::default()
                                .text("We")
                                .start_ms(9000)
                                .end_ms(9500)
                                .ends_with_space(true)
                                .build()
                                .unwrap(),
                            LyricSyllableBuilder::default()
                                .text("sing")
                                .start_ms(9600)
                                .end_ms(10200)
                                .build()
                                .unwrap(),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ..Default::default()
            })
            .build()
            .unwrap(),
    ];

    let mut metadata = MetadataStore::new();
    metadata
        .add(&CanonicalMetadataKey::Songwriter.to_string(), "作曲家1号")
        .unwrap();

    metadata.add("agent", "v1=演唱者1号").unwrap();
    metadata.add("agent", "v1000=合唱").unwrap();

    let options = TtmlGenerationOptionsBuilder::default()
        .timing_mode(TtmlTimingMode::Word)
        .use_apple_format_rules(true)
        .format(true)
        .build()
        .unwrap();

    let agent_store = MetadataStore::to_agent_store(&metadata);
    let ttml_output = generate_ttml(&lines, &metadata, &agent_store, &options).unwrap();

    insta::assert_snapshot!(ttml_output);
}

#[test]
fn test_auto_word_splitting_snapshot() {
    let lines = vec![
        LyricLineBuilder::default()
            .start_ms(1000)
            .end_ms(5000)
            .track(AnnotatedTrack {
                content_type: ContentType::Main,
                content: LyricTrack {
                    words: vec![Word {
                        syllables: vec![
                            LyricSyllableBuilder::default()
                                .text("Split this,你好世界")
                                .start_ms(1000)
                                .end_ms(5000)
                                .build()
                                .unwrap(),
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                ..Default::default()
            })
            .build()
            .unwrap(),
    ];

    let options = TtmlGenerationOptionsBuilder::default()
        .timing_mode(TtmlTimingMode::Word)
        .auto_word_splitting(true)
        .format(true)
        .punctuation_weight(0.1)
        .build()
        .unwrap();

    let metadata = MetadataStore::default();
    let agent_store = MetadataStore::to_agent_store(&metadata);
    let ttml_output = generate_ttml(&lines, &metadata, &agent_store, &options).unwrap();

    insta::assert_snapshot!(ttml_output);
}

#[test]
fn test_generate_timed_romanization_snapshot() {
    let mut romanization_track_metadata = HashMap::new();
    romanization_track_metadata.insert(TrackMetadataKey::Language, "ja-Latn".to_string());

    let main_syllable = LyricSyllableBuilder::default()
        .text("朝も")
        .start_ms(1000)
        .end_ms(2000)
        .build()
        .unwrap();

    let roma_syllables = vec![
        LyricSyllableBuilder::default()
            .text("Asa")
            .start_ms(1000)
            .end_ms(1500)
            .build()
            .unwrap(),
        LyricSyllableBuilder::default()
            .text("mo")
            .start_ms(1600)
            .end_ms(2000)
            .build()
            .unwrap(),
    ];

    let lines = vec![
        LyricLineBuilder::default()
            .start_ms(1000)
            .end_ms(2000)
            .track(AnnotatedTrack {
                content_type: ContentType::Main,
                content: LyricTrack {
                    words: vec![Word {
                        syllables: vec![main_syllable],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                romanizations: vec![LyricTrack {
                    words: vec![Word {
                        syllables: roma_syllables,
                        ..Default::default()
                    }],
                    metadata: romanization_track_metadata,
                }],
                ..Default::default()
            })
            .build()
            .unwrap(),
    ];

    let options = TtmlGenerationOptionsBuilder::default()
        .timing_mode(TtmlTimingMode::Word)
        .use_apple_format_rules(true)
        .format(true)
        .build()
        .unwrap();

    let metadata = MetadataStore::default();
    let agent_store = MetadataStore::to_agent_store(&metadata);
    let ttml_output = generate_ttml(&lines, &metadata, &agent_store, &options).unwrap();

    insta::assert_snapshot!(ttml_output);
}

#[test]
fn test_generate_timed_translation_snapshot() {
    let mut translation_track_metadata = HashMap::new();
    translation_track_metadata.insert(TrackMetadataKey::Language, "zh-Hans".to_string());

    let main_syllable = LyricSyllableBuilder::default()
        .text("鐘聲響起歸家")
        .start_ms(1000)
        .end_ms(2000)
        .build()
        .unwrap();

    let trans_syllables = vec![
        LyricSyllableBuilder::default()
            .text("钟声响起")
            .start_ms(1000)
            .end_ms(1500)
            .build()
            .unwrap(),
        LyricSyllableBuilder::default()
            .text("归家")
            .start_ms(1500)
            .end_ms(2000)
            .build()
            .unwrap(),
    ];

    let lines = vec![
        LyricLineBuilder::default()
            .start_ms(1000)
            .end_ms(2000)
            .track(AnnotatedTrack {
                content_type: ContentType::Main,
                content: LyricTrack {
                    words: vec![Word {
                        syllables: vec![main_syllable],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                translations: vec![LyricTrack {
                    words: vec![Word {
                        syllables: trans_syllables,
                        ..Default::default()
                    }],
                    metadata: translation_track_metadata,
                }],
                ..Default::default()
            })
            .build()
            .unwrap(),
    ];

    let options = TtmlGenerationOptionsBuilder::default()
        .timing_mode(TtmlTimingMode::Word)
        .use_apple_format_rules(true)
        .format(true)
        .build()
        .unwrap();

    let metadata = MetadataStore::default();
    let agent_store = MetadataStore::to_agent_store(&metadata);
    let ttml_output = generate_ttml(&lines, &metadata, &agent_store, &options).unwrap();

    insta::assert_snapshot!(ttml_output);
}
