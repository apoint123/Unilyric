//! 歌词转换器核心模块

pub mod generators;
pub mod parsers;
pub mod processors;
pub mod utils;

use std::{collections::HashMap, hash::BuildHasher};

use crate::converter::processors::{
    batch_processor, chinese_conversion_processor::ChineseConversionProcessor,
};
use tracing::{debug, warn};

use lyrics_helper_core::{
    AuxiliaryLineMatchingStrategy, ContentType, ConversionInput, ConversionOptions,
    ConversionResult, ConversionTask, ConvertError, FullConversionResult, InputFile, LyricFormat,
    LyricLine, LyricTrack, MetadataStore, ParsedSourceData, TrackMetadataKey,
};
use ttml_processor::{generate_ttml, parse_ttml};

// ==========================================================
//  顶级转换入口
// ==========================================================

/// 处理一个转换任务，可以是单文件转换或批量转换。
///
/// # 参数
///
/// * `task` - 一个 `ConversionTask` 枚举，描述了要执行的具体操作（单个或批量）。
/// * `options` - 一个 `ConversionOptions` 引用，包含应用于所有转换的配置选项。
///
/// # 返回
///
/// * `Result<ConversionResult, ConvertError>` - 成功时返回包含转换结果的 `ConversionResult`，
///   失败时返回具体的 `ConvertError`。
pub fn process_conversion_task(
    task: ConversionTask,
    options: &ConversionOptions,
) -> Result<ConversionResult, ConvertError> {
    match task {
        ConversionTask::Single(input) => {
            let full_result = convert_single_lyric(&input, options)?;
            Ok(ConversionResult::Single(full_result.output_lyrics))
        }
        ConversionTask::Batch(batch_input) => {
            let file_groups = batch_processor::discover_and_pair_files(&batch_input.input_dir)?;
            let (mut tasks, file_lookup) =
                batch_processor::create_batch_tasks(file_groups, batch_input.target_format);

            if tasks.is_empty() {
                warn!("在输入目录中未找到可执行的转换任务。");
                return Ok(ConversionResult::Batch(Vec::new()));
            }

            batch_processor::execute_batch_conversion(
                &mut tasks,
                &file_lookup,
                &batch_input.output_dir,
                options,
            )?;

            Ok(ConversionResult::Batch(tasks))
        }
    }
}

// ==========================================================
//  核心转换与生成逻辑
// ==========================================================

/// 将一个 `ConversionInput` 转换为 `FullConversionResult`
///
/// # 参数
///
/// * `input` - 包含所有源文件信息和目标格式的 `ConversionInput`。
/// * `options` - 转换过程中的配置选项。
///
/// # 返回
///
/// * `Result<FullConversionResult, ConvertError>` - 成功时返回 `FullConversionResult`。
pub fn convert_single_lyric(
    input: &ConversionInput,
    options: &ConversionOptions,
) -> Result<FullConversionResult, ConvertError> {
    let mut source_data = parse_and_merge(input, options)?;

    if let Some(additional) = &input.additional_metadata {
        for (key, values) in additional {
            source_data
                .raw_metadata
                .entry(key.clone())
                .or_default()
                .extend(values.clone());
        }
    }

    generate_from_parsed(
        source_data,
        input.target_format,
        options,
        &input.user_metadata_overrides,
    )
}

/// 从已解析的源数据生成目标格式的歌词。
pub fn generate_from_parsed<S: BuildHasher>(
    mut source_data: ParsedSourceData,
    target_format: LyricFormat,
    options: &ConversionOptions,
    user_metadata_overrides: &Option<HashMap<String, Vec<String>, S>>,
) -> Result<FullConversionResult, ConvertError> {
    ChineseConversionProcessor::process(&mut source_data.lines, &options.chinese_conversion);

    processors::metadata_stripper::strip_descriptive_metadata_lines(
        &mut source_data.lines,
        &options.metadata_stripper,
    );

    let mut metadata_store = MetadataStore::from(&source_data);

    if let Some(overrides) = user_metadata_overrides {
        for (key, values) in overrides {
            metadata_store.set_multiple(key, values.clone());
        }
        source_data.raw_metadata = overrides
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
    }

    metadata_store.deduplicate_values();

    let output_lyrics = match target_format {
        LyricFormat::Lrc => generators::lrc_generator::generate_lrc(
            &source_data.lines,
            &metadata_store,
            &options.lrc,
        ),
        LyricFormat::EnhancedLrc => generators::enhanced_lrc_generator::generate_enhanced_lrc(
            &source_data.lines,
            &metadata_store,
            &options.lrc,
        ),
        LyricFormat::Ass => {
            let agent_store = MetadataStore::to_agent_store(&metadata_store);
            generators::ass_generator::generate_ass(
                &source_data.lines,
                &metadata_store,
                &agent_store,
                source_data.is_line_timed_source,
                &options.ass,
            )
        }
        LyricFormat::Ttml => {
            let agent_store = MetadataStore::to_agent_store(&metadata_store);
            generate_ttml(
                &source_data.lines,
                &metadata_store,
                &agent_store,
                &options.ttml,
            )
        }
        LyricFormat::AppleMusicJson => {
            generators::apple_music_json_generator::generate_apple_music_json(
                &source_data.lines,
                &metadata_store,
                options,
            )
        }
        LyricFormat::Qrc => {
            generators::qrc_generator::generate_qrc(&source_data.lines, &metadata_store)
        }
        LyricFormat::Lqe => generators::lqe_generator::generate_lqe(
            &source_data.lines,
            &metadata_store,
            &options.lqe,
        ),
        LyricFormat::Krc => {
            generators::krc_generator::generate_krc(&source_data.lines, &metadata_store)
        }
        LyricFormat::Yrc => {
            generators::yrc_generator::generate_yrc(&source_data.lines, &metadata_store)
        }
        LyricFormat::Lys => {
            generators::lys_generator::generate_lys(&source_data.lines, &metadata_store)
        }
        LyricFormat::Spl => {
            generators::spl_generator::generate_spl(&source_data.lines, &metadata_store)
        }
        LyricFormat::Lyl => {
            generators::lyricify_lines_generator::generate_lyl(&source_data.lines, &metadata_store)
        }
    }?;

    Ok(FullConversionResult {
        output_lyrics,
        source_data,
    })
}

/// 解析并合并一个包含主歌词、翻译和罗马音的完整输入。
///
/// # 参数
/// * `input` - 包含所有源文件信息的 `ConversionInput`。
/// * `options` - 转换过程中的配置选项，主要用于元数据处理。
///
/// # 返回
/// * `Result<ParsedSourceData, ConvertError>` - 成功时返回一个包含所有合并信息的 `ParsedSourceData`。
///
pub fn parse_and_merge(
    input: &ConversionInput,
    options: &ConversionOptions,
) -> Result<ParsedSourceData, ConvertError> {
    let mut main_parsed_source = parse_input_file(&input.main_lyric, options)?;
    let mut main_new_lines = main_parsed_source.lines; // 直接获取，因为类型已经是新的 LyricLine
    main_parsed_source.lines = vec![]; // 临时清空，最后会重新赋值

    let mut translation_sources = vec![];
    for file in &input.translations {
        let mut parsed_source = parse_input_file(file, options)?;
        let new_lines = parsed_source.lines;
        parsed_source.lines = vec![];
        translation_sources.push((new_lines, parsed_source, file.language.clone()));
    }

    let mut romanization_sources = vec![];
    for file in &input.romanizations {
        let mut parsed_source = parse_input_file(file, options)?;
        let new_lines = parsed_source.lines;
        parsed_source.lines = vec![];
        romanization_sources.push((new_lines, parsed_source, file.language.clone()));
    }

    // 合并元数据
    for (_, source, _) in translation_sources
        .iter()
        .chain(romanization_sources.iter())
    {
        main_parsed_source
            .raw_metadata
            .extend(source.raw_metadata.clone());
    }

    merge_tracks(
        &mut main_new_lines,
        &translation_sources,
        &romanization_sources,
        options.matching_strategy,
    );

    ChineseConversionProcessor::process(&mut main_new_lines, &options.chinese_conversion);

    processors::metadata_stripper::strip_descriptive_metadata_lines(
        &mut main_new_lines,
        &options.metadata_stripper,
    );

    main_parsed_source.lines = main_new_lines;

    Ok(main_parsed_source)
}

/// 合并主歌词行与翻译、罗马音数据，将翻译和罗马音轨道按时间戳插入到主歌词行中。
pub fn merge_tracks(
    main_lines: &mut [LyricLine],
    translations: &[(Vec<LyricLine>, ParsedSourceData, Option<String>)],
    romanizations: &[(Vec<LyricLine>, ParsedSourceData, Option<String>)],
    strategy: AuxiliaryLineMatchingStrategy,
) {
    // 辅助函数：从源数据中提取带时间戳的内容轨道
    fn extract_content_tracks(
        sources: &[(Vec<LyricLine>, ParsedSourceData, Option<String>)],
    ) -> Vec<(u64, u64, LyricTrack)> {
        let mut timed_tracks = Vec::new();

        for (lines, _, lang) in sources {
            for line in lines {
                // 辅助文件的一行理论上只包含一个带主要内容的 AnnotatedTrack
                if let Some(annotated_track) = line.tracks.first() {
                    let mut content_track = annotated_track.content.clone();
                    // 如果轨道本身没有语言标签，则使用文件级别的语言标签
                    if let Some(l) = lang {
                        content_track
                            .metadata
                            .entry(TrackMetadataKey::Language)
                            .or_insert_with(|| l.clone());
                    }
                    timed_tracks.push((line.start_ms, line.end_ms, content_track));
                }
            }
        }
        timed_tracks.sort_by_key(|(start_ms, _, _)| *start_ms);
        timed_tracks
    }

    if translations.is_empty() && romanizations.is_empty() {
        return;
    }

    let tolerance_ms = if let AuxiliaryLineMatchingStrategy::SortedSync { tolerance_ms } = strategy
    {
        tolerance_ms
    } else {
        warn!("仅支持 'SortedSync' 合并策略，已回退到默认容差 20ms。");
        20
    };

    let translation_tracks = extract_content_tracks(translations);
    let romanization_tracks = extract_content_tracks(romanizations);

    let mut trans_iter = translation_tracks.iter().peekable();
    let mut roman_iter = romanization_tracks.iter().peekable();

    for main_line in main_lines.iter_mut() {
        // 假设主歌词行中有一个我们将要合并到的主要内容轨道
        if let Some(main_annotated_track) = main_line
            .tracks
            .iter_mut()
            .find(|at| at.content_type == ContentType::Main)
        {
            // 跳过所有时间上已经不可能匹配的旧翻译行
            while let Some((start_ms, ..)) = trans_iter.peek() {
                if *start_ms + tolerance_ms < main_line.start_ms {
                    trans_iter.next();
                } else {
                    break; // 到达了可能匹配的窗口
                }
            }
            // 匹配并消耗所有在当前主行时间窗口内的翻译行
            while let Some((start_ms, end_ms, track)) = trans_iter.peek() {
                if start_ms.abs_diff(main_line.start_ms) <= tolerance_ms {
                    main_annotated_track.translations.push((*track).clone());
                    main_line.end_ms = main_line.end_ms.max(*end_ms);
                    trans_iter.next(); // 匹配成功, 消耗掉
                } else {
                    // 由于列表已排序，此翻译行已在当前主行窗口之外，
                    // 后续的翻译行更不可能匹配，故中断循环。
                    // 迭代器指针将留在这里，供下一个主行匹配。
                    break;
                }
            }

            while let Some((start_ms, ..)) = roman_iter.peek() {
                if *start_ms + tolerance_ms < main_line.start_ms {
                    roman_iter.next();
                } else {
                    break;
                }
            }
            while let Some((start_ms, end_ms, track)) = roman_iter.peek() {
                if start_ms.abs_diff(main_line.start_ms) <= tolerance_ms {
                    main_annotated_track.romanizations.push((*track).clone());
                    main_line.end_ms = main_line.end_ms.max(*end_ms);
                    roman_iter.next();
                } else {
                    break;
                }
            }
        }
    }
}

// ==========================================================
//  辅助函数
// ==========================================================

/// 根据指定的格式解析单个歌词文件内容。
///
/// 这是一个底层的分派函数。
fn parse_input_file(
    file: &InputFile,
    options: &ConversionOptions,
) -> Result<ParsedSourceData, ConvertError> {
    debug!("正在解析文件，格式为: {:?}", file.format);
    match file.format {
        LyricFormat::Lrc => parsers::lrc_parser::parse_lrc(&file.content, &options.lrc_parsing),
        LyricFormat::EnhancedLrc => parsers::enhanced_lrc_parser::parse_enhanced_lrc(&file.content),
        LyricFormat::Krc => parsers::krc_parser::parse_krc(&file.content),
        LyricFormat::Ass => parsers::ass_parser::parse_ass(&file.content),
        LyricFormat::Ttml => parse_ttml(&file.content, &options.ttml_parsing),
        LyricFormat::AppleMusicJson => {
            parsers::apple_music_json_parser::parse_apple_music_json(&file.content)
        }
        LyricFormat::Qrc => parsers::qrc_parser::parse_qrc(&file.content),
        LyricFormat::Yrc => parsers::yrc_parser::parse_yrc(&file.content),
        LyricFormat::Lys => parsers::lys_parser::parse_lys(&file.content),
        LyricFormat::Spl => parsers::spl_parser::parse_spl(&file.content),
        LyricFormat::Lqe => parsers::lqe_parser::parse_lqe(&file.content, options),
        LyricFormat::Lyl => parsers::lyricify_lines_parser::parse_lyl(&file.content),
    }
}
