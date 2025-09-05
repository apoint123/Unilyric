use rand::{SeedableRng, rngs::StdRng, seq::SliceRandom};
use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter};
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let dictionary_txt_path = "dictionary.txt";
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("dictionary.fst");

    println!("cargo:rerun-if-changed={dictionary_txt_path}");

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

    let deviceid_txt_path = "deviceid.txt";

    println!("cargo:rerun-if-changed={deviceid_txt_path}");

    if Path::new(deviceid_txt_path).exists() {
        let deviceid_file = File::open(deviceid_txt_path)?;
        let mut all_deviceids: Vec<String> = BufReader::new(deviceid_file)
            .lines()
            .collect::<io::Result<_>>()?;

        if !all_deviceids.is_empty() {
            let mut rng = StdRng::from_os_rng();
            all_deviceids.shuffle(&mut rng);

            let selected_ids = all_deviceids
                .iter()
                .take(100.min(all_deviceids.len()))
                .cloned()
                .collect::<Vec<String>>()
                .join("\n");

            let dest_path_deviceids = Path::new(&out_dir).join("selected_deviceids.txt");
            fs::write(dest_path_deviceids, selected_ids)?;
        }
    }

    Ok(())
}
