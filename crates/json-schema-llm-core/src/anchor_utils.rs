//! Anchor and `$id` resolution utilities.
//!
//! Builds a map from absolute URI strings (incorporating `$id` base URI scoping)
//! to JSON Pointer paths. Used by `extract.rs` and `p0_normalize.rs` to resolve
//! `$anchor`-style `$ref` values (e.g., `"#stepId"`) into standard JSON Pointers.

use std::collections::HashMap;

use serde_json::Value;
use url::Url;

use crate::error::ConvertError;
use crate::schema_utils::escape_pointer_segment;

/// Default base URI used when no explicit document URI is known.
///
/// We use a `file:` scheme so that `Url::join` works correctly for
/// fragment-only references like `#anchor`.
const DEFAULT_BASE: &str = "file:///schema.json";

/// Build a map of absolute URI strings → JSON Pointer paths by scanning
/// the schema for `$anchor` declarations, respecting `$id` base URI scoping.
///
/// ## Arguments
///
/// * `schema` — The root JSON Schema document.
/// * `base_uri` — The default base URI for the document. Pass `None` to use
///   `file:///schema.json`.
///
/// ## Returns
///
/// A `HashMap<String, String>` where:
/// * **key** = absolute URI with fragment (e.g., `"file:///schema.json#stepId"`)
/// * **value** = JSON Pointer path (e.g., `"#/$defs/step-object"`)
///
/// ## Errors
///
/// Returns `ConvertError::SchemaError` for malformed schemas.
pub(crate) fn build_anchor_map(
    schema: &Value,
    base_uri: Option<&Url>,
) -> Result<HashMap<String, String>, ConvertError> {
    let default_base = Url::parse(DEFAULT_BASE).expect("DEFAULT_BASE is a valid URL");
    let base = base_uri.unwrap_or(&default_base);
    let mut map = HashMap::new();
    scan_anchors(schema, base, "#", &mut map)?;
    Ok(map)
}

/// Return the default base URI for documents with no explicit `$id`.
pub(crate) fn default_base_uri() -> Url {
    Url::parse(DEFAULT_BASE).expect("DEFAULT_BASE is a valid URL")
}

/// Resolve a `$ref` string against a base URI and look it up in the anchor map.
///
/// Returns `Some(json_pointer)` if the resolved URI maps to an anchor,
/// `None` otherwise.
pub(crate) fn resolve_ref_via_anchor_map(
    ref_str: &str,
    base_uri: &Url,
    anchor_map: &HashMap<String, String>,
) -> Option<String> {
    // Try to resolve the ref_str as a URL against the base URI.
    let resolved = base_uri.join(ref_str).ok()?;
    anchor_map.get(resolved.as_str()).cloned()
}

/// Result of resolving a `$ref` string through the anchor map.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ResolvedRef {
    /// The ref resolved to a canonical JSON Pointer (either it was already
    /// a pointer, or it was an anchor-style ref that mapped to one).
    Pointer(String),
    /// The ref could not be resolved (external URL, unknown anchor, etc.).
    /// Contains the original ref string.
    Unresolvable(String),
}

/// Resolve a `$ref` string: JSON Pointers pass through, anchor-style and
/// URI-style refs are looked up in the anchor map,  unresolvable refs
/// (external URLs, unknown anchors) are returned as `Unresolvable`.
///
/// This is the single entry point for ref resolution, used by both
/// `extract.rs` and `p0_normalize.rs`.
pub(crate) fn resolve_ref(
    ref_str: &str,
    base_uri: &Url,
    anchor_map: &HashMap<String, String>,
) -> ResolvedRef {
    if ref_str == "#" || ref_str.starts_with("#/") {
        ResolvedRef::Pointer(ref_str.to_string())
    } else {
        match resolve_ref_via_anchor_map(ref_str, base_uri, anchor_map) {
            Some(pointer) => ResolvedRef::Pointer(pointer),
            None => ResolvedRef::Unresolvable(ref_str.to_string()),
        }
    }
}

/// Recursive DFS scanner that builds the anchor map.
///
/// Tracks `current_base` (updated by `$id`) and `pointer` (the JSON Pointer
/// path to the current node).
fn scan_anchors(
    node: &Value,
    current_base: &Url,
    pointer: &str,
    map: &mut HashMap<String, String>,
) -> Result<(), ConvertError> {
    let obj = match node {
        Value::Object(obj) => obj,
        Value::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                let child_pointer = format!("{}/{}", pointer, i);
                scan_anchors(item, current_base, &child_pointer, map)?;
            }
            return Ok(());
        }
        _ => return Ok(()),
    };

    // Check for $id — updates the base URI for this scope and below.
    let scoped_base = if let Some(id_val) = obj.get("$id").and_then(Value::as_str) {
        match current_base.join(id_val) {
            Ok(new_base) => new_base,
            Err(_) => current_base.clone(), // malformed $id — ignore
        }
    } else {
        current_base.clone()
    };

    // Check for $anchor — register it in the map.
    if let Some(anchor_val) = obj.get("$anchor").and_then(Value::as_str) {
        let fragment = format!("#{}", anchor_val);
        if let Ok(absolute_uri) = scoped_base.join(&fragment) {
            let uri_str = absolute_uri.to_string();
            // First-wins: extracted schemas may contain the same $anchor at
            // multiple structural paths (root property + $defs copy). Only
            // the first occurrence is registered.
            map.entry(uri_str).or_insert_with(|| pointer.to_string());
        }
    }

    // Recurse into all child values.
    for (key, val) in obj {
        if key == "$anchor" || key == "$id" {
            continue; // already processed
        }
        let escaped_key = escape_pointer_segment(key);
        let child_pointer = format!("{}/{}", pointer, escaped_key);
        scan_anchors(val, &scoped_base, &child_pointer, map)?;
    }

    Ok(())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -----------------------------------------------------------------------
    // build_anchor_map — basic functionality
    // -----------------------------------------------------------------------

    #[test]
    fn test_empty_schema_produces_empty_map() {
        let schema = json!({ "type": "object" });
        let map = build_anchor_map(&schema, None).unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn test_single_anchor_in_defs() {
        let schema = json!({
            "$defs": {
                "step-object": {
                    "$anchor": "stepId",
                    "type": "object"
                }
            }
        });
        let map = build_anchor_map(&schema, None).unwrap();
        // The anchor should resolve to the JSON Pointer of the defining node.
        let key = "file:///schema.json#stepId";
        assert!(
            map.contains_key(key),
            "expected key '{}'; map: {:?}",
            key,
            map
        );
        assert_eq!(map[key], "#/$defs/step-object");
    }

    #[test]
    fn test_multiple_anchors() {
        let schema = json!({
            "$defs": {
                "A": { "$anchor": "aaa", "type": "string" },
                "B": { "$anchor": "bbb", "type": "integer" }
            }
        });
        let map = build_anchor_map(&schema, None).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map["file:///schema.json#aaa"], "#/$defs/A");
        assert_eq!(map["file:///schema.json#bbb"], "#/$defs/B");
    }

    #[test]
    fn test_duplicate_anchor_uses_first_wins() {
        let schema = json!({
            "$defs": {
                "A": { "$anchor": "dup", "type": "string" },
                "B": { "$anchor": "dup", "type": "integer" }
            }
        });
        let map = build_anchor_map(&schema, None).unwrap();
        // First-wins: whichever is visited first gets the slot.
        let val = &map["file:///schema.json#dup"];
        assert!(
            val == "#/$defs/A" || val == "#/$defs/B",
            "expected either A or B, got: {}",
            val
        );
    }

    // -----------------------------------------------------------------------
    // $id base URI scoping
    // -----------------------------------------------------------------------

    #[test]
    fn test_id_changes_base_uri_for_anchor() {
        let schema = json!({
            "$id": "https://example.com/root.json",
            "$defs": {
                "nested": {
                    "$id": "nested.json",
                    "$anchor": "foo",
                    "type": "object"
                }
            }
        });
        let map = build_anchor_map(&schema, None).unwrap();
        // The anchor "foo" should be scoped under the nested $id.
        let key = "https://example.com/nested.json#foo";
        assert!(
            map.contains_key(key),
            "expected key '{}'; map: {:?}",
            key,
            map
        );
    }

    #[test]
    fn test_root_id_sets_base() {
        let schema = json!({
            "$id": "https://example.com/arazzo.json",
            "$defs": {
                "step": {
                    "$anchor": "stepId",
                    "type": "object"
                }
            }
        });
        let map = build_anchor_map(&schema, None).unwrap();
        let key = "https://example.com/arazzo.json#stepId";
        assert!(
            map.contains_key(key),
            "expected key '{}'; map: {:?}",
            key,
            map
        );
    }

    // -----------------------------------------------------------------------
    // resolve_ref_via_anchor_map
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_ref_finds_anchor() {
        let base = Url::parse("file:///schema.json").unwrap();
        let mut map = HashMap::new();
        map.insert(
            "file:///schema.json#stepId".to_string(),
            "#/$defs/step-object".to_string(),
        );

        let result = resolve_ref_via_anchor_map("#stepId", &base, &map);
        assert_eq!(result, Some("#/$defs/step-object".to_string()));
    }

    #[test]
    fn test_resolve_ref_returns_none_for_unknown() {
        let base = Url::parse("file:///schema.json").unwrap();
        let map = HashMap::new();

        let result = resolve_ref_via_anchor_map("#unknown", &base, &map);
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_ref_with_scoped_id() {
        let base = Url::parse("https://example.com/nested.json").unwrap();
        let mut map = HashMap::new();
        map.insert(
            "https://example.com/nested.json#foo".to_string(),
            "#/$defs/nested".to_string(),
        );

        let result = resolve_ref_via_anchor_map("#foo", &base, &map);
        assert_eq!(result, Some("#/$defs/nested".to_string()));
    }
}
