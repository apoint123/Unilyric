pub mod matcher;

use lyrics_helper_core::MatchType;

/// 名称（标题/专辑）的匹配程度。
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum NameMatchType {
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
pub enum ArtistMatchType {
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
pub enum DurationMatchType {
    #[default]
    NoMatch,
    Low,
    Medium,
    High,
    VeryHigh,
    Perfect,
}

/// 将匹配结果转换为可计算的分数。
pub trait MatchScorable {
    fn get_score(&self) -> i32;
}

impl MatchScorable for MatchType {
    fn get_score(&self) -> i32 {
        match self {
            Self::Perfect => 8,
            Self::VeryHigh => 7,
            Self::High => 6,
            Self::PrettyHigh => 5,
            Self::Medium => 4,
            Self::Low => 3,
            Self::VeryLow => 2,
            Self::None => 0,
        }
    }
}

impl MatchScorable for NameMatchType {
    fn get_score(&self) -> i32 {
        match self {
            Self::Perfect => 7,
            Self::VeryHigh => 6,
            Self::High => 5,
            Self::Medium => 4,
            Self::Low => 2,
            Self::NoMatch => 0,
        }
    }
}

impl MatchScorable for ArtistMatchType {
    fn get_score(&self) -> i32 {
        match self {
            Self::Perfect => 7,
            Self::VeryHigh => 6,
            Self::High => 5,
            Self::Medium => 4,
            Self::Low => 2,
            Self::NoMatch => 0,
        }
    }
}

impl MatchScorable for DurationMatchType {
    fn get_score(&self) -> i32 {
        match self {
            Self::Perfect => 7,
            Self::VeryHigh => 6,
            Self::High => 5,
            Self::Medium => 4,
            Self::Low => 2,
            Self::NoMatch => 0,
        }
    }
}

impl<T: MatchScorable> MatchScorable for Option<T> {
    fn get_score(&self) -> i32 {
        self.as_ref().map_or(0, MatchScorable::get_score)
    }
}
