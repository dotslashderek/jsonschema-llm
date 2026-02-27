//! Pass 5: Recursion Breaking
//!
//! Walks the schema tree, inlines all remaining `$ref` nodes, and breaks
//! recursive cycles at `config.recursion_limit` by replacing them with
//! opaque JSON-string placeholders. Emits `RecursiveInflate` codec entries
//! for round-trip rehydration.
//!
//! ## Pipeline Position
//!
//! This pass **must** run after Pass 4 (`p4_opaque`). Pass 4's `is_opaque()`
//! and `is_untyped_opaque()` functions both check `obj.contains_key("$ref")`
//! to prevent stringifying schemas that contain unresolved references. If
//! recursion breaking were merged into Pass 0, those guards would disappear
//! and p4 would incorrectly stringify ref-bearing schemas.
//!
//! This pass uses [`SchemaFolder`](crate::schema_walker::SchemaFolder) for
//! AST traversal (unified walker, PR #218).

use std::collections::HashMap;

use serde_json::Value;

use crate::codec::Transform;
use crate::config::{ConvertOptions, Target};
use crate::error::ConvertError;

use super::pass_result::PassResult;
use super::pass_utils::REF_META_KEYWORDS;

pub fn break_recursion(schema: Value, config: &ConvertOptions) -> Result<PassResult, ConvertError> {
    // Gemini gate: native recursion support
    if config.target == Target::Gemini {
        return Ok(PassResult::schema_only(schema));
    }

    // Extract $defs for ref resolution
    let defs = schema
        .get("$defs")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));

    let mut transforms = Vec::new();
    let mut ref_counts: HashMap<String, usize> = HashMap::new();

    let mut folder = RecursionFolder {
        defs: &defs,
        config,
        ref_counts: &mut ref_counts,
        transforms: &mut transforms,
    };
    let result = crate::schema_walker::fold(schema, &mut folder, "#", 0)?;

    // Safety check: only strip $defs if no dangling $ref nodes remain
    let result = if has_remaining_refs(&result) {
        tracing::warn!("Schema still contains $ref nodes after Pass 5 — keeping $defs");
        result
    } else {
        strip_defs(result)
    };

    Ok(PassResult::with_transforms(result, transforms))
}

// ---------------------------------------------------------------------------
// RecursionFolder — SchemaFolder implementation
// ---------------------------------------------------------------------------

struct RecursionFolder<'a> {
    defs: &'a Value,
    config: &'a ConvertOptions,
    ref_counts: &'a mut HashMap<String, usize>,
    transforms: &'a mut Vec<Transform>,
}

impl crate::schema_walker::SchemaFolder for RecursionFolder<'_> {
    type Error = ConvertError;

    fn fold_schema(
        &mut self,
        schema: Value,
        path: &str,
        depth: usize,
    ) -> Result<crate::schema_walker::FoldAction, Self::Error> {
        let Value::Object(mut obj) = schema else {
            return Ok(crate::schema_walker::FoldAction::Continue(schema));
        };

        // Intercept `$ref` nodes — inline or break the cycle.
        if let Some(ref_str) = obj.get("$ref").and_then(Value::as_str).map(String::from) {
            let type_name = extract_type_name(&ref_str);
            let count = self.ref_counts.get(&ref_str).copied().unwrap_or(0);

            if count >= self.config.recursion_limit {
                // Break: replace with opaque string placeholder.
                self.transforms.push(Transform::RecursiveInflate {
                    path: path.to_string(),
                    original_ref: ref_str.clone(),
                });

                let example = lookup_def(&ref_str, self.defs)
                    .as_ref()
                    .map(|def| build_example_from_def(def, &type_name))
                    .unwrap_or_else(|| "{\\\"key\\\": \\\"value\\\"}".to_string());

                return Ok(crate::schema_walker::FoldAction::Replace(
                    serde_json::json!({
                        "type": "string",
                        "description": format!(
                            "MUST be a valid JSON object serialized as a string. \
                             This represents a {type_name} that was too deeply nested to inline. \
                             Output a complete JSON object as a string value, e.g. \
                             \"{example}\". \
                             Do NOT output plain text — the value must parse as JSON.",
                        )
                    }),
                ));
            }

            // Inline: look up the definition and fold it.
            if let Some(def) = lookup_def(&ref_str, self.defs) {
                *self.ref_counts.entry(ref_str.clone()).or_insert(0) += 1;
                let mut result = crate::schema_walker::fold(def, self, path, depth)?;

                // Strip resolution-mechanism keywords from the inlined def.
                // These ($anchor, $id, $schema, etc.) have no meaning after
                // inlining and may leak through if p9 can't visit all nodes
                // (e.g., nullable primitives at the depth limit).
                strip_ref_meta_keywords(&mut result);

                // Preserve structural siblings alongside the $ref.
                // Defs like `parameter-object` have `$ref: spec-ext` + type/properties.
                // Without merging, the siblings (type, properties, required…) are lost.
                //
                // Fold siblings BEFORE decrementing the ref count so that any
                // $refs within siblings that reference this same definition
                // are still counted against the recursion limit.
                //
                // Filter out JSON Schema identity/resolution keywords that
                // have no meaning in LLM output schemas ($anchor, $id, etc.).
                let siblings: serde_json::Map<String, Value> = obj
                    .into_iter()
                    .filter(|(k, _)| {
                        k != "$ref" && k != "$defs" && !REF_META_KEYWORDS.contains(&k.as_str())
                    })
                    .collect();

                let final_result = if siblings.is_empty() {
                    result
                } else {
                    let folded_siblings =
                        crate::schema_walker::fold(Value::Object(siblings), self, path, depth)?;
                    merge_ref_with_siblings(result, folded_siblings)
                };

                if let Some(c) = self.ref_counts.get_mut(&ref_str) {
                    *c -= 1;
                }

                return Ok(crate::schema_walker::FoldAction::Replace(final_result));
            }

            // Non-local ref — opaque-stringify for strict mode compliance.
            // External URLs and unresolvable anchors cannot be resolved at
            // conversion time. Replace with an opaque JSON-string placeholder
            // so the output is free of $ref nodes.
            self.transforms.push(Transform::JsonStringParse {
                path: path.to_string(),
            });
            return Ok(crate::schema_walker::FoldAction::Replace(
                serde_json::json!({
                    "type": "string",
                    "description": format!(
                        "MUST be a valid JSON value serialized as a string. \
                         This represents a {} reference that could not be resolved. \
                         Do NOT output plain text — the value must parse with JSON.parse().",
                        type_name
                    )
                }),
            ));
        }

        // At the root, strip `$defs` — we resolve from the pre-extracted copy.
        if depth == 0 {
            obj.remove("$defs");
        }

        Ok(crate::schema_walker::FoldAction::Continue(Value::Object(
            obj,
        )))
    }
}

/// Look up a `$ref` target in the `$defs` map.
fn lookup_def(ref_str: &str, defs: &Value) -> Option<Value> {
    // Expected format: "#/$defs/TypeName"
    let prefix = "#/$defs/";
    if let Some(name) = ref_str.strip_prefix(prefix) {
        defs.get(name).cloned()
    } else {
        None
    }
}

/// Extract a human-readable type name from a `$ref` pointer.
fn extract_type_name(ref_str: &str) -> String {
    ref_str.rsplit('/').next().unwrap_or(ref_str).to_string()
}

/// Build a concrete JSON example string from a schema definition.
///
/// Introspects the definition's `properties` to produce type-appropriate
/// placeholder values (e.g., `{"value":"...","children":null}`).
/// This gives the LLM a much stronger signal than a generic `{"key":"value"}`.
fn build_example_from_def(def: &Value, type_name: &str) -> String {
    let props = match def.get("properties").and_then(|p| p.as_object()) {
        Some(p) if !p.is_empty() => p,
        _ => return format!("{{\\\"type\\\":\\\"{type_name}\\\"}}"),
    };

    let mut parts = Vec::new();
    for (key, prop_schema) in props {
        let placeholder = infer_placeholder(prop_schema);
        parts.push(format!("\\\"{}\\\":{}", key, placeholder));
    }
    format!("{{{}}}", parts.join(","))
}

/// Infer a placeholder value for a property schema.
///
/// Returns an escaped JSON fragment suitable for embedding in a description string.
fn infer_placeholder(schema: &Value) -> &'static str {
    // Check for anyOf/oneOf with null (nullable) — use null as placeholder
    for kw in &["anyOf", "oneOf"] {
        if let Some(variants) = schema.get(*kw).and_then(|v| v.as_array()) {
            let has_null = variants
                .iter()
                .any(|v| v.get("type").and_then(|t| t.as_str()) == Some("null"));
            if has_null {
                return "null";
            }
        }
    }

    match schema.get("type").and_then(|t| t.as_str()) {
        Some("string") => "\\\"...\\\"",
        Some("integer") | Some("number") => "0",
        Some("boolean") => "false",
        Some("array") => "[]",
        Some("object") => "{}",
        Some("null") => "null",
        _ => "\\\"...\\\"", // default to string-like
    }
}

///
/// After p5 inlines a `$ref`, the resolved definition may contain `$anchor`,
/// `$id`, etc. These have no meaning in the output and can leak through if
/// downstream passes (p9) don't visit every node (e.g., nullable primitives
/// at the depth limit are returned early).
fn strip_ref_meta_keywords(schema: &mut Value) {
    match schema {
        Value::Object(obj) => {
            for keyword in REF_META_KEYWORDS {
                obj.remove(*keyword);
            }
            // Recurse into children, but skip literal values that might contain
            // objects with these keys as actual customer data, as well as vendor
            // extensions which may carry arbitrary payloads.
            for (k, v) in obj {
                if !["const", "enum", "examples", "default"].contains(&k.as_str())
                    && !k.starts_with("x-")
                {
                    strip_ref_meta_keywords(v);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                strip_ref_meta_keywords(item);
            }
        }
        _ => {}
    }
}

/// Merge a resolved `$ref` definition with its structural siblings.
///
/// Sibling annotations (description, title, etc.) override the def's values.
/// Structural siblings are merged via last-wins (siblings take precedence),
/// matching JSON Schema's `$ref`-with-siblings semantics from 2019-09+.
fn merge_ref_with_siblings(resolved: Value, siblings: Value) -> Value {
    let mut base = match resolved {
        Value::Object(o) => o,
        Value::Bool(true) => serde_json::Map::new(),
        Value::Bool(false) => {
            let mut m = serde_json::Map::new();
            m.insert("not".to_string(), serde_json::json!({}));
            m
        }
        _ => return siblings, // Invalid schema fallback
    };
    let Value::Object(sibling_map) = siblings else {
        return Value::Object(base);
    };

    // Sibling keywords generally override the base (e.g. annotations).
    // TODO(#255): Composition keywords (allOf, anyOf, oneOf) are currently
    // flat-overwritten (last-wins). If both def and siblings contribute
    // these, the def's variants are lost. This hasn't been observed in
    // real-world fixtures yet, but should be addressed if it manifests.
    // However, for structural object definition keywords (`properties`, `required`),
    // we perform a deep merge to preserve the base structure rather than clobbering it.
    for (k, v) in sibling_map {
        if k == "properties" {
            if let (Some(Value::Object(base_props)), Value::Object(sib_props)) =
                (base.get_mut("properties"), &v)
            {
                for (pk, pv) in sib_props {
                    base_props.insert(pk.clone(), pv.clone());
                }
                continue;
            }
        } else if k == "required" {
            if let (Some(Value::Array(base_req)), Value::Array(sib_req)) =
                (base.get_mut("required"), &v)
            {
                for req in sib_req {
                    if !base_req.contains(req) {
                        base_req.push(req.clone());
                    }
                }
                continue;
            }
        }

        // Default: flat overwrite (last-wins for annotations, type, etc.)
        base.insert(k, v);
    }

    Value::Object(base)
}

/// Remove `$defs` from the root schema if present.
fn strip_defs(mut schema: Value) -> Value {
    if let Some(obj) = schema.as_object_mut() {
        obj.remove("$defs");
    }
    schema
}

/// Check if any `$ref` nodes remain in the schema (excluding `$defs`).
fn has_remaining_refs(schema: &Value) -> bool {
    match schema {
        Value::Object(obj) => {
            for (key, value) in obj {
                if key == "$defs" {
                    continue;
                }
                if key == "$ref" {
                    return true;
                }
                if has_remaining_refs(value) {
                    return true;
                }
            }
            false
        }
        Value::Array(arr) => arr.iter().any(has_remaining_refs),
        _ => false,
    }
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
        let result = break_recursion(schema, &config).unwrap();

        // After recursion_limit=1: first expansion of B inlines B's content.
        // Inside that inlined B, the ref to A hits limit (A→B→A), so A becomes opaque string.
        // The schema should have no $ref nodes remaining.
        assert!(
            !serde_json::to_string(&result.schema)
                .unwrap()
                .contains("\"$ref\""),
            "Schema should have no remaining $ref nodes"
        );

        // At least one RecursiveInflate transform should be emitted
        assert!(
            result
                .transforms
                .iter()
                .any(|t| matches!(t, Transform::RecursiveInflate { .. })),
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
        let result = break_recursion(schema, &config).unwrap();

        assert!(
            !serde_json::to_string(&result.schema)
                .unwrap()
                .contains("\"$ref\""),
            "Schema should have no remaining $ref nodes"
        );
        assert!(
            result
                .transforms
                .iter()
                .any(|t| matches!(t, Transform::RecursiveInflate { .. })),
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
        let result = break_recursion(schema, &config).unwrap();

        assert!(
            !serde_json::to_string(&result.schema)
                .unwrap()
                .contains("\"$ref\""),
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
        let result_1 = break_recursion(schema.clone(), &config_with_limit(1)).unwrap();
        let count_1 = result_1
            .transforms
            .iter()
            .filter(|t| matches!(t, Transform::RecursiveInflate { .. }))
            .count();

        // With limit=3: Node expands three times before breaking.
        let result_3 = break_recursion(schema.clone(), &config_with_limit(3)).unwrap();
        let count_3 = result_3
            .transforms
            .iter()
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
            depth_3,
            depth_1
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

        let result = break_recursion(schema.clone(), &gemini_config()).unwrap();

        // Schema should be returned unchanged
        assert_eq!(
            result.schema, schema,
            "Gemini should pass schema through unchanged"
        );
        assert!(
            result.transforms.is_empty(),
            "Gemini should emit no transforms"
        );
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

        let result = break_recursion(schema.clone(), &default_config()).unwrap();

        assert_eq!(
            result.schema, schema,
            "Schema without refs should be unchanged"
        );
        assert!(
            result.transforms.is_empty(),
            "No transforms for refless schema"
        );
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
        let result = break_recursion(schema, &config).unwrap();

        let inflate = result
            .transforms
            .iter()
            .find_map(|t| {
                if let Transform::RecursiveInflate { path, original_ref } = t {
                    Some((path.clone(), original_ref.clone()))
                } else {
                    None
                }
            })
            .expect("Should have RecursiveInflate transform");

        // The original_ref should point to the $defs target
        assert_eq!(
            inflate.1, "#/$defs/Node",
            "original_ref should be the $defs target"
        );

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

        let result = break_recursion(schema, &default_config()).unwrap();

        // Both refs are non-recursive (DAG), should be fully inlined
        assert!(
            result.schema.get("$defs").is_none(),
            "$defs should be removed after full inlining"
        );
        assert!(
            !serde_json::to_string(&result.schema)
                .unwrap()
                .contains("\"$ref\""),
            "All $ref nodes should be inlined"
        );
        assert!(
            result.transforms.is_empty(),
            "DAG refs need no RecursiveInflate"
        );
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

        let result = break_recursion(schema, &default_config()).unwrap();

        // Both properties should have the Address schema inlined
        let billing = &result.schema["properties"]["billing"];
        assert_eq!(
            billing["type"], "object",
            "billing should have inlined Address"
        );
        assert!(
            billing.get("properties").is_some(),
            "billing should have properties"
        );

        let shipping = &result.schema["properties"]["shipping"];
        assert_eq!(
            shipping["type"], "object",
            "shipping should have inlined Address"
        );

        // No $ref remaining, no transforms needed
        assert!(
            !serde_json::to_string(&result.schema)
                .unwrap()
                .contains("\"$ref\""),
            "All $ref nodes should be inlined"
        );
        assert!(
            result.transforms.is_empty(),
            "DAG refs need no RecursiveInflate"
        );
    }

    // -----------------------------------------------------------------------
    // Test 10: Rehydration round-trip (RecursiveInflate)
    // -----------------------------------------------------------------------
    // This test lives in rehydrator.rs — see test_recursive_inflate_rehydration

    // -----------------------------------------------------------------------
    // strip_ref_meta_keywords unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_strip_meta_keywords_basic() {
        let mut schema = json!({
            "type": "object",
            "$anchor": "foo",
            "$id": "urn:x",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$dynamicRef": "#bar",
            "$dynamicAnchor": "bar",
            "properties": { "x": { "type": "string" } }
        });
        strip_ref_meta_keywords(&mut schema);
        let obj = schema.as_object().unwrap();
        assert!(obj.get("$anchor").is_none());
        assert!(obj.get("$id").is_none());
        assert!(obj.get("$schema").is_none());
        assert!(obj.get("$dynamicRef").is_none());
        assert!(obj.get("$dynamicAnchor").is_none());
        // Structural keywords preserved
        assert_eq!(obj.get("type").unwrap(), "object");
        assert!(obj.get("properties").is_some());
    }

    #[test]
    fn test_strip_meta_keywords_preserves_const() {
        let mut schema = json!({
            "const": { "$id": "should-remain", "$anchor": "keep" }
        });
        strip_ref_meta_keywords(&mut schema);
        let c = schema.get("const").unwrap().as_object().unwrap();
        assert!(c.get("$id").is_some(), "const payload should be untouched");
        assert!(
            c.get("$anchor").is_some(),
            "const payload should be untouched"
        );
    }

    #[test]
    fn test_strip_meta_keywords_preserves_enum() {
        let mut schema = json!({
            "enum": [{ "$id": "keep-me" }]
        });
        strip_ref_meta_keywords(&mut schema);
        let arr = schema.get("enum").unwrap().as_array().unwrap();
        assert!(arr[0].as_object().unwrap().get("$id").is_some());
    }

    #[test]
    fn test_strip_meta_keywords_preserves_vendor_extensions() {
        let mut schema = json!({
            "type": "string",
            "x-metadata": { "$id": "vendor-value", "$anchor": "ext" }
        });
        strip_ref_meta_keywords(&mut schema);
        let ext = schema.get("x-metadata").unwrap().as_object().unwrap();
        assert!(
            ext.get("$id").is_some(),
            "vendor ext payload should be untouched"
        );
        assert!(
            ext.get("$anchor").is_some(),
            "vendor ext payload should be untouched"
        );
    }

    // -----------------------------------------------------------------------
    // merge_ref_with_siblings unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_merge_siblings_deep_merge_properties() {
        let resolved = json!({
            "type": "object",
            "properties": {
                "base_prop": { "type": "string" }
            },
            "required": ["base_prop"]
        });
        let siblings = json!({
            "properties": {
                "sibling_prop": { "type": "integer" }
            },
            "required": ["sibling_prop"]
        });
        let merged = merge_ref_with_siblings(resolved, siblings);
        let obj = merged.as_object().unwrap();
        let props = obj.get("properties").unwrap().as_object().unwrap();
        assert!(props.contains_key("base_prop"), "base property preserved");
        assert!(props.contains_key("sibling_prop"), "sibling property added");
        let req: Vec<&str> = obj
            .get("required")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(req.contains(&"base_prop"));
        assert!(req.contains(&"sibling_prop"));
    }

    #[test]
    fn test_merge_siblings_required_dedup() {
        let resolved = json!({ "required": ["a", "b"] });
        let siblings = json!({ "required": ["b", "c"] });
        let merged = merge_ref_with_siblings(resolved, siblings);
        let req: Vec<&str> = merged
            .get("required")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(req.len(), 3, "should deduplicate 'b'");
        assert!(req.contains(&"a"));
        assert!(req.contains(&"b"));
        assert!(req.contains(&"c"));
    }

    #[test]
    fn test_merge_siblings_bool_true_resolved() {
        let resolved = json!(true); // unconstrained schema
        let siblings = json!({ "description": "overridden" });
        let merged = merge_ref_with_siblings(resolved, siblings);
        assert_eq!(merged.get("description").unwrap(), "overridden");
    }

    #[test]
    fn test_merge_siblings_bool_false_resolved() {
        let resolved = json!(false); // nothing-matches schema
        let siblings = json!({ "description": "fallback" });
        let merged = merge_ref_with_siblings(resolved, siblings);
        // false → {"not": {}} + sibling description
        assert!(merged.get("not").is_some(), "false should expand to not:{{}}");
        assert_eq!(merged.get("description").unwrap(), "fallback");
    }
}
