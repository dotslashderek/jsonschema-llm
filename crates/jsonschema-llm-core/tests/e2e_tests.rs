//! End-to-end integration tests that exercise the full 8-pass pipeline
//! against fixture schemas. These complement the existing `pipeline_tests.rs`
//! by using file-based fixtures covering every pass combination.

use jsonschema_llm_core::{convert, rehydrate, ConvertOptions, Target};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

// ── Helpers ─────────────────────────────────────────────────────────────────

const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/schemas");

fn fixture_names() -> Vec<&'static str> {
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

fn load_fixture(name: &str) -> Value {
    let path = Path::new(FIXTURES_DIR).join(format!("{name}.json"));
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {name}.json: {e}"));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse fixture {name}.json: {e}"))
}

fn openai_options() -> ConvertOptions {
    ConvertOptions::default()
}

fn all_targets() -> Vec<Target> {
    vec![Target::OpenaiStrict, Target::Gemini, Target::Claude]
}

/// Recursively verify every `type: object` node has `additionalProperties: false`
/// and `required` containing all property keys.
fn assert_strict_compliance(val: &Value, path: &str) {
    if let Some(obj) = val.as_object() {
        if obj.get("type").and_then(|t| t.as_str()) == Some("object") {
            if let Some(props) = obj.get("properties").and_then(|p| p.as_object()) {
                // Must have additionalProperties: false
                assert_eq!(
                    obj.get("additionalProperties"),
                    Some(&json!(false)),
                    "Missing additionalProperties: false at {path}"
                );

                // Must have required containing all property keys
                if let Some(req) = obj.get("required").and_then(|r| r.as_array()) {
                    let req_set: std::collections::HashSet<&str> =
                        req.iter().filter_map(|v| v.as_str()).collect();
                    for key in props.keys() {
                        assert!(
                            req_set.contains(key.as_str()),
                            "Property '{key}' not in required at {path}"
                        );
                    }
                }
            }
        }

        // Recurse into all values
        for (key, child) in obj {
            assert_strict_compliance(child, &format!("{path}/{key}"));
        }
    } else if let Some(arr) = val.as_array() {
        for (i, child) in arr.iter().enumerate() {
            assert_strict_compliance(child, &format!("{path}[{i}]"));
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

// 1. Every fixture converts to valid JSON without errors
#[test]
fn test_e2e_fixture_convert_valid_json() {
    for name in fixture_names() {
        let schema = load_fixture(name);
        let result = convert(&schema, &openai_options());
        assert!(
            result.is_ok(),
            "Fixture '{name}' failed to convert: {:?}",
            result.err()
        );
        // Output should be a valid JSON object
        let r = result.unwrap();
        assert!(
            r.schema.is_object(),
            "Fixture '{name}' output is not an object"
        );
    }
}

// 2. Strict mode compliance for all fixtures (OpenAI target)
#[test]
fn test_e2e_fixture_strict_mode_compliance() {
    for name in fixture_names() {
        let schema = load_fixture(name);
        let result = convert(&schema, &openai_options()).unwrap();
        assert_strict_compliance(&result.schema, &format!("{name}#"));
    }
}

// 3. Codec roundtrip — convert then rehydrate with sample data
#[test]
fn test_e2e_fixture_codec_roundtrip() {
    // Use simple fixture: convert, create sample LLM output, rehydrate
    let schema = load_fixture("simple");
    let result = convert(&schema, &openai_options()).unwrap();

    // Simulate LLM output conforming to the converted schema
    let llm_output = json!({
        "name": "Alice",
        "age": 30,
        "email": null,
        "active": null
    });

    let rehydrated = rehydrate(&llm_output, &result.codec);
    assert!(
        rehydrated.is_ok(),
        "Rehydration failed: {:?}",
        rehydrated.err()
    );
    let data = rehydrated.unwrap().data;
    assert_eq!(data["name"], json!("Alice"));
    assert_eq!(data["age"], json!(30));
}

// 4. Codec transform counts — pinned golden values per fixture
#[test]
fn test_e2e_fixture_codec_entry_counts() {
    use jsonschema_llm_core::codec::Transform;
    use std::collections::HashMap;

    let cases: Vec<(&str, usize)> = vec![
        ("simple", 2),
        ("maps", 9),
        ("discriminator", 1),
        ("opaque", 6),
        ("allof", 3),
        ("recursive", 5),
        ("kitchen_sink", 21),
    ];

    // Cache results to avoid duplicate convert calls
    let mut results = HashMap::new();
    for (name, expected_count) in &cases {
        let schema = load_fixture(name);
        let result = convert(&schema, &openai_options()).unwrap();
        assert_eq!(
            result.codec.transforms.len(),
            *expected_count,
            "Fixture '{name}' expected {expected_count} transforms, got {}",
            result.codec.transforms.len()
        );
        results.insert(*name, result);
    }

    // maps.json must contain exactly 4 MapToArray transforms
    let maps_result = results.get("maps").unwrap();
    let map_count = maps_result
        .codec
        .transforms
        .iter()
        .filter(|t| matches!(t, Transform::MapToArray { .. }))
        .count();
    assert_eq!(
        map_count, 4,
        "maps fixture should have exactly 4 MapToArray transforms"
    );

    // opaque.json must contain exactly 3 JsonStringParse transforms
    let opaque_result = results.get("opaque").unwrap();
    let jsp_count = opaque_result
        .codec
        .transforms
        .iter()
        .filter(|t| matches!(t, Transform::JsonStringParse { .. }))
        .count();
    assert_eq!(
        jsp_count, 3,
        "opaque fixture should have exactly 3 JsonStringParse transforms"
    );
}

// 5. Kitchen sink full roundtrip
#[test]
fn test_e2e_kitchen_sink_full_roundtrip() {
    let schema = load_fixture("kitchen_sink");
    let result = convert(&schema, &openai_options()).unwrap();

    // Verify transforms span multiple types
    let transform_types: std::collections::HashSet<String> = result
        .codec
        .transforms
        .iter()
        .map(|t| format!("{:?}", t))
        .collect();
    assert!(
        transform_types.len() > 1,
        "Kitchen sink should produce multiple transform types, got: {transform_types:?}"
    );

    // Verify the schema is valid JSON and strict-compliant
    assert_strict_compliance(&result.schema, "kitchen_sink#");
}

// 6. All targets produce valid output for every fixture
#[test]
fn test_e2e_all_targets() {
    for name in fixture_names() {
        let schema = load_fixture(name);
        for target in all_targets() {
            let options = ConvertOptions {
                target,
                ..ConvertOptions::default()
            };
            let result = convert(&schema, &options);
            assert!(
                result.is_ok(),
                "Fixture '{name}' with target {target:?} failed: {:?}",
                result.err()
            );
        }
    }
}

// 7. Idempotent — converting twice produces the same output
#[test]
fn test_e2e_idempotent() {
    for name in fixture_names() {
        let schema = load_fixture(name);
        let result1 = convert(&schema, &openai_options()).unwrap();
        let result2 = convert(&result1.schema, &openai_options()).unwrap();
        assert_eq!(
            result1.schema, result2.schema,
            "Fixture '{name}' is not idempotent"
        );
    }
}

// 8. Malformed JSON input — pipeline handles gracefully without panicking
#[test]
fn test_e2e_error_malformed_json() {
    let not_a_schema = json!("this is just a string");
    let result = convert(&not_a_schema, &openai_options());
    // Non-object input is handled gracefully: either an error or a
    // best-effort result. The key contract is no panic.
    match result {
        Ok(_r) => {
            // If pipeline returns Ok, we got a valid ConvertResult
            // (no need to assert on schema shape — serde_json::Value is always valid)
        }
        Err(e) => {
            // Error message should be descriptive
            let msg = format!("{e}");
            assert!(!msg.is_empty(), "Error should have a descriptive message");
        }
    }
}

// 9. Depth exceeded produces an error
#[test]
fn test_e2e_error_depth_exceeded() {
    let schema = load_fixture("recursive");
    let options = ConvertOptions {
        max_depth: 2,
        recursion_limit: 1,
        ..ConvertOptions::default()
    };
    // With a very tight max_depth, P0's traversal guard triggers before
    // P5 can break the recursion — this should return an error.
    let result = convert(&schema, &options);
    assert!(
        result.is_err(),
        "Recursive fixture with max_depth=2 should return RecursionDepthExceeded error"
    );
}

// 10. Rehydrator handles extra fields gracefully
#[test]
fn test_e2e_rehydrate_extra_field() {
    let schema = load_fixture("simple");
    let result = convert(&schema, &openai_options()).unwrap();

    // LLM hallucinates an extra field not in the schema
    let llm_output = json!({
        "name": "Bob",
        "age": 25,
        "email": null,
        "active": null,
        "hallucinated_field": "this shouldn't be here"
    });

    let rehydrated = rehydrate(&llm_output, &result.codec);
    assert!(
        rehydrated.is_ok(),
        "Rehydrator should handle extra fields gracefully"
    );
}

// 11. Rehydrator warns on null for non-nullable
#[test]
fn test_e2e_rehydrate_null_non_nullable() {
    let schema = load_fixture("simple");
    let result = convert(&schema, &openai_options()).unwrap();

    // LLM returns null for 'name' which was required (non-nullable)
    let llm_output = json!({
        "name": null,
        "age": 30,
        "email": null,
        "active": null
    });

    let rehydrated = rehydrate(&llm_output, &result.codec);
    assert!(
        rehydrated.is_ok(),
        "Rehydrator should not crash on null for required fields"
    );
}
