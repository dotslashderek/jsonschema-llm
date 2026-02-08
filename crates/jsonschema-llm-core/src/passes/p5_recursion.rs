//! Pass 5: Recursion Breaking
//!
//! Walks the schema tree, inlines all remaining `$ref` nodes, and breaks
//! recursive cycles at `config.recursion_limit` by replacing them with
//! opaque JSON-string placeholders. Emits `RecursiveInflate` codec entries
//! for round-trip rehydration.

use std::collections::HashMap;

use serde_json::Value;

use crate::codec::Transform;
use crate::config::{ConvertOptions, Target};
use crate::error::ConvertError;

/// Result of Pass 5.
pub struct RecursionPassResult {
    /// Schema with recursion broken.
    pub schema: Value,
    /// Codec transforms emitted during recursion breaking.
    pub transforms: Vec<Transform>,
}

/// Break recursive `$ref` cycles in the schema.
///
/// Uses dynamic cycle detection: maintains a per-branch count of how many times
/// each `$ref` target has been expanded. When the count reaches
/// `config.recursion_limit`, the ref is replaced with an opaque string schema.
///
/// For `Target::Gemini`, returns the schema unchanged (native recursion support).
pub fn break_recursion(
    schema: &Value,
    config: &ConvertOptions,
) -> Result<RecursionPassResult, ConvertError> {
    let _ = (schema, config);
    todo!("Pass 5 implementation")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn default_config() -> ConvertOptions {
        ConvertOptions::default()
    }

    fn config_with_limit(limit: usize) -> ConvertOptions {
        ConvertOptions {
            recursion_limit: limit,
            ..ConvertOptions::default()
        }
    }

    fn gemini_config() -> ConvertOptions {
        ConvertOptions {
            target: Target::Gemini,
            ..ConvertOptions::default()
        }
    }

    // -----------------------------------------------------------------------
    // Test 1: Simple A→B→A cycle
    // -----------------------------------------------------------------------
    #[test]
    fn test_simple_ab_cycle() {
        // A has a property "b" that refs B; B has a property "a" that refs A.
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "b": { "$ref": "#/$defs/B" }
            },
            "required": ["name"],
            "$defs": {
                "B": {
                    "type": "object",
                    "properties": {
                        "value": { "type": "integer" },
                        "a": { "$ref": "#/$defs/A" }
                    }
                },
                "A": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "b": { "$ref": "#/$defs/B" }
                    }
                }
            }
        });

        let config = config_with_limit(1);
        let result = break_recursion(&schema, &config).unwrap();

        // After recursion_limit=1: first expansion of B inlines B's content.
        // Inside that inlined B, the ref to A hits limit (A→B→A), so A becomes opaque string.
        // The schema should have no $ref nodes remaining.
        assert!(
            !serde_json::to_string(&result.schema).unwrap().contains("\"$ref\""),
            "Schema should have no remaining $ref nodes"
        );

        // At least one RecursiveInflate transform should be emitted
        assert!(
            result.transforms.iter().any(|t| matches!(t, Transform::RecursiveInflate { .. })),
            "Should emit RecursiveInflate transform"
        );

        // $defs should be cleaned up
        assert!(
            result.schema.get("$defs").is_none(),
            "$defs should be removed after full resolution"
        );
    }

    // -----------------------------------------------------------------------
    // Test 2: Multi-node cycle A→B→C→A
    // -----------------------------------------------------------------------
    #[test]
    fn test_multi_node_cycle() {
        let schema = json!({
            "$ref": "#/$defs/A",
            "$defs": {
                "A": {
                    "type": "object",
                    "properties": {
                        "next": { "$ref": "#/$defs/B" }
                    }
                },
                "B": {
                    "type": "object",
                    "properties": {
                        "next": { "$ref": "#/$defs/C" }
                    }
                },
                "C": {
                    "type": "object",
                    "properties": {
                        "next": { "$ref": "#/$defs/A" }
                    }
                }
            }
        });

        let config = config_with_limit(1);
        let result = break_recursion(&schema, &config).unwrap();

        assert!(
            !serde_json::to_string(&result.schema).unwrap().contains("\"$ref\""),
            "Schema should have no remaining $ref nodes"
        );
        assert!(
            result.transforms.iter().any(|t| matches!(t, Transform::RecursiveInflate { .. })),
            "Should emit RecursiveInflate transform"
        );
    }

    // -----------------------------------------------------------------------
    // Test 3: Self-referencing tree (TreeNode with children: [TreeNode])
    // -----------------------------------------------------------------------
    #[test]
    fn test_self_referencing_tree() {
        let schema = json!({
            "$ref": "#/$defs/TreeNode",
            "$defs": {
                "TreeNode": {
                    "type": "object",
                    "properties": {
                        "value": { "type": "string" },
                        "children": {
                            "type": "array",
                            "items": { "$ref": "#/$defs/TreeNode" }
                        }
                    },
                    "required": ["value"]
                }
            }
        });

        let config = config_with_limit(2);
        let result = break_recursion(&schema, &config).unwrap();

        assert!(
            !serde_json::to_string(&result.schema).unwrap().contains("\"$ref\""),
            "Schema should have no remaining $ref nodes"
        );

        // With limit=2: TreeNode expands twice before the third occurrence becomes opaque.
        // The opaque replacement should mention TreeNode in its description.
        let schema_str = serde_json::to_string(&result.schema).unwrap();
        assert!(
            schema_str.contains("TreeNode"),
            "Opaque description should reference TreeNode"
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: Depth is configurable
    // -----------------------------------------------------------------------
    #[test]
    fn test_depth_configurable() {
        let schema = json!({
            "$ref": "#/$defs/Node",
            "$defs": {
                "Node": {
                    "type": "object",
                    "properties": {
                        "value": { "type": "integer" },
                        "next": { "$ref": "#/$defs/Node" }
                    }
                }
            }
        });

        // With limit=1: Node expands once, then next occurrence is opaque.
        let result_1 = break_recursion(&schema, &config_with_limit(1)).unwrap();
        let count_1 = result_1.transforms.iter()
            .filter(|t| matches!(t, Transform::RecursiveInflate { .. }))
            .count();

        // With limit=3: Node expands three times before breaking.
        let result_3 = break_recursion(&schema, &config_with_limit(3)).unwrap();
        let count_3 = result_3.transforms.iter()
            .filter(|t| matches!(t, Transform::RecursiveInflate { .. }))
            .count();

        // Both should have exactly 1 RecursiveInflate (the break point),
        // but the schema depth should differ.
        assert_eq!(count_1, 1, "limit=1 should produce 1 RecursiveInflate");
        assert_eq!(count_3, 1, "limit=3 should produce 1 RecursiveInflate");

        // The limit=3 schema should be deeper (more nested properties)
        let depth_1 = serde_json::to_string(&result_1.schema).unwrap().len();
        let depth_3 = serde_json::to_string(&result_3.schema).unwrap().len();
        assert!(
            depth_3 > depth_1,
            "Higher limit should produce deeper (larger) schema: {} vs {}",
            depth_3, depth_1
        );
    }

    // -----------------------------------------------------------------------
    // Test 5: Gemini target skips recursion breaking
    // -----------------------------------------------------------------------
    #[test]
    fn test_gemini_skip() {
        let schema = json!({
            "$ref": "#/$defs/Node",
            "$defs": {
                "Node": {
                    "type": "object",
                    "properties": {
                        "next": { "$ref": "#/$defs/Node" }
                    }
                }
            }
        });

        let result = break_recursion(&schema, &gemini_config()).unwrap();

        // Schema should be returned unchanged
        assert_eq!(result.schema, schema, "Gemini should pass schema through unchanged");
        assert!(result.transforms.is_empty(), "Gemini should emit no transforms");
    }

    // -----------------------------------------------------------------------
    // Test 6: No refs — schema passes through unchanged
    // -----------------------------------------------------------------------
    #[test]
    fn test_no_refs() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            },
            "required": ["name"]
        });

        let result = break_recursion(&schema, &default_config()).unwrap();

        assert_eq!(result.schema, schema, "Schema without refs should be unchanged");
        assert!(result.transforms.is_empty(), "No transforms for refless schema");
    }

    // -----------------------------------------------------------------------
    // Test 7: Codec entry has correct path and original_ref
    // -----------------------------------------------------------------------
    #[test]
    fn test_codec_entry_emitted() {
        let schema = json!({
            "$ref": "#/$defs/Node",
            "$defs": {
                "Node": {
                    "type": "object",
                    "properties": {
                        "next": { "$ref": "#/$defs/Node" }
                    }
                }
            }
        });

        let config = config_with_limit(1);
        let result = break_recursion(&schema, &config).unwrap();

        let inflate = result.transforms.iter().find_map(|t| {
            if let Transform::RecursiveInflate { path, original_ref } = t {
                Some((path.clone(), original_ref.clone()))
            } else {
                None
            }
        }).expect("Should have RecursiveInflate transform");

        // The original_ref should point to the $defs target
        assert_eq!(inflate.1, "#/$defs/Node", "original_ref should be the $defs target");

        // The path should be a valid JSON Pointer (starts with #/)
        assert!(inflate.0.starts_with("#/"), "path should be a JSON Pointer");
    }

    // -----------------------------------------------------------------------
    // Test 8: $defs cleaned up after resolution
    // -----------------------------------------------------------------------
    #[test]
    fn test_defs_cleaned_up() {
        let schema = json!({
            "$ref": "#/$defs/Wrapper",
            "$defs": {
                "Wrapper": {
                    "type": "object",
                    "properties": {
                        "inner": { "$ref": "#/$defs/Inner" }
                    }
                },
                "Inner": {
                    "type": "object",
                    "properties": {
                        "value": { "type": "string" }
                    }
                }
            }
        });

        let result = break_recursion(&schema, &default_config()).unwrap();

        // Both refs are non-recursive (DAG), should be fully inlined
        assert!(
            result.schema.get("$defs").is_none(),
            "$defs should be removed after full inlining"
        );
        assert!(
            !serde_json::to_string(&result.schema).unwrap().contains("\"$ref\""),
            "All $ref nodes should be inlined"
        );
        assert!(result.transforms.is_empty(), "DAG refs need no RecursiveInflate");
    }

    // -----------------------------------------------------------------------
    // Test 9: DAG ref inlined (non-cyclic $ref remaining from Pass 0)
    // -----------------------------------------------------------------------
    #[test]
    fn test_dag_ref_inlined() {
        // Two properties reference the same $def (diamond, not cycle)
        let schema = json!({
            "type": "object",
            "properties": {
                "billing": { "$ref": "#/$defs/Address" },
                "shipping": { "$ref": "#/$defs/Address" }
            },
            "$defs": {
                "Address": {
                    "type": "object",
                    "properties": {
                        "street": { "type": "string" },
                        "city": { "type": "string" }
                    }
                }
            }
        });

        let result = break_recursion(&schema, &default_config()).unwrap();

        // Both properties should have the Address schema inlined
        let billing = &result.schema["properties"]["billing"];
        assert_eq!(billing["type"], "object", "billing should have inlined Address");
        assert!(billing.get("properties").is_some(), "billing should have properties");

        let shipping = &result.schema["properties"]["shipping"];
        assert_eq!(shipping["type"], "object", "shipping should have inlined Address");

        // No $ref remaining, no transforms needed
        assert!(
            !serde_json::to_string(&result.schema).unwrap().contains("\"$ref\""),
            "All $ref nodes should be inlined"
        );
        assert!(result.transforms.is_empty(), "DAG refs need no RecursiveInflate");
    }

    // -----------------------------------------------------------------------
    // Test 10: Rehydration round-trip (RecursiveInflate)
    // -----------------------------------------------------------------------
    // This test lives in rehydrator.rs — see test_recursive_inflate_rehydration
}
