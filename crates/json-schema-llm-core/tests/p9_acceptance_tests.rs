//! Acceptance tests for Pass 9 — Provider compatibility checks.
//!
//! These tests exercise the full pipeline (`convert()`) and assert on
//! `ConvertResult.provider_compat_errors`. They were originally written
//! before the implementation (constraint-based TDD) and now serve as
//! regression tests for the populated p9 pass.
//!
//! Each test maps to a specific sub-issue of #100:
//!   - #94: Root type enforcement (non-object roots must be wrapped)
//!   - #95: Depth budget exceeded (schema nesting > provider limit)
//!   - #96: Mixed enum types (heterogeneous enum values)
//!   - #97: Unconstrained schemas (boolean/empty sub-schemas)

use json_schema_llm_core::error::ProviderCompatError;
use json_schema_llm_core::{convert, ConvertOptions, Mode, Target};
use serde_json::{json, Value};

/// Helper: convert with OpenAI Strict defaults.
fn convert_strict(schema: &Value) -> json_schema_llm_core::ConvertResult {
    let mut opts = ConvertOptions::default();
    opts.target = Target::OpenaiStrict;
    opts.mode = Mode::Strict;
    convert(schema, &opts).expect("conversion should not hard-fail")
}

/// Helper: convert with a non-OpenAI target (should skip p9 checks).
fn convert_gemini(schema: &Value) -> json_schema_llm_core::ConvertResult {
    let mut opts = ConvertOptions::default();
    opts.target = Target::Gemini;
    opts.mode = Mode::Strict;
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
    assert!(
        root_errors.is_empty(),
        "object root should not trigger RootTypeIncompatible"
    );
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
    assert_eq!(
        root_errors.len(),
        1,
        "array root should trigger exactly 1 RootTypeIncompatible"
    );

    // The output schema should be wrapped in an object
    assert_eq!(
        result.schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "wrapped schema must have type: object at root"
    );
    assert!(
        result
            .schema
            .get("properties")
            .and_then(|p| p.get("result"))
            .is_some(),
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
    assert_eq!(
        root_errors.len(),
        1,
        "string root should trigger RootTypeIncompatible"
    );
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
        result
            .schema
            .pointer("/properties/result/properties/x")
            .is_some(),
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
    let rehydrated = json_schema_llm_core::rehydrate(&llm_output, &result.codec, &schema)
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
    assert!(
        depth_errors.is_empty(),
        "2-level schema should not exceed depth budget"
    );
}

#[test]
fn p9_deep_schema_emits_depth_error() {
    // Build a 12-level deep schema (exceeds OpenAI limit of 10).
    // The innermost node must be an object (not a primitive) because
    // primitive leaves are correctly skipped by the depth truncation guard.
    let mut inner = json!({
        "type": "object",
        "properties": {
            "value": { "type": "string" }
        }
    });
    for i in (0..12).rev() {
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
        "12-level deep schema should trigger exactly 1 DepthBudgetExceeded"
    );

    // Verify the error contains useful metadata
    if let ProviderCompatError::DepthBudgetExceeded {
        actual_depth,
        max_depth,
        ..
    } = &depth_errors[0]
    {
        assert!(
            *actual_depth >= *max_depth,
            "actual_depth should be at or above max_depth"
        );
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
    assert!(
        enum_errors.is_empty(),
        "homogeneous string enum should not trigger MixedEnumTypes"
    );
}

#[test]
fn p9_heterogeneous_enum_with_objects_handled_by_p8() {
    // Enums containing objects or null are intercepted by p8 (adaptive opaque)
    // and converted to opaque strings before p9 runs. P9 should NOT see a
    // MixedEnumTypes error for these — they're already stringified.
    //
    // Note: The `mixed` property is non-required, so p6 (strict mode) wraps
    // it in `anyOf: [{...}, {type: "null"}]` and promotes it to required.
    let schema = json!({
        "type": "object",
        "properties": {
            "mixed": {
                "enum": ["red", 1, true, null, { "x": 1 }]
            }
        }
    });
    let result = convert_strict(&schema);

    // P8 stringifies the enum, then p6 wraps it in anyOf for nullable.
    // The string variant is inside anyOf[0].
    let mixed_anyof = result
        .schema
        .pointer("/properties/mixed/anyOf")
        .and_then(|v| v.as_array())
        .expect("mixed should be wrapped in anyOf by p6 strict mode");

    let has_string_variant = mixed_anyof
        .iter()
        .any(|v| v.get("type").and_then(|t| t.as_str()) == Some("string"));
    assert!(
        has_string_variant,
        "p8 should convert enum with objects/null to opaque string"
    );

    // P9 should NOT report MixedEnumTypes (p8 already handled it)
    let enum_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::MixedEnumTypes { .. }))
        .collect();
    assert!(
        enum_errors.is_empty(),
        "p8 should handle enums with objects/null before p9 runs"
    );
}

#[test]
fn p9_heterogeneous_enum_without_objects_still_reported() {
    // Enums with mixed primitive types (no objects or null) are NOT caught
    // by p8 — they pass through to p9, which reports MixedEnumTypes.
    let schema = json!({
        "type": "object",
        "properties": {
            "mixed": {
                "enum": ["red", 1, true]
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
        "mixed primitive-type enum should trigger exactly 1 MixedEnumTypes"
    );
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
    assert!(
        uncon_errors.is_empty(),
        "fully typed schema should not trigger UnconstrainedSchema"
    );
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
    let mut opts = ConvertOptions::default();
    opts.target = Target::OpenaiStrict;
    opts.mode = Mode::Permissive;
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

// ═══════════════════════════════════════════════════════════════════════════
// #109 — Keyword recursion coverage
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn p9_mixed_enum_inside_anyof_detected() {
    // A mixed-type enum nested inside an anyOf variant must be found by CompatVisitor.
    let schema = json!({
        "type": "object",
        "properties": {
            "val": {
                "anyOf": [
                    { "type": "object", "properties": { "code": { "enum": ["a", 1] } } },
                    { "type": "null" }
                ]
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
        "mixed-type enum inside anyOf variant should trigger MixedEnumTypes"
    );
    if let ProviderCompatError::MixedEnumTypes { path, .. } = &enum_errors[0] {
        assert!(
            path.contains("code"),
            "path should reference the 'code' property, got: {path}"
        );
    }
}

#[test]
fn p9_unconstrained_inside_oneof_detected() {
    // An empty sub-schema inside a oneOf variant should be caught.
    // Note: earlier passes may fill in empty schemas, so we use `true` (boolean schema)
    // which p0 normalizes to `{}` and then p9 catches as unconstrained.
    let schema = json!({
        "type": "object",
        "properties": {
            "data": {
                "oneOf": [
                    { "type": "string" },
                    true
                ]
            }
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
        "boolean `true` schema inside oneOf should trigger UnconstrainedSchema"
    );
}

#[test]
fn p9_pattern_properties_stripped_before_enum_detection() {
    // #246: patternProperties is stripped/stringified before the visitor recurses
    // into children. At root with type:object (no explicit properties),
    // patternProperties is stripped.
    // The mixed enum inside patternProperties is never visited.
    let schema = json!({
        "type": "object",
        "patternProperties": {
            "^x-": {
                "type": "object",
                "properties": {
                    "code": { "enum": ["a", 1] }
                }
            }
        }
    });
    let result = convert_strict(&schema);
    // patternProperties should be stripped
    let pp_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| {
            matches!(
                e,
                ProviderCompatError::PatternPropertiesStripped { .. }
                    | ProviderCompatError::PatternPropertiesStringified { .. }
            )
        })
        .collect();
    assert!(
        !pp_errors.is_empty(),
        "patternProperties should trigger a PatternProperties error"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// #111 — Depth budget fix (combinator/nullable wrappers)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn p9_nullable_wrapper_does_not_inflate_depth() {
    // Build a 4-level deep schema where each level is wrapped in anyOf for
    // nullable support: anyOf: [{type: "object", properties: {...}}, {type: "null"}]
    // With root properties, semantic depth is: root props(1) → level_3(2) → level_2(3) → level_1(4) → level_0(5) → leaf = 5 property accesses.
    // Wait, let's count carefully with the loop:
    //   loop builds inner → level_0 → level_1 → level_2 → level_3 (4 iterations)
    //   Final wrap: {type:object, properties: {root: <4-nested>}}
    // Semantic depth: root.properties.root (1) → anyOf→properties.level_3 (2) → anyOf→properties.level_2 (3) → anyOf→properties.level_1 (4) → anyOf→properties.level_0 (5)
    // That's 5 (exactly at limit), not over. Should NOT trigger DepthBudgetExceeded.
    let mut inner: Value = json!({ "type": "string" });
    for i in (0..4).rev() {
        inner = json!({
            "anyOf": [
                {
                    "type": "object",
                    "properties": { format!("level_{i}"): inner }
                },
                { "type": "null" }
            ]
        });
    }
    // Wrap in a root object
    let schema = json!({
        "type": "object",
        "properties": { "root": inner }
    });
    let result = convert_strict(&schema);
    let depth_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::DepthBudgetExceeded { .. }))
        .collect();
    assert!(
        depth_errors.is_empty(),
        "4-level schema with nullable wrappers + root = 5 semantic depth, \
         should NOT exceed depth budget (max=5), but got {} depth errors",
        depth_errors.len()
    );
}

#[test]
fn p9_deep_combinator_chain_no_depth_inflation() {
    // Schema structure: root → allOf[0] → oneOf[0] → properties.a → properties.b
    // Combinators (allOf, oneOf) should NOT increment semantic depth.
    // Semantic depth: root(0) → a(1) → b(2) → leaf(3) = 3 levels.
    // Should be well under the 5-level OpenAI limit.
    let schema = json!({
        "type": "object",
        "allOf": [{
            "oneOf": [{
                "type": "object",
                "properties": {
                    "a": {
                        "type": "object",
                        "properties": {
                            "b": {
                                "type": "object",
                                "properties": {
                                    "leaf": { "type": "string" }
                                }
                            }
                        }
                    }
                }
            }]
        }]
    });
    let result = convert_strict(&schema);
    let depth_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::DepthBudgetExceeded { .. }))
        .collect();
    assert!(
        depth_errors.is_empty(),
        "allOf→oneOf→properties chain should not inflate depth beyond 3, \
         but got DepthBudgetExceeded"
    );
}

#[test]
fn p9_deep_combinator_stack_safety() {
    // 40 nested anyOf levels — safely under earlier passes' 50-level recursion limit.
    // Must not panic/crash/stack overflow.
    // Semantic depth stays low (no data-shape edges in anyOf), so no DepthBudgetExceeded.
    let mut inner: Value = json!({ "type": "string" });
    for _ in 0..40 {
        inner = json!({ "anyOf": [inner] });
    }
    let schema = json!({
        "type": "object",
        "properties": { "deep": inner }
    });
    // Should not panic
    let result = convert_strict(&schema);
    let depth_errors: Vec<_> = result
        .provider_compat_errors
        .iter()
        .filter(|e| matches!(e, ProviderCompatError::DepthBudgetExceeded { .. }))
        .collect();
    assert!(
        depth_errors.is_empty(),
        "40 nested anyOfs with no data edges should not report DepthBudgetExceeded"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// #110 — Inner schema strict enforcement after p9 wrapping
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn p9_wrapped_inner_schema_has_strict_enforcement() {
    // Root with `properties` but NO `type: object`.
    // After p9 wraps, the inner schema at properties.result must have
    // additionalProperties: false and required: ["x"].
    let schema = json!({
        "properties": {
            "x": { "type": "string" }
        }
    });
    let result = convert_strict(&schema);

    // Wrapper itself should be sealed
    assert_eq!(
        result.schema.get("additionalProperties"),
        Some(&json!(false)),
        "wrapper must be sealed"
    );

    // Inner schema at properties.result must also be sealed
    let inner = result
        .schema
        .pointer("/properties/result")
        .expect("inner schema must exist at properties.result");
    assert_eq!(
        inner.get("additionalProperties"),
        Some(&json!(false)),
        "inner schema must have additionalProperties: false"
    );

    // All properties must be required
    let required = inner
        .get("required")
        .and_then(|v| v.as_array())
        .expect("inner schema must have required array");
    assert!(
        required.contains(&json!("x")),
        "inner schema must require 'x', got: {required:?}"
    );
}

#[test]
fn p9_wrapped_inner_schema_optional_props_nullable() {
    // Root with required + optional props, no `type: object`.
    // After wrapping, optional prop `o` should be wrapped in anyOf nullable.
    let schema = json!({
        "properties": {
            "r": { "type": "string" },
            "o": { "type": "integer" }
        },
        "required": ["r"]
    });
    let result = convert_strict(&schema);

    let inner = result
        .schema
        .pointer("/properties/result")
        .expect("inner schema must exist at properties.result");

    // Both properties must be required (strict mode)
    let required = inner
        .get("required")
        .and_then(|v| v.as_array())
        .expect("inner schema must have required array");
    assert!(
        required.contains(&json!("r")),
        "inner schema must require 'r'"
    );
    assert!(
        required.contains(&json!("o")),
        "inner schema must require 'o' (promoted to required with nullable)"
    );

    // 'o' must be wrapped in anyOf nullable since it was originally optional
    let o_prop = inner
        .pointer("/properties/o")
        .expect("inner schema must have property 'o'");
    let any_of = o_prop
        .get("anyOf")
        .and_then(|v| v.as_array())
        .expect("optional 'o' must be wrapped in anyOf");
    let has_null = any_of.iter().any(|v| v.get("type") == Some(&json!("null")));
    assert!(
        has_null,
        "anyOf must include a null type branch, got: {any_of:?}"
    );
}

#[test]
fn p9_wrapped_array_root_no_double_enforcement() {
    // Array root — p9 wraps it, but the inner schema is an array, not an object.
    // Strict enforcement should not apply to non-object inner schemas.
    let schema = json!({
        "type": "array",
        "items": { "type": "string" }
    });
    let result = convert_strict(&schema);

    // Should wrap successfully
    assert_eq!(
        result.schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "wrapper must be type: object"
    );

    // Inner schema should be the array, no crash / no additionalProperties on array
    let inner = result
        .schema
        .pointer("/properties/result")
        .expect("inner schema must exist");
    // Array shouldn't have additionalProperties — it's not meaningful
    // Just verify we didn't panic and the inner schema is intact
    assert_eq!(
        inner.get("type").and_then(|v| v.as_str()),
        Some("array"),
        "inner schema should remain an array"
    );
}

#[test]
fn p9_wrapped_root_with_type_object_but_combinator() {
    // Root with type: object AND a combinator — p9 wraps it because
    // OpenAI strict mode forbids root-level combinators. p6 already ran
    // strict enforcement on the original schema, so the inner schema at
    // properties.result should already be strict.
    let schema = json!({
        "type": "object",
        "properties": {
            "a": { "type": "string" }
        },
        "anyOf": [
            {
                "properties": {
                    "b": { "type": "integer" }
                }
            }
        ]
    });
    let result = convert_strict(&schema);

    // Should be wrapped because of the root combinator
    assert_eq!(
        result.schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "wrapper must be type: object"
    );
    assert!(
        result.schema.pointer("/properties/result").is_some(),
        "schema with root combinator must be wrapped in properties.result"
    );
    assert_eq!(
        result.schema.get("additionalProperties"),
        Some(&json!(false)),
        "wrapper must have additionalProperties: false"
    );

    // Inner schema should have strict enforcement from p6
    let inner = result
        .schema
        .pointer("/properties/result")
        .expect("inner schema must exist");
    assert_eq!(
        inner.get("additionalProperties"),
        Some(&json!(false)),
        "inner schema should have additionalProperties: false from p6"
    );
}

#[test]
fn p9_inner_schema_with_partial_strict_still_enforced() {
    // Edge case: inner schema has additionalProperties: false (user-provided)
    // but incomplete required — enforce_object_strict must still complete it.
    let schema = json!({
        "properties": {
            "a": { "type": "string" },
            "b": { "type": "integer" }
        },
        "required": ["a"],
        "additionalProperties": false
    });
    let result = convert_strict(&schema);

    // Must be wrapped (no type: object)
    let inner = result
        .schema
        .pointer("/properties/result")
        .and_then(|v| v.as_object())
        .expect("inner schema must exist as object");

    // All properties must be required
    let req: Vec<String> = inner
        .get("required")
        .and_then(|v| v.as_array())
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    assert!(
        req.contains(&"a".to_string()),
        "inner schema must require 'a'"
    );
    assert!(
        req.contains(&"b".to_string()),
        "inner schema must require 'b' (promoted to required)"
    );

    // "b" was optional, so must be nullable-wrapped
    let b_prop = inner
        .get("properties")
        .and_then(|v| v.get("b"))
        .expect("inner schema must have property 'b'");
    let any_of = b_prop
        .get("anyOf")
        .and_then(|v| v.as_array())
        .expect("optional 'b' must be wrapped in anyOf");
    let has_null = any_of.iter().any(|v| v.get("type") == Some(&json!("null")));
    assert!(
        has_null,
        "anyOf must include a null type branch, got: {any_of:?}"
    );
}
