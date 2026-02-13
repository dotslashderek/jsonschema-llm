//! Pass 2: Polymorphism Simplification (oneOf → anyOf)
//!
//! Rewrites `oneOf` to `anyOf` throughout the schema tree. This is a
//! **codec-lossless** keyword rename — data shape doesn't change, so no codec
//! transforms are recorded. It is intentionally **validation-lossy**: `anyOf`
//! allows multiple variants to match whereas `oneOf` forbids it. For LLM
//! generation with discriminators, this is semantically equivalent and much
//! easier for models to generate against.
//!
//! When both `oneOf` and `anyOf` exist on the same node, both are wrapped into
//! an `allOf` array to preserve intersection semantics.

use serde_json::{Map, Value};

use crate::config::{ConvertOptions, PolymorphismStrategy, Target};
use crate::error::ConvertError;
use crate::schema_utils::recurse_into_children;

use super::pass_result::PassResult;

/// Apply polymorphism simplification to a schema.
///
/// Recursively walks the schema tree, renaming `oneOf` to `anyOf`. Handles
/// key collisions (both `oneOf` and `anyOf` present) by wrapping into `allOf`.
///
/// Skipped when `config.target == Target::Gemini` (Gemini handles `oneOf`
/// natively) or when `config.polymorphism == PolymorphismStrategy::Flatten`.
pub fn simplify_polymorphism(
    schema: Value,
    config: &ConvertOptions,
) -> Result<PassResult, ConvertError> {
    // Provider gate: Gemini supports oneOf natively.
    if config.target == Target::Gemini {
        return Ok(PassResult::schema_only(schema));
    }

    // Strategy gate: Flatten is future work.
    if config.polymorphism == PolymorphismStrategy::Flatten {
        return Ok(PassResult::schema_only(schema));
    }

    let result = walk(&schema, "#", 0, config)?;
    Ok(PassResult::schema_only(result))
}

// ---------------------------------------------------------------------------
// Recursive walker
// ---------------------------------------------------------------------------

/// Recursively descend through the schema tree, renaming `oneOf` to `anyOf`
/// and handling collisions.
fn walk(
    node: &Value,
    path: &str,
    depth: usize,
    config: &ConvertOptions,
) -> Result<Value, ConvertError> {
    if depth > config.max_depth {
        return Ok(node.clone());
    }

    match node {
        Value::Object(obj) => {
            let mut new_obj = obj.clone();

            // --- Rename oneOf → anyOf (with collision handling) ---
            rename_oneof_to_anyof(&mut new_obj);

            // --- Recurse into all child schemas via shared traversal ---
            recurse_into_children(&mut new_obj, path, depth, &mut |val, child_path, d| {
                walk(val, child_path, d, config)
            })?;

            Ok(Value::Object(new_obj))
        }
        _ => Ok(node.clone()),
    }
}

// ---------------------------------------------------------------------------
// Core rename logic
// ---------------------------------------------------------------------------

/// Rename `oneOf` to `anyOf` on a single schema object.
///
/// If the object has both `oneOf` and `anyOf`, both are wrapped into an `allOf`
/// array to preserve the intersection semantics (`oneOf` ∧ `anyOf` → `allOf`
/// of two `anyOf` entries). If `allOf` already exists, the entries are appended.
fn rename_oneof_to_anyof(obj: &mut Map<String, Value>) {
    let one_of = match obj.remove("oneOf") {
        Some(v) => v,
        None => return, // Nothing to do
    };

    if let Some(any_of) = obj.remove("anyOf") {
        // Collision: both oneOf and anyOf exist.
        // Wrap both into allOf to preserve intersection semantics.
        let converted_entry = serde_json::json!({ "anyOf": one_of });
        let existing_entry = serde_json::json!({ "anyOf": any_of });

        if let Some(all_of) = obj.get_mut("allOf").and_then(Value::as_array_mut) {
            // Append to existing allOf
            all_of.push(converted_entry);
            all_of.push(existing_entry);
        } else {
            // Create new allOf
            obj.insert(
                "allOf".to_string(),
                Value::Array(vec![converted_entry, existing_entry]),
            );
        }
    } else {
        // Simple case: no collision, just rename.
        obj.insert("anyOf".to_string(), one_of);
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

    fn run(schema: Value) -> Value {
        simplify_polymorphism(schema, &ConvertOptions::default())
            .unwrap()
            .schema
    }

    // Test 1: Simple oneOf → anyOf
    #[test]
    fn test_simple_oneof_to_anyof() {
        let input = json!({
            "oneOf": [
                { "type": "string" },
                { "type": "integer" }
            ]
        });
        let expected = json!({
            "anyOf": [
                { "type": "string" },
                { "type": "integer" }
            ]
        });
        assert_eq!(run(input), expected);
    }

    // Test 2: Discriminator preserved
    #[test]
    fn test_discriminator_preserved() {
        let input = json!({
            "oneOf": [
                { "type": "object", "properties": { "kind": {"const": "a"} } },
                { "type": "object", "properties": { "kind": {"const": "b"} } }
            ],
            "discriminator": { "propertyName": "kind" }
        });
        let result = run(input);
        assert!(result.get("anyOf").is_some(), "should have anyOf");
        assert!(result.get("oneOf").is_none(), "oneOf should be gone");
        assert_eq!(
            result["discriminator"],
            json!({ "propertyName": "kind" }),
            "discriminator preserved"
        );
        assert_eq!(result["anyOf"].as_array().unwrap().len(), 2);
    }

    // Test 3: Nested oneOf (inside properties of another schema)
    #[test]
    fn test_nested_oneof() {
        let input = json!({
            "type": "object",
            "properties": {
                "inner": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "number" }
                    ]
                }
            }
        });
        let result = run(input);
        assert!(result["properties"]["inner"].get("anyOf").is_some());
        assert!(result["properties"]["inner"].get("oneOf").is_none());
    }

    // Test 4: Existing anyOf passthrough
    #[test]
    fn test_anyof_passthrough() {
        let input = json!({
            "anyOf": [
                { "type": "string" },
                { "type": "boolean" }
            ]
        });
        let result = run(input.clone());
        assert_eq!(result, input);
    }

    // Test 5: Gemini skip
    #[test]
    fn test_gemini_skip() {
        let input = json!({
            "oneOf": [{ "type": "string" }]
        });
        let config = ConvertOptions {
            target: Target::Gemini,
            ..ConvertOptions::default()
        };
        let result = simplify_polymorphism(input.clone(), &config).unwrap();
        assert_eq!(result.schema, input, "Gemini should skip P2");
    }

    // Test 6: No transforms in result (codec-lossless)
    #[test]
    fn test_no_transforms() {
        // PassResult has no transforms — codec-lossless keyword rename.
        // This test just verifies the pass succeeds on a non-trivial input.
        let input = json!({
            "oneOf": [{ "type": "string" }]
        });
        let result = simplify_polymorphism(input, &ConvertOptions::default());
        assert!(result.is_ok());
    }

    // Test 7: oneOf inside items
    #[test]
    fn test_items_traversal() {
        let input = json!({
            "type": "array",
            "items": {
                "oneOf": [
                    { "type": "string" },
                    { "type": "integer" }
                ]
            }
        });
        let result = run(input);
        assert!(result["items"].get("anyOf").is_some());
        assert!(result["items"].get("oneOf").is_none());
    }

    // Test 8: Empty/primitive schema — identity
    #[test]
    fn test_empty_schema() {
        let input = json!({ "type": "string" });
        let result = run(input.clone());
        assert_eq!(result, input);
    }

    // Test 9: Collision — both oneOf and anyOf present
    #[test]
    fn test_collision_oneof_anyof() {
        let input = json!({
            "oneOf": [
                { "type": "string" }
            ],
            "anyOf": [
                { "type": "integer" }
            ]
        });
        let result = run(input);
        // Both wrapped into allOf
        assert!(result.get("oneOf").is_none(), "oneOf removed");
        assert!(result.get("anyOf").is_none(), "standalone anyOf removed");
        let all_of = result["allOf"].as_array().unwrap();
        assert_eq!(all_of.len(), 2);
        // First entry is the converted oneOf
        assert_eq!(all_of[0]["anyOf"], json!([{ "type": "string" }]));
        // Second entry is the original anyOf
        assert_eq!(all_of[1]["anyOf"], json!([{ "type": "integer" }]));
    }

    // Test 10: Collision with existing allOf — appended
    #[test]
    fn test_collision_with_existing_allof() {
        let input = json!({
            "allOf": [
                { "type": "boolean" }
            ],
            "oneOf": [
                { "type": "string" }
            ],
            "anyOf": [
                { "type": "integer" }
            ]
        });
        let result = run(input);
        let all_of = result["allOf"].as_array().unwrap();
        assert_eq!(all_of.len(), 3, "original + 2 wrapped");
        assert_eq!(all_of[0], json!({ "type": "boolean" }));
        assert_eq!(all_of[1]["anyOf"], json!([{ "type": "string" }]));
        assert_eq!(all_of[2]["anyOf"], json!([{ "type": "integer" }]));
    }

    // Test 11: $defs and definitions traversal
    #[test]
    fn test_defs_traversal() {
        let input = json!({
            "$defs": {
                "MyType": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "integer" }
                    ]
                }
            },
            "definitions": {
                "OldType": {
                    "oneOf": [
                        { "type": "boolean" }
                    ]
                }
            }
        });
        let result = run(input);
        assert!(result["$defs"]["MyType"].get("anyOf").is_some());
        assert!(result["$defs"]["MyType"].get("oneOf").is_none());
        assert!(result["definitions"]["OldType"].get("anyOf").is_some());
        assert!(result["definitions"]["OldType"].get("oneOf").is_none());
    }
}
