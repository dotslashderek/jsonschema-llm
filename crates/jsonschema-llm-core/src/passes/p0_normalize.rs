//! Pass 0: Schema Normalization
//! Resolves $ref, normalizes draft syntax, detects recursive cycles.
//!
//! This is the foundational pass — all downstream passes (1–7) assume refs are
//! resolved and syntax is normalized. Pass 0 performs:
//!
//! 1. `items` (array form) → `prefixItems` normalization
//! 2. `$ref` resolution via JSON Pointer traversal with DFS cycle detection
//! 3. `definitions` → `$defs` rename (post-resolution, Phase 3)
//! 4. `$defs` cleanup (strip fully-inlined entries, preserve recursive)

use serde_json::Value;

use crate::config::ConvertOptions;
use crate::error::ConvertError;

/// Result of running the schema normalization pass.
#[derive(Debug)]
pub struct NormalizePassResult {
    /// The normalized schema with all non-recursive refs resolved.
    pub schema: Value,
    /// JSON Pointer paths where recursive $ref cycles were detected.
    /// These are left as `$ref` for Pass 5 to break.
    pub recursive_refs: Vec<String>,
}

/// Apply schema normalization: resolve `$ref`, normalize syntax, detect cycles.
///
/// # Arguments
///
/// * `schema` - A JSON Schema (any draft)
/// * `config` - Conversion options (max_depth, target, etc.)
///
/// # Returns
///
/// A `NormalizePassResult` with the normalized schema and any recursive ref paths.
pub fn normalize(
    _schema: &Value,
    _config: &ConvertOptions,
) -> Result<NormalizePassResult, ConvertError> {
    todo!("Pass 0 implementation")
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use crate::config::ConvertOptions;

    fn run(schema: Value) -> (Value, Vec<String>) {
        let config = ConvertOptions::default();
        let result = normalize(&schema, &config).unwrap();
        (result.schema, result.recursive_refs)
    }

    fn run_err(schema: Value) -> ConvertError {
        let config = ConvertOptions::default();
        normalize(&schema, &config).unwrap_err()
    }

    // -----------------------------------------------------------------------
    // Test 1: Simple $ref → inline definition replaces ref (AC1)
    // -----------------------------------------------------------------------
    #[test]
    fn test_simple_ref_inline() {
        let input = json!({
            "type": "object",
            "properties": {
                "address": { "$ref": "#/$defs/Address" }
            },
            "$defs": {
                "Address": {
                    "type": "object",
                    "properties": {
                        "street": { "type": "string" }
                    }
                }
            }
        });

        let (output, recursive_refs) = run(input);

        // $ref should be replaced with the inline definition.
        assert_eq!(
            output["properties"]["address"],
            json!({
                "type": "object",
                "properties": {
                    "street": { "type": "string" }
                }
            })
        );
        // No $ref key should remain.
        assert!(output["properties"]["address"].get("$ref").is_none());
        // No recursive refs.
        assert!(recursive_refs.is_empty());
        // $defs should be stripped (fully inlined).
        assert!(output.get("$defs").is_none());
    }

    // -----------------------------------------------------------------------
    // Test 2: Chained refs A → B → C resolved transitively (AC2)
    // -----------------------------------------------------------------------
    #[test]
    fn test_chained_refs() {
        let input = json!({
            "type": "object",
            "properties": {
                "thing": { "$ref": "#/$defs/A" }
            },
            "$defs": {
                "A": { "$ref": "#/$defs/B" },
                "B": { "$ref": "#/$defs/C" },
                "C": { "type": "string" }
            }
        });

        let (output, recursive_refs) = run(input);

        // Chained refs should resolve transitively to the final target.
        assert_eq!(output["properties"]["thing"], json!({ "type": "string" }));
        assert!(recursive_refs.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test 3: $ref + sibling description — annotation override (AC3)
    // -----------------------------------------------------------------------
    #[test]
    fn test_ref_with_sibling_description() {
        let input = json!({
            "type": "object",
            "properties": {
                "addr": {
                    "$ref": "#/$defs/Address",
                    "description": "Shipping address"
                }
            },
            "$defs": {
                "Address": {
                    "type": "object",
                    "description": "A postal address",
                    "properties": {
                        "street": { "type": "string" }
                    }
                }
            }
        });

        let (output, _) = run(input);

        // Site-specific description should override the definition's description.
        assert_eq!(
            output["properties"]["addr"]["description"],
            "Shipping address"
        );
        // The resolved properties should still be present.
        assert_eq!(
            output["properties"]["addr"]["properties"]["street"],
            json!({ "type": "string" })
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: $ref + structural sibling → allOf wrap (AC3)
    // -----------------------------------------------------------------------
    #[test]
    fn test_ref_with_structural_sibling() {
        let input = json!({
            "type": "object",
            "properties": {
                "item": {
                    "$ref": "#/$defs/Base",
                    "required": ["extra"]
                }
            },
            "$defs": {
                "Base": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "extra": { "type": "integer" }
                    }
                }
            }
        });

        let (output, _) = run(input);

        // Structural siblings should be wrapped in allOf for Pass 1 to merge.
        let item = &output["properties"]["item"];
        assert!(item.get("allOf").is_some(), "should wrap in allOf");
        let all_of = item["allOf"].as_array().unwrap();
        assert_eq!(all_of.len(), 2);
        // First element: the resolved ref.
        assert_eq!(all_of[0]["type"], "object");
        assert!(all_of[0]["properties"]["name"].is_object());
        // Second element: the structural siblings.
        assert_eq!(all_of[1]["required"], json!(["extra"]));
    }

    // -----------------------------------------------------------------------
    // Test 5: Recursive A → B → A detected, ref left in place (AC4)
    // -----------------------------------------------------------------------
    #[test]
    fn test_recursive_cycle_detected() {
        let input = json!({
            "type": "object",
            "properties": {
                "node": { "$ref": "#/$defs/Node" }
            },
            "$defs": {
                "Node": {
                    "type": "object",
                    "properties": {
                        "value": { "type": "string" },
                        "child": { "$ref": "#/$defs/Node" }
                    }
                }
            }
        });

        let (output, recursive_refs) = run(input);

        // The outer ref should be resolved (first visit is not a cycle).
        assert_eq!(output["properties"]["node"]["type"], "object");
        // The inner recursive ref should be left as $ref.
        assert!(output["properties"]["node"]["properties"]["child"]
            .get("$ref")
            .is_some());
        // recursive_refs should record the cycle.
        assert!(!recursive_refs.is_empty());
        // $defs should be preserved for recursive refs (Pass 5 needs them).
        assert!(output.get("$defs").is_some());
    }

    // -----------------------------------------------------------------------
    // Test 6: Self-referencing A → A detected (AC4)
    // -----------------------------------------------------------------------
    #[test]
    fn test_self_referencing_cycle() {
        let input = json!({
            "$defs": {
                "Recursive": {
                    "type": "object",
                    "properties": {
                        "self": { "$ref": "#/$defs/Recursive" }
                    }
                }
            },
            "type": "object",
            "properties": {
                "root": { "$ref": "#/$defs/Recursive" }
            }
        });

        let (output, recursive_refs) = run(input);

        // Outer ref resolved, inner self-ref detected as cycle.
        assert_eq!(output["properties"]["root"]["type"], "object");
        assert!(output["properties"]["root"]["properties"]["self"]
            .get("$ref")
            .is_some());
        assert!(!recursive_refs.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test 7: `definitions` → `$defs` rename (AC5)
    // -----------------------------------------------------------------------
    #[test]
    fn test_definitions_to_defs_rename() {
        let input = json!({
            "type": "object",
            "properties": {
                "thing": { "$ref": "#/definitions/Thing" }
            },
            "definitions": {
                "Thing": { "type": "string" }
            }
        });

        let (output, _) = run(input);

        // `definitions` should be gone.
        assert!(output.get("definitions").is_none());
        // Ref should be resolved.
        assert_eq!(output["properties"]["thing"], json!({ "type": "string" }));
    }

    // -----------------------------------------------------------------------
    // Test 8: Both `definitions` + `$defs` → merged (AC5)
    // -----------------------------------------------------------------------
    #[test]
    fn test_definitions_and_defs_merged() {
        let input = json!({
            "type": "object",
            "properties": {
                "a": { "$ref": "#/definitions/OldType" },
                "b": { "$ref": "#/$defs/NewType" }
            },
            "definitions": {
                "OldType": { "type": "string" }
            },
            "$defs": {
                "NewType": { "type": "integer" }
            }
        });

        let (output, _) = run(input);

        // Both refs should be resolved.
        assert_eq!(output["properties"]["a"], json!({ "type": "string" }));
        assert_eq!(output["properties"]["b"], json!({ "type": "integer" }));
        // Both definition containers should be stripped after full inlining.
        assert!(output.get("definitions").is_none());
    }

    // -----------------------------------------------------------------------
    // Test 9: Same $ref used 3 times → each gets independent copy (AC6)
    // -----------------------------------------------------------------------
    #[test]
    fn test_multiple_uses_independent_copies() {
        let input = json!({
            "type": "object",
            "properties": {
                "home": { "$ref": "#/$defs/Address" },
                "work": { "$ref": "#/$defs/Address" },
                "mail": { "$ref": "#/$defs/Address" }
            },
            "$defs": {
                "Address": {
                    "type": "object",
                    "properties": {
                        "street": { "type": "string" }
                    }
                }
            }
        });

        let (output, _) = run(input);

        let addr_schema = json!({
            "type": "object",
            "properties": {
                "street": { "type": "string" }
            }
        });

        // All three should be independently resolved.
        assert_eq!(output["properties"]["home"], addr_schema);
        assert_eq!(output["properties"]["work"], addr_schema);
        assert_eq!(output["properties"]["mail"], addr_schema);
    }

    // -----------------------------------------------------------------------
    // Test 10: Deep $ref path (#/$defs/A/properties/x)
    // -----------------------------------------------------------------------
    #[test]
    fn test_deep_ref_path() {
        let input = json!({
            "type": "object",
            "properties": {
                "x_type": { "$ref": "#/$defs/Wrapper/properties/x" }
            },
            "$defs": {
                "Wrapper": {
                    "type": "object",
                    "properties": {
                        "x": { "type": "number", "minimum": 0 }
                    }
                }
            }
        });

        let (output, _) = run(input);

        // Deep ref should resolve to the nested property schema.
        assert_eq!(
            output["properties"]["x_type"],
            json!({ "type": "number", "minimum": 0 })
        );
    }

    // -----------------------------------------------------------------------
    // Test 11: Legacy #/definitions/X resolves correctly
    // -----------------------------------------------------------------------
    #[test]
    fn test_legacy_definitions_ref() {
        let input = json!({
            "type": "object",
            "properties": {
                "item": { "$ref": "#/definitions/Item" }
            },
            "definitions": {
                "Item": { "type": "boolean" }
            }
        });

        let (output, _) = run(input);

        assert_eq!(output["properties"]["item"], json!({ "type": "boolean" }));
    }

    // -----------------------------------------------------------------------
    // Test 12: Non-local $ref (no # prefix) → error
    // -----------------------------------------------------------------------
    #[test]
    fn test_non_local_ref_error() {
        let input = json!({
            "type": "object",
            "properties": {
                "ext": { "$ref": "https://example.com/schemas/Thing" }
            }
        });

        let err = run_err(input);
        let msg = err.to_string();
        assert!(
            msg.contains("Unsupported") || msg.contains("external") || msg.contains("non-local"),
            "Expected error about non-local $ref, got: {}",
            msg
        );
    }

    // -----------------------------------------------------------------------
    // Test 13: $ref pointing to nonexistent definition → error
    // -----------------------------------------------------------------------
    #[test]
    fn test_nonexistent_ref_error() {
        let input = json!({
            "type": "object",
            "properties": {
                "ghost": { "$ref": "#/$defs/DoesNotExist" }
            }
        });

        let err = run_err(input);
        let msg = err.to_string();
        assert!(
            msg.contains("Unresolvable")
                || msg.contains("not found")
                || msg.contains("DoesNotExist"),
            "Expected error about unresolvable ref, got: {}",
            msg
        );
    }

    // -----------------------------------------------------------------------
    // Test 14: `items` array form → `prefixItems` normalization
    // -----------------------------------------------------------------------
    #[test]
    fn test_items_array_to_prefix_items() {
        let input = json!({
            "type": "array",
            "items": [
                { "type": "string" },
                { "type": "integer" }
            ]
        });

        let (output, _) = run(input);

        // Array-form items should become prefixItems.
        assert!(output.get("items").is_none());
        assert_eq!(
            output["prefixItems"],
            json!([
                { "type": "string" },
                { "type": "integer" }
            ])
        );
    }

    // -----------------------------------------------------------------------
    // Test 15: Nested $ref inside properties/anyOf → resolved
    // -----------------------------------------------------------------------
    #[test]
    fn test_nested_ref_in_anyof() {
        let input = json!({
            "type": "object",
            "properties": {
                "field": {
                    "anyOf": [
                        { "$ref": "#/$defs/TypeA" },
                        { "$ref": "#/$defs/TypeB" }
                    ]
                }
            },
            "$defs": {
                "TypeA": { "type": "string" },
                "TypeB": { "type": "integer" }
            }
        });

        let (output, _) = run(input);

        let any_of = output["properties"]["field"]["anyOf"].as_array().unwrap();
        assert_eq!(any_of[0], json!({ "type": "string" }));
        assert_eq!(any_of[1], json!({ "type": "integer" }));
    }

    // -----------------------------------------------------------------------
    // Test 16: No-op — schema with no $ref passes through unchanged
    // -----------------------------------------------------------------------
    #[test]
    fn test_noop_no_refs() {
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            }
        });

        let (output, recursive_refs) = run(input.clone());

        assert_eq!(output, input);
        assert!(recursive_refs.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test 17: Depth guard (max_depth exceeded) → error
    // -----------------------------------------------------------------------
    #[test]
    fn test_depth_guard() {
        let input = json!({
            "type": "object",
            "properties": {
                "a": {
                    "type": "object",
                    "properties": {
                        "b": {
                            "type": "object",
                            "properties": {
                                "c": { "type": "string" }
                            }
                        }
                    }
                }
            }
        });

        let config = ConvertOptions {
            max_depth: 2,
            ..ConvertOptions::default()
        };

        let result = normalize(&input, &config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            ConvertError::RecursionDepthExceeded { max_depth, .. } => {
                assert_eq!(max_depth, 2);
            }
            other => panic!("expected RecursionDepthExceeded, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Test 18: $defs cleanup — inlined entries stripped, recursive preserved
    // -----------------------------------------------------------------------
    #[test]
    fn test_defs_cleanup() {
        let input = json!({
            "type": "object",
            "properties": {
                "simple": { "$ref": "#/$defs/Simple" },
                "tree": { "$ref": "#/$defs/TreeNode" }
            },
            "$defs": {
                "Simple": { "type": "string" },
                "TreeNode": {
                    "type": "object",
                    "properties": {
                        "value": { "type": "integer" },
                        "left": { "$ref": "#/$defs/TreeNode" },
                        "right": { "$ref": "#/$defs/TreeNode" }
                    }
                }
            }
        });

        let (output, recursive_refs) = run(input);

        // Simple should be inlined, no $ref remaining.
        assert_eq!(output["properties"]["simple"], json!({ "type": "string" }));

        // TreeNode is recursive, so recursive_refs should be non-empty.
        assert!(!recursive_refs.is_empty());

        // $defs should still exist (TreeNode is recursive, needed by Pass 5).
        let defs = output.get("$defs").and_then(|v| v.as_object());
        assert!(
            defs.is_some(),
            "$defs should be preserved for recursive entries"
        );
        let defs = defs.unwrap();

        // Simple should be stripped from $defs (fully inlined).
        assert!(
            !defs.contains_key("Simple"),
            "Simple should be stripped from $defs"
        );
        // TreeNode should remain in $defs (recursive).
        assert!(
            defs.contains_key("TreeNode"),
            "TreeNode should remain in $defs"
        );
    }
}
