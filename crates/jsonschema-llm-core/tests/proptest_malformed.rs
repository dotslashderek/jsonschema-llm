//! Property-based negative tests for malformed JSON Schemas.
//!
//! Validates that the `convert()` pipeline **never panics** — returns either
//! `Ok` (gracefully handled) or `Err(ConvertError)` — when given
//! structurally-valid JSON that is semantically invalid as JSON Schema.
//!
//! Note: many malformed schemas return `Ok` because the pipeline gracefully
//! ignores unrecognized or invalid keyword shapes. The invariant under test
//! is **no panics**, not necessarily `Err`.
//!
//! Complements the existing `fuzz/fuzz_targets/fuzz_convert.rs` (libfuzzer)
//! which covers arbitrary bytes → JSON parse → `convert()`. These tests use
//! **structure-aware** generation: every input is valid JSON but invalid schema
//! semantics.

use jsonschema_llm_core::{convert, ConvertOptions};
use proptest::prelude::*;
use serde_json::json;

fn default_opts() -> ConvertOptions {
    ConvertOptions::default()
}

// ===========================================================================
// 1. Deterministic negative tests — known malformed schemas
// ===========================================================================

/// `required` must be an array, not a string.
#[test]
fn malformed_required_as_string() {
    let schema = json!({
        "type": "object",
        "properties": { "name": { "type": "string" } },
        "required": "not_an_array"
    });
    let result = convert(&schema, &default_opts());
    // Must not panic — either Ok (gracefully handled) or Err
    let _ = result;
}

/// `anyOf` must be an array, not an object.
#[test]
fn malformed_anyof_as_object() {
    let schema = json!({
        "anyOf": { "not": "an_array" }
    });
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// `oneOf` must be an array, not a string.
#[test]
fn malformed_oneof_as_string() {
    let schema = json!({
        "oneOf": "not_an_array"
    });
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// `properties` must be an object, not a string.
#[test]
fn malformed_properties_as_string() {
    let schema = json!({
        "type": "object",
        "properties": "a_string"
    });
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// `type` must be a string (or array of strings), not a number.
#[test]
fn malformed_type_as_number() {
    let schema = json!({
        "type": 42
    });
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// `items` must be a schema (object/boolean), not a number.
#[test]
fn malformed_items_as_number() {
    let schema = json!({
        "type": "array",
        "items": 42
    });
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// `enum` must be an array, not a string.
#[test]
fn malformed_enum_as_string() {
    let schema = json!({
        "enum": "not_an_array"
    });
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// `$ref` must be a string, not a number.
#[test]
fn malformed_ref_as_number() {
    let schema = json!({
        "$ref": 42
    });
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// `allOf` must be an array, not an object.
#[test]
fn malformed_allof_as_object() {
    let schema = json!({
        "allOf": { "type": "string" }
    });
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// Nested malformed: valid outer object, malformed inner property.
#[test]
fn malformed_nested_property() {
    let schema = json!({
        "type": "object",
        "properties": {
            "good": { "type": "string" },
            "bad": {
                "type": "object",
                "properties": "not_an_object"
            }
        }
    });
    let result = convert(&schema, &default_opts());
    let _ = result;
}

// ===========================================================================
// 2. Expanded edge cases (from Gemini review)
// ===========================================================================

/// Unresolvable `$ref` — points to non-existent definition.
#[test]
fn malformed_unresolvable_ref() {
    let schema = json!({
        "$ref": "#/$defs/DoesNotExist"
    });
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// Negative numeric constraint — `minItems` should not be negative.
#[test]
fn malformed_negative_min_items() {
    let schema = json!({
        "type": "array",
        "items": { "type": "string" },
        "minItems": -1
    });
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// Invalid regex in `pattern`.
#[test]
fn malformed_invalid_pattern_regex() {
    let schema = json!({
        "type": "string",
        "pattern": "[invalid"
    });
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// Boolean `false` schema — valid JSON Schema (rejects everything).
/// Included as an edge case, not malformed.
#[test]
fn edge_case_boolean_false_schema() {
    let schema = json!(false);
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// Boolean `true` schema — valid JSON Schema (accepts everything).
/// Included as an edge case, not malformed.
#[test]
fn edge_case_boolean_true_schema() {
    let schema = json!(true);
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// Null schema — not a valid JSON Schema.
#[test]
fn malformed_null_schema() {
    let schema = json!(null);
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// Array as top-level schema — not a valid JSON Schema.
#[test]
fn malformed_array_schema() {
    let schema = json!([1, 2, 3]);
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// String as top-level schema — not a valid JSON Schema.
#[test]
fn malformed_string_schema() {
    let schema = json!("just_a_string");
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// Number as top-level schema — not a valid JSON Schema.
#[test]
fn malformed_number_schema() {
    let schema = json!(42);
    let result = convert(&schema, &default_opts());
    let _ = result;
}

/// Deeply nested malformed — malformed schema buried several levels deep.
#[test]
fn malformed_deeply_nested() {
    let schema = json!({
        "type": "object",
        "properties": {
            "level1": {
                "type": "object",
                "properties": {
                    "level2": {
                        "type": "object",
                        "properties": {
                            "level3": {
                                "required": 42
                            }
                        }
                    }
                }
            }
        }
    });
    let result = convert(&schema, &default_opts());
    let _ = result;
}

// ===========================================================================
// 3. Property-based negative tests — proptest strategies
// ===========================================================================

/// Strategy: generate a JSON Schema keyword with the WRONG value type.
fn arb_malformed_keyword() -> impl Strategy<Value = (&'static str, serde_json::Value)> {
    prop_oneof![
        // `required` should be an array → give it a string
        Just(("required", json!("not_an_array"))),
        // `required` should be an array → give it a number
        Just(("required", json!(42))),
        // `required` should be an array → give it an object
        Just(("required", json!({"key": "value"}))),
        // `properties` should be an object → give it a string
        Just(("properties", json!("not_an_object"))),
        // `properties` should be an object → give it an array
        Just(("properties", json!(["a", "b"]))),
        // `properties` should be an object → give it a number
        Just(("properties", json!(99))),
        // `type` should be a string → give it a number
        Just(("type", json!(42))),
        // `type` should be a string → give it an array of numbers
        Just(("type", json!([1, 2, 3]))),
        // `type` should be a string → give it null
        Just(("type", json!(null))),
        // `allOf` should be an array → give it an object
        Just(("allOf", json!({"type": "string"}))),
        // `anyOf` should be an array → give it a string
        Just(("anyOf", json!("bad"))),
        // `oneOf` should be an array → give it a boolean
        Just(("oneOf", json!(true))),
        // `items` should be a schema → give it a number
        Just(("items", json!(99))),
        // `items` should be a schema → give it a string
        Just(("items", json!("bad"))),
        // `enum` should be an array → give it a string
        Just(("enum", json!("bad"))),
        // `$ref` should be a string → give it a number
        Just(("$ref", json!(42))),
        // `$ref` should be a string → give it an array
        Just(("$ref", json!(["a", "b"]))),
        // `additionalProperties` should be bool/schema → give it an array
        Just(("additionalProperties", json!([1, 2]))),
        // `additionalProperties` should be bool/schema → give it a string
        Just(("additionalProperties", json!("bad"))),
    ]
}

/// Strategy: generate a malformed schema with 1-3 wrong keywords.
fn arb_malformed_schema() -> impl Strategy<Value = serde_json::Value> {
    proptest::collection::vec(arb_malformed_keyword(), 1..=3).prop_map(|keywords| {
        let mut obj = serde_json::Map::new();
        for (key, value) in keywords {
            obj.insert(key.to_string(), value);
        }
        serde_json::Value::Object(obj)
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..Default::default() })]

    /// Property: `convert()` NEVER panics on malformed schemas.
    ///
    /// Every generated input is valid JSON but semantically invalid JSON Schema.
    /// The pipeline must return `Ok` (gracefully handled) or `Err(ConvertError)`,
    /// never panic.
    #[test]
    fn convert_never_panics_on_malformed(schema in arb_malformed_schema()) {
        let opts = ConvertOptions::default();
        // Must not panic — the return value itself is not asserted
        let _ = convert(&schema, &opts);
    }

    /// Property: when `convert()` returns Err on malformed input, the error
    /// message is non-empty.
    #[test]
    fn convert_errors_have_messages(schema in arb_malformed_schema()) {
        let opts = ConvertOptions::default();
        if let Err(e) = convert(&schema, &opts) {
            let msg = e.to_string();
            prop_assert!(!msg.is_empty(), "ConvertError message should be non-empty");
        }
    }
}
