//! # JSON Schema Test Suite — Pipeline Conformance Harness
//!
//! Feeds every test-case schema from the [JSON Schema Test Suite](https://github.com/json-schema-org/JSON-Schema-Test-Suite)
//! through the full `convert()` pipeline and asserts **structural validity**:
//! no panics, and the output is well-formed JSON (object or boolean schema).
//!
//! ## Semantic Note
//!
//! The upstream suite tests *validators* (`data` + `valid` fields).
//! We test our *compiler* — only the `schema` field matters.
//! A test group passes if `convert()` returns `Ok` (valid output)
//! or a well-typed `Err(ConvertError)` (graceful rejection).
//! Only panics constitute failure.
//!
//! ## Coverage
//!
//! - **Draft 2020-12**: All keyword files (skips noted below)
//! - Draft 7 / 2019-09: Future scope

use json_schema_llm_core::{convert, ConvertOptions};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Test Suite data model
// ---------------------------------------------------------------------------

/// A single test case within a group (unused fields kept for completeness).
#[derive(Deserialize)]
struct _TestCase {
    #[allow(dead_code)]
    description: String,
    #[allow(dead_code)]
    data: serde_json::Value,
    #[allow(dead_code)]
    valid: bool,
}

/// A group of test cases sharing a schema.
#[derive(Deserialize)]
struct TestGroup {
    description: String,
    schema: serde_json::Value,
    #[allow(dead_code)]
    tests: Vec<_TestCase>,
}

// ---------------------------------------------------------------------------
// Shared harness
// ---------------------------------------------------------------------------

fn run_test_file(raw_json: &str, file_label: &str) {
    let groups: Vec<TestGroup> = serde_json::from_str(raw_json)
        .unwrap_or_else(|e| panic!("[{file_label}] parse error: {e}"));

    let options = ConvertOptions::default();
    let mut pass = 0usize;
    let mut graceful_err = 0usize;

    for (i, group) in groups.iter().enumerate() {
        let label = format!("{file_label}[{i}] {}", group.description);

        // The pipeline must not panic. Both Ok and well-typed Err are acceptable.
        match convert(&group.schema, &options) {
            Ok(result) => {
                // Output schema must be a JSON object or boolean (Draft 2020-12 allows both).
                assert!(
                    result.schema.is_object() || result.schema.is_boolean(),
                    "[{label}] convert() returned Ok but schema is neither object nor boolean: {:?}",
                    result.schema
                );
                // Codec must serialize cleanly.
                let _codec_json = serde_json::to_string(&result.codec)
                    .unwrap_or_else(|e| panic!("[{label}] codec serialization failed: {e}"));
                pass += 1;
            }
            Err(_e) => {
                // Graceful rejection — the pipeline recognized it can't handle this
                // schema and returned a structured error. This is acceptable.
                graceful_err += 1;
            }
        }
    }

    eprintln!(
        "  {file_label}: {pass} pass, {graceful_err} graceful err, {} total",
        groups.len()
    );
}

// ---------------------------------------------------------------------------
// Draft 2020-12 — one #[test] per keyword file
// ---------------------------------------------------------------------------

macro_rules! suite_test {
    ($name:ident, $file:literal) => {
        #[test]
        fn $name() {
            run_test_file(
                include_str!(concat!(
                    "../../../vendor/JSON-Schema-Test-Suite/tests/draft2020-12/",
                    $file
                )),
                stringify!($name),
            );
        }
    };
}

// ── Draft 2020-12 keyword files (alphabetical) ────────────────────────────
// Skipped: dynamicRef.json, refRemote.json, vocabulary.json

suite_test!(
    draft2020_12_additional_properties,
    "additionalProperties.json"
);
suite_test!(draft2020_12_all_of, "allOf.json");
suite_test!(draft2020_12_anchor, "anchor.json");
suite_test!(draft2020_12_any_of, "anyOf.json");
suite_test!(draft2020_12_boolean_schema, "boolean_schema.json");
suite_test!(draft2020_12_const, "const.json");
suite_test!(draft2020_12_contains, "contains.json");
suite_test!(draft2020_12_content, "content.json");
suite_test!(draft2020_12_default, "default.json");
suite_test!(draft2020_12_defs, "defs.json");
suite_test!(draft2020_12_dependent_required, "dependentRequired.json");
suite_test!(draft2020_12_dependent_schemas, "dependentSchemas.json");
// SKIP: dynamicRef.json — $dynamicRef/$dynamicAnchor not yet supported
suite_test!(draft2020_12_enum, "enum.json");
suite_test!(draft2020_12_exclusive_maximum, "exclusiveMaximum.json");
suite_test!(draft2020_12_exclusive_minimum, "exclusiveMinimum.json");
suite_test!(draft2020_12_format, "format.json");
suite_test!(draft2020_12_if_then_else, "if-then-else.json");
suite_test!(
    draft2020_12_infinite_loop_detection,
    "infinite-loop-detection.json"
);
suite_test!(draft2020_12_items, "items.json");
suite_test!(draft2020_12_max_contains, "maxContains.json");
suite_test!(draft2020_12_max_items, "maxItems.json");
suite_test!(draft2020_12_max_length, "maxLength.json");
suite_test!(draft2020_12_max_properties, "maxProperties.json");
suite_test!(draft2020_12_maximum, "maximum.json");
suite_test!(draft2020_12_min_contains, "minContains.json");
suite_test!(draft2020_12_min_items, "minItems.json");
suite_test!(draft2020_12_min_length, "minLength.json");
suite_test!(draft2020_12_min_properties, "minProperties.json");
suite_test!(draft2020_12_minimum, "minimum.json");
suite_test!(draft2020_12_multiple_of, "multipleOf.json");
suite_test!(draft2020_12_not, "not.json");
suite_test!(draft2020_12_one_of, "oneOf.json");
suite_test!(draft2020_12_pattern, "pattern.json");
suite_test!(draft2020_12_pattern_properties, "patternProperties.json");
suite_test!(draft2020_12_prefix_items, "prefixItems.json");
suite_test!(draft2020_12_properties, "properties.json");
suite_test!(draft2020_12_property_names, "propertyNames.json");
suite_test!(draft2020_12_ref, "ref.json");
// SKIP: refRemote.json — requires HTTP remote $ref resolution
suite_test!(draft2020_12_required, "required.json");
suite_test!(draft2020_12_type, "type.json");
suite_test!(draft2020_12_unevaluated_items, "unevaluatedItems.json");
suite_test!(
    draft2020_12_unevaluated_properties,
    "unevaluatedProperties.json"
);
suite_test!(draft2020_12_unique_items, "uniqueItems.json");
// SKIP: vocabulary.json — meta-schema vocabulary negotiation (not applicable)
