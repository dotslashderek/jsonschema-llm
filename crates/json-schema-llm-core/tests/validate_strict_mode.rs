//! Integration tests for `validate_strict_mode`.
//!
//! Tests per-rule correctness, clean-schema baseline, and fixture compliance.

use json_schema_llm_core::{validate_strict_mode, StrictModeRule, StrictModeViolation};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

// ── Helpers ──────────────────────────────────────────────────────────────

fn assert_has_violation(violations: &[StrictModeViolation], rule: &StrictModeRule) {
    assert!(
        violations.iter().any(|v| &v.rule_id == rule),
        "Expected violation {:?} but found: {:?}",
        rule,
        violations
    );
}

fn assert_no_violations(violations: &[StrictModeViolation]) {
    assert_eq!(
        violations,
        &[] as &[StrictModeViolation],
        "Expected no violations but found: {:?}",
        violations
    );
}

// ── Unit tests: one per rule ─────────────────────────────────────────────

#[test]
fn test_clean_schema_passes() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "integer" }
        },
        "required": ["name", "age"],
        "additionalProperties": false
    });
    assert_no_violations(&validate_strict_mode(&schema));
}

#[test]
fn test_root_not_object() {
    let schema = json!({
        "type": "string"
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::RootNotObject);
}

#[test]
fn test_missing_type() {
    // A sub-schema without type inside a valid root
    let schema = json!({
        "type": "object",
        "properties": {
            "data": {
                "properties": {
                    "name": { "type": "string" }
                },
                "additionalProperties": false
            }
        },
        "required": ["data"],
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::MissingType);
}

#[test]
fn test_missing_additional_properties() {
    let schema = json!({
        "type": "object",
        "properties": {
            "nested": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                }
            }
        },
        "required": ["nested"],
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::MissingAdditionalProperties);
}

#[test]
fn test_missing_items() {
    let schema = json!({
        "type": "object",
        "properties": {
            "tags": {
                "type": "array"
            }
        },
        "required": ["tags"],
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::MissingItems);
}

#[test]
fn test_banned_pattern_properties() {
    let schema = json!({
        "type": "object",
        "properties": { "name": { "type": "string" } },
        "patternProperties": { "^x-": { "type": "string" } },
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::BannedPatternProperties);
}

#[test]
fn test_banned_schema_ref() {
    let schema = json!({
        "type": "object",
        "properties": {
            "nested": { "$ref": "#/$defs/Foo" }
        },
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::BannedSchemaRef);
}

#[test]
fn test_banned_anchor() {
    let schema = json!({
        "type": "object",
        "$anchor": "myAnchor",
        "properties": {},
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::BannedAnchor);
}

#[test]
fn test_banned_dynamic_ref() {
    let schema = json!({
        "type": "object",
        "properties": {
            "x": { "$dynamicRef": "#foo" }
        },
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::BannedDynamicRef);
}

#[test]
fn test_banned_dynamic_anchor() {
    let schema = json!({
        "type": "object",
        "$dynamicAnchor": "foo",
        "properties": {},
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::BannedDynamicAnchor);
}

#[test]
fn test_banned_dependent_schemas() {
    let schema = json!({
        "type": "object",
        "properties": { "a": { "type": "string" } },
        "dependentSchemas": {
            "a": { "properties": { "b": { "type": "string" } } }
        },
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::BannedDependentSchemas);
}

#[test]
fn test_banned_dependent_required() {
    let schema = json!({
        "type": "object",
        "properties": { "a": { "type": "string" } },
        "dependentRequired": {
            "a": ["b"]
        },
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::BannedDependentRequired);
}

#[test]
fn test_banned_unevaluated_properties() {
    let schema = json!({
        "type": "object",
        "properties": { "a": { "type": "string" } },
        "unevaluatedProperties": false,
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::BannedUnevaluatedProperties);
}

#[test]
fn test_banned_unevaluated_items() {
    let schema = json!({
        "type": "object",
        "properties": {
            "data": {
                "type": "array",
                "items": { "type": "string" },
                "unevaluatedItems": false
            }
        },
        "required": ["data"],
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::BannedUnevaluatedItems);
}

#[test]
fn test_banned_contains() {
    let schema = json!({
        "type": "object",
        "properties": {
            "data": {
                "type": "array",
                "items": { "type": "string" },
                "contains": { "type": "string", "const": "required" }
            }
        },
        "required": ["data"],
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::BannedContains);
}

#[test]
fn test_banned_if() {
    let schema = json!({
        "type": "object",
        "properties": { "a": { "type": "string" } },
        "if": { "properties": { "a": { "const": "yes" } } },
        "then": { "properties": { "b": { "type": "string" } } },
        "else": { "properties": { "c": { "type": "string" } } },
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::BannedIf);
    assert_has_violation(&v, &StrictModeRule::BannedThen);
    assert_has_violation(&v, &StrictModeRule::BannedElse);
}

#[test]
fn test_banned_not() {
    let schema = json!({
        "type": "object",
        "properties": { "a": { "type": "string" } },
        "not": { "type": "null" },
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::BannedNot);
}

#[test]
fn test_depth_exceeded() {
    // Build a schema with 7 levels of nesting (exceeds limit 5)
    let mut schema = json!({ "type": "string" });
    for _ in 0..7 {
        schema = json!({
            "type": "object",
            "properties": { "nested": schema },
            "required": ["nested"],
            "additionalProperties": false
        });
    }
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::DepthExceeded);
}

#[test]
fn test_depth_exactly_at_limit() {
    // Build a schema at exactly depth 5 — should be flagged (>= 5)
    // Use an object leaf (not primitive) to trigger the depth check
    let mut schema = json!({
        "type": "object",
        "properties": { "x": { "type": "string" } },
        "additionalProperties": false
    });
    for _ in 0..5 {
        schema = json!({
            "type": "object",
            "properties": { "nested": schema },
            "required": ["nested"],
            "additionalProperties": false
        });
    }
    let v = validate_strict_mode(&schema);
    assert_has_violation(&v, &StrictModeRule::DepthExceeded);
}

#[test]
fn test_ref_as_property_name_not_flagged() {
    // $ref as a property NAME is legitimate — the walker visits the
    // property's value (a schema), not the property name itself.
    let schema = json!({
        "type": "object",
        "properties": {
            "$ref": { "type": "string" }
        },
        "required": ["$ref"],
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    // Should have no BannedSchemaRef (the "$ref" key is a prop name, not a schema keyword)
    assert!(
        !v.iter()
            .any(|viol| viol.rule_id == StrictModeRule::BannedSchemaRef),
        "$ref as property name should not be flagged, but got: {:?}",
        v
    );
}

#[test]
fn test_combinator_wrappers_exempt_from_type() {
    // Bare anyOf without `type` should NOT be flagged for MissingType
    let schema = json!({
        "type": "object",
        "properties": {
            "value": {
                "anyOf": [
                    { "type": "string" },
                    { "type": "null" }
                ]
            }
        },
        "required": ["value"],
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert!(
        !v.iter()
            .any(|viol| viol.rule_id == StrictModeRule::MissingType),
        "Bare anyOf wrapper should be exempt from MissingType, but got: {:?}",
        v
    );
}

#[test]
fn test_enum_exempt_from_missing_type() {
    // Schemas with enum but no explicit type should not trigger MissingType
    let schema = json!({
        "type": "object",
        "properties": {
            "status": {
                "enum": ["active", "inactive"]
            }
        },
        "required": ["status"],
        "additionalProperties": false
    });
    let v = validate_strict_mode(&schema);
    assert!(
        !v.iter()
            .any(|viol| viol.rule_id == StrictModeRule::MissingType),
        "Enum schema should be exempt from MissingType, but got: {:?}",
        v
    );
}

// ── Fixture integration tests ────────────────────────────────────────────

const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures");

fn load_fixture(rel_path: &str) -> Value {
    let path = Path::new(FIXTURES_DIR).join(rel_path);
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {rel_path}: {e}"));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse fixture {rel_path}: {e}"))
}

fn collect_fixture_schemas(base_dir: &str) -> Vec<(String, Value)> {
    let base = Path::new(FIXTURES_DIR).join(base_dir);
    let mut schemas = Vec::new();

    // Root schema
    let root_path = base.join("schema.json");
    if root_path.exists() {
        let label = format!("{}/schema.json", base_dir);
        schemas.push((label.clone(), load_fixture(&label)));
    }

    // Component schemas in $defs/*/schema.json
    let defs_dir = base.join("$defs");
    if defs_dir.exists() {
        let mut entries: Vec<_> = fs::read_dir(&defs_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let schema_file = entry.path().join("schema.json");
            if schema_file.exists() {
                let name = entry.file_name().to_string_lossy().to_string();
                let rel = format!("{}/$defs/{}/schema.json", base_dir, name);
                schemas.push((rel.clone(), load_fixture(&rel)));
            }
        }
    }

    schemas
}

#[test]
fn test_arazzo_fixtures_pass() {
    let fixtures = collect_fixture_schemas("arazzo/openai-strict");
    assert!(!fixtures.is_empty(), "Should find arazzo fixtures");
    for (label, schema) in &fixtures {
        let violations = validate_strict_mode(schema);
        assert_eq!(
            violations,
            vec![],
            "Arazzo fixture '{}' should have zero violations but found: {:#?}",
            label,
            violations
        );
    }
}

#[test]
fn test_oas31_fixtures_pass() {
    let fixtures = collect_fixture_schemas("oas31/openai-strict");
    assert!(!fixtures.is_empty(), "Should find oas31 fixtures");
    for (label, schema) in &fixtures {
        let violations = validate_strict_mode(schema);
        assert_eq!(
            violations,
            vec![],
            "OAS31 fixture '{}' should have zero violations but found: {:#?}",
            label,
            violations
        );
    }
}
