# TTML Processor

A high-performance Rust library for parsing and generating TTML files, specifically tailored for the Apple Music and [AMLL](<https://github.com/Steve-xmh/applemusic-like-lyrics>) formats.

> [CAUTION]
> This project is under heavy development and is not yet stable. Expect breaking changes. **Not ready for production use.**

## A Note on Specialization

This library is **not** a general-purpose TTML subtitle parser. It is specifically designed to handle the unique conventions, metadata structures, and extensions (e.g., `itunes:*` attributes, `<iTunesMetadata>`) found in TTML files used by Apple Music. Attempting to use it on generic TTML subtitle files may result in errors or incomplete data.

## Performance

This crate is engineered for speed. Benchmarks on a typical machine show that it can fully parse a complex, 59-line TTML file in approximately **~750 microseconds**.

## Usage

Add `ttml_processor` to your `Cargo.toml`:
```toml
[dependencies]
ttml_processor = "0.1.0" # Replace with the latest version
lyrics_helper_core = "0.1.0" # Required for data structures
```

### Parsing Example

```rust
use ttml_processor::parse_ttml;
use lyrics_helper_core::{TtmlParsingOptions, ContentType};

fn main() {
    let ttml_content = r#"
    <tt xmlns="http://www.w3.org/ns/ttml" itunes:timing="word">
      <body>
        <div>
          <p begin="5.000s" end="10.000s">
            <span begin="5.100s" end="5.500s">Hello</span>
            <span begin="5.600s" end="6.000s">world</span>
          </p>
        </div>
      </body>
    </tt>
    "#;

    let options = TtmlParsingOptions::default();
    let parsed_data = parse_ttml(ttml_content, &options).expect("Failed to parse TTML");

    assert_eq!(parsed_data.lines.len(), 1);
    let first_line = &parsed_data.lines[0];
    assert_eq!(first_line.start_ms, 5000);

    let main_track = first_line.tracks.iter().find(|t| t.content_type == ContentType::Main).unwrap();
    let syllables = &main_track.content.words[0].syllables;
    assert_eq!(syllables.len(), 2);
    assert_eq!(syllables[0].text, "Hello");
    assert_eq!(syllables[0].start_ms, 5100);

    println!("Successfully parsed TTML!");
}
```

### Generation Example

```rust
use ttml_processor::generate_ttml;
use lyrics_helper_core::{
    LyricLine, LyricSyllable, LyricTrack, AnnotatedTrack, Word,
    TtmlGenerationOptions, MetadataStore, AgentStore
};

fn main() {
    let line = LyricLine {
        start_ms: 1000,
        end_ms: 3000,
        tracks: vec![AnnotatedTrack {
            content: LyricTrack {
                words: vec![Word {
                    syllables: vec![
                        LyricSyllable { text: "Rust".to_string(), start_ms: 1200, end_ms: 1800, ..Default::default() },
                        LyricSyllable { text: "is".to_string(), start_ms: 1900, end_ms: 2500, ..Default::default() },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    };

    let options = TtmlGenerationOptions::default();
    let metadata_store = MetadataStore::new();
    let agent_store = AgentStore::new();

    let ttml_output = generate_ttml(&[line], &metadata_store, &agent_store, &options).unwrap();
    
    println!("{}", ttml_output);
}
```

## License

This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.
