//! 匹配算法模块。参考隔壁的 Lyricify Lyrics Helper

use crate::converter::processors::chinese_conversion_processor::convert;
use crate::model::match_type::MatchScorable;
use crate::model::match_type::{ArtistMatchType, DurationMatchType, NameMatchType};
use ferrous_opencc::config::BuiltinConfig;
use lyrics_helper_core::{MatchType, SearchResult, Track};
use std::collections::HashSet;

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
pub(crate) fn compare_track(track: &Track, result: &SearchResult) -> MatchType {
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

    let total_score = f64::from(title_match.get_score()) * TITLE_WEIGHT
        + f64::from(artist_match.get_score()) * ARTIST_WEIGHT
        + f64::from(album_match.get_score()) * ALBUM_WEIGHT
        + f64::from(duration_match.get_score()) * DURATION_WEIGHT;

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

    let name1_sc_lower = convert(name1_raw, BuiltinConfig::T2s).to_lowercase();
    let name2_sc_lower = convert(name2_raw, BuiltinConfig::T2s).to_lowercase();

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
        return Some(NameMatchType::Medium);
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

    let list1_raw = artists1?;
    let list2_raw = artists2?;
    if list1_raw.is_empty() || list2_raw.is_empty() {
        return None;
    }

    let list1: Vec<String> = list1_raw
        .iter()
        .map(|s| convert(s.as_ref(), BuiltinConfig::T2s).to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let list2: Vec<String> = list2_raw
        .iter()
        .map(|s| convert(s.as_ref(), BuiltinConfig::T2s).to_lowercase())
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

    let set1: HashSet<_> = list1.iter().collect();
    let set2: HashSet<_> = list2.iter().collect();
    let intersection_size = set1.intersection(&set2).count();
    let union_size = set1.union(&set2).count();
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

#[cfg(test)]
mod tests {
    use lyrics_helper_core::Artist;

    use super::*;

    fn assert_artist_match(
        artists1: &[&str],
        artists2: &[&str],
        expected_match: ArtistMatchType,
        case_description: &str,
    ) {
        let result = compare_artists(Some(artists1), Some(artists2)).unwrap();
        let set1: HashSet<_> = artists1
            .iter()
            .map(|s| convert(s, BuiltinConfig::T2s).to_lowercase())
            .collect();
        let set2: HashSet<_> = artists2
            .iter()
            .map(|s| convert(s, BuiltinConfig::T2s).to_lowercase())
            .collect();
        let intersection_size = set1.intersection(&set2).count();
        let union_size = set1.union(&set2).count();
        let intersection_size_f64 = f64::from(u32::try_from(intersection_size).unwrap_or(u32::MAX));
        let union_size_f64 = f64::from(u32::try_from(union_size).unwrap_or(u32::MAX));
        let jaccard_score = if union_size == 0 {
            1.0
        } else {
            intersection_size_f64 / union_size_f64
        };

        assert_eq!(
            result, expected_match,
            "\n[Artist Test Failed]: {case_description}\n  - Jaccard: {jaccard_score:.4}\n  - Expected: {expected_match:?}, Actual: {result:?}"
        );
    }

    #[test]
    fn test_compare_artists_with_jaccard() {
        assert_artist_match(
            &["A", "B"],
            &["B", "A"],
            ArtistMatchType::Perfect,
            "Perfect match (order)",
        );
        assert_artist_match(
            &["周杰伦"],
            &["周杰倫"],
            ArtistMatchType::Perfect,
            "Perfect match (zh-Hans/t)",
        );
        assert_artist_match(
            &["A", "B", "C", "D"],
            &["A", "B", "C", "D", "E"],
            ArtistMatchType::VeryHigh,
            "VeryHigh match (4/5)",
        );
        assert_artist_match(
            &["A", "B", "C"],
            &["A", "B"],
            ArtistMatchType::High,
            "High match (subset, 2/3)",
        );
        assert_artist_match(
            &["A"],
            &["A", "B"],
            ArtistMatchType::Medium,
            "Medium match (feat. scene, 1/2)",
        );
        assert_artist_match(
            &["A", "B", "C"],
            &["A", "D", "E"],
            ArtistMatchType::Low,
            "Low match (1/5)",
        );
        assert_artist_match(
            &["A", "B", "C"],
            &["A", "B", "C", "D", "E"],
            ArtistMatchType::High,
            "High match (subset, 3/5)",
        );
        assert_artist_match(
            &["A", "B"],
            &["C", "D"],
            ArtistMatchType::NoMatch,
            "No match",
        );
        assert_artist_match(
            &["Various Artists"],
            &["群星"],
            ArtistMatchType::High,
            "Special entity (Various Artists)",
        );
        let empty_list_result = compare_artists(Some(&["A"]), Some(&[] as &[&str]));
        assert!(empty_list_result.is_none(), "Should be None for empty list");
    }

    #[test]
    fn test_compare_name() {
        assert_eq!(
            compare_name(Some("Test"), Some("test")).unwrap(),
            NameMatchType::Perfect,
            "Case insensitive"
        );
        assert_eq!(
            compare_name(Some("测试"), Some("測試")).unwrap(),
            NameMatchType::Perfect,
            "Chinese simplified/traditional"
        );
        assert_eq!(
            compare_name(Some("Song (acoustic version)"), Some("Song (acoustic)")).unwrap(),
            NameMatchType::Perfect,
            "Semantic normalization"
        );
        assert_eq!(
            compare_name(Some("song（全角括号）"), Some("song (半角括号)")).unwrap(),
            NameMatchType::High,
            "Parenthesis normalization"
        );
        assert_eq!(
            compare_name(Some("Song - Live"), Some("Song (Live)")).unwrap(),
            NameMatchType::VeryHigh,
            "Structural variant: A - B vs A (B)"
        );
        assert_eq!(
            compare_name(Some("The Song (deluxe edition)"), Some("The Song")).unwrap(),
            NameMatchType::VeryHigh,
            "Suffix variant: (deluxe)"
        );
        assert_eq!(
            compare_name(Some("The Song (feat. B)"), Some("The Song")).unwrap(),
            NameMatchType::VeryHigh,
            "Suffix variant: (feat. B)"
        );
        assert_eq!(
            compare_name(Some("The Song (Live)"), Some("The Song (Remix)")).unwrap(),
            NameMatchType::High,
            "Base name match with different parens"
        );
        assert_eq!(
            compare_name(Some("The Song (Live)"), Some("The Song")).unwrap(),
            NameMatchType::Medium,
            "Base name match with one having parens"
        );
        assert_eq!(
            compare_name(Some("color"), Some("colour")).unwrap(),
            NameMatchType::High,
            "Levenshtein medium-high score"
        );
    }

    #[test]
    fn test_compare_duration_gaussian() {
        assert_eq!(
            compare_duration(Some(180_000), Some(180_000)).unwrap(),
            DurationMatchType::Perfect,
            "Duration: Perfect"
        );
        assert_eq!(
            compare_duration(Some(180_000), Some(180_250)).unwrap(),
            DurationMatchType::VeryHigh,
            "Duration: VeryHigh"
        );
        assert_eq!(
            compare_duration(Some(180_000), Some(180_700)).unwrap(),
            DurationMatchType::High,
            "Duration: High (~sigma diff)"
        );
        assert_eq!(
            compare_duration(Some(180_000), Some(181_000)).unwrap(),
            DurationMatchType::Medium,
            "Duration: Medium"
        );
        assert_eq!(
            compare_duration(Some(180_000), Some(181_500)).unwrap(),
            DurationMatchType::Low,
            "Duration: Low"
        );
        assert_eq!(
            compare_duration(Some(180_000), Some(185_000)).unwrap(),
            DurationMatchType::NoMatch,
            "Duration: NoMatch"
        );
        assert!(
            compare_duration(Some(180_000), None).is_none(),
            "Duration: Handles None"
        );
    }

    #[test]
    fn test_compare_track() {
        let perfect_track = Track {
            title: Some("Perfect Song"),
            artists: Some(&["Artist A"]),
            album: Some("Perfect Album"),
            duration: Some(180_000),
        };
        let perfect_result = SearchResult {
            title: "Perfect Song".to_string(),
            artists: vec![Artist {
                name: "Artist A".to_string(),
                ..Default::default()
            }],
            album: Some("Perfect Album".to_string()),
            duration: Some(180_000),
            ..Default::default()
        };
        assert_eq!(
            compare_track(&perfect_track, &perfect_result),
            MatchType::Perfect,
            "Track: Perfect match"
        );

        let high_result = SearchResult {
            title: "Perfect Song (Live)".to_string(),
            artists: vec![Artist {
                name: "Artist A".to_string(),
                ..Default::default()
            }],
            album: Some("Perfect Album".to_string()),
            duration: Some(180_200),
            ..Default::default()
        };
        assert_eq!(
            compare_track(&perfect_track, &high_result),
            MatchType::VeryHigh,
            "Track: VeryHigh match with minor differences"
        );

        let incomplete_track = Track {
            title: Some("Perfect Song"),
            artists: Some(&["Artist A"]),
            album: None,
            duration: None,
        };
        assert_eq!(
            compare_track(&incomplete_track, &perfect_result),
            MatchType::Perfect,
            "Track: Score normalization for incomplete query"
        );

        let medium_result = SearchResult {
            title: "A decent song".to_string(),
            artists: vec![Artist {
                name: "Artist A".to_string(),
                ..Default::default()
            }],
            album: Some("Different Album".to_string()),
            duration: Some(220_000),
            ..Default::default()
        };
        assert_eq!(
            compare_track(&perfect_track, &medium_result),
            MatchType::Low,
            "Track: Low match with mostly different info"
        );

        let low_result = SearchResult {
            title: "Completely Different Song".to_string(),
            artists: vec![Artist {
                name: "Artist A".to_string(),
                ..Default::default()
            }],
            album: Some("Another Album".to_string()),
            duration: Some(120_000),
            ..Default::default()
        };
        assert_eq!(
            compare_track(&perfect_track, &low_result),
            MatchType::Low,
            "Track: Low match with only artist matching"
        );

        let no_match_result = SearchResult {
            title: "Another Song".to_string(),
            artists: vec![Artist {
                name: "Artist B".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            compare_track(&perfect_track, &no_match_result),
            MatchType::None,
            "Track: No match"
        );
    }
}
