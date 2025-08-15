//! # TTML Processor: A Specialized Parser and Generator for Apple Music and AMLL Lyrics
//!
//! This crate provides robust, high-performance tools for handling TTML (Timed Text Markup Language)
//! files, specifically tailored to the format used by Apple Music. It offers a powerful streaming
//! parser and a flexible generator, making it easy to convert between TTML strings and structured
//! Rust objects.
//!
//! The two primary functions you will use are:
//! - [`parse_ttml`]: Converts a TTML string into a `ParsedSourceData` object from `lyrics_helper_core`.
//! - [`generate_ttml`]: Creates a TTML string from `LyricLine` data structures.
//!
//! ## ⚠️ Important: Not a General-Purpose Parser
//!
//! This library is **not** designed for generic TTML subtitle files. It is specifically
//! optimized for the conventions and extensions found in Apple Music and AMLL lyrics, such as
//! `itunes:*` attributes and the `<iTunesMetadata>` block. Using it on other types of TTML
//! may lead to unexpected results or errors.
//!
//! ## Examples
//!
//! Here is a basic round-trip example showing how to parse a TTML string and then
//! generate a new one from the parsed data.
//!
//! ```rust
//! use ttml_processor::{parse_ttml, generate_ttml};
//! use lyrics_helper_core::{
//!     TtmlParsingOptions, TtmlGenerationOptions,
//!     MetadataStore, AgentStore, ContentType
//! };
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // 1. Define some simple TTML content
//!     let ttml_content = r#"
//!     <tt xmlns="http://www.w3.org/ns/ttml" itunes:timing="word">
//!       <body>
//!         <div>
//!           <p begin="5.0s" end="10.0s">
//!             <span begin="5.1s" end="5.5s">Hello</span>
//!             <span begin="5.6s" end="6.0s"> world</span>
//!           </p>
//!         </div>
//!       </body>
//!     </tt>
//!     "#;
//!
//!     // 2. Parse the TTML string into structured data
//!     let parsing_options = TtmlParsingOptions::default();
//!     let parsed_data = parse_ttml(ttml_content, &parsing_options)?;
//!
//!     assert_eq!(parsed_data.lines.len(), 1);
//!     let first_line = &parsed_data.lines[0];
//!     assert_eq!(first_line.start_ms, 5000);
//!
//!     let main_track = first_line.tracks.iter().find(|t| t.content_type == ContentType::Main).unwrap();
//!     let syllables = &main_track.content.words[0].syllables;
//!     
//!     assert_eq!(syllables[0].text, "Hello");
//!     // The space before "world" is captured as a flag on the preceding syllable.
//!     assert_eq!(syllables[0].ends_with_space, true);
//!     // The text of the second syllable itself is trimmed.
//!     assert_eq!(syllables[1].text, "world");
//!     assert_eq!(syllables[1].ends_with_space, false);
//!
//!     println!("✅ Parsing successful!");
//!     // 3. Generate a new TTML string from the parsed data
//!     let generation_options = TtmlGenerationOptions::default();
//!     let generated_ttml = generate_ttml(
//!         &parsed_data.lines,
//!         &MetadataStore::new(), // Use empty stores for this example
//!         &AgentStore::new(),
//!         &generation_options
//!     )?;
//!
//!     println!("\nGenerated TTML:\n{}", generated_ttml);
//!     assert!(generated_ttml.contains("<span begin=\"5.100\""));
//!
//!     Ok(())
//! }
//! ```

pub mod generator;
pub mod parser;
mod utils;

pub use generator::generate_ttml;
pub use parser::parse_ttml;
