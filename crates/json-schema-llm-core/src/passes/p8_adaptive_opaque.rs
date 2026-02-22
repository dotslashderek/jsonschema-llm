//! Pass 8 — Adaptive opaque stringification.
//!
//! Detects sub-schemas that are technically valid but unreliable in structured output modes
//! (e.g. strict closed tuples, mixed enums with objects) and proactively replaces them
//! with an opaque string wrapper + description.
//!
//! This pass runs *before* Pass 9 (Provider Compat) so that we can choose "string"
//! as the strategy before p9 attempts to enforce structural compatibility.

use crate::codec::Transform;
use crate::config::{ConvertOptions, Mode, Target};
use crate::error::ConvertError;
use crate::schema_utils::{build_opaque_description, recurse_into_children};
use serde_json::{json, Value};

use super::pass_result::PassResult;

/// Run adaptive opaque stringification.
pub fn adaptive_opaque(schema: Value, config: &ConvertOptions) -> Result<PassResult, ConvertError> {
    // Only active for OpenAI Strict mode
    if !(config.target == Target::OpenaiStrict && config.mode == Mode::Strict) {
        return Ok(PassResult::schema_only(schema));
    }

    let mut transforms = Vec::new();
    let result = walk(schema, "#", 0, config, &mut transforms)?;

    Ok(PassResult::with_transforms(result, transforms))
}

/// Recursively detect and transform unreliable constructs.
fn walk(
    node: Value,
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

    let mut result = match node {
        Value::Object(obj) => obj,
        other => return Ok(other),
    };

    // Check detection heuristics on the current object
    if has_unreliable_prefix_items(&result) || has_unreliable_enum(&result) {
        // Replace with opaque string
        let description = build_opaque_description(&Value::Object(result));
        transforms.push(Transform::JsonStringParse {
            path: path.to_string(),
        });
        return Ok(json!({
            "type": "string",
            "description": description
        }));
    }

    // Recurse into children if not transformed
    recurse_into_children(&mut result, path, depth, &mut |val, child_path, d| {
        walk(val, child_path, d, config, transforms)
    })?;

    Ok(Value::Object(result))
}

/// Detects unreliable `prefixItems` variants or `contains`.
///
/// OpenAI strict mode struggles with:
/// 1. `prefixItems` + `items: false`: closed tuples can't enforce "no additional items"
/// 2. `prefixItems` + `items: {type: X}` where prefix types ≠ X: positional type
///    constraints can't be expressed — the union-fix (`anyOf`) lets any type anywhere
/// 3. `contains`: requires at least one item match, a cross-array constraint
fn has_unreliable_prefix_items(obj: &serde_json::Map<String, Value>) -> bool {
    if let Some(prefix_items) = obj.get("prefixItems").and_then(|v| v.as_array()) {
        match obj.get("items") {
            // Case 1: Closed tuple — items: false
            Some(items) if items.as_bool() == Some(false) => return true,

            // Case 2: Open tuple — check if prefix types differ from items type
            Some(items) if items.is_object() => {
                if let Some(items_type) = items.get("type").and_then(|v| v.as_str()) {
                    let has_mismatch = prefix_items.iter().any(|prefix| {
                        prefix.get("type").and_then(|v| v.as_str()) != Some(items_type)
                    });
                    if has_mismatch {
                        return true;
                    }
                }
            }

            _ => {}
        }
    }

    // Contains constraint
    if obj.contains_key("contains") {
        return true;
    }

    false
}

/// Detects `enum` arrays containing object values or null.
///
/// While p9's `EnumStringify` can handle mixed types by stringifying them,
/// if the enum contains an *object* (e.g. `{"x": 1}`), the stringified version
/// `"{"x":1}"` is structurally complex and often rejected by strict mode
/// complexity checks or simply hallucinates.
fn has_unreliable_enum(obj: &serde_json::Map<String, Value>) -> bool {
    if let Some(arr) = obj.get("enum").and_then(|v| v.as_array()) {
        for v in arr {
            if v.is_object() || v.is_null() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn opts() -> ConvertOptions {
        ConvertOptions {
            target: Target::OpenaiStrict,
            mode: Mode::Strict,
            ..ConvertOptions::default()
        }
    }

    #[test]
    fn test_closed_tuple_stringified() {
        let schema = json!({
            "type": "array",
            "prefixItems": [{"type": "string"}, {"type": "integer"}],
            "items": false
        });
        let result = adaptive_opaque(schema, &opts()).unwrap();

        assert_eq!(result.schema["type"], "string");
        assert!(result.schema["description"]
            .as_str()
            .unwrap()
            .contains("string"));
        assert!(result.schema["description"]
            .as_str()
            .unwrap()
            .contains("integer"));
        assert!(result.schema["description"]
            .as_str()
            .unwrap()
            .contains("(fixed length)"));
        assert_eq!(result.transforms.len(), 1);
        match &result.transforms[0] {
            Transform::JsonStringParse { path } => assert_eq!(path, "#"),
            _ => panic!("Expected JsonStringParse"),
        }
    }

    #[test]
    fn test_mixed_type_open_tuple_stringified() {
        // prefixItems: [string] + items: integer → types differ, stringify
        let schema = json!({
            "type": "array",
            "prefixItems": [{"type": "string"}],
            "items": {"type": "integer"}
        });
        let result = adaptive_opaque(schema, &opts()).unwrap();

        assert_eq!(result.schema["type"], "string");
        assert!(!result.transforms.is_empty());
        match &result.transforms[0] {
            Transform::JsonStringParse { path } => assert_eq!(path, "#"),
            _ => panic!("Expected JsonStringParse"),
        }
    }

    #[test]
    fn test_homogeneous_open_tuple_unchanged() {
        // prefixItems: [string] + items: string → types match, keep as real array
        let schema = json!({
            "type": "array",
            "prefixItems": [{"type": "string"}],
            "items": {"type": "string"}
        });
        let result = adaptive_opaque(schema, &opts()).unwrap();

        assert_eq!(result.schema["type"], "array");
        assert!(result.transforms.is_empty());
    }

    #[test]
    fn test_contains_array_stringified() {
        let schema = json!({
            "type": "array",
            "items": {"type": "string"},
            "contains": {"const": "special"}
        });
        let result = adaptive_opaque(schema, &opts()).unwrap();

        assert_eq!(result.schema["type"], "string");
        assert!(!result.transforms.is_empty());
    }

    #[test]
    fn test_object_enum_stringified() {
        let schema = json!({
            "enum": ["a", {"x": 1}]
        });
        let result = adaptive_opaque(schema, &opts()).unwrap();

        assert_eq!(result.schema["type"], "string");
        let desc = result.schema["description"].as_str().unwrap();
        assert!(desc.contains("One of"));
        // Check that the object value is serialized in the description
        assert!(desc.contains("{\"x\":1}"));
        assert!(!result.transforms.is_empty());
    }

    #[test]
    fn test_simple_enum_unchanged() {
        let schema = json!({
            "type": "string",
            "enum": ["a", "b"]
        });
        let result = adaptive_opaque(schema, &opts()).unwrap();

        assert_eq!(result.schema["type"], "string");
        // Only simple string enums should be preserved
        assert!(result.transforms.is_empty());
    }

    #[test]
    fn test_nested_detection() {
        let schema = json!({
            "type": "object",
            "properties": {
                "safe": {"type": "string"},
                "unsafe": {
                    "enum": [null]
                }
            }
        });
        let result = adaptive_opaque(schema, &opts()).unwrap();

        assert_eq!(result.schema["type"], "object");
        // Safe property unchanged
        assert_eq!(result.schema["properties"]["safe"]["type"], "string");
        assert!(result.schema["properties"]["safe"]["enum"].is_null());

        // Unsafe property stringified
        assert_eq!(result.schema["properties"]["unsafe"]["type"], "string");
        assert!(result.schema["properties"]["unsafe"]["description"]
            .as_str()
            .unwrap()
            .contains("null"));
    }

    #[test]
    fn test_non_openai_skipped() {
        let schema = json!({
            "enum": [null]
        });
        let mut options = opts();
        options.target = Target::Gemini;

        let result = adaptive_opaque(schema, &options).unwrap();
        // Should be untouched
        assert!(result.schema["type"].is_null()); // no type added
        assert!(result.transforms.is_empty());
    }

    #[test]
    fn test_description_captures_structure() {
        let schema = json!({
            "type": "object",
            "properties": {
                "foo": {"type": "integer"}
            },
            "enum": [{"foo": 1}, {"foo": 2}]
        });
        let result = adaptive_opaque(schema, &opts()).unwrap();

        let desc = result.schema["description"].as_str().unwrap();
        // Enum description takes precedence over structure
        assert!(desc.contains("One of:"));
        assert!(desc.contains("{\"foo\":1}"));
    }

    #[test]
    fn test_enum_description_fidelity() {
        let schema = json!({
            "enum": ["A", "B"]
        });
        let _result = adaptive_opaque(schema, &opts()).unwrap();
        // Although this is simple string enum (and normally wouldn't trigger *if* it had a type),
        // here it lacks type, but wait... `has_unreliable_enum` checks for object or null.
        // "A", "B" are strings. So `has_unreliable_enum` returns false.
        // Let's force it by adding a null.
        let schema = json!({
            "enum": ["A", "B", null]
        });
        let result = adaptive_opaque(schema, &opts()).unwrap();
        let desc = result.schema["description"].as_str().unwrap();
        assert!(desc.contains("One of: [\"A\", \"B\", null]"));
    }
}
