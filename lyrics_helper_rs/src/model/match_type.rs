//! 定义了用于匹配度量和评分的数据结构。

use lyrics_helper_core::MatchType;

/// 名称（标题/专辑）的匹配程度。
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) enum NameMatchType {
    #[default]
    NoMatch,
    Low,
    Medium,
    High,
    VeryHigh,
    Perfect,
}

/// 艺术家列表的匹配程度。
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) enum ArtistMatchType {
    #[default]
    NoMatch,
    Low,
    Medium,
    High,
    VeryHigh,
    Perfect,
}

/// 时长的匹配程度。
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) enum DurationMatchType {
    #[default]
    NoMatch,
    Low,
    Medium,
    High,
    VeryHigh,
    Perfect,
}

/// 将匹配结果转换为可计算的分数。
pub(crate) trait MatchScorable {
    fn get_score(&self) -> i32;
}

impl MatchScorable for MatchType {
    fn get_score(&self) -> i32 {
        match self {
            MatchType::Perfect => 8,
            MatchType::VeryHigh => 7,
            MatchType::High => 6,
            MatchType::PrettyHigh => 5,
            MatchType::Medium => 4,
            MatchType::Low => 3,
            MatchType::VeryLow => 2,
            MatchType::None => 0,
        }
    }
}

impl MatchScorable for NameMatchType {
    fn get_score(&self) -> i32 {
        match self {
            NameMatchType::Perfect => 7,
            NameMatchType::VeryHigh => 6,
            NameMatchType::High => 5,
            NameMatchType::Medium => 4,
            NameMatchType::Low => 2,
            NameMatchType::NoMatch => 0,
        }
    }
}

impl MatchScorable for ArtistMatchType {
    fn get_score(&self) -> i32 {
        match self {
            ArtistMatchType::Perfect => 7,
            ArtistMatchType::VeryHigh => 6,
            ArtistMatchType::High => 5,
            ArtistMatchType::Medium => 4,
            ArtistMatchType::Low => 2,
            ArtistMatchType::NoMatch => 0,
        }
    }
}

impl MatchScorable for DurationMatchType {
    fn get_score(&self) -> i32 {
        match self {
            DurationMatchType::Perfect => 7,
            DurationMatchType::VeryHigh => 6,
            DurationMatchType::High => 5,
            DurationMatchType::Medium => 4,
            DurationMatchType::Low => 2,
            DurationMatchType::NoMatch => 0,
        }
    }
}

impl<T: MatchScorable> MatchScorable for Option<T> {
    fn get_score(&self) -> i32 {
        self.as_ref().map_or(0, MatchScorable::get_score)
    }
}
