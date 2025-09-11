//! 批量转换处理器。

use std::{
    collections::HashMap,
    fs,
    hash::BuildHasher,
    path::{Path, PathBuf},
};

use crate::converter::convert_single_lyric;

use lyrics_helper_core::{
    BatchConversionConfig, BatchEntryStatus, BatchFileId, BatchLoadedFile, ConversionInput,
    ConversionOptions, ConvertError, InputFile, LyricFormat,
};

/// 表示一组相关联的歌词文件（主歌词、翻译、罗马音）。
#[derive(Debug, Default)]
pub struct FileGroup {
    /// 主歌词文件
    pub main_lyric: Option<PathBuf>,
    /// 翻译文件
    pub translations: Vec<PathBuf>,
    /// 罗马音文件
    pub romanizations: Vec<PathBuf>,
}

/// 扫描指定目录，根据文件名对歌词文件进行配对。
///
/// # 参数
/// * `input_dir` - 要扫描的输入目录路径。
///
/// # 返回
/// 成功时返回一个 `HashMap`，键是歌曲的基础名，值是配对好的 `FileGroup`。
/// 失败时返回 `ConvertError`。
pub fn discover_and_pair_files(
    input_dir: &Path,
) -> Result<HashMap<String, FileGroup>, ConvertError> {
    if !input_dir.is_dir() {
        return Err(ConvertError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "指定的输入路径不是一个目录或不存在",
        )));
    }

    let mut file_groups: HashMap<String, FileGroup> = HashMap::new();

    for entry in fs::read_dir(input_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file()
            && let Some(file_stem_str) = path.file_stem().and_then(|s| s.to_str())
        {
            let (base_name, tag) = match file_stem_str.rsplit_once('.') {
                Some((base, potential_tag)) if is_language_tag(potential_tag) => {
                    (base.to_string(), Some(potential_tag.to_lowercase()))
                }
                _ => (file_stem_str.to_string(), None),
            };

            let group = file_groups.entry(base_name).or_default();

            match tag.as_deref() {
                Some(t) if t.contains("latn") || t == "romaji" || t == "roman" => {
                    group.romanizations.push(path);
                }
                Some(_) => {
                    group.translations.push(path);
                }
                None => {
                    if group.main_lyric.is_none() {
                        group.main_lyric = Some(path);
                    } else {
                        tracing::warn!(
                            "为基础名 '{}' 发现了多个可能的主歌词文件，将只使用第一个: {:?}",
                            file_stem_str,
                            &group.main_lyric
                        );
                    }
                }
            }
        }
    }

    Ok(file_groups)
}

/// 简单的辅助函数，用于判断一个字符串是否可能是一个语言标签。
fn is_language_tag(tag: &str) -> bool {
    if ["romaji", "roman", "roma"].contains(&tag.to_lowercase().as_str()) {
        return true;
    }
    tag.len() <= 5 && tag.chars().all(|c| c.is_alphanumeric() || c == '-')
}

/// 根据发现的文件组创建批量转换任务配置列表和一个文件查找表。
///
/// # 返回
/// 一个元组 `(Vec<BatchConversionConfig>, HashMap<BatchFileId, BatchLoadedFile>)`
pub fn create_batch_tasks<S: BuildHasher>(
    file_groups: HashMap<String, FileGroup, S>,
    target_format: LyricFormat,
) -> (
    Vec<BatchConversionConfig>,
    HashMap<BatchFileId, BatchLoadedFile>,
) {
    let mut tasks = Vec::new();
    let mut file_lookup: HashMap<BatchFileId, BatchLoadedFile> = HashMap::new();

    for (base_name, group) in file_groups {
        if let Some(main_path) = group.main_lyric {
            let main_lyric_file = BatchLoadedFile::new(main_path);
            let main_lyric_id = main_lyric_file.id;

            let translation_files: Vec<BatchLoadedFile> = group
                .translations
                .into_iter()
                .map(BatchLoadedFile::new)
                .collect();
            let romanization_files: Vec<BatchLoadedFile> = group
                .romanizations
                .into_iter()
                .map(BatchLoadedFile::new)
                .collect();

            // 将所有文件添加到查找表中
            file_lookup.insert(main_lyric_id, main_lyric_file);
            for file in translation_files.iter().cloned() {
                file_lookup.insert(file.id, file);
            }
            for file in romanization_files.iter().cloned() {
                file_lookup.insert(file.id, file);
            }

            let output_filename = format!("{}.{}", base_name, target_format.to_extension_str());
            let mut config =
                BatchConversionConfig::new(main_lyric_id, target_format, output_filename);

            config.translation_lyric_ids = translation_files.iter().map(|f| f.id).collect();
            config.romanization_lyric_ids = romanization_files.iter().map(|f| f.id).collect();

            tasks.push(config);
        }
    }
    (tasks, file_lookup)
}

/// 执行批量转换任务。
///
/// 遍历任务列表，读取文件，调用核心转换逻辑，并将结果写入输出目录。
/// 它会直接修改传入的 `tasks` 切片来更新每个任务的状态。
///
/// # 参数
/// * `tasks` - 一个可变的任务配置切片。
/// * `file_lookup` - 一个包含所有已加载文件信息的查找表。
/// * `output_dir` - 保存转换后文件的目录。
/// * `options` - 应用于所有转换的 `ConversionOptions`。
pub fn execute_batch_conversion<S: BuildHasher>(
    tasks: &mut [BatchConversionConfig],
    file_lookup: &HashMap<BatchFileId, BatchLoadedFile, S>,
    output_dir: &Path,
    options: &ConversionOptions,
) -> Result<(), ConvertError> {
    // 确保输出目录存在
    fs::create_dir_all(output_dir)?;

    for task in tasks.iter_mut() {
        task.status = BatchEntryStatus::Converting;

        // 辅助闭包，用于读取文件并构建 InputFile
        let read_and_build_input = |file_id: &BatchFileId| -> Result<InputFile, ConvertError> {
            let loaded_file = file_lookup.get(file_id).ok_or_else(|| {
                ConvertError::Internal(format!("文件ID {file_id:?} 未在查找表中找到"))
            })?;

            let content = fs::read_to_string(&loaded_file.path)?;
            let format = get_format_from_path(&loaded_file.path).ok_or_else(|| {
                ConvertError::InvalidLyricFormat(loaded_file.path.to_string_lossy().to_string())
            })?;

            Ok(InputFile {
                content,
                format,
                // TODO: 从路径获取语言和文件名
                language: None,
                filename: Some(loaded_file.filename.clone()),
            })
        };

        let conversion_result = (|| -> Result<String, ConvertError> {
            // 读取主歌词文件
            let main_lyric = read_and_build_input(&task.main_lyric_id)?;

            // 读取所有翻译文件
            let translations = task
                .translation_lyric_ids
                .iter()
                .map(read_and_build_input)
                .collect::<Result<Vec<_>, _>>()?;

            // 读取所有罗马音文件
            let romanizations = task
                .romanization_lyric_ids
                .iter()
                .map(read_and_build_input)
                .collect::<Result<Vec<_>, _>>()?;

            // 构建核心转换函数的输入
            let conversion_input = ConversionInput {
                main_lyric,
                translations,
                romanizations,
                target_format: task.target_format,
                user_metadata_overrides: None,
                additional_metadata: None,
            };

            // 调用核心转换函数
            convert_single_lyric(&conversion_input, options)
                .map(|full_result| full_result.output_lyrics)
        })();

        match conversion_result {
            Ok(result_string) => {
                let output_path = output_dir.join(&task.output_filename_preview);
                match fs::write(&output_path, result_string) {
                    Ok(()) => {
                        task.status = BatchEntryStatus::Completed {
                            output_path,
                            warnings: Vec::new(),
                        };
                    }
                    Err(e) => {
                        task.status = BatchEntryStatus::Failed(format!("写入文件失败: {e}"));
                    }
                }
            }
            Err(e) => {
                task.status = BatchEntryStatus::Failed(e.to_string());
            }
        }
    }

    Ok(())
}

/// 从文件路径的扩展名推断歌词格式。
fn get_format_from_path(path: &Path) -> Option<LyricFormat> {
    path.extension()
        .and_then(|s| s.to_str())
        .and_then(LyricFormat::from_string)
}
