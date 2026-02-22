//! Type coercion — fixes LLM-generated type mismatches using the original schema.
//!
//! LLMs sometimes output values with incorrect JSON types (e.g. a number where
//! a string was expected). This module walks the original schema and data in
//! parallel, applying safe, lossless coercions.

use serde_json::Value;

use crate::codec_warning::{Warning, WarningKind};
use crate::schema_utils::escape_pointer_segment;

/// Coerce data types to match the original schema expectations.
///
/// LLMs sometimes return the wrong JSON type for a field — e.g. a bare `123`
/// where `type: "string"` was expected, or `"42"` where `type: "integer"` was
/// expected. This function walks the original schema and data in parallel,
/// applying safe coercions:
///
/// | Got | Expected | Action |
/// |-----|----------|--------|
/// | number/integer | `"string"` | `value.to_string()` |
/// | boolean | `"string"` | `"true"` / `"false"` |
/// | `"string"` | `"number"` | parse as f64 (validate roundtrip) |
/// | `"string"` | `"integer"` | parse as i64 (validate roundtrip) |
///
/// Returns warnings for each coercion applied.
pub fn coerce_types(data: &mut Value, original_schema: &Value) -> Vec<Warning> {
    let mut warnings = Vec::new();
    coerce_walk(data, original_schema, "", &mut warnings);
    warnings
}

/// Recursive walker for type coercion.
fn coerce_walk(data: &mut Value, schema: &Value, path: &str, warnings: &mut Vec<Warning>) {
    let schema_obj = match schema.as_object() {
        Some(o) => o,
        None => return,
    };

    // Determine expected type(s). Handle both `type: "string"` and `type: ["string", "null"]`.
    let expected_types: Vec<&str> = match schema_obj.get("type") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(arr)) => arr.iter().filter_map(|v| v.as_str()).collect(),
        _ => vec![],
    };

    // If data is null and null is an allowed type, skip coercion
    if data.is_null() && expected_types.contains(&"null") {
        return;
    }

    // --- Attempt type coercion at this node ---
    if !expected_types.is_empty() {
        if let Some(msg) = try_coerce(data, &expected_types) {
            warnings.push(Warning {
                data_path: if path.is_empty() {
                    "/".to_string()
                } else {
                    path.to_string()
                },
                schema_path: path.to_string(),
                kind: WarningKind::ConstraintViolation {
                    constraint: "type".to_string(),
                },
                message: msg,
            });
        }
    }

    // --- Recurse into children ---

    // Object properties
    if expected_types.contains(&"object") || schema_obj.contains_key("properties") {
        if let (Some(data_obj), Some(props)) = (
            data.as_object_mut(),
            schema_obj.get("properties").and_then(|v| v.as_object()),
        ) {
            let keys: Vec<String> = data_obj.keys().cloned().collect();
            for key in keys {
                if let Some(prop_schema) = props.get(&key) {
                    let child_path = format!("{}/{}", path, escape_pointer_segment(&key));
                    if let Some(child_data) = data_obj.get_mut(&key) {
                        coerce_walk(child_data, prop_schema, &child_path, warnings);
                    }
                }
            }
        }
    }

    // Array items — handle both prefixItems (tuples) and uniform items
    if expected_types.contains(&"array")
        || schema_obj.contains_key("items")
        || schema_obj.contains_key("prefixItems")
    {
        if let Some(data_arr) = data.as_array_mut() {
            let prefix_items = schema_obj.get("prefixItems").and_then(|v| v.as_array());
            let items_schema = schema_obj.get("items");

            for (i, item) in data_arr.iter_mut().enumerate() {
                let child_path = format!("{}/{}", path, i);
                // Use positional schema from prefixItems if available, else fallback to items
                if let Some(positional) = prefix_items.and_then(|pi| pi.get(i)) {
                    coerce_walk(item, positional, &child_path, warnings);
                } else if let Some(uniform) = items_schema {
                    coerce_walk(item, uniform, &child_path, warnings);
                }
            }
        }
    }

    // anyOf / oneOf — try each variant, use the first that matches the data's type
    for keyword in &["anyOf", "oneOf"] {
        if let Some(variants) = schema_obj.get(*keyword).and_then(|v| v.as_array()) {
            for variant in variants {
                let variant_types: Vec<&str> = match variant.get("type") {
                    Some(Value::String(s)) => vec![s.as_str()],
                    Some(Value::Array(arr)) => arr.iter().filter_map(|v| v.as_str()).collect(),
                    _ => continue,
                };
                let data_type = json_type_name(data);
                // "integer" data satisfies "number" schema (JSON spec compatibility)
                let matches = variant_types.contains(&data_type)
                    || (data_type == "integer" && variant_types.contains(&"number"));
                if matches {
                    coerce_walk(data, variant, path, warnings);
                    break;
                }
            }
        }
    }
}

/// Attempt to coerce a value to match one of the expected types.
/// Returns `Some(message)` if coercion was applied, `None` if no action needed.
fn try_coerce(value: &mut Value, expected_types: &[&str]) -> Option<String> {
    let actual_type = json_type_name(value);

    // If the actual type already matches, no coercion needed.
    // Special case: "integer" values also satisfy "number" expectations.
    if expected_types.contains(&actual_type)
        || (actual_type == "integer" && expected_types.contains(&"number"))
    {
        return None;
    }

    for expected in expected_types {
        match *expected {
            "string" => match value {
                Value::Number(n) => {
                    let s = n.to_string();
                    let msg = format!("coerced number {} to string \"{}\"", s, s);
                    *value = Value::String(s);
                    return Some(msg);
                }
                Value::Bool(b) => {
                    let s = b.to_string();
                    let msg = format!("coerced boolean {} to string \"{}\"", b, s);
                    *value = Value::String(s);
                    return Some(msg);
                }
                _ => {}
            },
            "integer" => {
                if let Some(s) = value.as_str() {
                    if let Ok(n) = s.parse::<i64>() {
                        // Roundtrip check: parsed.to_string() == original
                        if n.to_string() == s {
                            let msg = format!("coerced string \"{}\" to integer {}", s, n);
                            *value = Value::Number(serde_json::Number::from(n));
                            return Some(msg);
                        }
                    }
                }
            }
            "number" => {
                if let Some(s) = value.as_str() {
                    if let Ok(n) = s.parse::<f64>() {
                        // Roundtrip check: ensure formatting preserves the value
                        let formatted = n.to_string();
                        if let Ok(roundtripped) = formatted.parse::<f64>() {
                            if (roundtripped - n).abs() < f64::EPSILON {
                                if let Some(num) = serde_json::Number::from_f64(n) {
                                    let msg = format!("coerced string \"{}\" to number {}", s, n);
                                    *value = Value::Number(num);
                                    return Some(msg);
                                }
                            }
                        }
                    }
                }
            }
            "boolean" => {
                if let Some(s) = value.as_str() {
                    match s {
                        "true" => {
                            *value = Value::Bool(true);
                            return Some("coerced string \"true\" to boolean true".to_string());
                        }
                        "false" => {
                            *value = Value::Bool(false);
                            return Some("coerced string \"false\" to boolean false".to_string());
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    None
}

/// Return the JSON type name for a value.
fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "integer"
            } else {
                "number"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
