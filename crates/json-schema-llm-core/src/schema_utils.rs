//! Shared schema utilities for consistent traversal and path construction.
//!
//! Provides two concerns used across all conversion passes:
//! 1. **JSON Pointer escaping** (RFC 6901) for property keys with `/` or `~`
//! 2. **Shared traversal** that visits ALL schema-bearing keywords consistently
//!
//! ## Cross-Reference
//!
//! P1 (Composition) cannot use `recurse_into_children` because it consumes
//! values and handles `allOf` specially. When adding new keywords here,
//! also update `p1_composition.rs` manually.

use std::borrow::Cow;

use serde_json::{Map, Value};

use crate::error::ConvertError;

// ---------------------------------------------------------------------------
// JSON Pointer escaping (RFC 6901)
// ---------------------------------------------------------------------------

/// Escape a single path segment per RFC 6901.
///
/// - `~` → `~0`
/// - `/` → `~1`
///
/// Returns `Cow::Borrowed` when no escaping is needed (the common case).
pub fn escape_pointer_segment(segment: &str) -> Cow<'_, str> {
    if segment.contains('~') || segment.contains('/') {
        Cow::Owned(segment.replace('~', "~0").replace('/', "~1"))
    } else {
        Cow::Borrowed(segment)
    }
}

/// Build a JSON Pointer path by appending segments to a parent path.
///
/// Each segment is escaped per RFC 6901 before joining.
///
/// # Example
/// ```
/// use json_schema_llm_core::build_path;
/// assert_eq!(build_path("#", &["properties", "a/b"]), "#/properties/a~1b");
/// ```
pub fn build_path(parent: &str, segments: &[&str]) -> String {
    let mut path = parent.to_string();
    for segment in segments {
        path.push('/');
        path.push_str(&escape_pointer_segment(segment));
    }
    path
}

/// Unescape a single path segment per RFC 6901.
///
/// - `~1` → `/`
/// - `~0` → `~`
///
/// Order matters: unescape `~1` first to avoid double-unescaping.
/// Returns `Cow::Borrowed` when no unescaping is needed (the common case).
pub fn unescape_pointer_segment(segment: &str) -> Cow<'_, str> {
    if segment.contains("~0") || segment.contains("~1") {
        Cow::Owned(segment.replace("~1", "/").replace("~0", "~"))
    } else {
        Cow::Borrowed(segment)
    }
}

/// Split a JSON Pointer path into decoded segments.
///
/// Strips the leading `#` fragment identifier (if present), splits on `/`,
/// and unescapes each segment per RFC 6901.
///
/// # Example
/// ```
/// use json_schema_llm_core::split_path;
/// assert_eq!(split_path("#/properties/a~1b/items"), vec!["properties", "a/b", "items"]);
/// assert_eq!(split_path("#"), Vec::<String>::new());
/// ```
pub fn split_path(path: &str) -> Vec<String> {
    let stripped = path.strip_prefix('#').unwrap_or(path);

    // An empty fragment ("#" or "") refers to the whole document.
    if stripped.is_empty() {
        return Vec::new();
    }

    let mut segments_iter = stripped.split('/');

    // A leading "/" produces an initial empty segment from split('/') that
    // represents the root — skip it. Subsequent empty segments are significant
    // per RFC 6901 (e.g. "#/" → [""] refers to the empty-string key).
    if stripped.starts_with('/') {
        segments_iter.next();
    }

    segments_iter
        .map(|s| unescape_pointer_segment(s).into_owned())
        .collect()
}

// ---------------------------------------------------------------------------
// Shared schema traversal
// ---------------------------------------------------------------------------

/// Recursively visit all schema-bearing children of a JSON Schema object.
///
/// Calls `walk_fn` for each child schema found under any recognised keyword.
/// Uses the extract-recurse-reinsert pattern to avoid borrow conflicts.
///
/// # Keywords visited
///
/// | Category       | Keywords                                                                         |
/// |----------------|----------------------------------------------------------------------------------|
/// | Object         | `properties`, `patternProperties`, `additionalProperties`,                       |
/// |                | `unevaluatedProperties`, `propertyNames`                                         |
/// | Array          | `items` (object + array), `prefixItems`, `additionalItems`,                     |
/// |                | `unevaluatedItems`, `contains`                                                   |
/// | Composition    | `anyOf`, `oneOf`, `allOf`                                                        |
/// | Conditional    | `if`, `then`, `else`                                                             |
/// | Negation       | `not`                                                                            |
/// | Definitions    | `$defs`, `definitions`                                                           |
/// | Dependencies   | `dependentSchemas`                                                               |
///
/// ## Cross-Reference
///
/// When adding keywords here, also update `p1_composition::walk()` and
/// `CompatVisitor` in `p9_provider_compat.rs`.
pub fn recurse_into_children<F>(
    obj: &mut Map<String, Value>,
    path: &str,
    depth: usize,
    walk_fn: &mut F,
) -> Result<(), ConvertError>
where
    F: FnMut(Value, &str, usize) -> Result<Value, ConvertError>,
{
    // --- Map-of-schemas keywords ---
    // `properties`, `patternProperties`, `$defs`, `definitions`, `dependentSchemas`
    for keyword in [
        "properties",
        "patternProperties",
        "$defs",
        "definitions",
        "dependentSchemas",
    ] {
        if let Some(val) = obj.remove(keyword) {
            match val {
                Value::Object(map) => {
                    let mut new_map = Map::new();
                    for (key, val) in map {
                        let child_path = build_path(path, &[keyword, &key]);
                        new_map.insert(key, walk_fn(val, &child_path, depth + 1)?);
                    }
                    obj.insert(keyword.to_string(), Value::Object(new_map));
                }
                other => {
                    // Not a map-of-schemas — preserve as-is
                    obj.insert(keyword.to_string(), other);
                }
            }
        }
    }

    // --- Single-schema keywords ---
    // `additionalProperties`, `unevaluatedProperties`, `propertyNames`,
    // `unevaluatedItems`, `contains`, `not`, `if`, `then`, `else`
    for keyword in [
        "additionalProperties",
        "unevaluatedProperties",
        "propertyNames",
        "unevaluatedItems",
        "contains",
        "not",
        "if",
        "then",
        "else",
        "additionalItems",
    ] {
        if let Some(val) = obj.remove(keyword) {
            if val.is_object() {
                let child_path = build_path(path, &[keyword]);
                let walked = walk_fn(val, &child_path, depth + 1)?;
                obj.insert(keyword.to_string(), walked);
            } else {
                // Not a schema (e.g. `additionalProperties: false`) — preserve as-is
                obj.insert(keyword.to_string(), val);
            }
        }
    }

    // --- Array-of-schemas keywords ---
    // `anyOf`, `oneOf`, `allOf`, `prefixItems`
    for keyword in ["anyOf", "oneOf", "allOf", "prefixItems"] {
        if let Some(val) = obj.remove(keyword) {
            match val {
                Value::Array(variants) => {
                    let mut walked = Vec::with_capacity(variants.len());
                    for (i, variant) in variants.into_iter().enumerate() {
                        let child_path = build_path(path, &[keyword, &i.to_string()]);
                        walked.push(walk_fn(variant, &child_path, depth + 1)?);
                    }
                    obj.insert(keyword.to_string(), Value::Array(walked));
                }
                other => {
                    // Not an array-of-schemas — preserve as-is
                    obj.insert(keyword.to_string(), other);
                }
            }
        }
    }

    // --- `items` (object or array form) ---
    if let Some(items) = obj.remove("items") {
        match items {
            Value::Object(_) => {
                let child_path = build_path(path, &["items"]);
                let walked = walk_fn(items, &child_path, depth + 1)?;
                obj.insert("items".to_string(), walked);
            }
            Value::Array(arr) => {
                // Tuple validation (Draft 4-7 array form)
                let mut walked = Vec::with_capacity(arr.len());
                for (i, item) in arr.into_iter().enumerate() {
                    let child_path = build_path(path, &["items", &i.to_string()]);
                    walked.push(walk_fn(item, &child_path, depth + 1)?);
                }
                obj.insert("items".to_string(), Value::Array(walked));
            }
            other => {
                // `items: true/false` — preserve
                obj.insert("items".to_string(), other);
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// JSON Pointer resolution (RFC 6901)
// ---------------------------------------------------------------------------

/// Resolve a JSON Pointer against a document root.
///
/// Supports paths like `#/$defs/Address`, `#/definitions/Thing`,
/// `#/components/schemas/Pet`, `#/$defs/User/properties/name`, and bare `#`.
///
/// Returns `None` if the pointer cannot be resolved.
pub(crate) fn resolve_pointer<'a>(root: &'a Value, pointer: &str) -> Option<&'a Value> {
    let path = pointer.strip_prefix('#')?;
    if path.is_empty() {
        return Some(root);
    }
    let path = path.strip_prefix('/')?;

    let mut current = root;
    for segment in path.split('/') {
        // Unescape RFC 6901 sequences.
        let key = segment.replace("~1", "/").replace("~0", "~");
        match current {
            Value::Object(obj) => {
                current = obj.get(&key)?;
            }
            Value::Array(arr) => {
                let idx: usize = key.parse().ok()?;
                current = arr.get(idx)?;
            }
            _ => return None,
        }
    }

    Some(current)
}

// ---------------------------------------------------------------------------
// Opaque string description helpers
// ---------------------------------------------------------------------------

/// Summarise a sub-schema that is about to be replaced with an opaque
/// `type: "string"` so that the LLM knows what JSON structure to produce.
///
/// Recursively describes the entire sub-tree in a compact TypeScript-like
/// syntax so the LLM can produce structurally correct stringified JSON.
/// Also extracts constraint hints (contains, minItems, maxItems, etc.) so
/// the LLM knows about semantic requirements that would otherwise be lost.
///
/// Example output:
///   `"A JSON-encoded string. Structure: [any]. Must contain: \"magic_token\". Produce valid, parseable JSON."`
pub(crate) fn build_opaque_description(schema: &Value) -> String {
    let structure = describe_schema_structure(schema, 0);
    let constraints = collect_constraint_hints(schema);
    if constraints.is_empty() {
        format!("MUST be a valid JSON value serialized as a string. Structure: {structure}. Do NOT output plain text — the value must parse with JSON.parse().")
    } else {
        format!(
            "MUST be a valid JSON value serialized as a string. Structure: {structure}. {constraints} Do NOT output plain text — the value must parse with JSON.parse()."
        )
    }
}

/// Extract human-readable constraint hints from a schema that is about to be
/// opaque-stringified. These constraints would otherwise be silently lost.
fn collect_constraint_hints(schema: &Value) -> String {
    let obj = match schema.as_object() {
        Some(o) => o,
        None => return String::new(),
    };

    let mut hints = Vec::new();

    // contains constraint
    if let Some(contains) = obj.get("contains") {
        if let Some(const_val) = contains.get("const") {
            let val_str = serde_json::to_string(const_val).unwrap_or_default();
            hints.push(format!("array must include {val_str} as an element"));
        } else if let Some(type_val) = contains.get("type").and_then(|v| v.as_str()) {
            hints.push(format!(
                "array must include at least one element of type {type_val}"
            ));
        } else {
            hints.push("array must include at least one matching element".to_string());
        }
    }

    // minContains / maxContains
    if let Some(n) = obj.get("minContains").and_then(|v| v.as_u64()) {
        if n > 1 {
            hints.push(format!("minimum {n} matching item(s)"));
        }
    }
    if let Some(n) = obj.get("maxContains").and_then(|v| v.as_u64()) {
        hints.push(format!("maximum {n} matching item(s)"));
    }

    // minItems / maxItems
    if let Some(n) = obj.get("minItems").and_then(|v| v.as_u64()) {
        hints.push(format!("minimum {n} item(s)"));
    }
    if let Some(n) = obj.get("maxItems").and_then(|v| v.as_u64()) {
        hints.push(format!("maximum {n} item(s)"));
    }

    // uniqueItems
    if obj.get("uniqueItems").and_then(|v| v.as_bool()) == Some(true) {
        hints.push("items must be unique".to_string());
    }

    // patternProperties
    if let Some(pp) = obj.get("patternProperties").and_then(|v| v.as_object()) {
        let patterns: Vec<String> = pp.keys().cloned().collect();
        if !patterns.is_empty() {
            hints.push(format!(
                "includes properties matching pattern(s) [{}]",
                patterns.join(", ")
            ));
        }
    }

    let mut joined = hints.join(". ");
    if !joined.is_empty() {
        joined.push('.');
    }
    joined
}

/// Recursively convert a schema into a compact structural description.
/// Uses a TypeScript-like object/array syntax for clarity.
/// Depth parameter prevents runaway recursion (cap at 10 levels of description).
pub(crate) fn describe_schema_structure(schema: &Value, depth: usize) -> String {
    if depth > 10 {
        return "...".to_string();
    }

    // [New] Check for enum *before* type check, as enum is more specific
    if let Some(enum_vals) = schema.get("enum").and_then(|v| v.as_array()) {
        if !enum_vals.is_empty() {
            if enum_vals.len() <= 10 {
                // List values directly
                let variants: Vec<String> = enum_vals
                    .iter()
                    .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "...".to_string()))
                    .collect();
                return format!("One of: [{}]", variants.join(", "));
            } else {
                return format!("One of {} allowed values", enum_vals.len());
            }
        }
    }

    let schema_type = schema.get("type").and_then(|v| v.as_str()).unwrap_or("any");

    match schema_type {
        "object" => {
            if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
                let fields: Vec<String> = props
                    .iter()
                    .take(30) // cap field count
                    .map(|(name, sub)| {
                        let desc = describe_schema_structure(sub, depth + 1);
                        format!("{name}: {desc}")
                    })
                    .collect();
                let suffix = if props.len() > 30 {
                    ", ...".to_string()
                } else {
                    String::new()
                };
                format!("{{{}{}}}", fields.join(", "), suffix)
            } else {
                "object".to_string()
            }
        }
        "array" => {
            // Case 1: Uniform array (no prefixItems, items is schema)
            if schema.get("prefixItems").is_none() {
                if let Some(items) = schema.get("items") {
                    if items.is_object() {
                        let item_desc = describe_schema_structure(items, depth + 1);
                        return format!("[{item_desc}]");
                    }
                }
            }

            // Case 2: Tuple / Mixed / Prefix
            let mut parts = Vec::new();
            if let Some(prefix) = schema.get("prefixItems").and_then(|v| v.as_array()) {
                for item in prefix.iter().take(10) {
                    parts.push(describe_schema_structure(item, depth + 1));
                }
            } else if let Some(items_arr) = schema.get("items").and_then(|v| v.as_array()) {
                // draft-4 tuple (if not normalized)
                for item in items_arr.iter().take(10) {
                    parts.push(describe_schema_structure(item, depth + 1));
                }
            }

            let mut closed = false;
            if let Some(items) = schema.get("items") {
                if items.as_bool() == Some(false) {
                    closed = true;
                } else if items.is_object() {
                    // prefix + open items
                    let item_desc = describe_schema_structure(items, depth + 1);
                    parts.push(format!("...{}", item_desc));
                } else if items.as_bool() == Some(true) {
                    if parts.is_empty() {
                        parts.push("any".to_string());
                    } else {
                        parts.push("...any".to_string());
                    }
                }
            } else {
                // items missing -> default true (unless we processed legacy items array, which usually defaults to true too)
                if parts.is_empty() {
                    parts.push("any".to_string());
                } else {
                    parts.push("...any".to_string());
                }
            }

            let suffix = if closed { " (fixed length)" } else { "" };
            format!("[{}]{suffix}", parts.join(", "))
        }
        _ => schema_type.to_string(),
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // --- Escaping tests ---

    #[test]
    fn test_escape_no_special() {
        let result = escape_pointer_segment("foo");
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "foo");
    }

    #[test]
    fn test_escape_tilde() {
        assert_eq!(escape_pointer_segment("a~b"), "a~0b");
    }

    #[test]
    fn test_escape_slash() {
        assert_eq!(escape_pointer_segment("a/b"), "a~1b");
    }

    #[test]
    fn test_escape_both() {
        assert_eq!(escape_pointer_segment("~/"), "~0~1");
    }

    #[test]
    fn test_escape_multiple() {
        assert_eq!(escape_pointer_segment("a/b~c/d"), "a~1b~0c~1d");
    }

    #[test]
    fn test_build_path_simple() {
        assert_eq!(
            build_path("#", &["properties", "name"]),
            "#/properties/name"
        );
    }

    #[test]
    fn test_build_path_escaping() {
        assert_eq!(build_path("#", &["properties", "a/b"]), "#/properties/a~1b");
    }

    #[test]
    fn test_build_path_empty() {
        assert_eq!(build_path("#", &[]), "#");
    }

    // --- Traversal tests ---

    #[test]
    fn test_recurse_visits_all_keywords() {
        // Build a schema with every keyword that should be visited
        let mut schema = json!({
            "type": "object",
            "properties": {
                "a": { "type": "string" }
            },
            "patternProperties": {
                "^x-": { "type": "string" }
            },
            "additionalProperties": { "type": "integer" },
            "unevaluatedProperties": { "type": "boolean" },
            "propertyNames": { "type": "string", "maxLength": 10 },
            "items": { "type": "string" },
            "prefixItems": [{ "type": "integer" }],
            "unevaluatedItems": { "type": "null" },
            "contains": { "type": "number" },
            "anyOf": [{ "type": "string" }],
            "oneOf": [{ "type": "integer" }],
            "allOf": [{ "type": "boolean" }],
            "if": { "type": "string" },
            "then": { "type": "integer" },
            "else": { "type": "boolean" },
            "not": { "type": "null" },
            "$defs": { "Foo": { "type": "string" } },
            "definitions": { "Bar": { "type": "integer" } },
            "dependentSchemas": { "a": { "type": "object" } }
        });

        let obj = schema.as_object_mut().unwrap();
        let visit_count = AtomicUsize::new(0);

        recurse_into_children(
            obj,
            "#",
            0,
            &mut |val: Value, _path: &str, _depth: usize| {
                visit_count.fetch_add(1, Ordering::Relaxed);
                Ok(val)
            },
        )
        .unwrap();

        // Count expected visits:
        // properties:1, patternProperties:1, additionalProperties:1,
        // unevaluatedProperties:1, propertyNames:1, items:1, prefixItems:1,
        // unevaluatedItems:1, contains:1, anyOf:1, oneOf:1, allOf:1,
        // if:1, then:1, else:1, not:1, $defs:1, definitions:1, dependentSchemas:1
        assert_eq!(visit_count.load(Ordering::Relaxed), 19);
    }

    #[test]
    fn test_recurse_preserves_non_schema_values() {
        let mut schema = json!({
            "additionalProperties": false,
            "items": true
        });

        let obj = schema.as_object_mut().unwrap();
        recurse_into_children(
            obj,
            "#",
            0,
            &mut |val: Value, _path: &str, _depth: usize| Ok(val),
        )
        .unwrap();

        // Non-schema values preserved
        assert_eq!(obj.get("additionalProperties"), Some(&json!(false)));
        assert_eq!(obj.get("items"), Some(&json!(true)));
    }

    #[test]
    fn test_recurse_builds_correct_paths() {
        let mut schema = json!({
            "properties": {
                "a/b": { "type": "string" }
            },
            "anyOf": [{ "type": "integer" }]
        });

        let obj = schema.as_object_mut().unwrap();
        let mut paths = Vec::new();

        recurse_into_children(obj, "#", 0, &mut |val: Value, path: &str, _depth: usize| {
            paths.push(path.to_string());
            Ok(val)
        })
        .unwrap();

        assert!(paths.contains(&"#/properties/a~1b".to_string()));
        assert!(paths.contains(&"#/anyOf/0".to_string()));
    }

    #[test]
    fn test_recurse_items_array_form() {
        let mut schema = json!({
            "items": [
                { "type": "string" },
                { "type": "integer" }
            ]
        });

        let obj = schema.as_object_mut().unwrap();
        let mut paths = Vec::new();

        recurse_into_children(obj, "#", 0, &mut |val: Value, path: &str, _depth: usize| {
            paths.push(path.to_string());
            Ok(val)
        })
        .unwrap();

        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&"#/items/0".to_string()));
        assert!(paths.contains(&"#/items/1".to_string()));
    }

    #[test]
    fn test_recurse_preserves_unexpected_types_on_keywords() {
        // Regression: map-of-schemas keyword with non-object value,
        // and array-of-schemas keyword with non-array value should
        // be preserved — not silently dropped.
        let mut schema = json!({
            "$defs": true,
            "anyOf": "invalid",
            "allOf": 42
        });

        let obj = schema.as_object_mut().unwrap();
        recurse_into_children(
            obj,
            "#",
            0,
            &mut |val: Value, _path: &str, _depth: usize| Ok(val),
        )
        .unwrap();

        assert_eq!(obj.get("$defs"), Some(&json!(true)));
        assert_eq!(obj.get("anyOf"), Some(&json!("invalid")));
        assert_eq!(obj.get("allOf"), Some(&json!(42)));
    }

    // --- unescape_pointer_segment tests ---

    #[test]
    fn test_unescape_no_special() {
        assert_eq!(unescape_pointer_segment("hello"), "hello");
    }

    #[test]
    fn test_unescape_tilde() {
        assert_eq!(unescape_pointer_segment("a~0b"), "a~b");
    }

    #[test]
    fn test_unescape_slash() {
        assert_eq!(unescape_pointer_segment("a~1b"), "a/b");
    }

    #[test]
    fn test_unescape_both() {
        assert_eq!(unescape_pointer_segment("a~0b~1c"), "a~b/c");
    }

    #[test]
    fn test_escape_unescape_roundtrip() {
        let original = "my/key~with~special/chars";
        let escaped = escape_pointer_segment(original);
        let unescaped = unescape_pointer_segment(&escaped);
        assert_eq!(unescaped, original);
    }

    // --- split_path tests ---

    #[test]
    fn test_split_path_simple() {
        assert_eq!(split_path("#/properties/name"), vec!["properties", "name"]);
    }

    #[test]
    fn test_split_path_with_escapes() {
        assert_eq!(
            split_path("#/properties/a~1b/items"),
            vec!["properties", "a/b", "items"]
        );
    }

    #[test]
    fn test_split_path_root() {
        assert_eq!(split_path("#"), Vec::<String>::new());
    }

    #[test]
    fn test_split_path_no_fragment() {
        assert_eq!(split_path("/properties/x"), vec!["properties", "x"]);
    }
}
