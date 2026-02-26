//! #232 — End-to-End Strict Emission Compliance Tests
//!
//! Asserts that the `convert` pipeline (Target::OpenaiStrict) never emits
//! banned JSON Schema keywords in its output. These keywords are rejected
//! by OpenAI's strict mode and indicate a leak in the pipeline.
//!
//! The walker is intentionally exhaustive: it recurses into objects AND arrays
//! at every level to catch violations hidden in deeply nested structures.

use json_schema_llm_core::{
    convert, convert_all_components, ConvertOptions, ExtractOptions, Target,
};
use serde_json::Value;
use std::fs;
use std::path::Path;

// ── Banned Keywords ──────────────────────────────────────────────────────
//
// OpenAI strict mode rejects schemas containing any of these keywords.
// If *any* appear in the convert() output, the pipeline has a bug.
//
// NOTE: `$id` is intentionally EXCLUDED — OpenAI uses it for schema naming
// and it's valid in strict mode output.
//
// NOTE: `$ref` is tracked separately because the pipeline soft-fails on
// unresolvable external references, leaving dangling `$ref` values.
const BANNED_KEYWORDS: &[&str] = &[
    // Conditional composition
    "if",
    "then",
    "else",
    "not",
    // Evaluation / containment
    "unevaluatedProperties",
    "unevaluatedItems",
    "contains",
    "minContains",
    "maxContains",
    // Dependency keywords
    "dependentSchemas",
    "dependentRequired",
    // #246: patternProperties are stripped/opaque-stringified by p9
    "patternProperties",
];

// Keywords banned by OpenAI strict mode but EXPECTED as pipeline leaks.
// These are tracked via pinned counts rather than hard assertions because:
// - `$ref` / `$anchor` / `$dynamicRef` / `$dynamicAnchor`: the pipeline
//   soft-fails on unresolvable external references.
const SOFT_FAIL_KEYWORDS: &[&str] = &["$ref", "$anchor", "$dynamicRef", "$dynamicAnchor"];

// ── Walker ────────────────────────────────────────────────────────────────

/// Recursively walk a JSON Value and panic if any banned key is found.
fn assert_no_banned_keys(value: &Value, path: &str, banned: &[&str]) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if banned.contains(&key.as_str()) {
                    panic!("Banned keyword '{}' found at path {}/{}", key, path, key);
                }
                assert_no_banned_keys(child, &format!("{}/{}", path, key), banned);
            }
        }
        Value::Array(arr) => {
            for (i, child) in arr.iter().enumerate() {
                assert_no_banned_keys(child, &format!("{}[{}]", path, i), banned);
            }
        }
        _ => {}
    }
}

/// Collect all banned key occurrences (non-panicking).
/// Returns a Vec of (keyword, path) tuples.
fn collect_banned_keys(value: &Value, path: &str, banned: &[&str]) -> Vec<(String, String)> {
    let mut found = Vec::new();
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if banned.contains(&key.as_str()) {
                    found.push((key.clone(), format!("{}/{}", path, key)));
                }
                found.extend(collect_banned_keys(
                    child,
                    &format!("{}/{}", path, key),
                    banned,
                ));
            }
        }
        Value::Array(arr) => {
            for (i, child) in arr.iter().enumerate() {
                found.extend(collect_banned_keys(
                    child,
                    &format!("{}[{}]", path, i),
                    banned,
                ));
            }
        }
        _ => {}
    }
    found
}

// ── Helpers ───────────────────────────────────────────────────────────────

const INLINE_FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/schemas");
const REAL_FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures");

fn strict_options() -> ConvertOptions {
    let mut opts = ConvertOptions::default();
    opts.target = Target::OpenaiStrict;
    opts
}

fn inline_fixture_names() -> Vec<&'static str> {
    vec![
        "simple",
        "maps",
        "discriminator",
        "opaque",
        "allof",
        "recursive",
        "kitchen_sink",
    ]
}

fn load_inline_fixture(name: &str) -> Value {
    let path = Path::new(INLINE_FIXTURES_DIR).join(format!("{name}.json"));
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {name}.json: {e}"));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse fixture {name}.json: {e}"))
}

fn load_real_fixture(rel_path: &str) -> Value {
    let path = Path::new(REAL_FIXTURES_DIR).join(rel_path);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read real fixture {rel_path}: {e}"));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse real fixture {rel_path}: {e}"))
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// 1. Inline test fixtures: full-schema convert produces no banned keywords.
#[test]
fn test_strict_emission_inline_fixtures_no_banned_keywords() {
    let options = strict_options();
    for name in inline_fixture_names() {
        let schema = load_inline_fixture(name);
        let result = convert(&schema, &options)
            .unwrap_or_else(|e| panic!("Fixture '{name}' failed to convert: {e}"));
        assert_no_banned_keys(&result.schema, &format!("{name}#"), BANNED_KEYWORDS);
    }
}

/// 2. Real-world schemas: full-schema convert produces no banned keywords.
///
/// Tests the top-level `convert()` output for all three real-world source schemas.
/// Full-schema conversion should fully resolve all internal references.
#[test]
fn test_strict_emission_real_world_full_schema_no_banned_keywords() {
    let options = strict_options();

    let real_fixtures = [
        ("arazzo", "arazzo/source/arazzo-schema.json"),
        ("asyncapi", "asyncapi/source/asyncapi-2.6-schema-local.json"),
        ("oas31", "oas31/source/oas31-schema.json"),
    ];

    for (label, path) in &real_fixtures {
        let schema = load_real_fixture(path);
        let result = convert(&schema, &options)
            .unwrap_or_else(|e| panic!("{label} full convert failed: {e}"));
        assert_no_banned_keys(&result.schema, &format!("{label}#"), BANNED_KEYWORDS);
    }
}

/// 3. Real-world schemas: per-component convert is CLEAN of core banned keywords.
///
/// Core banned keywords (if/then/else/not/unevaluated*/contains*/dependent*)
/// should NEVER appear in component output. Hard zero tolerance.
#[test]
fn test_strict_emission_real_world_components_no_core_banned() {
    let options = strict_options();
    let extract_opts = ExtractOptions::default();

    let real_fixtures = [
        ("arazzo", "arazzo/source/arazzo-schema.json"),
        ("asyncapi", "asyncapi/source/asyncapi-2.6-schema-local.json"),
        ("oas31", "oas31/source/oas31-schema.json"),
    ];

    for (label, path) in &real_fixtures {
        let schema = load_real_fixture(path);
        let all = convert_all_components(&schema, &options, &extract_opts)
            .unwrap_or_else(|e| panic!("{label} convert_all_components failed: {e}"));

        for (pointer, comp_result) in &all.components {
            assert_no_banned_keys(
                &comp_result.schema,
                &format!("{label}:{pointer}"),
                BANNED_KEYWORDS,
            );
        }
    }
}

/// 4. Real-world schemas: per-component ref-like keyword leak tracking.
///
/// Component extraction may produce schemas with unresolved `$anchor`,
/// `$dynamicRef`, or `$ref` when the source uses advanced referencing.
/// This test pins the current counts as regression guards.
#[test]
fn test_strict_emission_real_world_components_ref_leak_pins() {
    let options = strict_options();
    let extract_opts = ExtractOptions::default();

    // (label, fixture_path, max_ref_like_leaks)
    //
    // Pins capture the CURRENT state. If a pass fix drops the count, update
    // the pin. If the count rises, you have a regression.
    let cases = [
        ("arazzo", "arazzo/source/arazzo-schema.json", 40),
        (
            "asyncapi",
            "asyncapi/source/asyncapi-2.6-schema-local.json",
            466,
        ),
        ("oas31", "oas31/source/oas31-schema.json", 55),
    ];

    for (label, path, max_leaks) in &cases {
        let schema = load_real_fixture(path);
        let all = convert_all_components(&schema, &options, &extract_opts)
            .unwrap_or_else(|e| panic!("{label} convert_all_components failed: {e}"));

        let mut total_leaks = Vec::new();
        for (pointer, comp_result) in &all.components {
            let leaks = collect_banned_keys(
                &comp_result.schema,
                &format!("{label}:{pointer}"),
                SOFT_FAIL_KEYWORDS,
            );
            total_leaks.extend(leaks);
        }

        assert!(
            total_leaks.len() <= *max_leaks,
            "{label}: soft-fail keyword leaks INCREASED from {max_leaks} to {} — regression!\nLeaks:\n{}",
            total_leaks.len(),
            total_leaks.iter()
                .map(|(kw, p)| format!("  {kw} at {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        );

        if total_leaks.len() < *max_leaks {
            eprintln!(
                "NOTE: {label} soft-fail leaks decreased from {max_leaks} to {} — tighten the pin!",
                total_leaks.len()
            );
        }
    }
}

/// 4. Negative test: the walker actually detects banned keywords.
///    Proves the walker isn't silently passing everything.
#[test]
fn test_strict_emission_walker_catches_violations() {
    use serde_json::json;

    // Flat violation — unevaluatedProperties at root
    let flat_violation = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        },
        "unevaluatedProperties": false
    });
    let result = std::panic::catch_unwind(|| {
        assert_no_banned_keys(&flat_violation, "#", BANNED_KEYWORDS);
    });
    assert!(
        result.is_err(),
        "Walker should catch 'unevaluatedProperties' at root level"
    );

    // Nested $ref inside array items — checked against REF_LIKE_KEYWORDS
    let nested_ref = json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "ref": { "$ref": "#/$defs/Foo" }
                    }
                }
            }
        }
    });
    let result = std::panic::catch_unwind(|| {
        assert_no_banned_keys(&nested_ref, "#", SOFT_FAIL_KEYWORDS);
    });
    assert!(
        result.is_err(),
        "Walker should catch '$ref' nested inside array items when using REF_LIKE_KEYWORDS"
    );

    // Nested inside array of arrays
    let deeply_nested = json!([
        [
            { "not": { "type": "string" } }
        ]
    ]);
    let result = std::panic::catch_unwind(|| {
        assert_no_banned_keys(&deeply_nested, "#", BANNED_KEYWORDS);
    });
    assert!(
        result.is_err(),
        "Walker should catch 'not' inside nested arrays"
    );
}

/// 6. Negative test: collect_banned_keys returns accurate counts.
#[test]
fn test_strict_emission_collector_counts_violations() {
    use serde_json::json;

    let schema_with_leaks = json!({
        "type": "object",
        "properties": {
            "a": { "$ref": "#/defs/A" },
            "b": {
                "type": "object",
                "unevaluatedProperties": false,
                "properties": {
                    "c": { "not": { "type": "null" } }
                }
            }
        }
    });

    // BANNED_KEYWORDS should find 2: unevaluatedProperties + not
    let core_leaks = collect_banned_keys(&schema_with_leaks, "#", BANNED_KEYWORDS);
    assert_eq!(
        core_leaks.len(),
        2,
        "Should find exactly 2 core violations (unevaluatedProperties, not), found: {:?}",
        core_leaks
    );

    // REF_LIKE_KEYWORDS should find 1: $ref
    let ref_leaks = collect_banned_keys(&schema_with_leaks, "#", SOFT_FAIL_KEYWORDS);
    assert_eq!(
        ref_leaks.len(),
        1,
        "Should find exactly 1 ref-like violation ($ref), found: {:?}",
        ref_leaks
    );
}
