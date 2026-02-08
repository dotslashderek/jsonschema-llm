//! Pass 6: Strict Mode Enforcement
//!
//! For every `type: object` node:
//! 1. Set `additionalProperties: false`
//! 2. Move all properties into `required`
//! 3. Wrap originally-optional properties in `anyOf: [OriginalType, {type: null}]`
//!
//! Emits `NullableOptional` codec entries for each optional→nullable transformation.

use serde_json::{json, Map, Value};

use crate::codec::Transform;
use crate::config::ConvertOptions;
use crate::error::ConvertError;

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

    // If this is a `type: object` with `properties`, enforce strict mode.
    if is_typed_object(&result) && result.contains_key("properties") {
        enforce_object_strict(&mut result, path, transforms);
    }

    // Recurse into all structural children that may contain nested schemas.
    recurse_into_properties(&mut result, path, depth, config, transforms)?;
    recurse_into_items(&mut result, path, depth, config, transforms)?;
    recurse_into_variants(&mut result, "anyOf", path, depth, config, transforms)?;
    recurse_into_variants(&mut result, "oneOf", path, depth, config, transforms)?;
    recurse_into_variants(&mut result, "allOf", path, depth, config, transforms)?;
    recurse_into_additional_properties(&mut result, path, depth, config, transforms)?;

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
        .filter(|k| !required_keys.contains(k))
        .cloned()
        .collect();

    // 1. Wrap each optional property with anyOf: [T, {type: null}]
    wrap_optional_properties(obj, &optional_keys, path, transforms);

    // 2. Set `required` to all property keys (preserving original order + appending optionals)
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
fn extract_required_set(obj: &Map<String, Value>) -> Vec<String> {
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
/// If the original schema has a top-level `description`, move it inside the
/// non-null `anyOf` variant.
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
            let wrapped = wrap_nullable(prop_schema);
            props.insert(key.clone(), wrapped);

            transforms.push(Transform::NullableOptional {
                path: format!("{}/properties/{}", path, key),
                original_required: false,
            });
        }
    }
}

/// Wrap a single schema in `anyOf: [schema, {type: null}]`.
/// Hoists any top-level `description` into the non-null variant.
fn wrap_nullable(mut schema: Value) -> Value {
    // Extract description from the top level — it belongs on the non-null variant.
    let description = schema.as_object_mut().and_then(|o| o.remove("description"));

    let mut non_null_variant = schema;

    // If we extracted a description, put it inside the non-null variant.
    if let Some(desc) = description {
        if let Some(obj) = non_null_variant.as_object_mut() {
            obj.insert("description".to_string(), desc);
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

// ---------------------------------------------------------------------------
// Recursive descent into child schemas
// ---------------------------------------------------------------------------

/// Recurse into each value inside `properties`.
fn recurse_into_properties(
    obj: &mut Map<String, Value>,
    path: &str,
    depth: usize,
    config: &ConvertOptions,
    transforms: &mut Vec<Transform>,
) -> Result<(), ConvertError> {
    if let Some(Value::Object(props)) = obj.get("properties").cloned() {
        let mut new_props = Map::new();
        for (key, val) in &props {
            let child_path = format!("{}/properties/{}", path, key);
            let walked = walk(val, &child_path, depth + 1, config, transforms)?;
            new_props.insert(key.clone(), walked);
        }
        obj.insert("properties".to_string(), Value::Object(new_props));
    }
    Ok(())
}

/// Recurse into `items` (array element schema).
fn recurse_into_items(
    obj: &mut Map<String, Value>,
    path: &str,
    depth: usize,
    config: &ConvertOptions,
    transforms: &mut Vec<Transform>,
) -> Result<(), ConvertError> {
    if let Some(items) = obj.get("items").cloned() {
        let child_path = format!("{}/items", path);
        let walked = walk(&items, &child_path, depth + 1, config, transforms)?;
        obj.insert("items".to_string(), walked);
    }
    Ok(())
}

/// Recurse into each variant of an `anyOf`, `oneOf`, or `allOf` array.
fn recurse_into_variants(
    obj: &mut Map<String, Value>,
    keyword: &str,
    path: &str,
    depth: usize,
    config: &ConvertOptions,
    transforms: &mut Vec<Transform>,
) -> Result<(), ConvertError> {
    if let Some(Value::Array(variants)) = obj.get(keyword).cloned() {
        let mut new_variants = Vec::new();
        for (i, variant) in variants.iter().enumerate() {
            let child_path = format!("{}/{}/{}", path, keyword, i);
            let walked = walk(variant, &child_path, depth + 1, config, transforms)?;
            new_variants.push(walked);
        }
        obj.insert(keyword.to_string(), Value::Array(new_variants));
    }
    Ok(())
}

/// Recurse into `additionalProperties` when it is a schema object (not a bool).
fn recurse_into_additional_properties(
    obj: &mut Map<String, Value>,
    path: &str,
    depth: usize,
    config: &ConvertOptions,
    transforms: &mut Vec<Transform>,
) -> Result<(), ConvertError> {
    if let Some(ap) = obj.get("additionalProperties").cloned() {
        if ap.is_object() {
            let child_path = format!("{}/additionalProperties", path);
            let walked = walk(&ap, &child_path, depth + 1, config, transforms)?;
            obj.insert("additionalProperties".to_string(), walked);
        }
    }
    Ok(())
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
    // Test 7 (bonus): Depth guard triggers
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
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Recursion depth exceeded"));
    }
}
