//! Pass 7: Constraint Pruning & Enum Sorting
//!
//! Removes constraints that the target provider doesn't support, normalizes
//! `const` → `enum`, and sorts enum arrays to place the default value first.
//!
//! Emits `DroppedConstraint` codec entries for every pruned keyword.

use serde_json::{json, Map, Value};

use crate::codec::DroppedConstraint;
use crate::config::{ConvertOptions, Target};
use crate::error::ConvertError;
use crate::schema_utils::recurse_into_children;

/// Result of running the constraint pruning pass.
#[derive(Debug)]
pub struct ConstraintPassResult {
    /// The transformed schema with unsupported constraints removed.
    pub schema: Value,
    /// Constraints that were dropped during this pass.
    pub dropped_constraints: Vec<DroppedConstraint>,
}

/// Prune unsupported constraints from a schema based on the target provider.
///
/// Recursively walks every node and:
/// 1. Normalizes `const` → `enum: [value]` (except Gemini, which supports `const`)
/// 2. Sorts `enum` to place `default` value first (before `default` is dropped)
/// 3. Drops unsupported constraints per target, emitting `DroppedConstraint` entries
pub fn prune_constraints(
    schema: &Value,
    config: &ConvertOptions,
) -> Result<ConstraintPassResult, ConvertError> {
    let mut dropped = Vec::new();
    let result = walk(schema, "#", 0, config, &mut dropped)?;
    Ok(ConstraintPassResult {
        schema: result,
        dropped_constraints: dropped,
    })
}

// ---------------------------------------------------------------------------
// Recursive walker
// ---------------------------------------------------------------------------

/// Recursively descend through the schema tree, pruning constraints at each
/// node and collecting dropped constraint records.
fn walk(
    node: &Value,
    path: &str,
    depth: usize,
    config: &ConvertOptions,
    dropped: &mut Vec<DroppedConstraint>,
) -> Result<Value, ConvertError> {
    if depth > config.max_depth {
        return Err(ConvertError::RecursionDepthExceeded {
            path: path.to_string(),
            max_depth: config.max_depth,
        });
    }

    let obj = match node.as_object() {
        Some(o) => o,
        None => return Ok(node.clone()),
    };

    let mut result = obj.clone();

    // 1. const → enum normalization (before sorting or pruning)
    normalize_const_to_enum(&mut result, config.target);

    // 2. Enum default-first sorting (before default is dropped)
    sort_enum_default_first(&mut result);

    // 3. Prune unsupported constraints
    prune_node_constraints(&mut result, path, config.target, dropped);

    // Recurse into all structural children
    recurse_into_children(&mut result, path, depth, &mut |val, child_path, d| {
        walk(val, child_path, d, config, dropped)
    })?;

    Ok(Value::Object(result))
}

// ---------------------------------------------------------------------------
// Step 1: const → enum normalization
// ---------------------------------------------------------------------------

/// Normalize `const` to `enum: [value]` for providers that don't support `const`.
///
/// Gemini supports `const` natively — skip normalization for that target.
/// If both `const` and `enum` exist, `const` wins (intersection semantics).
fn normalize_const_to_enum(obj: &mut Map<String, Value>, target: Target) {
    if target == Target::Gemini {
        return;
    }

    if let Some(const_val) = obj.remove("const") {
        obj.insert("enum".to_string(), json!([const_val]));
    }
}

// ---------------------------------------------------------------------------
// Step 2: Enum default-first sorting
// ---------------------------------------------------------------------------

/// If both `default` and `enum` exist, move the default value to `enum[0]`.
///
/// Uses `serde_json::Value::PartialEq` for deep comparison (handles complex
/// enum values like objects/arrays). If default is not found in enum, skips
/// silently (no panic, no duplicate).
fn sort_enum_default_first(obj: &mut Map<String, Value>) {
    let default_val = match obj.get("default") {
        Some(v) => v.clone(),
        None => return,
    };

    let enum_arr = match obj.get_mut("enum").and_then(Value::as_array_mut) {
        Some(arr) => arr,
        None => return,
    };

    if let Some(pos) = enum_arr.iter().position(|v| v == &default_val) {
        let val = enum_arr.remove(pos);
        enum_arr.insert(0, val);
    }
}

// ---------------------------------------------------------------------------
// Step 3: Constraint pruning
// ---------------------------------------------------------------------------

/// Remove unsupported constraints from a single node, emitting `DroppedConstraint`
/// codec entries for each one.
fn prune_node_constraints(
    obj: &mut Map<String, Value>,
    path: &str,
    target: Target,
    dropped: &mut Vec<DroppedConstraint>,
) {
    for keyword in unsupported_constraints(target) {
        if let Some(value) = obj.remove(*keyword) {
            dropped.push(DroppedConstraint {
                path: path.to_string(),
                constraint: keyword.to_string(),
                value,
            });
        }
    }
}

/// Return the list of constraint keywords to prune for a given target.
///
/// Data-driven approach — each target has an explicit list of keywords to remove.
/// This avoids nested conditionals and makes the provider matrix easy to audit.
fn unsupported_constraints(target: Target) -> &'static [&'static str] {
    // Keywords unsupported by ALL providers
    const UNIVERSAL_DROP: &[&str] = &[
        "uniqueItems",
        "default",
        "not",
        "if",
        "then",
        "else",
        // Extended keywords (Gemini review feedback)
        "multipleOf",
        "minProperties",
        "maxProperties",
        "propertyNames",
        "dependencies",
        "dependentRequired",
        "dependentSchemas",
        "contains",
        "minContains",
        "maxContains",
        "format",
    ];

    // OpenAI: drops validation constraints that aren't enum/pattern
    const OPENAI_DROP: &[&str] = &[
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        // Universal drops
        "uniqueItems",
        "default",
        "not",
        "if",
        "then",
        "else",
        "multipleOf",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "minProperties",
        "maxProperties",
        "propertyNames",
        "dependencies",
        "dependentRequired",
        "dependentSchemas",
        "contains",
        "minContains",
        "maxContains",
        "format",
    ];

    // Claude: drops most constraints including pattern
    const CLAUDE_DROP: &[&str] = &[
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        "pattern",
        // Universal drops
        "uniqueItems",
        "default",
        "not",
        "if",
        "then",
        "else",
        "multipleOf",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "minProperties",
        "maxProperties",
        "propertyNames",
        "dependencies",
        "dependentRequired",
        "dependentSchemas",
        "contains",
        "minContains",
        "maxContains",
        "format",
    ];

    match target {
        Target::OpenaiStrict => OPENAI_DROP,
        Target::Gemini => UNIVERSAL_DROP,
        Target::Claude => CLAUDE_DROP,
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use crate::config::{ConvertOptions, Target};

    fn run(schema: Value, target: Target) -> (Value, Vec<DroppedConstraint>) {
        let config = ConvertOptions {
            target,
            ..ConvertOptions::default()
        };
        let result = prune_constraints(&schema, &config).unwrap();
        (result.schema, result.dropped_constraints)
    }

    fn run_openai(schema: Value) -> (Value, Vec<DroppedConstraint>) {
        run(schema, Target::OpenaiStrict)
    }

    // -----------------------------------------------------------------------
    // Test 1: Drop minimum/maximum for OpenAI, preserve for Gemini
    // -----------------------------------------------------------------------
    #[test]
    fn test_drop_minmax_openai_preserve_gemini() {
        let input = json!({
            "type": "integer",
            "minimum": 0,
            "maximum": 100
        });

        // OpenAI: both dropped
        let (openai_out, openai_dropped) = run(input.clone(), Target::OpenaiStrict);
        assert!(
            openai_out.get("minimum").is_none(),
            "minimum should be dropped for OpenAI"
        );
        assert!(
            openai_out.get("maximum").is_none(),
            "maximum should be dropped for OpenAI"
        );
        assert_eq!(openai_dropped.len(), 2);

        // Gemini: both preserved
        let (gemini_out, gemini_dropped) = run(input, Target::Gemini);
        assert_eq!(
            gemini_out["minimum"],
            json!(0),
            "minimum should be preserved for Gemini"
        );
        assert_eq!(
            gemini_out["maximum"],
            json!(100),
            "maximum should be preserved for Gemini"
        );
        assert_eq!(gemini_dropped.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 2: const → enum normalization
    // -----------------------------------------------------------------------
    #[test]
    fn test_const_to_enum_normalization() {
        let input = json!({
            "type": "string",
            "const": "active"
        });

        // OpenAI: const → enum: ["active"], const removed
        let (openai_out, _) = run(input.clone(), Target::OpenaiStrict);
        assert_eq!(openai_out["enum"], json!(["active"]));
        assert!(openai_out.get("const").is_none());

        // Claude: same behavior
        let (claude_out, _) = run(input.clone(), Target::Claude);
        assert_eq!(claude_out["enum"], json!(["active"]));
        assert!(claude_out.get("const").is_none());

        // Gemini: const preserved as-is
        let (gemini_out, _) = run(input, Target::Gemini);
        assert_eq!(gemini_out["const"], json!("active"));
    }

    // -----------------------------------------------------------------------
    // Test 3: Enum default-first sorting
    // -----------------------------------------------------------------------
    #[test]
    fn test_enum_default_first_sorting() {
        let input = json!({
            "type": "string",
            "enum": ["alpha", "beta", "gamma"],
            "default": "beta"
        });

        let (out, dropped) = run_openai(input);

        // beta should be first
        assert_eq!(out["enum"], json!(["beta", "alpha", "gamma"]));

        // default should be dropped (unsupported by all providers)
        assert!(out.get("default").is_none());

        // default should appear in dropped_constraints
        let default_dropped = dropped.iter().find(|d| d.constraint == "default");
        assert!(
            default_dropped.is_some(),
            "default must be in dropped_constraints"
        );
        assert_eq!(default_dropped.unwrap().value, json!("beta"));
    }

    // -----------------------------------------------------------------------
    // Test 4: Drop not / if-then-else with codec annotation
    // -----------------------------------------------------------------------
    #[test]
    fn test_drop_not_if_then_else() {
        let input = json!({
            "type": "string",
            "not": { "enum": ["bad"] },
            "if": { "minLength": 5 },
            "then": { "pattern": "^[A-Z]" },
            "else": { "pattern": "^[a-z]" }
        });

        let (out, dropped) = run_openai(input);

        assert!(out.get("not").is_none());
        assert!(out.get("if").is_none());
        assert!(out.get("then").is_none());
        assert!(out.get("else").is_none());

        // 4 dropped constraints: not, if, then, else
        assert_eq!(dropped.len(), 4);
        let dropped_names: Vec<&str> = dropped.iter().map(|d| d.constraint.as_str()).collect();
        assert!(dropped_names.contains(&"not"));
        assert!(dropped_names.contains(&"if"));
        assert!(dropped_names.contains(&"then"));
        assert!(dropped_names.contains(&"else"));
    }

    // -----------------------------------------------------------------------
    // Test 5: pattern preserved for OpenAI, dropped for Claude
    // -----------------------------------------------------------------------
    #[test]
    fn test_pattern_openai_vs_claude() {
        let input = json!({
            "type": "string",
            "pattern": "^[A-Z]+"
        });

        // OpenAI: preserved
        let (openai_out, openai_dropped) = run(input.clone(), Target::OpenaiStrict);
        assert_eq!(openai_out["pattern"], json!("^[A-Z]+"));
        assert_eq!(openai_dropped.len(), 0);

        // Claude: dropped
        let (claude_out, claude_dropped) = run(input, Target::Claude);
        assert!(claude_out.get("pattern").is_none());
        assert_eq!(claude_dropped.len(), 1);
        assert_eq!(claude_dropped[0].constraint, "pattern");
    }

    // -----------------------------------------------------------------------
    // Test 6: Multiple constraints on same node — all handled
    // -----------------------------------------------------------------------
    #[test]
    fn test_multiple_constraints_same_node() {
        let input = json!({
            "type": "integer",
            "minimum": 0,
            "maximum": 100,
            "default": 50,
            "multipleOf": 5,
            "exclusiveMinimum": 0
        });

        let (out, dropped) = run_openai(input);

        // All should be dropped for OpenAI
        assert!(out.get("minimum").is_none());
        assert!(out.get("maximum").is_none());
        assert!(out.get("default").is_none());
        assert!(out.get("multipleOf").is_none());
        assert!(out.get("exclusiveMinimum").is_none());

        // type preserved
        assert_eq!(out["type"], json!("integer"));

        // 5 dropped constraints
        assert_eq!(dropped.len(), 5);
    }

    // -----------------------------------------------------------------------
    // Test 7: Nested structures — constraints pruned at all depths
    // -----------------------------------------------------------------------
    #[test]
    fn test_nested_recursion() {
        let input = json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 100
                },
                "items": {
                    "type": "array",
                    "items": {
                        "type": "integer",
                        "minimum": 0
                    },
                    "minItems": 1
                }
            }
        });

        let (out, dropped) = run_openai(input);

        // name constraints dropped
        assert!(out["properties"]["name"].get("minLength").is_none());
        assert!(out["properties"]["name"].get("maxLength").is_none());

        // array item constraint dropped
        assert!(out["properties"]["items"]["items"].get("minimum").is_none());

        // array-level constraint dropped
        assert!(out["properties"]["items"].get("minItems").is_none());

        // 4 total dropped
        assert_eq!(dropped.len(), 4);
    }

    // -----------------------------------------------------------------------
    // Test 8: Depth guard triggers
    // -----------------------------------------------------------------------
    #[test]
    fn test_depth_guard() {
        let input = json!({
            "type": "object",
            "properties": {
                "nested": {
                    "type": "object",
                    "properties": {
                        "deep": { "type": "string", "minLength": 1 }
                    }
                }
            }
        });

        let config = ConvertOptions {
            max_depth: 1,
            ..ConvertOptions::default()
        };

        let result = prune_constraints(&input, &config);
        assert!(result.is_err(), "should fail on depth exceeded");
    }

    // -----------------------------------------------------------------------
    // Test 9: Non-object schemas pass through unchanged
    // -----------------------------------------------------------------------
    #[test]
    fn test_empty_schema_passthrough() {
        let input = json!("string");
        let (out, dropped) = run_openai(input.clone());
        assert_eq!(out, input);
        assert_eq!(dropped.len(), 0);

        let input_bool = json!(true);
        let (out_bool, dropped_bool) = run_openai(input_bool.clone());
        assert_eq!(out_bool, input_bool);
        assert_eq!(dropped_bool.len(), 0);
    }
}
