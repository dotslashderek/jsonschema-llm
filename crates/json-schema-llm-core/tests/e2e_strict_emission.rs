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
// `$ref` is included because the pipeline fully resolves all internal
// references. The walker's `parent_key` context-awareness skips `$ref`
// when it appears as a user-defined property name (e.g., AsyncAPI's
// ReferenceObject), so no false positives occur.
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
    // Reference-mechanism keywords — fully resolved/stripped by pipeline
    "$ref",
    "$anchor",
    "$dynamicRef",
    "$dynamicAnchor",
];

// ── Walker ────────────────────────────────────────────────────────────────

/// Recursively walk a JSON Value and panic if any banned key is found.
///
/// The `parent_key` parameter is used to suppress false positives: when the
/// parent key is `"properties"`, child keys are property *names* (not schema
/// keywords), so they are not checked against the banned list.
fn assert_no_banned_keys(value: &Value, path: &str, banned: &[&str]) {
    assert_no_banned_keys_inner(value, path, banned, "");
}

fn assert_no_banned_keys_inner(value: &Value, path: &str, banned: &[&str], parent_key: &str) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                // Skip banned-key check when inside a `properties` map —
                // child keys are user-defined property names, not schema keywords.
                if parent_key != "properties" && banned.contains(&key.as_str()) {
                    panic!("Banned keyword '{}' found at path {}/{}", key, path, key);
                }
                assert_no_banned_keys_inner(child, &format!("{}/{}", path, key), banned, key);
            }
        }
        Value::Array(arr) => {
            for (i, child) in arr.iter().enumerate() {
                assert_no_banned_keys_inner(child, &format!("{}[{}]", path, i), banned, parent_key);
            }
        }
        _ => {}
    }
}

/// Collect all banned key occurrences (non-panicking).
/// Returns a Vec of (keyword, path) tuples.
///
/// Skips banned-key checks when inside a `properties` map (property names
/// are user-defined, not schema keywords).
fn collect_banned_keys(value: &Value, path: &str, banned: &[&str]) -> Vec<(String, String)> {
    collect_banned_keys_inner(value, path, banned, "")
}

fn collect_banned_keys_inner(
    value: &Value,
    path: &str,
    banned: &[&str],
    parent_key: &str,
) -> Vec<(String, String)> {
    let mut found = Vec::new();
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if parent_key != "properties" && banned.contains(&key.as_str()) {
                    found.push((key.clone(), format!("{}/{}", path, key)));
                }
                found.extend(collect_banned_keys_inner(
                    child,
                    &format!("{}/{}", path, key),
                    banned,
                    key,
                ));
            }
        }
        Value::Array(arr) => {
            for (i, child) in arr.iter().enumerate() {
                found.extend(collect_banned_keys_inner(
                    child,
                    &format!("{}[{}]", path, i),
                    banned,
                    parent_key,
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

/// 3. Real-world schemas: per-component output is fully clean.
///
/// All banned keywords — including `$ref` — must be absent from
/// component output. The walker's `parent_key` awareness prevents
/// false positives on user-defined property names like `$ref`.
#[test]
fn test_strict_emission_real_world_components_no_banned_keywords() {
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

    // Nested $ref inside array items — now caught by BANNED_KEYWORDS
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
        assert_no_banned_keys(&nested_ref, "#", BANNED_KEYWORDS);
    });
    assert!(
        result.is_err(),
        "Walker should catch '$ref' nested inside array items"
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

/// 6. Negative test: collect_banned_keys returns accurate violations.
#[test]
fn test_strict_emission_collector_counts_violations() {
    use serde_json::json;
    use std::collections::HashSet;

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

    // BANNED_KEYWORDS should find: unevaluatedProperties, not, $ref
    let leaks = collect_banned_keys(&schema_with_leaks, "#", BANNED_KEYWORDS);
    let found_keywords: HashSet<&str> = leaks.iter().map(|(kw, _)| kw.as_str()).collect();
    let expected: HashSet<&str> = ["unevaluatedProperties", "not", "$ref"]
        .into_iter()
        .collect();
    assert_eq!(
        found_keywords, expected,
        "Expected violations {:?}, found {:?}",
        expected, found_keywords
    );
}
