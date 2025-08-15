use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use lyrics_helper_core::TtmlParsingOptions;
use ttml_processor::parse_ttml;

const SAMPLE_TTML: &str = include_str!("../tests/test_data/real_world.ttml");

fn benchmark_parse_ttml(c: &mut Criterion) {
    let mut group = c.benchmark_group("TTML Parsing");

    group.measurement_time(Duration::from_secs(20));
    group.sample_size(200);

    let default_opitons = TtmlParsingOptions::default();

    group.bench_function("parse_normal_ttml", |b| {
        b.iter(|| {
            let parsed_data = parse_ttml(black_box(SAMPLE_TTML), black_box(&default_opitons))
                .expect("样本解析失败");

            black_box(parsed_data);
        });
    });

    group.finish();
}

criterion_group!(benches, benchmark_parse_ttml);

criterion_main!(benches);
