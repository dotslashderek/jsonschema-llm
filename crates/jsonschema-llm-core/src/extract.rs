//! Schema component extraction.
//!
//! Extracts a single component from a `$defs`-heavy schema by JSON Pointer
//! (RFC 6901), transitively resolving all reachable `$ref` dependencies and
//! producing a self-contained sub-schema.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use jsonschema_llm_core::{extract_component, ExtractOptions};
//! use serde_json::json;
//!
//! let schema = json!({
//!     "$defs": {
//!         "Pet": {
//!             "type": "object",
//!             "properties": {
//!                 "name": { "type": "string" },
//!                 "tag": { "$ref": "#/$defs/Tag" }
//!             }
//!         },
//!         "Tag": { "type": "object", "properties": { "id": { "type": "integer" } } }
//!     }
//! });
//!
//! let result = extract_component(&schema, "#/$defs/Pet", &ExtractOptions::default()).unwrap();
//! assert_eq!(result.dependency_count, 1);
//! assert!(result.schema["$defs"]["Tag"].is_object());
//! ```

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::ConvertError;
use crate::schema_utils::resolve_pointer;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Options for component extraction.
#[derive(Debug, Clone, Default)]
pub struct ExtractOptions {
    /// Maximum DFS depth for transitive dependency resolution.
    /// `None` means unbounded (default).
    pub max_depth: Option<usize>,
}

/// Result of a successful component extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractResult {
    /// The self-contained extracted schema.
    pub schema: Value,
    /// The JSON Pointer used for extraction.
    pub pointer: String,
    /// Number of unique transitive dependencies pulled in.
    pub dependency_count: usize,
    /// Pointers of transitive deps that could not be resolved locally.
    /// These `$ref` values are left as-is in the output; callers decide how to handle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_refs: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract a single component from a schema by JSON Pointer.
///
/// Resolves the target pointer, performs a DFS transitive closure over all
/// reachable `$ref` dependencies, tree-shakes the result to include only
/// discovered definitions, and rewrites `$ref` pointers to the new namespace.
///
/// # Arguments
///
/// * `schema` - The source JSON Schema document
/// * `pointer` - RFC 6901 JSON Pointer to the component (e.g., `#/$defs/Pet`
///   or `#/components/schemas/Pet`)
/// * `options` - Extraction options (depth limit)
///
/// # Returns
///
/// An [`ExtractResult`] with the self-contained schema, or a [`ConvertError`]
/// if the target pointer does not exist or a reachable `$ref` is external.
///
/// Unresolvable **transitive** deps are reported in [`ExtractResult::missing_refs`]
/// and left as dangling `$ref` values in the output (soft-fail).
pub fn extract_component(
    schema: &Value,
    pointer: &str,
    options: &ExtractOptions,
) -> Result<ExtractResult, ConvertError> {
    // Phase 1: Resolve the target pointer (hard error if missing).
    let target = resolve_pointer(schema, pointer).ok_or_else(|| ConvertError::UnresolvableRef {
        path: pointer.to_string(),
        reference: pointer.to_string(),
    })?;
    let target = target.clone();

    // Phase 2: Transitive closure — DFS to collect all reachable deps.
    let max_depth = options.max_depth.unwrap_or(usize::MAX);
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(pointer.to_string());

    // deps: pointer → (key, resolved_value). BTreeMap for deterministic output.
    let mut deps: BTreeMap<String, (String, Value)> = BTreeMap::new();
    let mut missing_refs: Vec<String> = Vec::new();

    collect_deps(
        &target,
        schema,
        pointer,
        0,
        max_depth,
        &mut visited,
        &mut deps,
        &mut missing_refs,
    )?;

    let dependency_count = deps.len();

    // Phase 3: Tree-shaking — build new root + $defs.
    // The target node becomes the root schema; deps become $defs.
    // Build a ref-rewrite map: old pointer → new "#/$defs/<key>" reference.
    let rewrite_map: BTreeMap<String, String> = deps
        .iter()
        .map(|(ptr, (key, _))| (ptr.clone(), format!("#/$defs/{}", key)))
        .collect();

    // Phase 4: Rewrite refs in the target node and all dep nodes.
    let mut root = rewrite_refs(target, &rewrite_map);

    // Phase 5: Assemble $defs if there are any deps.
    if !deps.is_empty() {
        let mut defs_map = Map::new();
        for (_ptr, (key, value)) in deps {
            let rewritten = rewrite_refs(value, &rewrite_map);
            defs_map.insert(key, rewritten);
        }
        if let Value::Object(ref mut obj) = root {
            obj.insert("$defs".to_string(), Value::Object(defs_map));
        }
    }

    // Sort missing_refs for deterministic output.
    missing_refs.sort();
    missing_refs.dedup();

    Ok(ExtractResult {
        schema: root,
        pointer: pointer.to_string(),
        dependency_count,
        missing_refs,
    })
}

// ---------------------------------------------------------------------------
// DFS transitive closure
// ---------------------------------------------------------------------------

/// DFS walker: collects all reachable `$ref` dependencies from `node`.
///
/// For each `$ref` encountered:
/// - External refs (not starting with `#`) → hard error
/// - Already visited → skip (cycle break)
/// - Unresolvable local ref → record in `missing_refs`, leave as-is (soft-fail)
/// - Resolvable → compute key, add to `deps`, recurse
#[allow(clippy::too_many_arguments)]
fn collect_deps(
    node: &Value,
    root_schema: &Value,
    current_path: &str,
    depth: usize,
    max_depth: usize,
    visited: &mut HashSet<String>,
    deps: &mut BTreeMap<String, (String, Value)>,
    missing_refs: &mut Vec<String>,
) -> Result<(), ConvertError> {
    if depth > max_depth {
        return Err(ConvertError::RecursionDepthExceeded {
            path: current_path.to_string(),
            max_depth,
        });
    }

    match node {
        Value::Object(obj) => {
            // Check for $ref at this node.
            if let Some(ref_val) = obj.get("$ref").and_then(Value::as_str) {
                // Hard error on external refs.
                if !ref_val.starts_with('#') {
                    return Err(ConvertError::UnsupportedFeature {
                        path: current_path.to_string(),
                        feature: format!("external $ref: {}", ref_val),
                    });
                }

                // Skip if already visited (cycle break).
                if visited.contains(ref_val) {
                    return Ok(());
                }

                // Attempt to resolve.
                match resolve_pointer(root_schema, ref_val) {
                    None => {
                        // Soft-fail: record as missing, leave $ref dangling.
                        missing_refs.push(ref_val.to_string());
                    }
                    Some(resolved) => {
                        let key = pointer_to_key(ref_val, deps);
                        let resolved_clone = resolved.clone();
                        visited.insert(ref_val.to_string());
                        deps.insert(ref_val.to_string(), (key, resolved_clone.clone()));
                        collect_deps(
                            &resolved_clone,
                            root_schema,
                            ref_val,
                            depth + 1,
                            max_depth,
                            visited,
                            deps,
                            missing_refs,
                        )?;
                    }
                }
                // Don't recurse into siblings of a $ref node — the ref IS the schema.
                return Ok(());
            }

            // No $ref — recurse into all values.
            for (key, val) in obj {
                let child_path = format!("{}/{}", current_path, key);
                collect_deps(
                    val,
                    root_schema,
                    &child_path,
                    depth + 1,
                    max_depth,
                    visited,
                    deps,
                    missing_refs,
                )?;
            }
        }
        Value::Array(arr) => {
            for (i, val) in arr.iter().enumerate() {
                let child_path = format!("{}/{}", current_path, i);
                collect_deps(
                    val,
                    root_schema,
                    &child_path,
                    depth + 1,
                    max_depth,
                    visited,
                    deps,
                    missing_refs,
                )?;
            }
        }
        _ => {}
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Key computation
// ---------------------------------------------------------------------------

/// Compute a canonical, collision-safe key for a JSON Pointer.
///
/// Strategy:
/// 1. Always try the **last segment** first (e.g. `#/components/schemas/Tag` → `"Tag"`)
/// 2. If that key is **already taken** by a different pointer, fall back to the
///    full-path sanitized form (join all segments with `_`)
///
/// All segment strings are RFC 6901 unescaped before use.
fn pointer_to_key(pointer: &str, deps: &BTreeMap<String, (String, Value)>) -> String {
    let stripped = pointer.strip_prefix('#').unwrap_or(pointer);
    let segments: Vec<&str> = stripped.split('/').filter(|s| !s.is_empty()).collect();

    // Preferred: last segment.
    let last = segments
        .last()
        .map(|s| unescape_segment(s))
        .unwrap_or_else(|| "root".to_string());

    // Check for collision: is this key already used by a DIFFERENT pointer?
    let collision = deps
        .iter()
        .any(|(existing_ptr, (existing_key, _))| existing_key == &last && existing_ptr != pointer);

    if collision {
        // Full-path sanitized key as tiebreak.
        segments
            .iter()
            .map(|s| unescape_segment(s))
            .collect::<Vec<_>>()
            .join("_")
    } else {
        last
    }
}

/// Unescape a single RFC 6901 pointer segment.
fn unescape_segment(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

// ---------------------------------------------------------------------------
// Ref rewriting
// ---------------------------------------------------------------------------

/// Walk a schema value and rewrite all `$ref` strings using the provided map.
fn rewrite_refs(value: Value, rewrite_map: &BTreeMap<String, String>) -> Value {
    match value {
        Value::Object(mut obj) => {
            // Rewrite $ref if present and in the map.
            if let Some(Value::String(ref_str)) = obj.get("$ref").cloned() {
                if let Some(new_ref) = rewrite_map.get(&ref_str) {
                    obj.insert("$ref".to_string(), Value::String(new_ref.clone()));
                }
            }
            let rewritten: Map<String, Value> = obj
                .into_iter()
                .map(|(k, v)| (k, rewrite_refs(v, rewrite_map)))
                .collect();
            Value::Object(rewritten)
        }
        Value::Array(arr) => Value::Array(
            arr.into_iter()
                .map(|v| rewrite_refs(v, rewrite_map))
                .collect(),
        ),
        other => other,
    }
}

// ===========================================================================
// Tests (TDD — written before implementation)
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConvertOptions;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn opts() -> ExtractOptions {
        ExtractOptions::default()
    }

    // -----------------------------------------------------------------------
    // AC1: extract_component() produces valid, self-contained JSON Schema
    // -----------------------------------------------------------------------

    #[test]
    fn test_simple_extract_no_deps() {
        let schema = json!({
            "$defs": {
                "Foo": { "type": "string", "minLength": 1 },
                "Bar": { "type": "integer" }
            }
        });

        let result = extract_component(&schema, "#/$defs/Foo", &opts()).unwrap();

        assert_eq!(result.schema, json!({ "type": "string", "minLength": 1 }));
        assert_eq!(result.dependency_count, 0);
        assert!(result.missing_refs.is_empty());
        assert!(
            result.schema.get("$defs").is_none(),
            "empty $defs should be omitted"
        );
    }

    #[test]
    fn test_extract_with_direct_dep() {
        let schema = json!({
            "$defs": {
                "A": {
                    "type": "object",
                    "properties": {
                        "b": { "$ref": "#/$defs/B" }
                    }
                },
                "B": { "type": "integer" }
            }
        });

        let result = extract_component(&schema, "#/$defs/A", &opts()).unwrap();

        assert_eq!(result.dependency_count, 1);
        assert!(result.schema["$defs"]["B"].is_object());
        assert_eq!(result.schema["$defs"]["B"], json!({ "type": "integer" }));
        // $ref should be rewritten
        assert_eq!(result.schema["properties"]["b"]["$ref"], json!("#/$defs/B"));
        assert!(result.missing_refs.is_empty());
    }

    #[test]
    fn test_transitive_deps() {
        let schema = json!({
            "$defs": {
                "A": { "type": "object", "properties": { "b": { "$ref": "#/$defs/B" } } },
                "B": { "type": "object", "properties": { "c": { "$ref": "#/$defs/C" } } },
                "C": { "type": "string" }
            }
        });

        let result = extract_component(&schema, "#/$defs/A", &opts()).unwrap();

        assert_eq!(result.dependency_count, 2);
        assert!(result.schema["$defs"]["B"].is_object());
        assert!(result.schema["$defs"]["C"].is_object());
        assert!(result.missing_refs.is_empty());
    }

    #[test]
    fn test_deep_nesting_5_levels() {
        let schema = json!({
            "$defs": {
                "A": { "$ref": "#/$defs/B" },
                "B": { "$ref": "#/$defs/C" },
                "C": { "$ref": "#/$defs/D" },
                "D": { "$ref": "#/$defs/E" },
                "E": { "$ref": "#/$defs/F" },
                "F": { "type": "string" }
            }
        });

        // Extract A — should resolve chain B→C→D→E→F
        let result = extract_component(&schema, "#/$defs/A", &opts()).unwrap();
        // A itself just contains $ref to B (as root), so deps should include B,C,D,E,F
        assert_eq!(result.dependency_count, 5);
        assert!(result.missing_refs.is_empty());
    }

    // -----------------------------------------------------------------------
    // AC3: Circular refs handled without panic or infinite loop
    // -----------------------------------------------------------------------

    #[test]
    fn test_circular_refs_no_panic() {
        let schema = json!({
            "$defs": {
                "A": {
                    "type": "object",
                    "properties": {
                        "b": { "$ref": "#/$defs/B" }
                    }
                },
                "B": {
                    "type": "object",
                    "properties": {
                        "a": { "$ref": "#/$defs/A" }
                    }
                }
            }
        });

        // Must not panic or loop forever
        let result = extract_component(&schema, "#/$defs/A", &opts()).unwrap();

        // Both A and B should be discovered
        assert!(result.dependency_count >= 1);
        assert!(result.missing_refs.is_empty());
    }

    #[test]
    fn test_self_referencing_no_panic() {
        let schema = json!({
            "$defs": {
                "Node": {
                    "type": "object",
                    "properties": {
                        "child": { "$ref": "#/$defs/Node" }
                    }
                }
            }
        });

        let result = extract_component(&schema, "#/$defs/Node", &opts()).unwrap();
        // Node refs itself — visited set should break the cycle
        assert!(result.missing_refs.is_empty());
    }

    // -----------------------------------------------------------------------
    // AC4: External $ref → ConvertError::UnsupportedFeature
    // -----------------------------------------------------------------------

    #[test]
    fn test_external_ref_error() {
        let schema = json!({
            "$defs": {
                "Pet": {
                    "type": "object",
                    "properties": {
                        "tag": { "$ref": "http://example.com/schemas/Tag" }
                    }
                }
            }
        });

        let err = extract_component(&schema, "#/$defs/Pet", &opts()).unwrap_err();
        match err {
            ConvertError::UnsupportedFeature { feature, .. } => {
                assert!(feature.contains("external $ref"), "got: {}", feature);
            }
            other => panic!("expected UnsupportedFeature, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Target pointer not found → hard error
    // -----------------------------------------------------------------------

    #[test]
    fn test_missing_pointer_error() {
        let schema = json!({ "$defs": { "Foo": { "type": "string" } } });

        let err = extract_component(&schema, "#/$defs/DoesNotExist", &opts()).unwrap_err();
        match err {
            ConvertError::UnresolvableRef { reference, .. } => {
                assert!(reference.contains("DoesNotExist"), "got: {}", reference);
            }
            other => panic!("expected UnresolvableRef, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Soft-fail: transitive dep missing → missing_refs, not error
    // -----------------------------------------------------------------------

    #[test]
    fn test_partial_extraction_missing_dep() {
        let schema = json!({
            "$defs": {
                "A": {
                    "type": "object",
                    "properties": {
                        "b": { "$ref": "#/$defs/B" }
                    }
                }
                // B is intentionally missing
            }
        });

        let result = extract_component(&schema, "#/$defs/A", &opts()).unwrap();

        // A should be extracted without error
        assert!(result.schema["properties"]["b"]["$ref"].is_string());
        // B should appear in missing_refs
        assert_eq!(result.missing_refs, vec!["#/$defs/B"]);
        // dependency_count counts only resolved deps
        assert_eq!(result.dependency_count, 0);
    }

    // -----------------------------------------------------------------------
    // OAS-style path: #/components/schemas/...
    // -----------------------------------------------------------------------

    #[test]
    fn test_oas_style_path() {
        let schema = json!({
            "components": {
                "schemas": {
                    "Pet": {
                        "type": "object",
                        "properties": {
                            "tag": { "$ref": "#/components/schemas/Tag" }
                        }
                    },
                    "Tag": { "type": "object", "properties": { "id": { "type": "integer" } } }
                }
            }
        });

        let result = extract_component(&schema, "#/components/schemas/Pet", &opts()).unwrap();

        assert_eq!(result.dependency_count, 1);
        // Tag should be in $defs
        assert!(result.schema["$defs"]["Tag"].is_object());
        // $ref should be rewritten to #/$defs/Tag
        assert_eq!(
            result.schema["properties"]["tag"]["$ref"],
            json!("#/$defs/Tag")
        );
        assert!(result.missing_refs.is_empty());
    }

    // -----------------------------------------------------------------------
    // Deterministic output
    // -----------------------------------------------------------------------

    #[test]
    fn test_deterministic_output() {
        let schema = json!({
            "$defs": {
                "A": {
                    "anyOf": [
                        { "$ref": "#/$defs/B" },
                        { "$ref": "#/$defs/C" },
                        { "$ref": "#/$defs/D" }
                    ]
                },
                "B": { "type": "integer" },
                "C": { "type": "string" },
                "D": { "type": "boolean" }
            }
        });

        // Run 3 times, results must be identical
        let r1 = extract_component(&schema, "#/$defs/A", &opts()).unwrap();
        let r2 = extract_component(&schema, "#/$defs/A", &opts()).unwrap();
        let r3 = extract_component(&schema, "#/$defs/A", &opts()).unwrap();

        let s1 = serde_json::to_string(&r1.schema).unwrap();
        let s2 = serde_json::to_string(&r2.schema).unwrap();
        let s3 = serde_json::to_string(&r3.schema).unwrap();

        assert_eq!(s1, s2, "round 1 vs 2 differ");
        assert_eq!(s2, s3, "round 2 vs 3 differ");

        // $defs keys must be sorted (B < C < D)
        let defs = r1.schema["$defs"].as_object().unwrap();
        let keys: Vec<&str> = defs.keys().map(String::as_str).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "$defs keys not sorted");
    }

    // -----------------------------------------------------------------------
    // AC2: Extracted schema can be passed to convert()
    // -----------------------------------------------------------------------

    #[test]
    fn test_extracted_passes_convert() {
        let schema = json!({
            "$defs": {
                "Pet": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "tag": { "$ref": "#/$defs/Tag" }
                    },
                    "required": ["name"]
                },
                "Tag": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer" }
                    }
                }
            }
        });

        let result = extract_component(&schema, "#/$defs/Pet", &opts()).unwrap();
        let convert_result = crate::convert(&result.schema, &ConvertOptions::default());
        assert!(
            convert_result.is_ok(),
            "convert() failed on extracted schema: {:?}",
            convert_result.unwrap_err()
        );
    }

    // -----------------------------------------------------------------------
    // Root-level extraction (#)
    // -----------------------------------------------------------------------

    #[test]
    fn test_schema_with_no_defs_root_extraction() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });

        let result = extract_component(&schema, "#", &opts()).unwrap();
        assert_eq!(result.dependency_count, 0);
        assert!(result.schema["properties"]["name"].is_object());
        assert!(result.schema.get("$defs").is_none());
    }

    // -----------------------------------------------------------------------
    // max_depth exceeded
    // -----------------------------------------------------------------------

    #[test]
    fn test_max_depth_exceeded() {
        let schema = json!({
            "$defs": {
                "A": { "properties": { "b": { "$ref": "#/$defs/B" } } },
                "B": { "properties": { "c": { "$ref": "#/$defs/C" } } },
                "C": { "properties": { "d": { "$ref": "#/$defs/D" } } },
                "D": { "properties": { "e": { "$ref": "#/$defs/E" } } },
                "E": { "type": "string" }
            }
        });

        let limited = ExtractOptions { max_depth: Some(1) };
        let err = extract_component(&schema, "#/$defs/A", &limited).unwrap_err();
        match err {
            ConvertError::RecursionDepthExceeded { max_depth, .. } => {
                assert_eq!(max_depth, 1);
            }
            other => panic!("expected RecursionDepthExceeded, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Key collision: two pointers with same last segment
    // -----------------------------------------------------------------------

    #[test]
    fn test_ref_rewrite_name_collision() {
        // #/components/schemas/Name and #/components/parameters/Name both referenced
        let schema = json!({
            "components": {
                "schemas": {
                    "Pet": {
                        "type": "object",
                        "properties": {
                            "a": { "$ref": "#/components/schemas/Name" },
                            "b": { "$ref": "#/components/parameters/Name" }
                        }
                    },
                    "Name": { "type": "string" }
                },
                "parameters": {
                    "Name": { "type": "integer" }
                }
            }
        });

        let result = extract_component(&schema, "#/components/schemas/Pet", &opts()).unwrap();

        // Both deps were collected
        assert_eq!(result.dependency_count, 2);
        // Keys must be distinct (no silent overwrite)
        let defs = result.schema["$defs"].as_object().unwrap();
        assert_eq!(
            defs.len(),
            2,
            "expected 2 distinct $defs keys, got: {:?}",
            defs.keys().collect::<Vec<_>>()
        );
        assert!(result.missing_refs.is_empty());
    }
}
