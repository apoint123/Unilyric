pub mod amll_fetcher;
pub mod types;

pub use types::{AmllIndexEntry, AmllSearchField, FetchedAmllTtmlLyrics};

pub use amll_fetcher::download_ttml_from_entry;
