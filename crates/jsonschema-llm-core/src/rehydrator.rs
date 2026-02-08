//! Rehydrator — reconstructs original data shape from LLM output using the codec.

use crate::codec::{Codec, Transform};
use crate::error::ConvertError;
use serde_json::Value;

/// Rehydrate LLM output using the codec sidecar.
///
/// Applies transforms in REVERSE order (LIFO) to undo the stack of changes.
pub fn rehydrate(data: &Value, codec: &Codec) -> Result<Value, ConvertError> {
    let mut result = data.clone();

    for transform in codec.transforms.iter().rev() {
        let path_str = match transform {
            Transform::MapToArray { path, .. } => path,
            Transform::JsonStringParse { path } => path,
            Transform::NullableOptional { path, .. } => path,
            Transform::DiscriminatorAnyOf { path, .. } => path,
            Transform::ExtractAdditionalProperties { path, .. } => path,
        };

        // Normalize path: split by '/', filter empty and '#'
        let path_parts: Vec<&str> = path_str
            .split('/')
            .filter(|p| !p.is_empty() && *p != "#")
            .collect();

        apply_transform(&mut result, &path_parts, transform)?;
    }

    Ok(result)
}

fn apply_transform(
    data: &mut Value,
    path_parts: &[&str],
    transform: &Transform,
) -> Result<(), ConvertError> {
    // 1. Array Iteration: Handle "items" segment
    if let Some(&"items") = path_parts.first() {
        if let Some(arr) = data.as_array_mut() {
            for item in arr {
                apply_transform(item, &path_parts[1..], transform)?;
            }
        }
        // If data is not an array (e.g. null or mismatched), we skip silently (robustness).
        return Ok(());
    }

    // 2. Object Navigation: Handle "properties" -> "key"
    if let Some(&"properties") = path_parts.first() {
        if let Some(key) = path_parts.get(1) {
            // SPECIAL CASE: NullableOptional
            // If we are at the parent object and the path targets a child property that is Nullable,
            // we perform the check here to have access to the parent map for removal.
            if let Transform::NullableOptional { .. } = transform {
                // If this is the last hop (properties/KEY)
                if path_parts.len() == 2 {
                    if let Some(obj) = data.as_object_mut() {
                        // Check if the value exists and is null
                        if let Some(val) = obj.get(*key) {
                            if val.is_null() {
                                obj.remove(*key);
                            }
                        }
                    }
                    return Ok(());
                }
            }

            // Normal Navigation
            if let Some(obj) = data.as_object_mut() {
                // If the key exists, recurse. If not, we can't traverse further, so we stop.
                if let Some(child) = obj.get_mut(*key) {
                    apply_transform(child, &path_parts[2..], transform)?;
                }
            }
            return Ok(());
        }
    }

    // 3. End of Path - Execute Value Transforms
    if path_parts.is_empty() {
        match transform {
            Transform::MapToArray { key_field, .. } => {
                restore_map(data, key_field)?;
            }
            Transform::JsonStringParse { .. } => {
                parse_json_string(data)?;
            }
            Transform::ExtractAdditionalProperties { property_name, .. } => {
                restore_additional_properties(data, property_name)?;
            }
            Transform::NullableOptional { .. } => {
                // Should have been handled in the navigation step.
            }
            Transform::DiscriminatorAnyOf { .. } => {
                // No-op
            }
        }
    }

    Ok(())
}

fn restore_map(data: &mut Value, key_field: &str) -> Result<(), ConvertError> {
    // Expecting Array of Objects -> Object
    if let Some(arr) = data.as_array() {
        // Pre-validate: every entry must be an object with key_field (string) and "value".
        // If any entry is malformed, skip the entire transform to avoid data loss.
        let all_valid = arr.iter().all(|item| {
            item.as_object()
                .map(|obj| {
                    obj.get(key_field).and_then(|v| v.as_str()).is_some()
                        && obj.contains_key("value")
                })
                .unwrap_or(false)
        });

        if !all_valid {
            return Ok(()); // Skip silently — preserve original array
        }

        let mut map = serde_json::Map::new();
        for item in arr {
            let obj = item.as_object().unwrap(); // safe: pre-validated
            let k = obj.get(key_field).unwrap().as_str().unwrap();
            let v = obj.get("value").unwrap();
            map.insert(k.to_string(), v.clone()); // Duplicate keys: last wins
        }
        *data = Value::Object(map);
    }
    Ok(())
}

fn parse_json_string(data: &mut Value) -> Result<(), ConvertError> {
    if let Some(s) = data.as_str() {
        match serde_json::from_str::<Value>(s) {
            Ok(parsed) => *data = parsed,
            Err(e) => {
                // Truncate to avoid leaking large LLM output into logs
                let preview: String = s.chars().take(100).collect();
                return Err(ConvertError::RehydrationError(format!(
                    "Failed to parse JSON string ({}): {}...",
                    e, preview
                )));
            }
        }
    }
    Ok(())
}

fn restore_additional_properties(
    data: &mut Value,
    property_name: &str,
) -> Result<(), ConvertError> {
    if let Some(obj) = data.as_object_mut() {
        // Validate type BEFORE removing — don't drop non-object values
        let is_object = obj
            .get(property_name)
            .map(|v| v.is_object())
            .unwrap_or(false);

        if is_object {
            let extra = obj.remove(property_name).unwrap(); // safe: checked above
            for (k, v) in extra.as_object().unwrap() {
                obj.insert(k.clone(), v.clone());
            }
        }
        // If property_name is missing or not an object, skip silently
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Test 1: Strip Nullable
    #[test]
    fn test_strip_nullable() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::NullableOptional {
            path: "#/properties/optional".to_string(),
            original_required: false,
        });

        // Case A: Null value -> should be removed
        let data = json!({
            "fixed": 1,
            "optional": null
        });
        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result, json!({"fixed": 1}));

        // Case B: Present value -> should be kept
        let data_present = json!({
            "fixed": 1,
            "optional": "kept"
        });
        let result_present = rehydrate(&data_present, &codec).unwrap();
        assert_eq!(result_present, json!({"fixed": 1, "optional": "kept"}));
    }

    // Test 2: Restore Map
    #[test]
    fn test_restore_map() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::MapToArray {
            path: "#/properties/map".to_string(),
            key_field: "key".to_string(),
        });

        let data = json!({
            "map": [
                {"key": "a", "value": 1},
                {"key": "b", "value": 2}
            ]
        });

        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(
            result,
            json!({
                "map": {"a": 1, "b": 2}
            })
        );
    }

    // Test 3: Parse JSON String
    #[test]
    fn test_parse_json_string() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::JsonStringParse {
            path: "#/properties/config".to_string(),
        });

        let data = json!({
            "config": "{\"debug\": true}"
        });

        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(
            result,
            json!({
                "config": {"debug": true}
            })
        );
    }

    // Test 4: Parse JSON String - Error
    #[test]
    fn test_parse_json_string_error() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::JsonStringParse {
            path: "#/properties/config".to_string(),
        });

        let data = json!({
            "config": "{invalid"
        });

        let result = rehydrate(&data, &codec);
        assert!(matches!(result, Err(ConvertError::RehydrationError(_))));
    }

    // Test 5: Combined
    #[test]
    fn test_combined() {
        let mut codec = Codec::new();
        // Order matters! Transforms are applied LIFO.
        // During hydration (forward): 1. StripNullable 2. MapToArray
        // During rehydration (reverse): 1. MapToArray 2. StripNullable (actually independent here)
        codec.transforms.push(Transform::NullableOptional {
            path: "#/properties/optional".to_string(),
            original_required: false,
        });
        codec.transforms.push(Transform::MapToArray {
            path: "#/properties/map".to_string(),
            key_field: "key".to_string(),
        });

        let data = json!({
            "optional": null,
            "map": [{"key": "a", "value": 1}]
        });

        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(
            result,
            json!({
                "map": {"a": 1}
            })
        );
    }

    // Test 6: Nested / Arrays
    #[test]
    fn test_array_traversal() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::JsonStringParse {
            path: "#/properties/list/items/properties/data".to_string(),
        });

        let data = json!({
            "list": [
                {"data": "{\"id\": 1}"},
                {"data": "{\"id\": 2}"}
            ]
        });

        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result["list"][0]["data"], json!({"id": 1}));
        assert_eq!(result["list"][1]["data"], json!({"id": 2}));
    }

    // Test 7: Extract Additional Properties
    #[test]
    fn test_extract_ap() {
        let mut codec = Codec::new();
        // Forward: 1. ExtractAP (moves fields to _extra)
        // Reverse: 1. RestoreAP (moves _extra fields back to parent)
        codec
            .transforms
            .push(Transform::ExtractAdditionalProperties {
                path: "#".to_string(),
                property_name: "_extra".to_string(),
            });

        let data = json!({
            "fixed": "keep",
            "_extra": {
                "dynamic1": 100,
                "dynamic2": 200
            }
        });

        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(
            result,
            json!({
                "fixed": "keep",
                "dynamic1": 100,
                "dynamic2": 200
            })
        );
    }

    // Test 8: Empty Codec
    #[test]
    fn test_empty_codec() {
        let codec = Codec::new();
        let data = json!({"a": 1});
        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result, data);
    }

    // Test 9: Restore map with duplicate keys — last wins
    #[test]
    fn test_restore_map_dup_keys() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::MapToArray {
            path: "#/properties/map".to_string(),
            key_field: "key".to_string(),
        });

        let data = json!({
            "map": [
                {"key": "dup", "value": 1},
                {"key": "dup", "value": 2}
            ]
        });

        let result = rehydrate(&data, &codec).unwrap();
        // Last wins semantics
        assert_eq!(result["map"]["dup"], json!(2));
    }

    // Test 10: Nested transforms at different depths
    #[test]
    fn test_nested_depth() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::NullableOptional {
            path: "#/properties/outer/properties/inner".to_string(),
            original_required: false,
        });
        codec.transforms.push(Transform::JsonStringParse {
            path: "#/properties/outer/properties/config".to_string(),
        });

        let data = json!({
            "outer": {
                "inner": null,
                "config": "{\"x\": 1}"
            }
        });

        let result = rehydrate(&data, &codec).unwrap();
        assert!(result["outer"].get("inner").is_none());
        assert_eq!(result["outer"]["config"], json!({"x": 1}));
    }

    // Test 11: Malformed map entries — preserve original array
    #[test]
    fn test_restore_map_malformed_skips() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::MapToArray {
            path: "#/properties/map".to_string(),
            key_field: "key".to_string(),
        });

        // Second entry is missing "value" field → entire transform should be skipped
        let data = json!({
            "map": [
                {"key": "a", "value": 1},
                {"key": "b"}
            ]
        });

        let result = rehydrate(&data, &codec).unwrap();
        // Original array preserved, not partially converted
        assert!(result["map"].is_array());
        assert_eq!(result["map"].as_array().unwrap().len(), 2);
    }

    // Test 12: Non-object extra property — preserve original value
    #[test]
    fn test_extract_ap_non_object_preserved() {
        let mut codec = Codec::new();
        codec
            .transforms
            .push(Transform::ExtractAdditionalProperties {
                path: "#".to_string(),
                property_name: "_extra".to_string(),
            });

        // _extra is a string, not an object → should be preserved as-is
        let data = json!({
            "fixed": "keep",
            "_extra": "not an object"
        });

        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result["_extra"], json!("not an object"));
        assert_eq!(result["fixed"], json!("keep"));
    }
}
