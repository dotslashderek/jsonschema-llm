//! Criterion benchmarks for the jsonschema-llm-core conversion pipeline.
//!
//! Fixtures are pre-parsed outside the benchmark loop to measure only the
//! conversion/rehydration logic, not JSON parsing or file I/O.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use serde_json::Value;
use std::fs;
use std::path::Path;

use jsonschema_llm_core::{convert, rehydrate, ConvertOptions};

/// Load and parse a fixture schema from the shared test fixtures directory.
fn load_fixture(name: &str) -> Value {
    let fixtures_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/schemas");
    let path = Path::new(fixtures_dir).join(name);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {}: {}", path.display(), e));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse fixture {}: {}", path.display(), e))
}

fn bench_convert_simple(c: &mut Criterion) {
    let schema = load_fixture("simple.json");
    let options = ConvertOptions::default();

    c.bench_function("convert/simple", |b| {
        b.iter(|| convert(black_box(&schema), black_box(&options)).unwrap())
    });
}

fn bench_convert_kitchen_sink(c: &mut Criterion) {
    let schema = load_fixture("kitchen_sink.json");
    let options = ConvertOptions::default();

    c.bench_function("convert/kitchen_sink", |b| {
        b.iter(|| convert(black_box(&schema), black_box(&options)).unwrap())
    });
}

fn bench_convert_recursive(c: &mut Criterion) {
    let schema = load_fixture("recursive.json");
    let options = ConvertOptions::default();

    c.bench_function("convert/recursive", |b| {
        b.iter(|| convert(black_box(&schema), black_box(&options)).unwrap())
    });
}

fn bench_rehydrate_roundtrip(c: &mut Criterion) {
    let schema = load_fixture("simple.json");
    let options = ConvertOptions::default();

    // Pre-convert to get the codec — only benchmark the rehydrate step
    let result = convert(&schema, &options).unwrap();
    let llm_output: Value = serde_json::json!({
        "name": "Ada Lovelace",
        "age": 36
    });

    c.bench_function("rehydrate/simple_roundtrip", |b| {
        b.iter(|| {
            rehydrate(
                black_box(&llm_output),
                black_box(&result.codec),
                black_box(&schema),
            )
            .unwrap()
        })
    });
}

criterion_group!(
    benches,
    bench_convert_simple,
    bench_convert_kitchen_sink,
    bench_convert_recursive,
    bench_rehydrate_roundtrip,
);
criterion_main!(benches);
