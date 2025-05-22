pub mod amll_fetcher;
pub mod types;

pub use types::{AmllIndexEntry, AmllSearchField, FetchedAmllTtmlLyrics};

pub use amll_fetcher::{
    download_and_parse_index, download_ttml_from_entry, search_lyrics_in_index,
};
