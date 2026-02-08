//! Pass 4: Opaque Type Stringification
//!
//! Detects "opaque" object schemas — `{type: object}` with no properties,
//! no pattern properties, no composition keywords, and no enum/const — and
//! converts them to `{type: string}` with a description instructing the LLM
//! to produce a JSON-encoded string.
//!
//! This is the "escape hatch" for open-ended configuration objects that LLM
//! providers can't generate structured output for.
//!
//! The rehydrator already handles the inverse via `Transform::JsonStringParse`.

use serde_json::{Map, Value};

use crate::codec::Transform;
use crate::config::ConvertOptions;
use crate::error::ConvertError;
use crate::schema_utils::recurse_into_children;

/// Default description for opaque objects that have no existing description.
const DEFAULT_OPAQUE_DESC: &str =
    "A JSON-encoded string representing the object. Parse with JSON.parse() after generation.";

/// Suffix appended to existing descriptions for opaque objects.
const OPAQUE_DESC_SUFFIX: &str =
    "\n\n(Note: This field represents an opaque object. The value should be a JSON-encoded string. Parse with JSON.parse() after generation.)";

/// Result of running the opaque type stringification pass.
#[derive(Debug)]
pub struct OpaquePassResult {
    /// The transformed schema with opaque objects converted to strings.
    pub schema: Value,
    /// Codec transforms produced by this pass.
    pub transforms: Vec<Transform>,
}

/// Apply opaque type stringification to a schema.
///
/// Recursively walks every node. For opaque objects (type: object with no
/// structural constraints), converts them to `{type: string}` with a
/// descriptive instruction and emits a `JsonStringParse` codec transform.
pub fn stringify_opaque(
    schema: &Value,
    config: &ConvertOptions,
) -> Result<OpaquePassResult, ConvertError> {
    let mut transforms = Vec::new();
    let result = walk(schema, "#", 0, config, &mut transforms)?;
    Ok(OpaquePassResult {
        schema: result,
        transforms,
    })
}

// ---------------------------------------------------------------------------
// Recursive walker
// ---------------------------------------------------------------------------

/// Recursively descend through the schema tree, stringifying opaque objects
/// and collecting codec transforms.
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

    // Check for opaque pattern BEFORE recursing into children.
    if is_opaque(&result) {
        let stringified = stringify_object(&result, path, transforms);
        return Ok(stringified);
    }

    // Recurse into all structural children via shared traversal.
    recurse_into_children(&mut result, path, depth, &mut |val, child_path, d| {
        walk(val, child_path, d, config, transforms)
    })?;

    Ok(Value::Object(result))
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Check if a schema object is "opaque" — an open-ended object with no
/// structural definition that an LLM cannot generate.
///
/// An object is opaque if ALL of:
/// - `type: "object"` (explicit)
/// - No `properties` (or empty)
/// - No `patternProperties`
/// - No `additionalProperties` that is a typed schema (maps are Pass 3)
/// - No `additionalProperties: false` (sealed empty is well-defined)
/// - No `allOf`, `oneOf`, `anyOf`
/// - No `enum` or `const`
fn is_opaque(obj: &Map<String, Value>) -> bool {
    // Must be explicitly typed as object.
    if obj.get("type").and_then(Value::as_str) != Some("object") {
        return false;
    }

    // Has non-empty properties → structured, not opaque.
    if obj
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|p| !p.is_empty())
    {
        return false;
    }

    // Has non-empty patternProperties → has structure.
    if obj
        .get("patternProperties")
        .and_then(Value::as_object)
        .is_some_and(|p| !p.is_empty())
    {
        return false;
    }

    // Has composition or conditional keywords → composite, not opaque.
    if obj.contains_key("allOf")
        || obj.contains_key("oneOf")
        || obj.contains_key("anyOf")
        || obj.contains_key("not")
        || obj.contains_key("if")
        || obj.contains_key("then")
        || obj.contains_key("else")
    {
        return false;
    }

    // Has enum or const → constrained values, not opaque.
    if obj.contains_key("enum") || obj.contains_key("const") {
        return false;
    }

    // Has reference keywords → not opaque (defers definition).
    if obj.contains_key("$ref")
        || obj.contains_key("$dynamicRef")
        || obj.contains_key("$recursiveRef")
    {
        return false;
    }

    // Check additionalProperties:
    // - Missing → opaque (implicit any)
    // - true → opaque (explicit any)
    // - false → NOT opaque (sealed empty object, well-defined)
    // - Schema object → NOT opaque (map pattern for Pass 3)
    match obj.get("additionalProperties") {
        None => true,                                     // implicit any
        Some(Value::Bool(true)) => true,                  // explicit any
        Some(Value::Bool(false)) => false,                // sealed empty
        Some(Value::Object(ap)) if ap.is_empty() => true, // empty schema = any
        Some(Value::Object(_)) => false,                  // typed schema = map
        _ => false,                                       // unexpected, skip
    }
}

// ---------------------------------------------------------------------------
// Transformation
// ---------------------------------------------------------------------------

/// Convert an opaque object to a string schema.
fn stringify_object(
    obj: &Map<String, Value>,
    path: &str,
    transforms: &mut Vec<Transform>,
) -> Value {
    // 2. Clone the object and modify into a string schema.
    // Instead of rebuilding from a whitelist (which drops unknown keywords),
    // we clone everything and strip object-specific validation keywords.
    let mut result = obj.clone();

    // Set type to string.
    result.insert("type".to_string(), Value::String("string".to_string()));

    // Strip object validation keywords.
    for key in [
        "properties",
        "patternProperties",
        "additionalProperties",
        "required",
        "minProperties",
        "maxProperties",
        "dependentRequired",
        "dependentSchemas",
        "propertyNames",
        "unevaluatedProperties",
        // Enum/const were checked in is_opaque, but strip defensively
        "enum",
        "const",
        // Conditional applicators — not expected on opaque schemas, but strip for coherence
        "not",
        "if",
        "then",
        "else",
    ] {
        result.remove(key);
    }

    // If a description exists, append the opaque suffix; otherwise, set the default description.
    if let Some(desc) = result.get("description").and_then(Value::as_str) {
        let new_desc = format!("{}{}", desc, OPAQUE_DESC_SUFFIX);
        result.insert("description".to_string(), Value::String(new_desc));
    } else {
        result.insert(
            "description".to_string(),
            Value::String(DEFAULT_OPAQUE_DESC.to_string()),
        );
    }

    // Stringify object/array values in `default` to match the new string type.
    if let Some(val) = result.get("default").cloned() {
        if val.is_object() || val.is_array() {
            if let Ok(s) = serde_json::to_string(&val) {
                result.insert("default".to_string(), Value::String(s));
            }
        }
    }

    // Stringify object/array entries in `examples` to match the new string type.
    if let Some(examples) = result.get("examples").and_then(Value::as_array).cloned() {
        let stringified: Vec<Value> = examples
            .into_iter()
            .map(|v| {
                if v.is_object() || v.is_array() {
                    serde_json::to_string(&v).map(Value::String).unwrap_or(v)
                } else {
                    v
                }
            })
            .collect();
        result.insert("examples".to_string(), Value::Array(stringified));
    }

    // Emit codec transform.
    transforms.push(Transform::JsonStringParse {
        path: path.to_string(),
    });

    Value::Object(result)
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
        let config = ConvertOptions::default();
        let result = stringify_opaque(&schema, &config).unwrap();
        (result.schema, result.transforms)
    }

    // -----------------------------------------------------------------------
    // Test 1: Simple opaque object → string
    // -----------------------------------------------------------------------
    #[test]
    fn test_simple_opaque() {
        let input = json!({ "type": "object" });

        let (output, transforms) = run(input);

        assert_eq!(output["type"], "string");
        assert_eq!(output["description"], DEFAULT_OPAQUE_DESC);
        assert_eq!(transforms.len(), 1);
        match &transforms[0] {
            Transform::JsonStringParse { path } => assert_eq!(path, "#"),
            other => panic!("expected JsonStringParse, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Test 2: Opaque with existing description → appended
    // -----------------------------------------------------------------------
    #[test]
    fn test_opaque_with_description() {
        let input = json!({
            "type": "object",
            "description": "Plugin configuration"
        });

        let (output, transforms) = run(input);

        assert_eq!(output["type"], "string");
        let desc = output["description"].as_str().unwrap();
        assert!(desc.starts_with("Plugin configuration"));
        assert!(desc.contains("JSON-encoded string"));
        assert_eq!(transforms.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Test 3: Non-opaque — has properties → unchanged
    // -----------------------------------------------------------------------
    #[test]
    fn test_not_opaque_has_properties() {
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            }
        });

        let (output, transforms) = run(input.clone());

        assert_eq!(output, input);
        assert_eq!(transforms.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 4: Map (additionalProperties: Schema) → unchanged (Pass 3 handles)
    // -----------------------------------------------------------------------
    #[test]
    fn test_not_opaque_map() {
        let input = json!({
            "type": "object",
            "additionalProperties": { "type": "string" }
        });

        let (output, transforms) = run(input.clone());

        assert_eq!(output, input);
        assert_eq!(transforms.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 5: Sealed empty (additionalProperties: false) → unchanged
    // -----------------------------------------------------------------------
    #[test]
    fn test_not_opaque_sealed_empty() {
        let input = json!({
            "type": "object",
            "additionalProperties": false
        });

        let (output, transforms) = run(input.clone());

        assert_eq!(output, input);
        assert_eq!(transforms.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 6: Composite (allOf present) → unchanged
    // -----------------------------------------------------------------------
    #[test]
    fn test_not_opaque_composite() {
        let input = json!({
            "type": "object",
            "allOf": [
                { "properties": { "a": { "type": "string" } } }
            ]
        });

        let (output, transforms) = run(input.clone());

        assert_eq!(output, input);
        assert_eq!(transforms.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 7: Object with enum → unchanged
    // -----------------------------------------------------------------------
    #[test]
    fn test_not_opaque_has_enum() {
        let input = json!({
            "type": "object",
            "enum": [{"a": 1}, {"b": 2}]
        });

        let (output, transforms) = run(input.clone());

        assert_eq!(output, input);
        assert_eq!(transforms.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 8: Nested opaque inside properties → inner converted
    // -----------------------------------------------------------------------
    #[test]
    fn test_nested_opaque() {
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "config": { "type": "object" }
            }
        });

        let (output, transforms) = run(input);

        // Outer object is NOT opaque (has properties).
        assert_eq!(output["type"], "object");
        assert_eq!(output["properties"]["name"], json!({ "type": "string" }));

        // Inner "config" IS opaque → converted to string.
        assert_eq!(output["properties"]["config"]["type"], "string");
        assert_eq!(transforms.len(), 1);
        match &transforms[0] {
            Transform::JsonStringParse { path } => {
                assert_eq!(path, "#/properties/config");
            }
            other => panic!("expected JsonStringParse, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Test 9: Opaque with minProperties → string, keyword stripped
    // -----------------------------------------------------------------------
    #[test]
    fn test_opaque_strips_object_keywords() {
        let input = json!({
            "type": "object",
            "minProperties": 1,
            "maxProperties": 10
        });

        let (output, transforms) = run(input);

        assert_eq!(output["type"], "string");
        assert!(output.get("minProperties").is_none());
        assert!(output.get("maxProperties").is_none());
        assert_eq!(transforms.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Test 10: Metadata preservation (title, nullable)
    // -----------------------------------------------------------------------
    #[test]
    fn test_metadata_preserved() {
        let input = json!({
            "type": "object",
            "title": "Dynamic Config",
            "nullable": true,
            "description": "User-defined settings"
        });

        let (output, transforms) = run(input);

        assert_eq!(output["type"], "string");
        assert_eq!(output["title"], "Dynamic Config");
        assert_eq!(output["nullable"], true);
        let desc = output["description"].as_str().unwrap();
        assert!(desc.starts_with("User-defined settings"));
        assert_eq!(transforms.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Test 11: additionalProperties: true → opaque
    // -----------------------------------------------------------------------
    #[test]
    fn test_opaque_additional_properties_true() {
        let input = json!({
            "type": "object",
            "additionalProperties": true
        });

        let (output, transforms) = run(input);

        assert_eq!(output["type"], "string");
        assert!(output.get("additionalProperties").is_none());
        assert_eq!(transforms.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Test 12: Depth guard
    // -----------------------------------------------------------------------
    #[test]
    fn test_depth_guard() {
        // Create a deeply nested structure that will exceed max_depth.
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

        let result = stringify_opaque(&input, &config);
        let err = result.unwrap_err();
        match err {
            ConvertError::RecursionDepthExceeded { max_depth, .. } => {
                assert_eq!(max_depth, 2);
            }
            other => panic!("expected RecursionDepthExceeded, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Test 13: Idempotent — running output through again produces no changes
    // -----------------------------------------------------------------------
    #[test]
    fn test_idempotent() {
        let input = json!({ "type": "object" });

        let (first_output, first_transforms) = run(input);
        let (second_output, second_transforms) = run(first_output.clone());

        // Already a string, so no further changes.
        assert_eq!(second_output, first_output);
        assert_eq!(second_transforms.len(), 0);
        assert!(!first_transforms.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test 14: Object with $ref → unchanged (not opaque)
    // -----------------------------------------------------------------------
    #[test]
    fn test_not_opaque_ref() {
        let input = json!({
            "type": "object",
            "$ref": "#/definitions/SomeType"
        });

        // Should NOT be stringified because $ref means it's not opaque
        let (output, transforms) = run(input.clone());

        assert_eq!(output, input);
        assert_eq!(transforms.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 15: Default value handling
    // -----------------------------------------------------------------------
    #[test]
    fn test_default_handling_objects() {
        let input = json!({
            "type": "object",
            "default": { "a": 1 }, // Object default -> stringified
            "description": "desc"
        });

        let (output, _) = run(input);

        // Object default should be stringified
        assert_eq!(output["default"], json!("{\"a\":1}"));
    }

    #[test]
    fn test_default_handling_primitives() {
        let input = json!({
            "type": "object",
            "default": "some-string", // String default -> preserved as-is
        });

        let (output, _) = run(input);

        // String default should be preserved, NOT double-encoded
        assert_eq!(output["default"], json!("some-string"));
    }

    // -----------------------------------------------------------------------
    // Test 16: Opaque with empty patternProperties -> opaque
    // -----------------------------------------------------------------------
    #[test]
    fn test_opaque_pattern_properties_empty() {
        let input = json!({
            "type": "object",
            "patternProperties": {}
        });

        // Should be stringified because patternProperties is empty
        let (output, _) = run(input);

        // If opaque, type becomes string
        assert_eq!(output["type"], "string");
    }

    // -----------------------------------------------------------------------
    // Test 17: Extended metadata preservation (deprecated, readOnly, etc.)
    // -----------------------------------------------------------------------
    #[test]
    fn test_metadata_preserved_extended() {
        let input = json!({
            "type": "object",
            "deprecated": true,
            "readOnly": true,
            "minProperties": 1, // Should be stripped
            "extraField": "value" // Should be preserved
        });

        let (output, _) = run(input);

        assert_eq!(output["type"], "string");
        assert_eq!(output["deprecated"], true);
        assert_eq!(output["readOnly"], true);
        assert_eq!(output["extraField"], "value");
        assert!(output.get("minProperties").is_none());
    }

    // -----------------------------------------------------------------------
    // Test 18: Conditional keywords (if/not) → not opaque
    // -----------------------------------------------------------------------
    #[test]
    fn test_not_opaque_conditional() {
        let input = json!({
            "type": "object",
            "if": { "properties": { "x": { "const": 1 } } },
            "then": { "required": ["y"] }
        });

        let (output, transforms) = run(input.clone());
        assert_eq!(output, input);
        assert_eq!(transforms.len(), 0);
    }

    #[test]
    fn test_not_opaque_not() {
        let input = json!({
            "type": "object",
            "not": { "required": ["forbidden"] }
        });

        let (output, transforms) = run(input.clone());
        assert_eq!(output, input);
        assert_eq!(transforms.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 19: Examples with object/array entries → stringified
    // -----------------------------------------------------------------------
    #[test]
    fn test_examples_stringified() {
        let input = json!({
            "type": "object",
            "examples": [{"a": 1}, "plain-string", [1, 2, 3]]
        });

        let (output, _) = run(input);

        assert_eq!(output["type"], "string");
        let examples = output["examples"].as_array().unwrap();
        // Object example → stringified
        assert_eq!(examples[0], json!("{\"a\":1}"));
        // String example → preserved
        assert_eq!(examples[1], json!("plain-string"));
        // Array example → stringified
        assert_eq!(examples[2], json!("[1,2,3]"));
    }
}
