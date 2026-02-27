//! Shared helpers for conversion passes.
//!
//! Contains reusable schema introspection and mutation utilities that multiple
//! passes (p1–p9) need. Separated from `schema_utils` because these helpers
//! are codec-aware (they produce `Transform` entries), while `schema_utils`
//! stays at the JSON/path level.

use std::collections::HashSet;

use serde_json::{json, Map, Value};

use crate::codec::Transform;
use crate::schema_utils::build_path;

// ---------------------------------------------------------------------------
// Reference-mechanism keywords
// ---------------------------------------------------------------------------

/// JSON Schema identity and resolution keywords that must be stripped after
/// `$ref` inlining. These keywords have no semantic value in the output
/// schema and are rejected by providers like OpenAI strict mode.
///
/// Used by p5 (post-inline stripping, sibling filtering) and p9 (belt-and-
/// suspenders pre-depth stripping). Keeping a single source of truth avoids
/// the lists silently diverging.
pub const REF_META_KEYWORDS: &[&str] =
    &["$anchor", "$dynamicAnchor", "$dynamicRef", "$id", "$schema"];

// ---------------------------------------------------------------------------
// Type introspection
// ---------------------------------------------------------------------------

/// Check whether a JSON object has `"type": "object"`.
///
/// Used by p3 (dictionary detection), p6 (strict enforcement gate), and
/// p9 (inner-schema enforcement after wrapping).
pub fn is_typed_object(obj: &Map<String, Value>) -> bool {
    obj.get("type").and_then(Value::as_str) == Some("object")
}

/// Extract type(s) from a schema's `type` field, handling both string and
/// array forms.
///
/// - `type: "object"` → `["object"]`
/// - `type: ["object", "null"]` → `["object", "null"]`
/// - absent or non-string/non-array → `[]`
///
/// Used by p1 (composition type intersection) and p9 (root type check).
pub fn extract_types(schema: &Value) -> Vec<String> {
    schema
        .get("type")
        .map(extract_type_strings)
        .unwrap_or_default()
}

/// Extract type strings from a raw `type` field value (not the enclosing
/// schema). Handles both `"string"` and `["string", "null"]` forms.
pub fn extract_type_strings(type_val: &Value) -> Vec<String> {
    match type_val {
        Value::String(s) => vec![s.clone()],
        Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------
// Nullable detection & wrapping
// ---------------------------------------------------------------------------

/// Check if a `type` value includes `"null"` — handles both string and array
/// forms.
pub fn type_contains_null(type_val: &Value) -> bool {
    match type_val {
        Value::String(s) => s == "null",
        Value::Array(arr) => arr.iter().any(|t| t.as_str() == Some("null")),
        _ => false,
    }
}

/// Check if a schema already allows `null`.
///
/// Two forms are recognised:
/// - `type: ["...", "null"]` (type array containing "null")
/// - `anyOf: [... , V]` where V has `type: "null"` or `type: ["null"]`
pub fn is_already_nullable(schema: &Value) -> bool {
    if let Some(obj) = schema.as_object() {
        // Check type array form: type: ["string", "null"]
        if let Some(type_val) = obj.get("type") {
            if type_contains_null(type_val) {
                return true;
            }
        }
        // Check anyOf form: anyOf: [..., {type: "null"}] or {type: ["null"]}
        if let Some(any_of) = obj.get("anyOf").and_then(Value::as_array) {
            if any_of
                .iter()
                .any(|v| v.get("type").map(type_contains_null).unwrap_or(false))
            {
                return true;
            }
        }
    }
    false
}

/// Wrap a single schema in `anyOf: [schema, {type: null}]`.
/// Hoists any top-level `description` and `title` into the non-null variant.
pub fn wrap_nullable(mut schema: Value) -> Value {
    // Extract metadata from the top level — it belongs on the non-null variant.
    let description = schema.as_object_mut().and_then(|o| o.remove("description"));
    let title = schema.as_object_mut().and_then(|o| o.remove("title"));

    let mut non_null_variant = schema;

    // Re-insert extracted metadata inside the non-null variant.
    if let Some(obj) = non_null_variant.as_object_mut() {
        if let Some(desc) = description {
            obj.insert("description".to_string(), desc);
        }
        if let Some(t) = title {
            obj.insert("title".to_string(), t);
        }
    }

    json!({
        "anyOf": [non_null_variant, {"type": "null"}]
    })
}

// ---------------------------------------------------------------------------
// Property introspection
// ---------------------------------------------------------------------------

/// Extract the current `required` array from a schema object as a set of
/// strings.
pub fn extract_required_set(obj: &Map<String, Value>) -> HashSet<String> {
    obj.get("required")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Extract all property keys from a schema object's `properties` map.
pub fn extract_property_keys(obj: &Map<String, Value>) -> Vec<String> {
    obj.get("properties")
        .and_then(Value::as_object)
        .map(|props| props.keys().cloned().collect())
        .unwrap_or_default()
}

/// Set `required` to all property keys.
pub fn set_all_required(obj: &mut Map<String, Value>, all_keys: &[String]) {
    let required: Vec<Value> = all_keys.iter().map(|k| Value::String(k.clone())).collect();
    obj.insert("required".to_string(), Value::Array(required));
}

// ---------------------------------------------------------------------------
// Strict enforcement
// ---------------------------------------------------------------------------

/// Wrap each optional property in `anyOf: [original_schema, {type: null}]`.
/// If the property schema is already nullable (has `type: ["...", "null"]`
/// or `anyOf` containing `{type: "null"}`), skips the wrap but still emits
/// a `NullableOptional` transform so the rehydrator knows to strip `null`.
pub fn wrap_optional_properties(
    obj: &mut Map<String, Value>,
    optional_keys: &[String],
    path: &str,
    transforms: &mut Vec<Transform>,
) {
    let props = match obj.get_mut("properties").and_then(Value::as_object_mut) {
        Some(p) => p,
        None => return,
    };

    for key in optional_keys {
        if let Some(prop_schema) = props.get(key) {
            if !is_already_nullable(prop_schema) {
                // Only clone when we actually need to wrap
                let wrapped = wrap_nullable(prop_schema.clone());
                props.insert(key.clone(), wrapped);
            }
            // Always emit transform — rehydrator needs to know null → undefined
            transforms.push(Transform::NullableOptional {
                path: build_path(path, &["properties", key]),
                original_required: false,
            });
        }
    }
}

/// Apply the three strict-mode transformations to a single object node:
///
/// 1. Wrap each optional property with `anyOf: [T, {type: null}]`
/// 2. Set `required` to all property keys in `properties` order
/// 3. Seal the object with `additionalProperties: false`
///
/// Produces `NullableOptional` codec transforms for each wrapped property.
pub fn enforce_object_strict(
    obj: &mut Map<String, Value>,
    path: &str,
    transforms: &mut Vec<Transform>,
) {
    let required_keys = extract_required_set(obj);
    let all_keys = extract_property_keys(obj);
    let optional_keys: Vec<String> = all_keys
        .iter()
        .filter(|k| !required_keys.contains(k.as_str()))
        .cloned()
        .collect();

    // 1. Wrap each optional property with anyOf: [T, {type: null}]
    wrap_optional_properties(obj, &optional_keys, path, transforms);

    // 2. Set `required` to all property keys in `properties` order
    set_all_required(obj, &all_keys);

    // 3. Seal the object
    obj.insert("additionalProperties".to_string(), Value::Bool(false));

    // 4. Strip unevaluatedProperties — it's redundant once additionalProperties
    //    is set, and providers like OpenAI strict mode reject it outright.
    obj.remove("unevaluatedProperties");
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- Type introspection ---

    #[test]
    fn test_is_typed_object_true() {
        let obj = json!({"type": "object"});
        assert!(is_typed_object(obj.as_object().unwrap()));
    }

    #[test]
    fn test_is_typed_object_false_string() {
        let obj = json!({"type": "string"});
        assert!(!is_typed_object(obj.as_object().unwrap()));
    }

    #[test]
    fn test_is_typed_object_false_missing() {
        let obj = json!({"properties": {}});
        assert!(!is_typed_object(obj.as_object().unwrap()));
    }

    #[test]
    fn test_extract_types_string() {
        let schema = json!({"type": "object"});
        assert_eq!(extract_types(&schema), vec!["object"]);
    }

    #[test]
    fn test_extract_types_array() {
        let schema = json!({"type": ["object", "null"]});
        assert_eq!(extract_types(&schema), vec!["object", "null"]);
    }

    #[test]
    fn test_extract_types_missing() {
        let schema = json!({"properties": {}});
        assert!(extract_types(&schema).is_empty());
    }

    // --- Nullable ---

    #[test]
    fn test_type_contains_null_string() {
        assert!(type_contains_null(&json!("null")));
        assert!(!type_contains_null(&json!("string")));
    }

    #[test]
    fn test_type_contains_null_array() {
        assert!(type_contains_null(&json!(["string", "null"])));
        assert!(!type_contains_null(&json!(["string", "integer"])));
    }

    #[test]
    fn test_is_already_nullable_type_array() {
        let schema = json!({"type": ["string", "null"]});
        assert!(is_already_nullable(&schema));
    }

    #[test]
    fn test_is_already_nullable_anyof() {
        let schema = json!({"anyOf": [{"type": "string"}, {"type": "null"}]});
        assert!(is_already_nullable(&schema));
    }

    #[test]
    fn test_is_already_nullable_no() {
        let schema = json!({"type": "string"});
        assert!(!is_already_nullable(&schema));
    }

    #[test]
    fn test_wrap_nullable() {
        let schema = json!({"type": "integer", "description": "count"});
        let wrapped = wrap_nullable(schema);
        let any_of = wrapped.get("anyOf").unwrap().as_array().unwrap();
        assert_eq!(any_of.len(), 2);
        assert_eq!(any_of[0]["type"], "integer");
        assert_eq!(any_of[0]["description"], "count");
        assert_eq!(any_of[1]["type"], "null");
    }

    // --- Property introspection ---

    #[test]
    fn test_extract_required_set() {
        let obj = json!({"required": ["a", "b"]});
        let set = extract_required_set(obj.as_object().unwrap());
        assert!(set.contains("a"));
        assert!(set.contains("b"));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_extract_required_set_missing() {
        let obj = json!({"properties": {}});
        assert!(extract_required_set(obj.as_object().unwrap()).is_empty());
    }

    #[test]
    fn test_extract_property_keys() {
        let obj = json!({"properties": {"x": {}, "y": {}}});
        let keys = extract_property_keys(obj.as_object().unwrap());
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"x".to_string()));
        assert!(keys.contains(&"y".to_string()));
    }

    #[test]
    fn test_set_all_required() {
        let mut obj = serde_json::Map::new();
        set_all_required(&mut obj, &["a".into(), "b".into()]);
        let req = obj.get("required").unwrap().as_array().unwrap();
        assert_eq!(req, &[json!("a"), json!("b")]);
    }

    // --- Strict enforcement ---

    #[test]
    fn test_enforce_object_strict_basic() {
        let mut schema = json!({
            "properties": {
                "r": {"type": "string"},
                "o": {"type": "integer"}
            },
            "required": ["r"]
        });
        let obj = schema.as_object_mut().unwrap();
        let mut transforms = Vec::new();

        enforce_object_strict(obj, "#", &mut transforms);

        // additionalProperties sealed
        assert_eq!(obj.get("additionalProperties"), Some(&json!(false)));

        // All properties required
        let req: Vec<String> = obj
            .get("required")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        assert!(req.contains(&"r".to_string()));
        assert!(req.contains(&"o".to_string()));

        // "o" wrapped in anyOf nullable
        let o_prop = obj.get("properties").unwrap().get("o").unwrap();
        assert!(o_prop.get("anyOf").is_some());

        // Transform emitted for optional "o"
        assert_eq!(transforms.len(), 1);
        assert!(matches!(
            &transforms[0],
            Transform::NullableOptional { path, .. } if path == "#/properties/o"
        ));
    }

    #[test]
    fn test_enforce_object_strict_strips_unevaluated_properties() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            },
            "required": ["name"],
            "unevaluatedProperties": false
        });
        let obj = schema.as_object_mut().unwrap();
        let mut transforms = Vec::new();

        enforce_object_strict(obj, "#", &mut transforms);

        // additionalProperties sealed
        assert_eq!(obj.get("additionalProperties"), Some(&json!(false)));
        // unevaluatedProperties stripped — OpenAI strict mode rejects this keyword
        assert!(
            obj.get("unevaluatedProperties").is_none(),
            "unevaluatedProperties should be stripped after enforce_object_strict"
        );
    }
}
