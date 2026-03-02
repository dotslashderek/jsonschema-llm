//! Value-level transform execution — the leaf operations applied at data nodes.
//!
//! Each function handles one type of codec transform: map restoration,
//! JSON string parsing, additional properties restoration, root object unwrapping,
//! and enum de-stringification.

use serde_json::Value;

use crate::codec::Transform;
use crate::codec_warning::{Warning, WarningKind};
use crate::error::ConvertError;

/// Execute a value-level transform at the current data node.
///
/// Warnings are accumulated into the provided `warnings` vec rather than returned
/// as errors, allowing the rehydration pipeline to continue processing.
pub(super) fn execute_transform(
    data: &mut Value,
    transform: &Transform,
    warnings: &mut Vec<Warning>,
) -> Result<(), ConvertError> {
    match transform {
        Transform::MapToArray { key_field, .. } => {
            restore_map(data, key_field)?;
        }
        Transform::JsonStringParse { path, .. } => {
            parse_json_string(data, path, warnings)?;
        }
        Transform::ExtractAdditionalProperties { property_name, .. } => {
            restore_additional_properties(data, property_name)?;
        }
        Transform::NullableOptional { .. } => {
            // Handled in the navigation step.
        }
        Transform::DiscriminatorAnyOf { .. } => {
            // No-op
        }
        Transform::RecursiveInflate { path, .. } => {
            parse_json_string(data, path, warnings)?;
        }
        Transform::RootObjectWrapper { wrapper_key, .. } => {
            // Unwrap: extract data[wrapper_key] and promote it to root.
            // Fail loudly if the wrapper object is missing/invalid to avoid silently
            // accepting malformed LLM output.
            let obj = data.as_object_mut().ok_or_else(|| {
                ConvertError::RehydrationError(format!(
                    "Expected root object with wrapper key `{}` but found non-object value",
                    wrapper_key
                ))
            })?;

            if !obj.contains_key(wrapper_key) {
                return Err(ConvertError::RehydrationError(format!(
                    "Expected wrapper key `{}` at root object but it was missing",
                    wrapper_key
                )));
            }

            // LLMs (especially OpenAI with anyOf schemas) sometimes leak properties
            // from inner branches to the outer wrapper object. Strip extra keys
            // with a warning rather than hard-failing, since the wrapper key's data
            // is still intact and usable.
            if obj.len() > 1 {
                let extra_keys: Vec<String> = obj
                    .keys()
                    .filter(|k| k.as_str() != wrapper_key)
                    .cloned()
                    .collect();
                tracing::warn!(
                    "Root wrapper object had extra keys beyond `{}`: {:?}; stripping",
                    wrapper_key,
                    extra_keys
                );
                for key in &extra_keys {
                    obj.remove(key);
                }
            }

            if let Some(inner) = obj.remove(wrapper_key) {
                *data = inner;
            }
        }
        Transform::EnumStringify {
            original_values, ..
        } => {
            // Reverse stringify: match the string value back to the original typed value
            if let Some(s) = data.as_str() {
                for orig in original_values {
                    let stringified = match orig {
                        Value::String(os) => os.clone(),
                        other => other.to_string(),
                    };
                    if s == stringified {
                        *data = orig.clone();
                        break;
                    }
                }
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
            let obj = item
                .as_object()
                .expect("invariant: all_valid guard above ensures every item is an object");
            let k = obj
                .get(key_field)
                .expect("invariant: all_valid guard above ensures key_field is present")
                .as_str()
                .expect("invariant: all_valid guard above ensures key_field is a string");
            let v = obj
                .get("value")
                .expect("invariant: all_valid guard above ensures 'value' key is present");
            map.insert(k.to_string(), v.clone()); // Duplicate keys: last wins
        }
        *data = Value::Object(map);
    }
    Ok(())
}

/// Parse a JSON-encoded string value back into a structured JSON value.
///
/// Gracefully handles common LLM failure modes:
/// - **Empty/whitespace-only strings** → `Value::Null` (the LLM's way of saying "nothing"
///   when the field is required and typed as string)
/// - **Non-empty, non-JSON strings** → `Value::Null` + advisory warning (possible hallucination)
/// - **Valid JSON strings** → parsed value (happy path)
/// - **Non-string values** (null, numbers, etc.) → no-op
fn parse_json_string(
    data: &mut Value,
    schema_path: &str,
    warnings: &mut Vec<Warning>,
) -> Result<(), ConvertError> {
    if let Some(s) = data.as_str() {
        // Empty or whitespace-only → null (clear "nothing" intent from LLM)
        if s.trim().is_empty() {
            *data = Value::Null;
            return Ok(());
        }

        match serde_json::from_str::<Value>(s) {
            Ok(parsed) => *data = parsed,
            Err(e) => {
                // Non-JSON string → coerce to null and emit advisory warning
                let preview: String = s.chars().take(100).collect();
                tracing::warn!(
                    path = %schema_path,
                    error = %e,
                    preview = %preview,
                    "json_string_parse: non-JSON string coerced to null"
                );
                warnings.push(Warning {
                    data_path: schema_path.to_string(),
                    schema_path: schema_path.to_string(),
                    kind: WarningKind::InvalidTransformInput {
                        transform: "json_string_parse".to_string(),
                    },
                    message: format!(
                        "json_string_parse: value is not valid JSON ({}), coerced to null: {}...",
                        e, preview
                    ),
                });
                *data = Value::Null;
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
            let extra = obj
                .remove(property_name)
                .expect("invariant: is_object guard above confirms property_name exists");
            for (k, v) in extra
                .as_object()
                .expect("invariant: is_object guard above confirms this is an object")
            {
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

    // -----------------------------------------------------------------------
    // restore_map
    // -----------------------------------------------------------------------

    #[test]
    fn restore_map_empty_array_yields_empty_object() {
        let mut data = json!([]);
        restore_map(&mut data, "key").unwrap();
        // Empty array → all_valid = true (vacuously), empty map
        assert_eq!(data, json!({}));
    }

    #[test]
    fn restore_map_non_array_is_no_op() {
        let mut data = json!({"not": "an array"});
        let original = data.clone();
        restore_map(&mut data, "key").unwrap();
        assert_eq!(data, original);
    }

    #[test]
    fn restore_map_malformed_entry_skips_transform() {
        // Missing "value" field — should skip the entire transform
        let mut data = json!([{"key": "a"}]);
        let original = data.clone();
        restore_map(&mut data, "key").unwrap();
        assert_eq!(data, original);
    }

    // -----------------------------------------------------------------------
    // parse_json_string
    // -----------------------------------------------------------------------

    #[test]
    fn parse_json_string_non_string_is_no_op() {
        let mut data = json!(42);
        let mut warnings = Vec::new();
        parse_json_string(&mut data, "#/test", &mut warnings).unwrap();
        assert_eq!(data, json!(42));
        assert!(warnings.is_empty());
    }

    #[test]
    fn parse_json_string_null_is_no_op() {
        let mut data = json!(null);
        let mut warnings = Vec::new();
        parse_json_string(&mut data, "#/test", &mut warnings).unwrap();
        assert_eq!(data, json!(null));
        assert!(warnings.is_empty());
    }

    #[test]
    fn parse_json_string_valid_json_object() {
        let mut data = json!("{\"key\": true}");
        let mut warnings = Vec::new();
        parse_json_string(&mut data, "#/test", &mut warnings).unwrap();
        assert_eq!(data, json!({"key": true}));
        assert!(warnings.is_empty());
    }

    #[test]
    fn parse_json_string_valid_null_string() {
        let mut data = json!("null");
        let mut warnings = Vec::new();
        parse_json_string(&mut data, "#/test", &mut warnings).unwrap();
        assert_eq!(data, json!(null));
        assert!(warnings.is_empty());
    }

    #[test]
    fn parse_json_string_empty_string_yields_null() {
        let mut data = json!("");
        let mut warnings = Vec::new();
        parse_json_string(&mut data, "#/test", &mut warnings).unwrap();
        assert_eq!(data, json!(null));
        assert!(warnings.is_empty());
    }

    #[test]
    fn parse_json_string_whitespace_only_yields_null() {
        let mut data = json!("   ");
        let mut warnings = Vec::new();
        parse_json_string(&mut data, "#/test", &mut warnings).unwrap();
        assert_eq!(data, json!(null));
        assert!(warnings.is_empty());
    }

    #[test]
    fn parse_json_string_non_json_yields_null_with_warning() {
        let mut data = json!("my-endpoint");
        let mut warnings = Vec::new();
        parse_json_string(&mut data, "#/properties/dlq", &mut warnings).unwrap();
        assert_eq!(data, json!(null));
        assert_eq!(warnings.len(), 1);
        assert!(matches!(
            warnings[0].kind,
            WarningKind::InvalidTransformInput { .. }
        ));
        assert!(warnings[0].message.contains("not valid JSON"));
    }

    #[test]
    fn parse_json_string_slash_yields_null_with_warning() {
        let mut data = json!("/");
        let mut warnings = Vec::new();
        parse_json_string(&mut data, "#/properties/dlq", &mut warnings).unwrap();
        assert_eq!(data, json!(null));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("not valid JSON"));
    }

    // -----------------------------------------------------------------------
    // restore_additional_properties
    // -----------------------------------------------------------------------

    #[test]
    fn restore_additional_properties_missing_key_is_no_op() {
        let mut data = json!({"name": "Alice"});
        let original = data.clone();
        restore_additional_properties(&mut data, "extra").unwrap();
        assert_eq!(data, original);
    }

    #[test]
    fn restore_additional_properties_non_object_value_is_no_op() {
        let mut data = json!({"name": "Alice", "extra": "not_an_object"});
        let original = data.clone();
        restore_additional_properties(&mut data, "extra").unwrap();
        assert_eq!(data, original);
    }

    // -----------------------------------------------------------------------
    // execute_transform: EnumStringify round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn enum_stringify_reverses_integer() {
        let mut data = json!("42");
        let mut warnings = Vec::new();
        let transform = Transform::EnumStringify {
            path: String::new(),
            original_values: vec![json!(42), json!(99)],
        };
        execute_transform(&mut data, &transform, &mut warnings).unwrap();
        assert_eq!(data, json!(42));
    }

    #[test]
    fn enum_stringify_reverses_boolean() {
        let mut data = json!("true");
        let mut warnings = Vec::new();
        let transform = Transform::EnumStringify {
            path: String::new(),
            original_values: vec![json!(true), json!(false)],
        };
        execute_transform(&mut data, &transform, &mut warnings).unwrap();
        assert_eq!(data, json!(true));
    }

    #[test]
    fn enum_stringify_no_match_preserves_string() {
        let mut data = json!("unknown");
        let mut warnings = Vec::new();
        let transform = Transform::EnumStringify {
            path: String::new(),
            original_values: vec![json!(1), json!(2)],
        };
        execute_transform(&mut data, &transform, &mut warnings).unwrap();
        assert_eq!(data, json!("unknown"));
    }
}
