//! Pass 0: Schema Normalization
//! Resolves $ref pointers (root-relative only), normalizes draft syntax,
//! and detects recursive cycles.
//!
//! This is the foundational pass — all downstream passes (1–7) assume refs are
//! resolved and syntax is normalized. Pass 0 performs:
//!
//! 1. `items` (array form) → `prefixItems` + `additionalItems` → `items`
//! 2. `$ref` resolution via JSON Pointer traversal with DFS cycle detection
//! 3. `definitions` → `$defs` rename (post-resolution, Phase 3)
//! 4. `$defs` cleanup (strip fully-inlined entries, preserve recursive)
//!
//! ## Limitations
//!
//! - Only root-relative JSON Pointers (`#/...`) are supported.
//! - `$id` / `$anchor` scoped resolution is not implemented.
//! - External (`http://...`) and dynamic refs are rejected with errors.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::config::ConvertOptions;
use crate::error::ConvertError;

use super::pass_result::PassResult;

/// Shared traversal context for $ref resolution, reducing argument count.
struct RefContext<'a> {
    root: &'a Value,
    config: &'a ConvertOptions,
    visiting: HashSet<String>,
    /// Memoization cache: stores already-resolved values keyed by $ref string.
    /// Prevents O(fan_out^depth) re-expansion when multiple sibling properties
    /// reference the same definition (e.g., meta-schema defs with 10+ self-refs).
    resolved_cache: HashMap<String, Value>,
    recursive_refs: Vec<String>,
}

/// Result of running the schema normalization pass.
#[derive(Debug)]
pub struct NormalizePassResult {
    /// Shared pass result containing the normalized schema.
    pub pass: PassResult,
    /// JSON Pointer paths where recursive $ref cycles were detected.
    /// These are left as `$ref` for Pass 5 to break.
    pub recursive_refs: Vec<String>,
}

// ---------------------------------------------------------------------------
// Annotation keywords — site-specific values override definition values
// when $ref has siblings.
// ---------------------------------------------------------------------------
const ANNOTATION_KEYWORDS: &[&str] = &[
    "description",
    "title",
    "$comment",
    "examples",
    "default",
    "deprecated",
    "readOnly",
    "writeOnly",
];

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
    schema: &Value,
    config: &ConvertOptions,
) -> Result<NormalizePassResult, ConvertError> {
    // Phase 1: normalize items array → prefixItems, strip annotations
    // and meta-schema fragments that would break ref resolution.
    let mut root = schema.clone();
    normalize_items_recursive(&mut root);
    strip_examples_recursive(&mut root);
    strip_problematic_root_defs(&mut root);
    strip_nested_defs_recursive(&mut root, true);

    // Phase 2: resolve $ref.
    let frozen_root = root.clone();
    let mut ctx = RefContext {
        root: &frozen_root,
        config,
        visiting: HashSet::new(),
        resolved_cache: HashMap::new(),
        recursive_refs: Vec::new(),
    };
    let result = resolve_refs(root, "#", 0, &mut ctx)?;

    // Phase 3: cleanup.
    let recursive_refs = ctx.recursive_refs;
    let result = cleanup(result, &recursive_refs);

    Ok(NormalizePassResult {
        pass: PassResult::schema_only(result),
        recursive_refs,
    })
}

// ---------------------------------------------------------------------------
// Phase 1: items array → prefixItems normalization (recursive)
// ---------------------------------------------------------------------------

fn normalize_items_recursive(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };

    // If `items` is an array and `prefixItems` doesn't already exist, rename.
    if obj.get("items").is_some_and(|v| v.is_array()) {
        if !obj.contains_key("prefixItems") {
            if let Some(items) = obj.remove("items") {
                obj.insert("prefixItems".to_string(), items);
            }
            // Draft 4-7: `additionalItems` becomes `items` in 2020-12.
            if let Some(additional) = obj.remove("additionalItems") {
                obj.insert("items".to_string(), additional);
            }
        } else {
            // Both exist — drop the array-form items (redundant in 2020-12).
            obj.remove("items");
            // Still migrate legacy `additionalItems` → `items` in 2020-12.
            if let Some(additional) = obj.remove("additionalItems") {
                obj.insert("items".to_string(), additional);
            }
        }
    }

    // Recurse into all schema-bearing children.
    for key in [
        "properties",
        "patternProperties",
        "$defs",
        "definitions",
        "dependentSchemas",
    ] {
        if let Some(Value::Object(map)) = obj.get_mut(key) {
            for val in map.values_mut() {
                normalize_items_recursive(val);
            }
        }
    }

    for key in [
        "additionalProperties",
        "unevaluatedProperties",
        "propertyNames",
        "unevaluatedItems",
        "contains",
        "not",
        "if",
        "then",
        "else",
        "items",
    ] {
        if let Some(child) = obj.get_mut(key) {
            if child.is_object() {
                normalize_items_recursive(child);
            }
        }
    }

    for key in ["anyOf", "oneOf", "allOf", "prefixItems"] {
        if let Some(Value::Array(arr)) = obj.get_mut(key) {
            for item in arr.iter_mut() {
                normalize_items_recursive(item);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 1b: Strip examples arrays (contain non-schema $ref strings)
// ---------------------------------------------------------------------------

/// Recursively remove all `examples` keys whose value is an array.
///
/// `examples` is an annotation keyword (not schema-bearing). In bundled
/// spec schemas like AsyncAPI 2.6, examples arrays contain sample data with
/// `$ref`-like strings (e.g., `#/components/schemas/signup`) that are NOT
/// JSON Schema references. If left in, the ref resolver crashes on them.
fn strip_examples_recursive(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };

    // Remove examples if it's an array
    if obj.get("examples").is_some_and(|v| v.is_array()) {
        obj.remove("examples");
    }

    // Recurse into all values
    for val in obj.values_mut() {
        match val {
            Value::Object(_) => strip_examples_recursive(val),
            Value::Array(arr) => {
                for item in arr.iter_mut() {
                    strip_examples_recursive(item);
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 1b': Strip problematic root-level definitions (meta-schemas)
// ---------------------------------------------------------------------------

/// Maximum self-references before a definition is considered problematic.
const SELF_REF_THRESHOLD: usize = 5;

/// Detect and remove root-level definitions that are meta-schemas (e.g.,
/// Draft-07, OpenAPI 3.0, Avro). These defs contain nested `definitions`
/// blocks and/or 5+ self-referential `$ref`s, causing exponential blowup
/// during ref resolution.
///
/// Stripped defs' `$ref` pointers are neutralized to `{}` (accept-all).
fn strip_problematic_root_defs(schema: &mut Value) {
    let Some(root) = schema.as_object_mut() else {
        return;
    };

    // Identify which root-level definitions container to inspect.
    let defs_key = if root.contains_key("definitions") {
        "definitions"
    } else if root.contains_key("$defs") {
        "$defs"
    } else {
        return;
    };

    // Phase 1: Identify problematic def names.
    let problematic_names: Vec<String> = {
        let defs = match root.get(defs_key).and_then(|v| v.as_object()) {
            Some(m) => m,
            None => return,
        };
        defs.iter()
            .filter(|(name, defn)| is_problematic_def(name, defn, defs_key))
            .map(|(name, _)| name.clone())
            .collect()
    };

    if problematic_names.is_empty() {
        return;
    }

    // Phase 2: Remove problematic defs from the definitions map.
    if let Some(Value::Object(defs)) = root.get_mut(defs_key) {
        for name in &problematic_names {
            defs.remove(name);
        }
    }

    // Phase 3: Build ref patterns to neutralize.
    let dead_prefixes: Vec<String> = problematic_names
        .iter()
        .flat_map(|name| {
            vec![
                format!("#/{}/{}", defs_key, name),
                // Also match the alternate key (definitions vs $defs)
                if defs_key == "definitions" {
                    format!("#/$defs/{}", name)
                } else {
                    format!("#/definitions/{}", name)
                },
            ]
        })
        .collect();

    // Phase 4: Neutralize all $ref pointers to stripped defs.
    neutralize_dead_refs(schema, &dead_prefixes);
}

/// Returns true if a definition is a problematic meta-schema.
///
/// Heuristic (matches Python `preprocess-asyncapi.py`):
/// 1. Contains nested `definitions` or `$defs` blocks, OR
/// 2. Has 5+ self-referential `$ref` pointers.
fn is_problematic_def(name: &str, defn: &Value, defs_key: &str) -> bool {
    if !defn.is_object() {
        return false;
    }

    // Check for nested definitions blocks
    if has_nested_defs(defn) {
        return true;
    }

    // Count self-referential $refs
    let self_ref_pattern = format!("#/{}/{}", defs_key, name);
    let count = count_ref_occurrences(defn, &self_ref_pattern);
    count >= SELF_REF_THRESHOLD
}

/// Recursively check if a value contains `definitions` or `$defs` keys.
fn has_nested_defs(value: &Value) -> bool {
    match value {
        Value::Object(obj) => {
            if obj.contains_key("definitions") || obj.contains_key("$defs") {
                return true;
            }
            obj.values().any(has_nested_defs)
        }
        Value::Array(arr) => arr.iter().any(has_nested_defs),
        _ => false,
    }
}

/// Count how many `$ref` values in a subtree match a given pattern.
fn count_ref_occurrences(value: &Value, pattern: &str) -> usize {
    match value {
        Value::Object(obj) => {
            let self_hit = obj
                .get("$ref")
                .and_then(Value::as_str)
                .is_some_and(|s| s == pattern);
            let child_count: usize = obj
                .values()
                .map(|v| count_ref_occurrences(v, pattern))
                .sum();
            (if self_hit { 1 } else { 0 }) + child_count
        }
        Value::Array(arr) => arr.iter().map(|v| count_ref_occurrences(v, pattern)).sum(),
        _ => 0,
    }
}

/// Recursively replace `$ref` nodes whose value starts with any dead prefix
/// with `{}` (accept-all schema).
fn neutralize_dead_refs(value: &mut Value, dead_prefixes: &[String]) {
    let Some(obj) = value.as_object_mut() else {
        if let Value::Array(arr) = value {
            for item in arr.iter_mut() {
                neutralize_dead_refs(item, dead_prefixes);
            }
        }
        return;
    };

    // Check if this node's $ref points to a dead prefix
    if let Some(ref_str) = obj.get("$ref").and_then(Value::as_str).map(String::from) {
        if dead_prefixes
            .iter()
            .any(|p| ref_str == *p || ref_str.starts_with(&format!("{}/", p)))
        {
            obj.clear();
            return;
        }
    }

    for val in obj.values_mut() {
        neutralize_dead_refs(val, dead_prefixes);
    }
}

// ---------------------------------------------------------------------------
// Phase 1c: Strip nested definitions blocks (meta-schema fragments)
// ---------------------------------------------------------------------------

/// Recursively remove `definitions` and `$defs` blocks that appear INSIDE
/// definitions — i.e., meta-schema fragments.
///
/// Bundled spec schemas (e.g., AsyncAPI 2.6) embed meta-schemas like Draft-07
/// as definitions. These meta-schemas contain their own nested `definitions`
/// blocks (e.g., `schemaArray`, `nonNegativeInteger`). When the ref resolver
/// walks into these nested blocks, it creates pointer chains that fail after
/// the `definitions→$defs` rename.
///
/// Also neutralizes `$ref` pointers that reference paths INSIDE nested
/// definitions (e.g., `#/definitions/X/definitions/Y`) since those targets
/// no longer exist after stripping.
///
/// The `is_root` parameter preserves the root-level `definitions`/`$defs`
/// (the actual schema definitions) while stripping nested ones.
fn strip_nested_defs_recursive(value: &mut Value, is_root: bool) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };

    // At non-root levels, strip definitions and $defs entirely
    if !is_root {
        obj.remove("definitions");
        obj.remove("$defs");
    }

    // Neutralize $ref pointers into nested definition paths.
    // A ref like "#/definitions/X/definitions/Y" has "definitions" appearing
    // twice — meaning it points into a nested block we just stripped.
    if let Some(ref_val) = obj.get("$ref").and_then(Value::as_str).map(String::from) {
        if refs_into_nested_defs(&ref_val) {
            // Replace the entire node with accept-all {} (remove all keys)
            obj.clear();
            return;
        }
    }

    // Recurse into all values (children are never root)
    for val in obj.values_mut() {
        match val {
            Value::Object(_) => strip_nested_defs_recursive(val, false),
            Value::Array(arr) => {
                for item in arr.iter_mut() {
                    strip_nested_defs_recursive(item, false);
                }
            }
            _ => {}
        }
    }
}

/// Returns true if a `$ref` pointer references a path inside a nested
/// definitions block (i.e., `definitions` or `$defs` appears more than
/// once in the JSON Pointer path segments).
fn refs_into_nested_defs(ref_str: &str) -> bool {
    let path = ref_str.strip_prefix('#').unwrap_or(ref_str);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let def_count = segments
        .iter()
        .filter(|s| **s == "definitions" || **s == "$defs")
        .count();
    def_count >= 2
}

// ---------------------------------------------------------------------------
// Phase 2: $ref resolution via DFS with cycle detection
// ---------------------------------------------------------------------------

fn resolve_refs(
    node: Value,
    path: &str,
    depth: usize,
    ctx: &mut RefContext<'_>,
) -> Result<Value, ConvertError> {
    if depth > ctx.config.max_depth {
        return Err(ConvertError::RecursionDepthExceeded {
            path: path.to_string(),
            max_depth: ctx.config.max_depth,
        });
    }

    let mut result = match node {
        Value::Object(obj) => obj,
        other => return Ok(other),
    };

    // Check for $ref.
    if let Some(ref_val) = result.get("$ref").and_then(Value::as_str).map(String::from) {
        return resolve_single_ref(&result, &ref_val, path, depth, ctx);
    }

    // No $ref — recurse into children.
    recurse_children(&mut result, path, depth, ctx)?;

    Ok(Value::Object(result))
}

/// Resolve a single $ref node, handling cycles, siblings, and chained refs.
#[allow(clippy::too_many_arguments)]
fn resolve_single_ref(
    obj: &Map<String, Value>,
    ref_str: &str,
    path: &str,
    depth: usize,
    ctx: &mut RefContext<'_>,
) -> Result<Value, ConvertError> {
    // Only root-relative JSON Pointers ("#" or "#/...") are supported.
    if !ref_str.starts_with('#') {
        return Err(ConvertError::UnsupportedFeature {
            path: path.to_string(),
            feature: format!("non-local $ref: {}", ref_str),
        });
    }

    // Reject anchor-style fragment refs (e.g., "#Foo") — we only support
    // JSON Pointer syntax. Without this check they'd fall through to
    // `UnresolvableRef` with a confusing error message.
    if ref_str != "#" && !ref_str.starts_with("#/") {
        return Err(ConvertError::UnsupportedFeature {
            path: path.to_string(),
            feature: format!("$anchor / non-pointer fragment $ref: {}", ref_str),
        });
    }

    // Check memoization cache — if we've already resolved this ref, reuse it.
    // This turns O(fan_out^depth) into O(unique_defs) for schemas with
    // multiple sibling $refs pointing to the same definition.
    if let Some(cached) = ctx.resolved_cache.get(ref_str).cloned() {
        let siblings: Map<String, Value> = obj
            .iter()
            .filter(|(k, _)| k.as_str() != "$ref")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        return merge_ref_siblings(cached, siblings);
    }

    // Check for cycles.
    if ctx.visiting.contains(ref_str) {
        ctx.recursive_refs.push(path.to_string());
        return Ok(Value::Object(obj.clone()));
    }

    // Resolve the pointer against the root document.
    let target =
        resolve_pointer(ctx.root, ref_str).ok_or_else(|| ConvertError::UnresolvableRef {
            path: path.to_string(),
            reference: ref_str.to_string(),
        })?;

    // Mark as visiting for cycle detection.
    ctx.visiting.insert(ref_str.to_string());

    // Recursively resolve the target (handles chained refs like A→B→C).
    let resolved = resolve_refs(target.clone(), path, depth + 1, ctx)?;

    // Unmark after resolution.
    ctx.visiting.remove(ref_str);

    // Cache the resolved value for future reuse (only non-recursive refs).
    ctx.resolved_cache
        .insert(ref_str.to_string(), resolved.clone());

    // Handle sibling keywords alongside $ref.
    let siblings: Map<String, Value> = obj
        .iter()
        .filter(|(k, _)| k.as_str() != "$ref")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    merge_ref_siblings(resolved, siblings)
}

/// Merge sibling keywords from a `$ref` site into the resolved definition.
///
/// Annotations (description, title, etc.) override; structural siblings are
/// wrapped into an `allOf` for Pass 1 to handle.
fn merge_ref_siblings(
    resolved: Value,
    siblings: Map<String, Value>,
) -> Result<Value, ConvertError> {
    if siblings.is_empty() {
        return Ok(resolved);
    }

    // Split siblings into annotations vs structural.
    let mut annotations = Map::new();
    let mut structural = Map::new();
    for (k, v) in siblings {
        if ANNOTATION_KEYWORDS.contains(&k.as_str()) {
            annotations.insert(k, v);
        } else {
            structural.insert(k, v);
        }
    }

    // Apply annotation overrides onto the resolved definition.
    let mut merged = match resolved {
        Value::Object(m) => m,
        Value::Bool(true) => Map::new(),
        Value::Bool(false) => {
            let mut m = Map::new();
            m.insert("not".to_string(), Value::Object(Map::new()));
            m
        }
        other => {
            // Resolved to a non-object primitive — wrap it for annotation merge.
            let mut m = Map::new();
            m.insert("const".to_string(), other);
            m
        }
    };

    for (k, v) in annotations {
        merged.insert(k, v);
    }

    if structural.is_empty() {
        return Ok(Value::Object(merged));
    }

    // Structural siblings → wrap in allOf for Pass 1 to handle.
    Ok(Value::Object(Map::from_iter([(
        "allOf".to_string(),
        Value::Array(vec![Value::Object(merged), Value::Object(structural)]),
    )])))
}

/// Resolve a JSON Pointer against a root document.
/// Delegates to [`crate::schema_utils::resolve_pointer`].
fn resolve_pointer<'a>(root: &'a Value, pointer: &str) -> Option<&'a Value> {
    crate::schema_utils::resolve_pointer(root, pointer)
}

/// Recurse into all schema-bearing children of an object.
///
/// Delegates to [`crate::schema_utils::recurse_into_children`] for keyword
/// iteration, threading the `RefContext` via closure.
fn recurse_children(
    obj: &mut Map<String, Value>,
    path: &str,
    depth: usize,
    ctx: &mut RefContext<'_>,
) -> Result<(), ConvertError> {
    crate::schema_utils::recurse_into_children(
        obj,
        path,
        depth,
        &mut |val, child_path, child_depth| resolve_refs(val, child_path, child_depth, ctx),
    )
}

// ---------------------------------------------------------------------------
// Phase 3: Cleanup
// ---------------------------------------------------------------------------

fn cleanup(schema: Value, _recursive_refs: &[String]) -> Value {
    // Normalize top-level boolean schemas to canonical object form.
    let schema = match schema {
        Value::Bool(true) => Value::Object(Map::new()),
        Value::Bool(false) => {
            let mut m = Map::new();
            m.insert("not".to_string(), Value::Object(Map::new()));
            Value::Object(m)
        }
        other => other,
    };

    let Value::Object(mut obj) = schema else {
        return schema;
    };

    // Collect which $defs entries are still referenced by remaining $ref
    // nodes (i.e., the recursive refs that weren't inlined).
    let referenced_defs = collect_remaining_refs(&Value::Object(obj.clone()));

    // Rename `definitions` → `$defs` if needed.
    if let Some(Value::Object(def_map)) = obj.remove("definitions") {
        let defs_entry = obj
            .entry("$defs")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(existing_defs) = defs_entry {
            for (k, v) in def_map {
                // Don't overwrite existing $defs entries.
                if !existing_defs.contains_key(&k) {
                    existing_defs.insert(k, v);
                }
            }
        }

        // Rewrite remaining `$ref` pointers from #/definitions/ to #/$defs/.
        let mut schema = Value::Object(obj);
        rewrite_definition_refs(&mut schema);
        obj = schema.as_object().unwrap().clone();
    }

    // Strip $defs entries that are not referenced by remaining recursive refs.
    if let Some(Value::Object(defs)) = obj.get_mut("$defs") {
        let keys_to_remove: Vec<String> = defs
            .keys()
            .filter(|k| !referenced_defs.contains(k.as_str()))
            .cloned()
            .collect();
        for k in keys_to_remove {
            defs.remove(&k);
        }
        // If $defs is now empty, remove it entirely.
        if defs.is_empty() {
            obj.remove("$defs");
        }
    }

    Value::Object(obj)
}

/// Rewrite `$ref` pointers from `#/definitions/` to `#/$defs/` after rename.
fn rewrite_definition_refs(value: &mut Value) {
    match value {
        Value::Object(obj) => {
            if let Some(Value::String(ref_str)) = obj.get("$ref") {
                if let Some(rest) = ref_str.strip_prefix("#/definitions/") {
                    let new_ref = format!("#/$defs/{}", rest);
                    obj.insert("$ref".to_string(), Value::String(new_ref));
                }
            }
            for v in obj.values_mut() {
                rewrite_definition_refs(v);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                rewrite_definition_refs(v);
            }
        }
        _ => {}
    }
}

/// Walk the schema and collect definition names that are still referenced
/// by remaining `$ref` pointers (i.e., recursive refs that weren't inlined).
fn collect_remaining_refs(schema: &Value) -> HashSet<String> {
    let mut refs = HashSet::new();
    collect_refs_recursive(schema, &mut refs);
    refs
}

fn collect_refs_recursive(value: &Value, refs: &mut HashSet<String>) {
    match value {
        Value::Object(obj) => {
            if let Some(ref_val) = obj.get("$ref").and_then(Value::as_str) {
                // Extract the definition name from the pointer.
                // e.g., "#/$defs/TreeNode" → "TreeNode"
                // e.g., "#/definitions/Thing" → "Thing"
                if let Some(rest) = ref_val
                    .strip_prefix("#/$defs/")
                    .or_else(|| ref_val.strip_prefix("#/definitions/"))
                {
                    // RFC 6901: unescape pointer segments before matching.
                    let def_name = rest.split('/').next().unwrap_or(rest);
                    let def_name = def_name.replace("~1", "/").replace("~0", "~");
                    refs.insert(def_name);
                }
            }
            for v in obj.values() {
                collect_refs_recursive(v, refs);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                collect_refs_recursive(v, refs);
            }
        }
        _ => {}
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

    use crate::config::ConvertOptions;

    fn run(schema: Value) -> (Value, Vec<String>) {
        let config = ConvertOptions::default();
        let result = normalize(&schema, &config).unwrap();
        (result.pass.schema, result.recursive_refs)
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
            json!([{ "type": "string" }, { "type": "integer" }])
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

    #[test]
    fn test_anchor_style_ref_error() {
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "$ref": "#Foo" }
            }
        });
        let config = ConvertOptions::default();
        let result = normalize(&input, &config);
        assert!(result.is_err());
        let err = result.unwrap_err();
        match &err {
            ConvertError::UnsupportedFeature { feature, .. } => {
                assert!(
                    feature.contains("non-pointer fragment"),
                    "Expected non-pointer fragment error, got: {}",
                    feature
                );
            }
            other => panic!("Expected UnsupportedFeature, got: {:?}", other),
        }
    }

    #[test]
    fn test_additional_items_migration_basic() {
        let input = json!({
            "items": [{ "type": "string" }, { "type": "integer" }],
            "additionalItems": { "type": "boolean" }
        });
        let (output, _) = run(input);

        // items array → prefixItems
        assert_eq!(
            output["prefixItems"],
            json!([{ "type": "string" }, { "type": "integer" }])
        );
        // additionalItems → items
        assert_eq!(output["items"], json!({ "type": "boolean" }));
        // additionalItems should be gone
        assert!(output.get("additionalItems").is_none());
    }

    #[test]
    fn test_additional_items_migration_with_existing_prefix_items() {
        let input = json!({
            "prefixItems": [{ "type": "string" }],
            "items": [{ "type": "integer" }],
            "additionalItems": { "type": "boolean" }
        });
        let (output, _) = run(input);

        // prefixItems preserved
        assert_eq!(output["prefixItems"], json!([{ "type": "string" }]));
        // redundant array-form items dropped, additionalItems → items
        assert_eq!(output["items"], json!({ "type": "boolean" }));
        assert!(output.get("additionalItems").is_none());
    }

    #[test]
    fn test_boolean_schema_cleanup() {
        // Top-level `true` schema → empty object
        let config = ConvertOptions::default();
        let result = normalize(&json!(true), &config).unwrap();
        assert_eq!(result.pass.schema, json!({}));

        // Top-level `false` schema → { "not": {} }
        let result = normalize(&json!(false), &config).unwrap();
        assert_eq!(result.pass.schema, json!({ "not": {} }));
    }

    #[test]
    fn test_additional_items_ref_traversal() {
        let input = json!({
            "$defs": {
                "Extra": { "type": "number" }
            },
            "prefixItems": [{ "type": "string" }],
            "items": [{ "type": "integer" }],
            "additionalItems": { "$ref": "#/$defs/Extra" }
        });
        let (output, _) = run(input);

        // additionalItems had a $ref that should be resolved and migrated to items
        assert_eq!(output["items"], json!({ "type": "number" }));
        assert!(output.get("additionalItems").is_none());
    }

    // -----------------------------------------------------------------------
    // Test: examples arrays with non-schema $ref strings (#201)
    // examples are annotation data, not schema-bearing — $ref strings
    // inside examples (e.g., "#/components/schemas/signup") are sample data,
    // not JSON Schema $refs. They must be stripped before ref resolution.
    // -----------------------------------------------------------------------
    #[test]
    fn test_examples_with_fake_refs_stripped() {
        let input = json!({
            "type": "object",
            "properties": {
                "channel": {
                    "type": "object",
                    "properties": {
                        "subscribe": { "type": "object" }
                    },
                    "examples": [
                        {
                            "subscribe": {
                                "message": {
                                    "payload": {
                                        "$ref": "#/components/schemas/signup"
                                    }
                                }
                            }
                        }
                    ]
                }
            }
        });

        let config = ConvertOptions::default();
        let result = normalize(&input, &config);

        // Should not crash on the fake $ref inside examples
        assert!(
            result.is_ok(),
            "examples with non-schema $ref should not cause an error: {:?}",
            result.unwrap_err()
        );

        // examples should be stripped from the output
        let output = result.unwrap().pass.schema;
        assert!(
            output["properties"]["channel"].get("examples").is_none(),
            "examples should be stripped during normalization"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Nested definitions inside defs don't break ref resolution (#201)
    // Meta-schema defs like Draft-07 contain nested `definitions` blocks.
    // These should not be traversed for ref resolution since their internal
    // refs point into meta-schema scope, not the parent document.
    // -----------------------------------------------------------------------
    #[test]
    fn test_nested_definitions_inside_defs_skipped() {
        // Simulates a def with nested definitions (like Draft-07 meta-schema)
        let input = json!({
            "type": "object",
            "properties": {
                "config": { "$ref": "#/$defs/MetaSchema" }
            },
            "$defs": {
                "MetaSchema": {
                    "type": "object",
                    "definitions": {
                        "schemaArray": {
                            "type": "array",
                            "items": { "$ref": "#/definitions/MetaSchema" }
                        }
                    },
                    "properties": {
                        "title": { "type": "string" },
                        "allOf": { "$ref": "#/definitions/MetaSchema/definitions/schemaArray" }
                    }
                }
            }
        });

        let config = ConvertOptions::default();
        let result = normalize(&input, &config);

        // Should not crash on unresolvable refs inside nested definitions
        assert!(
            result.is_ok(),
            "nested definitions inside defs should not cause crash: {:?}",
            result.unwrap_err()
        );
    }
}
