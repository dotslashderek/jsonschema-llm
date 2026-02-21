//! Schema component extraction.
//!
//! Extracts a single component from a `$defs`-heavy schema by JSON Pointer
//! (RFC 6901), transitively resolving all reachable `$ref` dependencies and
//! producing a self-contained sub-schema.
//!
//! Supports `$anchor`-style fragment references (e.g., `"#stepId"`) by
//! building an anchor map via [`crate::anchor_utils::build_anchor_map`].
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
use crate::schema_utils::{escape_pointer_segment, resolve_pointer};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Options for component extraction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "kebab-case", default)]
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

/// Enumerate all extractable component JSON Pointers in a schema.
///
/// Walks the schema recursively, discovering components in:
/// - `$defs` (JSON Schema Draft 2019-09+)
/// - `definitions` (legacy Draft 4-7)
/// - `components/schemas` at root level only (OpenAPI Specification)
///
/// Returns a sorted, deduplicated list of JSON Pointer strings (RFC 6901).
///
/// # Notes
///
/// - OAS `components/schemas` is only detected at the document root (`#`) to
///   avoid false positives when user schemas happen to have properties named
///   `components`.
/// - Keys are RFC 6901-escaped via [`crate::schema_utils::escape_pointer_segment`].
/// - Nested `$defs` inside `$defs` entries are discovered with the correct
///   full path (e.g. `#/$defs/Outer/$defs/Inner`).
///
/// # Example
///
/// ```rust,no_run
/// use jsonschema_llm_core::list_components;
/// use serde_json::json;
///
/// let schema = json!({ "$defs": { "Pet": { "type": "object" } } });
/// assert_eq!(list_components(&schema), vec!["#/$defs/Pet"]);
/// ```
pub fn list_components(schema: &Value) -> Vec<String> {
    let mut pointers: Vec<String> = Vec::new();
    collect_components(schema, "#", true, &mut pointers);
    pointers.sort();
    pointers.dedup();
    pointers
}

/// Recursive helper for [`list_components`].
///
/// * `node`        — current schema node
/// * `path`        — JSON Pointer prefix for this node (e.g. `"#"` or `"#/$defs/Outer"`)
/// * `is_root`     — true only when `path == "#"` (gates OAS `components/schemas` detection)
/// * `out`         — accumulator for discovered pointers
fn collect_components(node: &Value, path: &str, is_root: bool, out: &mut Vec<String>) {
    match node {
        // ------------------------------------------------------------------
        // Array: recurse into each element (handles allOf/anyOf/oneOf/items)
        // ------------------------------------------------------------------
        Value::Array(arr) => {
            for (i, elem) in arr.iter().enumerate() {
                let child_path = format!("{}/{}", path, i);
                collect_components(elem, &child_path, false, out);
            }
        }

        // ------------------------------------------------------------------
        // Object: enumerate $defs/definitions/components/schemas and recurse
        // ------------------------------------------------------------------
        Value::Object(obj) => {
            // ---------------------------------------------------------------
            // Detect `$defs` / `definitions` map-of-components at this node
            // ---------------------------------------------------------------
            for keyword in ["$defs", "definitions"] {
                if let Some(Value::Object(defs)) = obj.get(keyword) {
                    for key in defs.keys() {
                        let escaped = escape_pointer_segment(key);
                        let ptr = format!("{}/{}/{}", path, keyword, escaped);
                        out.push(ptr.clone());
                        // Recurse into this component's schema (for nested $defs).
                        // The component itself is NOT the root — no OAS detection inside it.
                        collect_components(&defs[key], &ptr, false, out);
                    }
                }
            }

            // ---------------------------------------------------------------
            // Detect OAS `components/schemas` — ONLY at the document root
            // ---------------------------------------------------------------
            if is_root {
                if let Some(Value::Object(components)) = obj.get("components") {
                    if let Some(Value::Object(schemas)) = components.get("schemas") {
                        for key in schemas.keys() {
                            let escaped = escape_pointer_segment(key);
                            let ptr = format!("#/components/schemas/{}", escaped);
                            out.push(ptr.clone());
                            // Recurse for nested $defs inside OAS component schemas.
                            collect_components(&schemas[key], &ptr, false, out);
                        }
                    }
                }
            }

            // ---------------------------------------------------------------
            // Recurse into all remaining object values.
            // Skip keys already processed above to avoid double-counting.
            // ---------------------------------------------------------------
            for (key, val) in obj {
                if key == "$defs" || key == "definitions" {
                    // Already enumerated component keys above; recursion into each
                    // component's body is handled in the enumeration loop above.
                    continue;
                }
                if is_root && key == "components" {
                    // Already handled OAS components/schemas above.
                    continue;
                }
                let child_path = format!("{}/{}", path, escape_pointer_segment(key));
                collect_components(val, &child_path, false, out);
            }
        }

        // All other Value variants (String, Number, Bool, Null) cannot contain $defs.
        _ => {}
    }
}

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

    // Phase 1b: Build resolver engine for $anchor/$id resolution.
    let resolver = crate::resolver::ResolverEngine::new(schema)?;

    // Phase 2: Transitive closure — DFS to collect all reachable deps.
    let mut ctx = DfsCtx {
        root_schema: schema,
        max_depth: options.max_depth.unwrap_or(usize::MAX),
        visited: HashSet::new(),
        deps: BTreeMap::new(),
        missing_refs: Vec::new(),
        resolver: &resolver,
        base_uri: resolver.base_uri().clone(),
    };
    ctx.visited.insert(pointer.to_string());

    collect_deps(&target, pointer, 0, &mut ctx)?;

    let DfsCtx {
        deps,
        mut missing_refs,
        ..
    } = ctx;

    let dependency_count = deps.len();

    // Phase 3: Tree-shaking — build new root + $defs.
    // The target node becomes the root schema; deps become $defs.
    // Build a ref-rewrite map: old pointer → new "#/$defs/<key>" reference.
    // IMPORTANT: re-escape the key for use in a JSON Pointer.
    let rewrite_map: BTreeMap<String, String> = deps
        .iter()
        .map(|(ptr, (key, _))| {
            let escaped_key = escape_pointer_segment(key);
            (ptr.clone(), format!("#/$defs/{}", escaped_key))
        })
        .collect();

    // Phase 3b: Also add anchor-ref → new-pointer entries to the rewrite map
    // so that anchor-style $refs (e.g., "#stepId") get rewritten in the output.
    let mut full_rewrite_map = rewrite_map.clone();
    for (uri, pointer_path) in resolver.anchor_map() {
        if let Some(new_ref) = rewrite_map.get(pointer_path) {
            // Extract the fragment from the URI (e.g., "#stepId").
            if let Ok(parsed) = url::Url::parse(uri) {
                if let Some(fragment) = parsed.fragment() {
                    let anchor_ref = format!("#{}", fragment);
                    full_rewrite_map.insert(anchor_ref, new_ref.clone());
                }
            }
            // Also add the full URI as a rewrite target for URI-style refs.
            full_rewrite_map.insert(uri.clone(), new_ref.clone());
            // And the relative form (e.g., "nested.json#foo").
            // We compute this by stripping the base from the URI.
        }
    }

    // Phase 4: Rewrite refs in the target node and all dep nodes.
    let mut root = crate::schema_walker::rewrite_refs(target, &full_rewrite_map)?;

    // Phase 5: Assemble $defs if there are any deps.
    if !deps.is_empty() {
        let mut defs_map = Map::new();
        for (_ptr, (key, value)) in deps {
            let rewritten = crate::schema_walker::rewrite_refs(value, &full_rewrite_map)?;
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
// DFS context
// ---------------------------------------------------------------------------

/// Mutable + readonly context threaded through the DFS traversal.
struct DfsCtx<'a> {
    root_schema: &'a Value,
    max_depth: usize,
    visited: HashSet<String>,
    /// pointer → (key, resolved_value). `BTreeMap` for deterministic output.
    deps: BTreeMap<String, (String, Value)>,
    missing_refs: Vec<String>,
    /// Centralized resolver engine for $ref resolution.
    resolver: &'a crate::resolver::ResolverEngine,
    /// Current base URI for $id scoping during DFS.
    base_uri: url::Url,
}

// ---------------------------------------------------------------------------
// DFS transitive closure
// ---------------------------------------------------------------------------

/// DFS walker: collects all reachable `$ref` dependencies from `node`.
///
/// For each `$ref` encountered:
/// - External refs (not starting with `#`) → hard error
/// - Anchor-style refs (`#Foo`) → hard error
/// - Already visited → skip (cycle break), but still traverse siblings
/// - Unresolvable local ref → record in `missing_refs`, leave as-is (soft-fail)
/// - Resolvable → compute key, add to `deps`, recurse
fn collect_deps(
    node: &Value,
    current_path: &str,
    depth: usize,
    ctx: &mut DfsCtx<'_>,
) -> Result<(), ConvertError> {
    if depth > ctx.max_depth {
        return Err(ConvertError::RecursionDepthExceeded {
            path: current_path.to_string(),
            max_depth: ctx.max_depth,
        });
    }

    match node {
        Value::Object(obj) => {
            // Save base URI — $id scoping is lexical (per-subtree), not global.
            let saved_base = ctx.base_uri.clone();

            // Track $id for base URI scoping.
            if let Some(id_val) = obj.get("$id").and_then(Value::as_str) {
                if let Ok(new_base) = ctx.base_uri.join(id_val) {
                    ctx.base_uri = new_base;
                }
            }

            if let Some(ref_val) = obj.get("$ref").and_then(Value::as_str) {
                // Resolve via centralized ResolverEngine.
                let effective_ref = match ctx.resolver.resolve(ref_val, &ctx.base_uri) {
                    crate::resolver::ResolvedRef::Pointer(p) => p,
                    crate::resolver::ResolvedRef::Unresolvable(_) => {
                        // Soft-fail: external or unresolvable anchor refs are
                        // recorded as missing and left as-is.
                        ctx.missing_refs.push(ref_val.to_string());
                        ctx.base_uri = saved_base;
                        return Ok(());
                    }
                };

                let ref_val = effective_ref.as_str();

                // Skip if already visited (cycle break) — but still traverse siblings below.
                let already_visited = ctx.visited.contains(ref_val);

                if !already_visited {
                    // Attempt to resolve.
                    match resolve_pointer(ctx.root_schema, ref_val) {
                        None => {
                            // Soft-fail: record as missing, leave $ref dangling.
                            ctx.missing_refs.push(ref_val.to_string());
                        }
                        Some(resolved) => {
                            let key = pointer_to_key(ref_val, &ctx.deps);
                            let resolved_clone = resolved.clone();
                            ctx.visited.insert(ref_val.to_string());
                            ctx.deps
                                .insert(ref_val.to_string(), (key, resolved_clone.clone()));
                            // Only increment depth for $ref hops (not AST traversal).
                            collect_deps(&resolved_clone, ref_val, depth + 1, ctx)?;
                        }
                    }
                }

                // Always continue DFS into siblings (regardless of cycle detection).
                // JSON Schema Draft 2019-09+ allows schemas alongside $ref.
                for (key, val) in obj {
                    if key == "$ref" {
                        continue;
                    }
                    let child_path = format!("{}/{}", current_path, key);
                    // Do NOT increment depth here — sibling traversal is not a ref hop.
                    collect_deps(val, &child_path, depth, ctx)?;
                }
                ctx.base_uri = saved_base;
                return Ok(());
            }

            // No $ref — recurse into all values (depth unchanged: not a ref hop).
            for (key, val) in obj {
                let child_path = format!("{}/{}", current_path, key);
                collect_deps(val, &child_path, depth, ctx)?;
            }
            ctx.base_uri = saved_base;
        }
        Value::Array(arr) => {
            for (i, val) in arr.iter().enumerate() {
                let child_path = format!("{}/{}", current_path, i);
                // Depth unchanged: array traversal is not a ref hop.
                collect_deps(val, &child_path, depth, ctx)?;
            }
        }
        _ => {}
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Key computation
// ---------------------------------------------------------------------------

/// Compute a canonical, collision-safe, unique key for a JSON Pointer.
///
/// Strategy:
/// 1. Always try the **last segment** first (e.g. `#/components/schemas/Tag` → `"Tag"`)
/// 2. If that key is **already taken** by a different pointer, fall back to the
///    full-path sanitized form (join all segments with `_`)
/// 3. If the fallback ALSO collides (rare path ambiguity), append `_N` (N=2,3,...)
///    until unique.
///
/// All segment strings are RFC 6901 unescaped (for human-readable key names).
/// Callers must escape keys when building JSON Pointer strings.
fn pointer_to_key(pointer: &str, deps: &BTreeMap<String, (String, Value)>) -> String {
    let stripped = pointer.strip_prefix('#').unwrap_or(pointer);
    let segments: Vec<&str> = stripped.split('/').filter(|s| !s.is_empty()).collect();

    let last = segments
        .last()
        .map(|s| unescape_segment(s))
        .unwrap_or_else(|| "root".to_string());

    // Check for collision: key taken by a DIFFERENT pointer?
    let key_exists = |candidate: &str| {
        deps.iter()
            .any(|(ptr, (k, _))| k == candidate && ptr != pointer)
    };

    if !key_exists(&last) {
        return last;
    }

    // Fallback: full-path join.
    let joined = segments
        .iter()
        .map(|s| unescape_segment(s))
        .collect::<Vec<_>>()
        .join("_");

    if !key_exists(&joined) {
        return joined;
    }

    // Last resort: append numeric suffix until unique.
    let mut n = 2usize;
    loop {
        let candidate = format!("{}_{}", joined, n);
        if !key_exists(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Unescape a single RFC 6901 pointer segment.
fn unescape_segment(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

// ---------------------------------------------------------------------------
// Ref rewriting

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
    fn test_external_ref_soft_fail() {
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

        let result = extract_component(&schema, "#/$defs/Pet", &opts()).unwrap();
        assert!(
            result
                .missing_refs
                .iter()
                .any(|r| r.contains("example.com")),
            "expected external ref in missing_refs; got: {:?}",
            result.missing_refs
        );
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

    // -----------------------------------------------------------------------
    // list_components() — TDD gate tests (written before implementation)
    // -----------------------------------------------------------------------

    #[test]
    fn test_list_components_empty_schema() {
        let schema = json!({});
        let result = list_components(&schema);
        assert!(result.is_empty(), "empty schema should return empty vec");
    }

    #[test]
    fn test_list_components_no_defs() {
        let schema = json!({
            "type": "object",
            "properties": { "name": { "type": "string" } }
        });
        let result = list_components(&schema);
        assert!(
            result.is_empty(),
            "schema without $defs should return empty vec"
        );
    }

    #[test]
    fn test_list_components_defs_five_entries() {
        let schema = json!({
            "$defs": {
                "A": { "type": "string" },
                "B": { "type": "integer" },
                "C": { "type": "boolean" },
                "D": { "type": "number" },
                "E": { "type": "null" }
            }
        });
        let result = list_components(&schema);
        assert_eq!(result.len(), 5, "should enumerate all 5 $defs entries");
        assert!(result.contains(&"#/$defs/A".to_string()));
        assert!(result.contains(&"#/$defs/B".to_string()));
        assert!(result.contains(&"#/$defs/C".to_string()));
        assert!(result.contains(&"#/$defs/D".to_string()));
        assert!(result.contains(&"#/$defs/E".to_string()));
    }

    #[test]
    fn test_list_components_output_sorted() {
        let schema = json!({
            "$defs": {
                "Zebra": { "type": "string" },
                "Apple": { "type": "integer" },
                "Mango": { "type": "boolean" }
            }
        });
        let result = list_components(&schema);
        assert_eq!(result.len(), 3);
        let mut expected = result.clone();
        expected.sort();
        assert_eq!(result, expected, "output must be sorted");
    }

    #[test]
    fn test_list_components_deterministic() {
        let schema = json!({
            "$defs": {
                "X": { "type": "string" },
                "Y": { "type": "integer" }
            }
        });
        let r1 = list_components(&schema);
        let r2 = list_components(&schema);
        let r3 = list_components(&schema);
        assert_eq!(r1, r2);
        assert_eq!(r2, r3);
    }

    #[test]
    fn test_list_components_legacy_definitions() {
        // Pre-Draft-2019 `definitions` key should be enumerated
        let schema = json!({
            "definitions": {
                "Foo": { "type": "string" },
                "Bar": { "type": "integer" }
            }
        });
        let result = list_components(&schema);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"#/definitions/Foo".to_string()));
        assert!(result.contains(&"#/definitions/Bar".to_string()));
    }

    #[test]
    fn test_list_components_oas_components_schemas() {
        // OAS `components/schemas` at root level
        let schema = json!({
            "components": {
                "schemas": {
                    "Pet": { "type": "object" },
                    "Tag": { "type": "object" }
                }
            }
        });
        let result = list_components(&schema);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"#/components/schemas/Pet".to_string()));
        assert!(result.contains(&"#/components/schemas/Tag".to_string()));
    }

    #[test]
    fn test_list_components_mixed_defs_and_oas() {
        let schema = json!({
            "$defs": { "LocalType": { "type": "string" } },
            "components": {
                "schemas": { "OasType": { "type": "object" } }
            }
        });
        let result = list_components(&schema);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"#/$defs/LocalType".to_string()));
        assert!(result.contains(&"#/components/schemas/OasType".to_string()));
    }

    #[test]
    fn test_list_components_defs_inside_all_of_array() {
        // Regression for array traversal bug: $defs nested inside allOf entries
        // must be discovered even though allOf is a Value::Array.
        let schema = json!({
            "allOf": [
                {
                    "$defs": {
                        "InnerA": { "type": "string" }
                    }
                },
                {
                    "anyOf": [
                        {
                            "definitions": {
                                "DeepDefined": { "type": "integer" }
                            }
                        }
                    ]
                }
            ]
        });
        let result = list_components(&schema);
        assert!(
            result.iter().any(|p| p.contains("InnerA")),
            "InnerA (nested in allOf[0]/$defs) should be discovered; got: {:?}",
            result
        );
        assert!(
            result.iter().any(|p| p.contains("DeepDefined")),
            "DeepDefined (nested in allOf[1]/anyOf[0]/definitions) should be discovered; got: {:?}",
            result
        );
    }

    #[test]
    fn test_list_components_nested_defs_inside_defs() {
        // $defs entries that themselves contain nested $defs
        let schema = json!({
            "$defs": {
                "Outer": {
                    "type": "object",
                    "$defs": {
                        "Inner": { "type": "string" }
                    }
                }
            }
        });
        let result = list_components(&schema);
        // Should find both Outer and Outer/Inner
        assert!(
            result.contains(&"#/$defs/Outer".to_string()),
            "missing Outer; got: {:?}",
            result
        );
        assert!(
            result.contains(&"#/$defs/Outer/$defs/Inner".to_string()),
            "missing Outer/$defs/Inner; got: {:?}",
            result
        );
    }

    #[test]
    fn test_list_components_special_char_key_escaped() {
        // Keys with `/` must be RFC 6901 escaped in the returned pointer
        let schema = json!({
            "$defs": {
                "user/name": { "type": "string" }
            }
        });
        let result = list_components(&schema);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "#/$defs/user~1name");
    }

    #[test]
    fn test_list_components_oas_not_at_nested_level() {
        // components/schemas nested inside properties should NOT be enumerated
        let schema = json!({
            "properties": {
                "components": {
                    "properties": {
                        "schemas": {
                            "properties": {
                                "Thing": { "type": "string" }
                            }
                        }
                    }
                }
            }
        });
        let result = list_components(&schema);
        assert!(
            result.is_empty(),
            "components/schemas buried in properties should not be listed; got: {:?}",
            result
        );
    }

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

    // -----------------------------------------------------------------------
    // AC: $anchor resolution in extract_component (#217)
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_resolves_anchor_ref() {
        // Schema where a property references another def via $anchor.
        let schema = json!({
            "$defs": {
                "step-object": {
                    "$anchor": "stepId",
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" }
                    }
                },
                "workflow-object": {
                    "type": "object",
                    "properties": {
                        "step": { "$ref": "#stepId" }
                    }
                }
            }
        });
        let result = extract_component(&schema, "#/$defs/workflow-object", &opts())
            .expect("extract_component should resolve $anchor refs");
        // step-object should be pulled in as a dependency.
        assert_eq!(
            result.dependency_count, 1,
            "step-object should be a dependency"
        );
        // The $ref should be rewritten to point to the new $defs location.
        let step_ref = result.schema["properties"]["step"]["$ref"]
            .as_str()
            .expect("step.$ref should exist");
        assert!(
            step_ref.starts_with("#/$defs/"),
            "step.$ref should be rewritten to #/$defs/...; got: {}",
            step_ref
        );
    }

    #[test]
    fn test_extract_anchor_ref_with_id_scoping() {
        // Schema where $id changes the base URI and $anchor is scoped.
        let schema = json!({
            "$id": "https://example.com/root.json",
            "$defs": {
                "nested": {
                    "$id": "nested.json",
                    "$anchor": "foo",
                    "type": "string"
                },
                "consumer": {
                    "type": "object",
                    "properties": {
                        "val": { "$ref": "nested.json#foo" }
                    }
                }
            }
        });
        let result = extract_component(&schema, "#/$defs/consumer", &opts())
            .expect("extract_component should resolve $id-scoped $anchor refs");
        assert_eq!(result.dependency_count, 1);
    }
}
