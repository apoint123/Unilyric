use super::{ArtistMatchType, DurationMatchType, MatchScorable, NameMatchType};
use crate::utils::chinese::convert;
use lyrics_helper_core::{ChineseConversionConfig, MatchType, SearchResult, Track};
use std::collections::HashSet;

pub fn sort_and_rate_results(
    query: &Track<'_>,
    mut candidates: Vec<SearchResult>,
) -> Vec<SearchResult> {
    for candidate in &mut candidates {
        candidate.match_type = compare_track(query, candidate);
    }

    candidates.sort_unstable_by_key(|c| std::cmp::Reverse(c.match_type.get_score()));

    candidates
}

/// 计算两个字符串的归一化 Levenshtein 相似度，并转换为百分比。
fn compute_text_same(text1: &str, text2: &str) -> f64 {
    strsim::normalized_levenshtein(text1, text2) * 100.0
}

/// 归一化名称字符串
fn normalize_name_for_comparison(name: &str) -> String {
    name.replace('’', "'")
        .replace('，', ",")
        .replace(['（', '【', '['], " (")
        .replace(['）', '】', ']'], ") ")
        .replace("  ", " ")
        .replace("acoustic version", "acoustic")
        .trim()
        .to_string()
}

/// 比较用户查询和搜索结果，返回一个综合的匹配等级。
pub fn compare_track(track: &Track, result: &SearchResult) -> MatchType {
    const TITLE_WEIGHT: f64 = 1.0;
    const ARTIST_WEIGHT: f64 = 1.0;
    const ALBUM_WEIGHT: f64 = 0.4;
    const DURATION_WEIGHT: f64 = 1.0;
    const MAX_SINGLE_SCORE: f64 = 7.0;

    const SCORE_THRESHOLDS: &[(f64, MatchType)] = &[
        (21.0, MatchType::Perfect),
        (19.0, MatchType::VeryHigh),
        (17.0, MatchType::High),
        (15.0, MatchType::PrettyHigh),
        (11.0, MatchType::Medium),
        (6.5, MatchType::Low),
        (2.5, MatchType::VeryLow),
    ];

    let title_match = compare_name(track.title, Some(&result.title));
    let result_artist_names: Vec<String> = result.artists.iter().map(|a| a.name.clone()).collect();
    let artist_match = compare_artists(track.artists, Some(&result_artist_names));
    let album_match = compare_name(track.album, result.album.as_deref());
    let duration_match = compare_duration(track.duration, result.duration);

    let mut total_score = f64::from(duration_match.get_score()) * DURATION_WEIGHT;
    total_score = f64::from(album_match.get_score()).mul_add(ALBUM_WEIGHT, total_score);
    total_score = f64::from(artist_match.get_score()).mul_add(ARTIST_WEIGHT, total_score);
    total_score = f64::from(title_match.get_score()).mul_add(TITLE_WEIGHT, total_score);

    // 计算理论最高分
    let mut possible_score = MAX_SINGLE_SCORE * (TITLE_WEIGHT + ARTIST_WEIGHT);
    if album_match.is_some() {
        possible_score += MAX_SINGLE_SCORE * ALBUM_WEIGHT;
    }
    if duration_match.is_some() {
        possible_score += MAX_SINGLE_SCORE * DURATION_WEIGHT;
    }

    // 如果查询信息不完整，按比例放大总分
    let full_score_base =
        MAX_SINGLE_SCORE * (TITLE_WEIGHT + ARTIST_WEIGHT + ALBUM_WEIGHT + DURATION_WEIGHT);
    let normalized_score = if possible_score > 0.0 && possible_score < full_score_base {
        total_score * (full_score_base / possible_score)
    } else {
        total_score
    };

    for &(threshold, match_type) in SCORE_THRESHOLDS {
        if normalized_score > threshold {
            return match_type;
        }
    }

    MatchType::None
}

fn check_dash_paren_equivalence(s_dash: &str, s_paren: &str) -> bool {
    let is_dash = s_dash.contains(" - ") && !s_dash.contains('(');
    let is_paren = s_paren.contains('(') && !s_paren.contains(" - ");

    if is_dash
        && is_paren
        && let Some((base, suffix)) = s_dash.split_once(" - ")
    {
        return format!("{} ({})", base.trim(), suffix.trim()) == s_paren;
    }
    false
}

fn compare_name(name1_opt: Option<&str>, name2_opt: Option<&str>) -> Option<NameMatchType> {
    let name1_raw = name1_opt?;
    let name2_raw = name2_opt?;

    let name1_sc_lower = convert(name1_raw, ChineseConversionConfig::T2s).to_lowercase();
    let name2_sc_lower = convert(name2_raw, ChineseConversionConfig::T2s).to_lowercase();

    if name1_sc_lower.trim() == name2_sc_lower.trim() {
        return Some(NameMatchType::Perfect);
    }

    let name1 = normalize_name_for_comparison(&name1_sc_lower);
    let name2 = normalize_name_for_comparison(&name2_sc_lower);
    if name1.trim() == name2.trim() {
        return Some(NameMatchType::Perfect);
    }

    if check_dash_paren_equivalence(&name1, &name2) || check_dash_paren_equivalence(&name2, &name1)
    {
        return Some(NameMatchType::VeryHigh);
    }

    let special_suffixes = [
        "deluxe",
        "explicit",
        "special edition",
        "bonus track",
        "feat",
        "with",
    ];
    for suffix in special_suffixes {
        let suffixed_form = format!("({suffix}");
        if (name1.contains(&suffixed_form)
            && !name2.contains(&suffixed_form)
            && name2 == name1.split(&suffixed_form).next().unwrap_or("").trim())
            || (name2.contains(&suffixed_form)
                && !name1.contains(&suffixed_form)
                && name1 == name2.split(&suffixed_form).next().unwrap_or("").trim())
        {
            return Some(NameMatchType::VeryHigh);
        }
    }

    if name1.contains('(')
        && name2.contains('(')
        && let (Some(n1_base), Some(n2_base)) = (name1.split('(').next(), name2.split('(').next())
        && n1_base.trim() == n2_base.trim()
    {
        return Some(NameMatchType::High);
    }

    if (name1.contains('(')
        && !name2.contains('(')
        && name2 == name1.split('(').next().unwrap_or("").trim())
        || (name2.contains('(')
            && !name1.contains('(')
            && name1 == name2.split('(').next().unwrap_or("").trim())
    {
        return Some(NameMatchType::Low);
    }

    if name1.chars().count() == name2.chars().count() {
        let count = name1
            .chars()
            .zip(name2.chars())
            .filter(|(c1, c2)| c1 == c2)
            .count();
        let len = name1.chars().count();
        let count_f64 = f64::from(u32::try_from(count).unwrap_or(u32::MAX));
        let len_f64 = f64::from(u32::try_from(len).unwrap_or(u32::MAX));
        let ratio = count_f64 / len_f64;
        if (ratio >= 0.8 && len >= 4) || (ratio >= 0.5 && (2..=3).contains(&len)) {
            return Some(NameMatchType::High);
        }
    }

    if compute_text_same(&name1, &name2) > 90.0 {
        return Some(NameMatchType::VeryHigh);
    }
    if compute_text_same(&name1, &name2) > 80.0 {
        return Some(NameMatchType::High);
    }
    if compute_text_same(&name1, &name2) > 68.0 {
        return Some(NameMatchType::Medium);
    }
    if compute_text_same(&name1, &name2) > 55.0 {
        return Some(NameMatchType::Low);
    }

    Some(NameMatchType::NoMatch)
}

fn compare_artists<S1, S2>(
    artists1: Option<&[S1]>,
    artists2: Option<&[S2]>,
) -> Option<ArtistMatchType>
where
    S1: AsRef<str>,
    S2: AsRef<str>,
{
    const JACCARD_THRESHOLDS: &[(f64, ArtistMatchType)] = &[
        (0.99, ArtistMatchType::Perfect),
        (0.80, ArtistMatchType::VeryHigh),
        (0.60, ArtistMatchType::High),
        (0.40, ArtistMatchType::Medium),
        (0.15, ArtistMatchType::Low),
    ];
    const LEVENSHTEIN_THRESHOLD: f64 = 88.0;

    let list1_raw = artists1?;
    let list2_raw = artists2?;
    if list1_raw.is_empty() || list2_raw.is_empty() {
        return None;
    }

    let list1: Vec<String> = list1_raw
        .iter()
        .map(|s| convert(s.as_ref(), ChineseConversionConfig::T2s).to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let list2: Vec<String> = list2_raw
        .iter()
        .map(|s| convert(s.as_ref(), ChineseConversionConfig::T2s).to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let is_l1_various = list1
        .iter()
        .any(|s| s.contains("various") || s.contains("群星"));
    let is_l2_various = list2
        .iter()
        .any(|s| s.contains("various") || s.contains("群星"));
    if (is_l1_various && (is_l2_various || list2.len() > 4)) || (is_l2_various && list1.len() > 4) {
        return Some(ArtistMatchType::High);
    }

    let mut intersection_size = 0;
    let mut matched_indices_in_list2 = HashSet::new();

    for artist1 in &list1 {
        let mut best_match_idx = None;
        for (i, artist2) in list2.iter().enumerate() {
            if matched_indices_in_list2.contains(&i) {
                continue;
            }

            if artist2.contains(artist1)
                || artist1.contains(artist2)
                || compute_text_same(artist1, artist2) > LEVENSHTEIN_THRESHOLD
            {
                best_match_idx = Some(i);
                break;
            }
        }

        if let Some(idx) = best_match_idx {
            intersection_size += 1;
            matched_indices_in_list2.insert(idx);
        }
    }

    let union_size = list1.len() + list2.len() - intersection_size;
    if union_size == 0 {
        return Some(ArtistMatchType::Perfect);
    }

    let intersection_size_f64 = f64::from(u32::try_from(intersection_size).unwrap_or(u32::MAX));
    let union_size_f64 = f64::from(u32::try_from(union_size).unwrap_or(u32::MAX));
    let jaccard_score = intersection_size_f64 / union_size_f64;

    for &(threshold, match_type) in JACCARD_THRESHOLDS {
        if jaccard_score >= threshold {
            return Some(match_type);
        }
    }

    Some(ArtistMatchType::NoMatch)
}

fn compare_duration(duration1: Option<u64>, duration2: Option<u64>) -> Option<DurationMatchType> {
    const DURATION_THRESHOLDS: &[(f64, DurationMatchType)] = &[
        (6.95, DurationMatchType::Perfect), // 差异 < 50ms
        (6.0, DurationMatchType::VeryHigh), // 差异 < 400ms
        (4.2, DurationMatchType::High),     // 差异 < 700ms (在 sigma 点上)
        (2.5, DurationMatchType::Medium),   // 差异 < 1100ms
        (0.7, DurationMatchType::Low),      // 差异 < 1600ms
    ];
    // 控制衰减的快慢，即对时长差异的容忍度。
    const SIGMA: f64 = 700.0;

    let d1 = duration1.filter(|&d| d > 0)?;
    let d2 = duration2.filter(|&d| d > 0)?;

    let d1_u32 = u32::try_from(d1).ok()?;
    let d2_u32 = u32::try_from(d2).ok()?;

    let diff = (f64::from(d1_u32) - f64::from(d2_u32)).abs();

    // 高斯衰减
    let gaussian_score = (-diff.powi(2) / (2.0 * SIGMA.powi(2))).exp();
    let max_score = f64::from(DurationMatchType::Perfect.get_score());
    let final_score = gaussian_score * max_score;

    for &(threshold, match_type) in DURATION_THRESHOLDS {
        if final_score >= threshold {
            return Some(match_type);
        }
    }

    Some(DurationMatchType::NoMatch)
}

/// Aggregates search results from multiple providers into a single list and then sorts them.
pub fn aggregate_and_sort_results(
    query: &Track<'_>,
    provider_results: Vec<Vec<SearchResult>>,
) -> Vec<SearchResult> {
    let all_candidates: Vec<SearchResult> = provider_results.into_iter().flatten().collect();
    sort_and_rate_results(query, all_candidates)
}
