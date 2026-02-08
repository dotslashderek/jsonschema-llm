//! Pass 6: Strict Mode Enforcement
//!
//! For every `type: object` node:
//! 1. Set `additionalProperties: false`
//! 2. Move all properties into `required`
//! 3. Wrap originally-optional properties in `anyOf: [OriginalType, {type: null}]`
//!
//! Emits `NullableOptional` codec entries for each optional→nullable transformation.

use std::collections::HashSet;

use serde_json::{json, Map, Value};

use crate::codec::Transform;
use crate::config::ConvertOptions;
use crate::error::ConvertError;
use crate::schema_utils::{build_path, recurse_into_children};

/// Result of running the strict enforcement pass.
#[derive(Debug)]
pub struct StrictPassResult {
    /// The transformed schema with strict mode applied.
    pub schema: Value,
    /// Codec transforms produced by this pass (one per optional→nullable field).
    pub transforms: Vec<Transform>,
}

/// Apply strict mode enforcement to a schema.
///
/// Recursively walks every node. For `type: object` nodes with `properties`,
/// seals them with `additionalProperties: false`, makes all properties required,
/// and wraps optional properties with `anyOf: [T, {type: null}]`.
pub fn enforce_strict(
    schema: &Value,
    config: &ConvertOptions,
) -> Result<StrictPassResult, ConvertError> {
    let mut transforms = Vec::new();
    let result = walk(schema, "#", 0, config, &mut transforms)?;
    Ok(StrictPassResult {
        schema: result,
        transforms,
    })
}

// ---------------------------------------------------------------------------
// Recursive walker
// ---------------------------------------------------------------------------

/// Recursively descend through the schema tree, enforcing strict mode at each
/// `type: object` node and collecting codec transforms.
fn walk(
    node: &Value,
    path: &str,
    depth: usize,
    config: &ConvertOptions,
    transforms: &mut Vec<Transform>,
) -> Result<Value, ConvertError> {
    if depth > config.max_depth {
        return Err(ConvertError::RecursionDepthExceeded {
            path: path.to_string(),
            max_depth: config.max_depth,
        });
    }

    let obj = match node.as_object() {
        Some(o) => o,
        None => return Ok(node.clone()),
    };

    let mut result = obj.clone();

    // If this is a `type: object`, enforce strict mode.
    // Objects without `properties` get sealed with an empty properties map.
    if is_typed_object(&result) {
        if !result.contains_key("properties") {
            result.insert("properties".to_string(), json!({}));
        }
        enforce_object_strict(&mut result, path, transforms);
    }

    // Recurse into all structural children that may contain nested schemas.
    recurse_into_children(&mut result, path, depth, &mut |val, child_path, d| {
        walk(val, child_path, d, config, transforms)
    })?;

    Ok(Value::Object(result))
}

// ---------------------------------------------------------------------------
// Object-level strict enforcement (the 3 transformations)
// ---------------------------------------------------------------------------

/// Apply the three strict-mode transformations to a single `type: object` node.
fn enforce_object_strict(
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
}

// ---------------------------------------------------------------------------
// Small composable helpers
// ---------------------------------------------------------------------------

/// Check whether a JSON object has `"type": "object"`.
fn is_typed_object(obj: &Map<String, Value>) -> bool {
    obj.get("type").and_then(Value::as_str) == Some("object")
}

/// Extract the current `required` array from a schema object as a set of strings.
fn extract_required_set(obj: &Map<String, Value>) -> HashSet<String> {
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
fn extract_property_keys(obj: &Map<String, Value>) -> Vec<String> {
    obj.get("properties")
        .and_then(Value::as_object)
        .map(|props| props.keys().cloned().collect())
        .unwrap_or_default()
}

/// Wrap each optional property in `anyOf: [original_schema, {type: null}]`.
/// If the property schema is already nullable (has `type: ["...", "null"]`
/// or `anyOf` containing `{type: "null"}`), skips the wrap but still emits
/// a `NullableOptional` transform so the rehydrator knows to strip `null`.
fn wrap_optional_properties(
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
        if let Some(prop_schema) = props.get(key).cloned() {
            if !is_already_nullable(&prop_schema) {
                let wrapped = wrap_nullable(prop_schema);
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

/// Check if a schema already allows `null`.
///
/// Two forms are recognised:
/// - `type: ["...", "null"]` (type array containing "null")
/// - `anyOf: [... , {type: "null"}]` (variant list containing null type)
fn is_already_nullable(schema: &Value) -> bool {
    if let Some(obj) = schema.as_object() {
        // Check type array form: type: ["string", "null"]
        if let Some(type_val) = obj.get("type") {
            if let Some(arr) = type_val.as_array() {
                if arr.iter().any(|t| t.as_str() == Some("null")) {
                    return true;
                }
            }
        }
        // Check anyOf form: anyOf: [..., {type: "null"}]
        if let Some(any_of) = obj.get("anyOf").and_then(Value::as_array) {
            if any_of
                .iter()
                .any(|v| v.get("type").and_then(Value::as_str) == Some("null"))
            {
                return true;
            }
        }
    }
    false
}

/// Wrap a single schema in `anyOf: [schema, {type: null}]`.
/// Hoists any top-level `description` and `title` into the non-null variant.
fn wrap_nullable(mut schema: Value) -> Value {
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

/// Set `required` to all property keys.
fn set_all_required(obj: &mut Map<String, Value>, all_keys: &[String]) {
    let required: Vec<Value> = all_keys.iter().map(|k| Value::String(k.clone())).collect();
    obj.insert("required".to_string(), Value::Array(required));
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

    fn run(schema: Value) -> (Value, Vec<Transform>) {
        let result = enforce_strict(&schema, &ConvertOptions::default()).unwrap();
        (result.schema, result.transforms)
    }

    // -----------------------------------------------------------------------
    // Test 1: Simple object with mixed required/optional
    // -----------------------------------------------------------------------
    #[test]
    fn test_simple_mixed_required_optional() {
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" },
                "email": { "type": "string" }
            },
            "required": ["name"]
        });

        let (output, transforms) = run(input);

        // name stays as-is (already required)
        assert_eq!(output["properties"]["name"], json!({"type": "string"}));

        // age + email wrapped in anyOf
        assert_eq!(
            output["properties"]["age"],
            json!({"anyOf": [{"type": "integer"}, {"type": "null"}]})
        );
        assert_eq!(
            output["properties"]["email"],
            json!({"anyOf": [{"type": "string"}, {"type": "null"}]})
        );

        // all keys required
        let required = output["required"].as_array().unwrap();
        assert!(required.contains(&json!("name")));
        assert!(required.contains(&json!("age")));
        assert!(required.contains(&json!("email")));

        // additionalProperties sealed
        assert_eq!(output["additionalProperties"], json!(false));

        // two codec entries (age + email)
        assert_eq!(transforms.len(), 2);
    }

    // -----------------------------------------------------------------------
    // Test 2: Nested objects — strict enforcement recurses
    // -----------------------------------------------------------------------
    #[test]
    fn test_nested_objects() {
        let input = json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "bio": { "type": "string" }
                    },
                    "required": ["name"]
                }
            },
            "required": ["user"]
        });

        let (output, transforms) = run(input);

        // Outer object sealed
        assert_eq!(output["additionalProperties"], json!(false));

        // Inner object also sealed
        let user = &output["properties"]["user"];
        assert_eq!(user["additionalProperties"], json!(false));

        // Inner optional (bio) wrapped
        assert_eq!(
            user["properties"]["bio"],
            json!({"anyOf": [{"type": "string"}, {"type": "null"}]})
        );

        // Inner required includes both keys
        let inner_required = user["required"].as_array().unwrap();
        assert!(inner_required.contains(&json!("name")));
        assert!(inner_required.contains(&json!("bio")));

        // 1 codec entry for bio
        assert_eq!(transforms.len(), 1);
        match &transforms[0] {
            Transform::NullableOptional { path, .. } => {
                assert_eq!(path, "#/properties/user/properties/bio");
            }
            _ => panic!("Expected NullableOptional transform"),
        }
    }

    // -----------------------------------------------------------------------
    // Test 3: Object with no `required` array — all become nullable
    // -----------------------------------------------------------------------
    #[test]
    fn test_no_required_array() {
        let input = json!({
            "type": "object",
            "properties": {
                "a": { "type": "string" },
                "b": { "type": "integer" }
            }
        });

        let (output, transforms) = run(input);

        // Both wrapped
        assert!(output["properties"]["a"]["anyOf"].is_array());
        assert!(output["properties"]["b"]["anyOf"].is_array());

        // Both in required
        let required = output["required"].as_array().unwrap();
        assert_eq!(required.len(), 2);

        // 2 codec entries
        assert_eq!(transforms.len(), 2);
    }

    // -----------------------------------------------------------------------
    // Test 4: Already strict — idempotent (no-op)
    // -----------------------------------------------------------------------
    #[test]
    fn test_idempotent() {
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "email": { "type": "string" }
            },
            "required": ["name"]
        });

        let (first_pass, transforms1) = run(input);
        let (second_pass, transforms2) = run(first_pass.clone());

        // Schema unchanged after second pass
        assert_eq!(first_pass, second_pass);

        // No new transforms on second pass (email is already anyOf-wrapped
        // and already in required, so no optional keys remain)
        assert_eq!(transforms2.len(), 0);

        // First pass produced 1 transform (email)
        assert_eq!(transforms1.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Test 5: Description on optional field — moves inside anyOf variant
    // -----------------------------------------------------------------------
    #[test]
    fn test_description_moves_inside_any_of() {
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "bio": {
                    "type": "string",
                    "description": "A short biography"
                }
            },
            "required": ["name"]
        });

        let (output, _) = run(input);

        let bio = &output["properties"]["bio"];
        let any_of = bio["anyOf"].as_array().unwrap();

        // Non-null variant has the description
        assert_eq!(any_of[0]["description"], json!("A short biography"));
        assert_eq!(any_of[0]["type"], json!("string"));

        // Null variant is clean
        assert_eq!(any_of[1], json!({"type": "null"}));

        // No top-level description on the wrapper
        assert!(bio.get("description").is_none());
    }

    // -----------------------------------------------------------------------
    // Test 6: Objects inside anyOf/arrays — strict enforcement at all depths
    // -----------------------------------------------------------------------
    #[test]
    fn test_objects_in_any_of_and_arrays() {
        let input = json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "value": { "type": "number" }
                        },
                        "required": ["id"]
                    }
                },
                "variant": {
                    "anyOf": [
                        {
                            "type": "object",
                            "properties": {
                                "x": { "type": "integer" }
                            }
                        },
                        {
                            "type": "object",
                            "properties": {
                                "y": { "type": "integer" }
                            },
                            "required": ["y"]
                        }
                    ]
                }
            },
            "required": ["items", "variant"]
        });

        let (output, transforms) = run(input);

        // Array item object is sealed
        let array_item = &output["properties"]["items"]["items"];
        assert_eq!(array_item["additionalProperties"], json!(false));
        assert_eq!(
            array_item["properties"]["value"],
            json!({"anyOf": [{"type": "number"}, {"type": "null"}]})
        );

        // anyOf variant 0 (no required → x becomes nullable)
        let v0 = &output["properties"]["variant"]["anyOf"][0];
        assert_eq!(v0["additionalProperties"], json!(false));
        assert!(v0["properties"]["x"]["anyOf"].is_array());

        // anyOf variant 1 (y already required → stays as-is)
        let v1 = &output["properties"]["variant"]["anyOf"][1];
        assert_eq!(v1["additionalProperties"], json!(false));
        assert_eq!(v1["properties"]["y"], json!({"type": "integer"}));

        // Codec entries: value (array item), x (anyOf variant 0)
        assert_eq!(transforms.len(), 2);
    }

    // -----------------------------------------------------------------------
    // Test 7: Depth guard triggers
    // -----------------------------------------------------------------------
    #[test]
    fn test_depth_guard() {
        let input = json!({
            "type": "object",
            "properties": {
                "nested": {
                    "type": "object",
                    "properties": {
                        "deep": { "type": "string" }
                    }
                }
            },
            "required": ["nested"]
        });

        let config = ConvertOptions {
            max_depth: 1,
            ..ConvertOptions::default()
        };

        let result = enforce_strict(&input, &config);
        let err = result.unwrap_err();
        match err {
            ConvertError::RecursionDepthExceeded { max_depth, .. } => {
                assert_eq!(max_depth, config.max_depth);
            }
            other => panic!("expected RecursionDepthExceeded, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Test 8: Empty object (no properties) — sealed with empty props
    // -----------------------------------------------------------------------
    #[test]
    fn test_empty_object_sealed() {
        let input = json!({"type": "object"});

        let (output, transforms) = run(input);

        assert_eq!(output["additionalProperties"], json!(false));
        assert_eq!(output["properties"], json!({}));
        assert_eq!(output["required"], json!([]));
        assert_eq!(transforms.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 9: Title hoisted alongside description in anyOf wrapping
    // -----------------------------------------------------------------------
    #[test]
    fn test_title_hoisted_in_any_of() {
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "bio": {
                    "type": "string",
                    "title": "Biography",
                    "description": "A short biography"
                }
            },
            "required": ["name"]
        });

        let (output, _) = run(input);

        let bio = &output["properties"]["bio"];
        let any_of = bio["anyOf"].as_array().unwrap();

        // Non-null variant has both title and description
        assert_eq!(any_of[0]["title"], json!("Biography"));
        assert_eq!(any_of[0]["description"], json!("A short biography"));
        assert_eq!(any_of[0]["type"], json!("string"));

        // No top-level title or description on wrapper
        assert!(bio.get("title").is_none());
        assert!(bio.get("description").is_none());
    }

    // -- #15: Skip already-nullable tests --

    #[test]
    fn test_skip_already_nullable_anyof() {
        // Optional property with anyOf: [T, {type: null}] — already nullable
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "alias": {
                    "anyOf": [
                        { "type": "string" },
                        { "type": "null" }
                    ]
                }
            },
            "required": ["name"]
        });
        let (schema, _transforms) = run(input);
        let alias = &schema["properties"]["alias"];
        // Should NOT be double-wrapped (no anyOf containing anyOf)
        let any_of = alias["anyOf"].as_array().unwrap();
        assert_eq!(
            any_of.len(),
            2,
            "should keep original 2 variants, not add another null"
        );
        assert_eq!(any_of[0], json!({ "type": "string" }));
        assert_eq!(any_of[1], json!({ "type": "null" }));
    }

    #[test]
    fn test_skip_already_nullable_type_array() {
        // Optional property with type: ["string", "null"] — already nullable
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "nickname": { "type": ["string", "null"] }
            },
            "required": ["name"]
        });
        let (schema, _transforms) = run(input);
        let nickname = &schema["properties"]["nickname"];
        // Should NOT be wrapped in anyOf — type array already allows null
        assert_eq!(nickname["type"], json!(["string", "null"]));
        assert!(
            nickname.get("anyOf").is_none(),
            "should not add anyOf wrapper"
        );
    }

    #[test]
    fn test_nullable_still_emits_transform() {
        // Even when we skip wrapping, we still emit NullableOptional transform
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "alias": { "type": ["string", "null"] }
            },
            "required": ["name"]
        });
        let (_schema, transforms) = run(input);
        // Transform should exist for the already-nullable field
        let alias_transform = transforms.iter().find(|t| match t {
            Transform::NullableOptional { path, .. } => path.contains("alias"),
            _ => false,
        });
        assert!(
            alias_transform.is_some(),
            "must emit transform even when skipping wrap"
        );
    }
}
