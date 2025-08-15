use std::env;
use std::error::Error;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter};
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let dictionary_txt_path = "dictionary.txt";
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("dictionary.fst");

    println!("cargo:rerun-if-changed={}", dictionary_txt_path);

    let reader = BufReader::new(File::open(dictionary_txt_path)?);
    let mut lines: Vec<String> = reader
        .lines()
        .map(|line| Ok(line?.to_lowercase()))
        .collect::<io::Result<_>>()?;

    lines.sort_unstable();
    lines.dedup();

    let mut writer = BufWriter::new(File::create(&dest_path)?);
    let mut build = fst::SetBuilder::new(&mut writer)?;
    build.extend_iter(lines.iter())?;
    build.finish()?;

    Ok(())
}
