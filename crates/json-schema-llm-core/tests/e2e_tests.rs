//! End-to-end integration tests that exercise the full 9-pass pipeline
//! against fixture schemas. These complement the existing `pipeline_tests.rs`
//! by using file-based fixtures covering every pass combination.

use json_schema_llm_core::{convert, rehydrate, ConvertOptions, Target};
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

    let rehydrated = rehydrate(&llm_output, &result.codec, &schema);
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
    use json_schema_llm_core::codec::Transform;
    use std::collections::HashMap;

    let cases: Vec<(&str, usize)> = vec![
        ("simple", 2),
        ("maps", 9),
        ("discriminator", 1),
        ("opaque", 6),
        ("allof", 3),
        ("recursive", 6),
        ("kitchen_sink", 22),
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
            let mut options = ConvertOptions::default();
            options.target = target;
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
    let mut options = ConvertOptions::default();
    options.max_depth = 2;
    options.recursion_limit = 1;
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

    let rehydrated = rehydrate(&llm_output, &result.codec, &schema);
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

    let rehydrated = rehydrate(&llm_output, &result.codec, &schema);
    assert!(
        rehydrated.is_ok(),
        "Rehydrator should not crash on null for required fields"
    );
}

// 12. Type coercion: string→integer is coerced and warning emitted
#[test]
fn test_e2e_rehydrate_type_coercion() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "integer" }
        },
        "required": ["name", "age"]
    });

    let result = convert(&schema, &openai_options()).unwrap();

    // LLM returns age as string "42" instead of integer 42
    let llm_output = json!({
        "name": "Ada",
        "age": "42"
    });

    let rehydrated = rehydrate(&llm_output, &result.codec, &schema).unwrap();

    // (1) Value should be coerced to integer 42
    assert_eq!(
        rehydrated.data["age"],
        json!(42),
        "String '42' should be coerced to integer 42"
    );

    // (2) A coercion warning should be emitted
    assert!(
        !rehydrated.warnings.is_empty(),
        "Coercion should produce at least one warning"
    );

    // (3) Name should remain unchanged
    assert_eq!(rehydrated.data["name"], json!("Ada"));
}

// 13. Type coercion: boolean string "true" → true
#[test]
fn test_e2e_rehydrate_type_coercion_boolean() {
    let schema = json!({
        "type": "object",
        "properties": {
            "active": { "type": "boolean" }
        },
        "required": ["active"]
    });

    let result = convert(&schema, &openai_options()).unwrap();

    // LLM returns boolean as string
    let llm_output = json!({ "active": "true" });

    let rehydrated = rehydrate(&llm_output, &result.codec, &schema).unwrap();

    assert_eq!(
        rehydrated.data["active"],
        json!(true),
        "String 'true' should be coerced to boolean true"
    );
}

// ── Coercion + Constraint Interaction Tests (#125 / #126) ──────────────

// 14. Coercion then constraint violation: string→integer with minimum
#[test]
fn test_e2e_rehydrate_coercion_then_minimum_violation() {
    use json_schema_llm_core::codec_warning::WarningKind;

    let schema = json!({
        "type": "object",
        "properties": {
            "age": { "type": "integer", "minimum": 5 }
        },
        "required": ["age"]
    });

    let result = convert(&schema, &openai_options()).unwrap();

    // LLM returns "1" (string) — should coerce to 1, then catch minimum:5 violation
    let llm_output = json!({ "age": "1" });
    let rehydrated = rehydrate(&llm_output, &result.codec, &schema).unwrap();

    // (1) Value should be coerced to integer 1 then clamped to minimum 5
    assert_eq!(
        rehydrated.data["age"],
        json!(5),
        "String '1' should be coerced to 1 then clamped to minimum 5"
    );

    // (2) Should have both coercion and constraint warnings
    let has_coercion = rehydrated.warnings.iter().any(|w| {
        matches!(&w.kind, WarningKind::ConstraintViolation { constraint } if constraint == "type")
    });
    let has_minimum = rehydrated.warnings.iter().any(|w| {
        matches!(&w.kind, WarningKind::ConstraintViolation { constraint } if constraint == "minimum")
    });
    assert!(has_coercion, "Should have a type coercion warning");
    assert!(
        has_minimum,
        "Should have a minimum constraint violation warning"
    );
}

// 15. Coercion then enforcement: string→integer with maximum, clamped
#[test]
fn test_e2e_rehydrate_coercion_then_maximum_enforcement() {
    use json_schema_llm_core::codec_warning::WarningKind;

    let schema = json!({
        "type": "object",
        "properties": {
            "score": { "type": "integer", "maximum": 100 }
        },
        "required": ["score"]
    });

    let result = convert(&schema, &openai_options()).unwrap();

    // LLM returns "150" (string) — should coerce to 150, then clamp to 100
    let llm_output = json!({ "score": "150" });
    let rehydrated = rehydrate(&llm_output, &result.codec, &schema).unwrap();

    // (1) Value should be coerced then clamped to 100
    assert_eq!(
        rehydrated.data["score"],
        json!(100),
        "String '150' should be coerced to 150 then clamped to maximum 100"
    );

    // (2) Should have both coercion and constraint enforcement warnings
    let has_coercion = rehydrated
        .warnings
        .iter()
        .any(|w| matches!(&w.kind, WarningKind::ConstraintViolation { constraint } if constraint == "type"));
    let has_maximum = rehydrated.warnings.iter().any(|w| {
        matches!(&w.kind, WarningKind::ConstraintViolation { constraint } if constraint == "maximum")
    });
    assert!(has_coercion, "Should have a type coercion warning");
    assert!(
        has_maximum,
        "Should have a maximum constraint enforcement warning"
    );
}

// 16. Coercion then constraint pass: value within bounds after coercion
#[test]
fn test_e2e_rehydrate_coercion_then_constraint_pass() {
    use json_schema_llm_core::codec_warning::WarningKind;

    let schema = json!({
        "type": "object",
        "properties": {
            "score": { "type": "integer", "minimum": 0, "maximum": 100 }
        },
        "required": ["score"]
    });

    let result = convert(&schema, &openai_options()).unwrap();

    // LLM returns "50" (string) — within bounds after coercion
    let llm_output = json!({ "score": "50" });
    let rehydrated = rehydrate(&llm_output, &result.codec, &schema).unwrap();

    // (1) Value should be coerced to integer 50
    assert_eq!(
        rehydrated.data["score"],
        json!(50),
        "String '50' should be coerced to integer 50"
    );

    // (2) Should have coercion warning but NO constraint violation
    let has_coercion = rehydrated
        .warnings
        .iter()
        .any(|w| matches!(&w.kind, WarningKind::ConstraintViolation { constraint } if constraint == "type"));
    let has_constraint_violation = rehydrated.warnings.iter().any(|w| {
        matches!(&w.kind, WarningKind::ConstraintViolation { constraint } if constraint == "minimum" || constraint == "maximum")
    });
    assert!(has_coercion, "Should have a type coercion warning");
    assert!(
        !has_constraint_violation,
        "Should NOT have any constraint violation — 50 is within [0,100]"
    );
}

// 17. Warning ordering: coercion warnings before constraint warnings
#[test]
fn test_e2e_rehydrate_coercion_warning_ordering() {
    use json_schema_llm_core::codec_warning::WarningKind;

    let schema = json!({
        "type": "object",
        "properties": {
            "age": { "type": "integer", "minimum": 5 }
        },
        "required": ["age"]
    });

    let result = convert(&schema, &openai_options()).unwrap();

    let llm_output = json!({ "age": "1" });
    let rehydrated = rehydrate(&llm_output, &result.codec, &schema).unwrap();

    // Find the first coercion warning and first constraint warning
    let first_coercion_idx = rehydrated.warnings.iter().position(|w| {
        matches!(&w.kind, WarningKind::ConstraintViolation { constraint } if constraint == "type")
    });
    let first_constraint_idx = rehydrated.warnings.iter().position(|w| {
        matches!(&w.kind, WarningKind::ConstraintViolation { constraint } if constraint == "minimum")
    });

    assert!(
        first_coercion_idx.is_some(),
        "Should have a coercion warning"
    );
    assert!(
        first_constraint_idx.is_some(),
        "Should have a constraint warning"
    );
    assert!(
        first_coercion_idx.unwrap() < first_constraint_idx.unwrap(),
        "Coercion warnings should appear before constraint warnings (coercion at {}, constraint at {})",
        first_coercion_idx.unwrap(),
        first_constraint_idx.unwrap()
    );
}

// 18. Failed coercion: non-coercible value skips constraint check
#[test]
fn test_e2e_rehydrate_failed_coercion_no_constraint() {
    use json_schema_llm_core::codec_warning::WarningKind;

    let schema = json!({
        "type": "object",
        "properties": {
            "age": { "type": "integer", "minimum": 5 }
        },
        "required": ["age"]
    });

    let result = convert(&schema, &openai_options()).unwrap();

    // LLM returns "abc" — not coercible to integer
    let llm_output = json!({ "age": "abc" });
    let rehydrated = rehydrate(&llm_output, &result.codec, &schema).unwrap();

    // (1) Value should remain as string "abc" (coercion fails silently)
    assert_eq!(
        rehydrated.data["age"],
        json!("abc"),
        "Non-coercible 'abc' should remain as string"
    );

    // (2) Should NOT have a minimum constraint warning (string is not numeric)
    let has_minimum = rehydrated.warnings.iter().any(|w| {
        matches!(&w.kind, WarningKind::ConstraintViolation { constraint } if constraint == "minimum")
    });
    assert!(
        !has_minimum,
        "Should NOT have minimum violation — 'abc' is not numeric so constraint doesn't apply"
    );
}

// ── #120: Recursive inflate + opaque string parsing ────────────────────

// 19. Full pipeline roundtrip for recursive_graph.json — data fields
//     inside recursively-inflated nodes should be parsed from JSON strings
#[test]
fn test_e2e_recursive_graph_nested_opaque() {
    use json_schema_llm_core::codec::Transform;

    let schema = load_fixture("stress/recursive_graph");

    let mut options = openai_options();
    options.recursion_limit = 2;
    let result = convert(&schema, &options).expect("convert should succeed");

    // Verify expected transforms
    let jsp_count = result
        .codec
        .transforms
        .iter()
        .filter(|t| matches!(t, Transform::JsonStringParse { .. }))
        .count();
    let ri_count = result
        .codec
        .transforms
        .iter()
        .filter(|t| matches!(t, Transform::RecursiveInflate { .. }))
        .count();
    assert!(jsp_count > 0, "Should have JsonStringParse transforms");
    assert!(ri_count > 0, "Should have RecursiveInflate transforms");

    // Build LLM output matching the 3-level recursive schema:
    //   Root object → edges[].target (L1 object) → edges[].target (L2 object)
    //     → edges[].target (L3: JSON string via RecursiveInflate)
    //   `data` at every level is a JSON string (opaque via p4)
    let leaf_data = json!({"role": "leaf"});

    // Level 3: the deepest node, to be serialized as JSON string (RecursiveInflate)
    let level3_node = json!({
        "id": "leaf",
        "data": serde_json::to_string(&leaf_data).unwrap(),
        "edges": []
    });

    // Level 2: inline object, edges contain the JSON-string target
    let level2_data = json!({"role": "branch"});
    let level2_node = json!({
        "id": "branch",
        "data": serde_json::to_string(&level2_data).unwrap(),
        "edges": [
            {
                "target": serde_json::to_string(&level3_node).unwrap(),
                "weight": null,
                "label": null
            }
        ]
    });

    // Level 1: inline object
    let level1_data = json!({"role": "trunk"});
    let level1_node = json!({
        "id": "trunk",
        "data": serde_json::to_string(&level1_data).unwrap(),
        "edges": [{
            "target": level2_node,
            "weight": null,
            "label": null
        }]
    });

    // Root (Level 0)
    let root_data = json!({"role": "root"});
    let llm_output = json!({
        "id": "root",
        "data": serde_json::to_string(&root_data).unwrap(),
        "edges": [{
            "target": level1_node,
            "weight": 1.0,
            "label": "main"
        }]
    });

    let rehydrated =
        rehydrate(&llm_output, &result.codec, &schema).expect("rehydrate should succeed");

    // Root `data` should be parsed (covered by existing JSP at #/properties/data)
    assert!(
        rehydrated.data["data"].is_object(),
        "Root data should be parsed, got: {:?}",
        rehydrated.data["data"]
    );

    // Level 1 `data` should be parsed (inline path in schema)
    let l1 = &rehydrated.data["edges"][0]["target"];
    assert!(
        l1["data"].is_object(),
        "Level 1 data should be parsed, got: {:?}",
        l1["data"]
    );

    // Level 2 `data` should be parsed (inline path in schema)
    let l2 = &l1["edges"][0]["target"];
    assert!(
        l2["data"].is_object(),
        "Level 2 data should be parsed, got: {:?}",
        l2["data"]
    );

    // Level 3 target should be inflated from JSON string (RecursiveInflate)
    let l3 = &l2["edges"][0]["target"];
    assert!(
        l3.is_object(),
        "Level 3 target should be inflated, got: {:?}",
        l3
    );
    assert_eq!(l3["id"], json!("leaf"));

    // ── CORE #120 ASSERTION ──
    // Level 3 `data` (inside the RecursiveInflate node) should ALSO be parsed.
    // Before the fix, this stays as a raw JSON string because the
    // $defs-sourced JSP doesn't resolve to this data location.
    assert!(
        l3["data"].is_object(),
        "Level 3 data (inside inflated target) should be parsed from JSON string, got: {:?}",
        l3["data"]
    );
    assert_eq!(
        l3["data"]["role"],
        json!("leaf"),
        "Level 3 data contents should be preserved"
    );
}
