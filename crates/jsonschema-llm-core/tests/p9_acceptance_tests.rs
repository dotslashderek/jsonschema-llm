//! Acceptance tests for Pass 9 — Provider compatibility checks.
//!
//! These tests exercise the full pipeline (`convert()`) and assert on
//! `ConvertResult.provider_compat_errors`. They are written BEFORE the
//! implementation (constraint-based TDD) and must fail until p9 is populated.
//!
//! Each test maps to a specific sub-issue of #100:
//!   - #94: Root type enforcement (non-object roots must be wrapped)
//!   - #95: Depth budget exceeded (schema nesting > provider limit)
//!   - #96: Mixed enum types (heterogeneous enum values)
//!   - #97: Unconstrained schemas (boolean/empty sub-schemas)

use jsonschema_llm_core::error::ProviderCompatError;
use jsonschema_llm_core::{convert, ConvertOptions, Mode, Target};
use serde_json::{json, Value};

/// Helper: convert with OpenAI Strict defaults.
fn convert_strict(schema: &Value) -> jsonschema_llm_core::ConvertResult {
    let opts = ConvertOptions {
        target: Target::OpenaiStrict,
        mode: Mode::Strict,
        ..ConvertOptions::default()
    };
    convert(schema, &opts).expect("conversion should not hard-fail")
}

/// Helper: convert with a non-OpenAI target (should skip p9 checks).
fn convert_gemini(schema: &Value) -> jsonschema_llm_core::ConvertResult {
    let opts = ConvertOptions {
        target: Target::Gemini,
        mode: Mode::Strict,
        ..ConvertOptions::default()
    };
    convert(schema, &opts).expect("conversion should not hard-fail")
}

// ═══════════════════════════════════════════════════════════════════════════
// #94 — Root type enforcement
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn p9_root_object_no_error() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        }
    });
    let result = convert_strict(&schema);
    let root_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::RootTypeIncompatible { .. }))
        .collect();
    assert!(root_errors.is_empty(), "object root should not trigger RootTypeIncompatible");
}

#[test]
fn p9_root_array_emits_error_and_wraps() {
    let schema = json!({
        "type": "array",
        "items": { "type": "string" }
    });
    let result = convert_strict(&schema);

    // Should emit an advisory error about root type
    let root_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::RootTypeIncompatible { .. }))
        .collect();
    assert_eq!(root_errors.len(), 1, "array root should trigger exactly 1 RootTypeIncompatible");

    // The output schema should be wrapped in an object
    assert_eq!(
        result.schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "wrapped schema must have type: object at root"
    );
    assert!(
        result.schema.get("properties").and_then(|p| p.get("result")).is_some(),
        "wrapped schema must have properties.result"
    );
}

#[test]
fn p9_root_string_emits_error_and_wraps() {
    let schema = json!({
        "type": "string",
        "minLength": 1
    });
    let result = convert_strict(&schema);

    let root_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::RootTypeIncompatible { .. }))
        .collect();
    assert_eq!(root_errors.len(), 1, "string root should trigger RootTypeIncompatible");
    assert_eq!(
        result.schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "wrapped schema must be type: object"
    );
}

#[test]
fn p9_root_missing_type_emits_error() {
    // No explicit `type` field — p9 wraps in object wrapper
    let schema = json!({
        "properties": {
            "x": { "type": "string" }
        }
    });
    let result = convert_strict(&schema);

    // Should emit RootTypeIncompatible since root has no type: object
    let root_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::RootTypeIncompatible { .. }))
        .collect();
    assert_eq!(
        root_errors.len(),
        1,
        "missing type should trigger exactly 1 RootTypeIncompatible"
    );

    // Final schema must be the wrapper object
    assert_eq!(
        result.schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "wrapper must have type: object"
    );
    assert_eq!(
        result.schema.get("additionalProperties"),
        Some(&json!(false)),
        "wrapper should be sealed"
    );

    // Original schema lives inside properties.result
    assert!(
        result.schema.pointer("/properties/result/properties/x").is_some(),
        "original property 'x' should be inside properties.result"
    );
}

#[test]
fn p9_root_array_roundtrip_rehydration() {
    let schema = json!({
        "type": "array",
        "items": { "type": "string" }
    });
    let result = convert_strict(&schema);

    // Simulate LLM output matching the wrapped schema
    let llm_output = json!({ "result": ["hello", "world"] });
    let rehydrated = jsonschema_llm_core::rehydrate(&llm_output, &result.codec)
        .expect("rehydration should succeed");

    // After rehydration, the wrapper should be removed
    assert!(
        rehydrated.data.is_array(),
        "rehydrated data should be an array, got: {}",
        rehydrated.data
    );
    assert_eq!(rehydrated.data, json!(["hello", "world"]));
}

// ═══════════════════════════════════════════════════════════════════════════
// #95 — Depth budget
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn p9_shallow_schema_no_depth_error() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "address": {
                "type": "object",
                "properties": {
                    "city": { "type": "string" }
                }
            }
        }
    });
    let result = convert_strict(&schema);
    let depth_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::DepthBudgetExceeded { .. }))
        .collect();
    assert!(depth_errors.is_empty(), "2-level schema should not exceed depth budget");
}

#[test]
fn p9_deep_schema_emits_depth_error() {
    // Build a 7-level deep schema (exceeds OpenAI limit of 5)
    let mut inner = json!({ "type": "string" });
    for i in (0..7).rev() {
        inner = json!({
            "type": "object",
            "properties": {
                format!("level_{i}"): inner
            }
        });
    }
    let result = convert_strict(&inner);
    let depth_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::DepthBudgetExceeded { .. }))
        .collect();
    assert_eq!(
        depth_errors.len(),
        1,
        "7-level deep schema should trigger exactly 1 DepthBudgetExceeded"
    );

    // Verify the error contains useful metadata
    if let ProviderCompatError::DepthBudgetExceeded {
        actual_depth,
        max_depth,
        ..
    } = &depth_errors[0]
    {
        assert!(*actual_depth > *max_depth, "actual_depth should exceed max_depth");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// #96 — Enum homogeneity
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn p9_homogeneous_enum_no_error() {
    let schema = json!({
        "type": "object",
        "properties": {
            "color": {
                "enum": ["red", "green", "blue"]
            }
        }
    });
    let result = convert_strict(&schema);
    let enum_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::MixedEnumTypes { .. }))
        .collect();
    assert!(enum_errors.is_empty(), "homogeneous string enum should not trigger MixedEnumTypes");
}

#[test]
fn p9_heterogeneous_enum_emits_error() {
    let schema = json!({
        "type": "object",
        "properties": {
            "mixed": {
                "enum": ["red", 1, true, null, { "x": 1 }]
            }
        }
    });
    let result = convert_strict(&schema);
    let enum_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::MixedEnumTypes { .. }))
        .collect();
    assert_eq!(
        enum_errors.len(),
        1,
        "mixed-type enum should trigger exactly 1 MixedEnumTypes"
    );

    if let ProviderCompatError::MixedEnumTypes { types_found, path, .. } = &enum_errors[0] {
        assert!(types_found.len() > 1, "should report multiple types");
        assert!(
            path.contains("mixed"),
            "path should reference the 'mixed' property, got: {path}"
        );
    }
}

#[test]
fn p9_nested_heterogeneous_enum_reports_path() {
    let schema = json!({
        "type": "object",
        "properties": {
            "outer": {
                "type": "object",
                "properties": {
                    "status": {
                        "enum": ["active", 0, false]
                    }
                }
            }
        }
    });
    let result = convert_strict(&schema);
    let enum_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::MixedEnumTypes { .. }))
        .collect();
    assert_eq!(enum_errors.len(), 1);

    if let ProviderCompatError::MixedEnumTypes { path, .. } = &enum_errors[0] {
        assert!(
            path.contains("outer") && path.contains("status"),
            "path should contain 'outer' and 'status', got: {path}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// #97 — Unconstrained (boolean/empty) schemas
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn p9_typed_schema_no_unconstrained_error() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "integer" }
        }
    });
    let result = convert_strict(&schema);
    let uncon_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. }))
        .collect();
    assert!(uncon_errors.is_empty(), "fully typed schema should not trigger UnconstrainedSchema");
}

#[test]
fn p9_nested_empty_schema_no_error_pipeline_handles() {
    // Empty `{}` sub-schemas are converted to `{"type": "string"}` with a
    // `json_string_parse` codec transform by earlier pipeline passes (p4/p5).
    // By the time p9 runs, the schema is well-typed — p9 correctly sees no issue.
    // This is defense-in-depth: earlier passes fix, p9 catches stragglers.
    let schema = json!({
        "type": "object",
        "properties": {
            "anything": {}
        }
    });
    let result = convert_strict(&schema);
    let uncon_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. }))
        .collect();
    assert!(
        uncon_errors.is_empty(),
        "empty sub-schema is handled by earlier passes; p9 should not flag it"
    );
}

#[test]
fn p9_nested_true_schema_emits_error() {
    // Note: after p0_normalize, boolean schemas at property level
    // become empty objects. The p9 check runs on the post-normalize schema.
    let schema = json!({
        "type": "object",
        "properties": {
            "wildcard": true
        }
    });
    let result = convert_strict(&schema);
    let uncon_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. }))
        .collect();
    assert!(
        !uncon_errors.is_empty(),
        "nested `true` schema should trigger UnconstrainedSchema"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Gate tests — non-OpenAI / Permissive should skip all checks
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn p9_gemini_target_skips_all_checks() {
    let schema = json!({
        "type": "array",
        "items": {
            "enum": ["a", 1, true]
        }
    });
    let result = convert_gemini(&schema);
    assert!(
        result.provider_compat_errors.is_empty(),
        "Gemini target should produce zero provider_compat_errors"
    );
}

#[test]
fn p9_permissive_mode_skips_all_checks() {
    let schema = json!({
        "type": "array",
        "items": { "type": "string" }
    });
    let opts = ConvertOptions {
        target: Target::OpenaiStrict,
        mode: Mode::Permissive,
        ..ConvertOptions::default()
    };
    let result = convert(&schema, &opts).expect("conversion should succeed");
    assert!(
        result.provider_compat_errors.is_empty(),
        "Permissive mode should produce zero provider_compat_errors"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Integration — multiple issues in one schema
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn p9_multiple_issues_all_reported() {
    // This schema has:
    //   - non-object root (array) → #94
    //   - heterogeneous enum inside items → #96
    let schema = json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "status": { "enum": ["active", 1] }
            }
        }
    });
    let result = convert_strict(&schema);

    let has_root = result
        .provider_compat_errors
        .iter()
        .any(|e| matches!(e, ProviderCompatError::RootTypeIncompatible { .. }));
    let has_enum = result
        .provider_compat_errors
        .iter()
        .any(|e| matches!(e, ProviderCompatError::MixedEnumTypes { .. }));

    assert!(has_root, "should report RootTypeIncompatible");
    assert!(has_enum, "should report MixedEnumTypes");
}
