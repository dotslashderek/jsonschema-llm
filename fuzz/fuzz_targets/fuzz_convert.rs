#![no_main]

use libfuzzer_sys::fuzz_target;

// Accepts arbitrary bytes, attempts to parse as JSON, feeds to convert().
// Goal: no panics, even on malformed input.
// TODO(#47): Consider a structure-aware fuzzer that generates valid
// serde_json::Value directly, bypassing JSON syntax parsing overhead.
fuzz_target!(|data: &[u8]| {
    if let Ok(schema) = serde_json::from_slice::<serde_json::Value>(data) {
        let _ = json_schema_llm_core::convert(&schema, &Default::default());
    }
});
