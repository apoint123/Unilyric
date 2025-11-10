use lyrics_helper_core::{MetadataStore, TtmlGenerationOptions, TtmlParsingOptions};
use std::fs;
use ttml_processor::{generate_ttml, parse_ttml};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input_path = "ttml_processor/tests/test_data/complex_round_trip.ttml";
    let output_path = "roundtrip_output.ttml";
    let ttml_content = fs::read_to_string(input_path)?;
    let parsing_options = TtmlParsingOptions::default();
    let parsed_data = parse_ttml(&ttml_content, &parsing_options)?;

    let mut metadata_store = MetadataStore::new();
    metadata_store.load_from_raw(&parsed_data.raw_metadata);

    let agent_store = &parsed_data.agents;

    let generation_options = TtmlGenerationOptions {
        format: true,
        use_apple_format_rules: true,
        ..Default::default()
    };

    let generated_ttml = generate_ttml(
        &parsed_data.lines,
        &metadata_store,
        agent_store,
        &generation_options,
    )?;

    fs::write(output_path, &generated_ttml)?;
    let preview: String = generated_ttml.lines().collect::<Vec<_>>().join("\n");
    println!("{preview}");

    Ok(())
}
