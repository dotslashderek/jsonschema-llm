//! Pass 3: Dictionary Transpilation (Map → Array)
//!
//! Detects JSON Schema "map" patterns (`{type: object, additionalProperties: Schema}`)
//! and converts them to `{type: array, items: {type: object, properties: {key, value}}}`.
//!
//! For mixed objects (both `properties` and `additionalProperties: Schema`), the dynamic
//! entries are extracted into a synthetic `_additional` property, keeping the object
//! Strict-compatible while preserving all data.
//!
//! Nested maps are handled via natural recursion — each level transpiles independently.

use serde_json::{json, Map, Value};

use crate::codec::Transform;
use crate::config::{ConvertOptions, Target};
use crate::error::ConvertError;

/// Field name for the map key in the transpiled array item.
const KEY_FIELD: &str = "key";
/// Field name for the map value in the transpiled array item.
const VALUE_FIELD: &str = "value";
/// Default property name for extracted `additionalProperties` in mixed objects.
const ADDITIONAL_PROPERTY: &str = "_additional";

/// Result of running the dictionary transpilation pass.
#[derive(Debug)]
pub struct DictPassResult {
    /// The transformed schema with maps converted to arrays.
    pub schema: Value,
    /// Codec transforms produced by this pass.
    pub transforms: Vec<Transform>,
}

/// Apply dictionary transpilation to a schema.
///
/// Recursively walks every node. For map-pattern objects, converts them to typed
/// arrays. For mixed objects, extracts `additionalProperties` into a synthetic
/// `_additional` property.
///
/// Skipped entirely when `config.target == Target::Gemini`.
pub fn transpile_dictionaries(
    schema: &Value,
    config: &ConvertOptions,
) -> Result<DictPassResult, ConvertError> {
    // Provider gate: Gemini supports additionalProperties natively.
    if config.target == Target::Gemini {
        return Ok(DictPassResult {
            schema: schema.clone(),
            transforms: Vec::new(),
        });
    }

    let mut transforms = Vec::new();
    let result = walk(schema, "#", 0, config, &mut transforms)?;
    Ok(DictPassResult {
        schema: result,
        transforms,
    })
}

// ---------------------------------------------------------------------------
// Recursive walker
// ---------------------------------------------------------------------------

/// Recursively descend through the schema tree, transpiling map-pattern objects
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

    // Check for map patterns BEFORE recursing into children.
    if is_pure_map(&result) {
        // Pure map: convert entire object to array.
        let array_schema = transpile_pure_map(&result, path, transforms);
        // Recurse into the newly created items schema.
        return walk(&array_schema, path, depth + 1, config, transforms);
    }

    if is_mixed_map(&result) {
        // Mixed: extract additionalProperties into _additional, then transpile it.
        extract_additional_properties(&mut result, path, transforms);
    }

    // Recurse into all structural children.
    recurse_into_properties(&mut result, path, depth, config, transforms)?;
    recurse_into_items(&mut result, path, depth, config, transforms)?;
    recurse_into_variants(&mut result, "anyOf", path, depth, config, transforms)?;
    recurse_into_variants(&mut result, "oneOf", path, depth, config, transforms)?;
    recurse_into_variants(&mut result, "allOf", path, depth, config, transforms)?;
    recurse_into_additional_properties(&mut result, path, depth, config, transforms)?;

    Ok(Value::Object(result))
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// A "pure map" is `{type: object, additionalProperties: Schema}` with no `properties`
/// (or empty `properties`).
fn is_pure_map(obj: &Map<String, Value>) -> bool {
    is_typed_object(obj) && has_schema_additional_properties(obj) && !has_non_empty_properties(obj)
}

/// A "mixed map" has BOTH `properties` (non-empty) AND `additionalProperties: Schema`.
fn is_mixed_map(obj: &Map<String, Value>) -> bool {
    is_typed_object(obj) && has_schema_additional_properties(obj) && has_non_empty_properties(obj)
}

/// Check whether a JSON object has `"type": "object"`.
fn is_typed_object(obj: &Map<String, Value>) -> bool {
    obj.get("type").and_then(Value::as_str) == Some("object")
}

/// Check if `additionalProperties` is a schema object (not a bool, not absent).
fn has_schema_additional_properties(obj: &Map<String, Value>) -> bool {
    matches!(obj.get("additionalProperties"), Some(v) if v.is_object())
}

/// Check if the object has a non-empty `properties` map.
fn has_non_empty_properties(obj: &Map<String, Value>) -> bool {
    obj.get("properties")
        .and_then(Value::as_object)
        .is_some_and(|p| !p.is_empty())
}

// ---------------------------------------------------------------------------
// Transformation functions
// ---------------------------------------------------------------------------

/// Convert a pure map object to an array schema.
///
/// Input:  `{type: object, additionalProperties: ValueSchema, description?, title?}`
/// Output: `{type: array, items: {type: object, properties: {key: {type: string}, value: ValueSchema}, required: [key, value], additionalProperties: false}, description?, title?}`
fn transpile_pure_map(
    obj: &Map<String, Value>,
    path: &str,
    transforms: &mut Vec<Transform>,
) -> Value {
    let value_schema = obj
        .get("additionalProperties")
        .cloned()
        .unwrap_or(json!({}));

    let mut array_schema = build_array_schema(&value_schema, KEY_FIELD);

    // Preserve metadata on the outer array.
    if let Some(desc) = obj.get("description") {
        array_schema
            .as_object_mut()
            .unwrap()
            .insert("description".to_string(), desc.clone());
    }
    if let Some(title) = obj.get("title") {
        array_schema
            .as_object_mut()
            .unwrap()
            .insert("title".to_string(), title.clone());
    }

    transforms.push(Transform::MapToArray {
        path: path.to_string(),
        key_field: KEY_FIELD.to_string(),
    });

    array_schema
}

/// Handle a mixed object by extracting `additionalProperties` into a synthetic
/// `_additional` property of type array.
fn extract_additional_properties(
    obj: &mut Map<String, Value>,
    path: &str,
    transforms: &mut Vec<Transform>,
) {
    let value_schema = obj.remove("additionalProperties").unwrap_or(json!({}));

    let array_schema = build_array_schema(&value_schema, KEY_FIELD);

    // Choose a property name that doesn't collide with existing properties.
    let props = obj
        .entry("properties")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .unwrap();

    let property_name = if props.contains_key(ADDITIONAL_PROPERTY) {
        // Find a unique name by appending underscores.
        let mut candidate = format!("{}_{}", ADDITIONAL_PROPERTY, "extra");
        while props.contains_key(&candidate) {
            candidate.push('_');
        }
        candidate
    } else {
        ADDITIONAL_PROPERTY.to_string()
    };

    props.insert(property_name.clone(), array_schema);

    // Emit codec entries: first the extraction, then the map-to-array.
    transforms.push(Transform::ExtractAdditionalProperties {
        path: path.to_string(),
        property_name: property_name.clone(),
    });
    transforms.push(Transform::MapToArray {
        path: format!("{}/properties/{}", path, property_name),
        key_field: KEY_FIELD.to_string(),
    });
}

/// Build the standard array schema for a map transpilation.
///
/// Returns: `{type: array, items: {type: object, properties: {<key_field>: {type: string}, value: <value_schema>}, required: [<key_field>, value], additionalProperties: false}}`
fn build_array_schema(value_schema: &Value, key_field: &str) -> Value {
    json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                key_field: { "type": "string" },
                VALUE_FIELD: value_schema,
            },
            "required": [key_field, VALUE_FIELD],
            "additionalProperties": false,
        }
    })
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
    if let Some(Value::Object(props)) = obj.get_mut("properties") {
        let keys: Vec<String> = props.keys().cloned().collect();
        for key in keys {
            if let Some(val) = props.remove(&key) {
                let child_path = format!("{}/properties/{}", path, key);
                let walked = walk(&val, &child_path, depth + 1, config, transforms)?;
                props.insert(key, walked);
            }
        }
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
    if let Some(items) = obj.remove("items") {
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
    if let Some(Value::Array(variants)) = obj.remove(keyword) {
        let mut walked_variants = Vec::with_capacity(variants.len());
        for (i, variant) in variants.into_iter().enumerate() {
            let child_path = format!("{}/{}/{}", path, keyword, i);
            walked_variants.push(walk(&variant, &child_path, depth + 1, config, transforms)?);
        }
        obj.insert(keyword.to_string(), Value::Array(walked_variants));
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
    if let Some(ap) = obj.remove("additionalProperties") {
        if ap.is_object() {
            let child_path = format!("{}/additionalProperties", path);
            let walked = walk(&ap, &child_path, depth + 1, config, transforms)?;
            obj.insert("additionalProperties".to_string(), walked);
        } else {
            obj.insert("additionalProperties".to_string(), ap);
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
        let config = ConvertOptions::default();
        let result = transpile_dictionaries(&schema, &config).unwrap();
        (result.schema, result.transforms)
    }

    // -----------------------------------------------------------------------
    // Test 1: Simple map — {type: object, additionalProperties: {type: string}}
    // -----------------------------------------------------------------------
    #[test]
    fn test_simple_map() {
        let input = json!({
            "type": "object",
            "additionalProperties": { "type": "string" }
        });

        let (output, transforms) = run(input);

        assert_eq!(output["type"], "array");
        assert_eq!(
            output["items"]["properties"]["key"],
            json!({"type": "string"})
        );
        assert_eq!(
            output["items"]["properties"]["value"],
            json!({"type": "string"})
        );
        assert_eq!(output["items"]["required"], json!(["key", "value"]));
        assert_eq!(output["items"]["additionalProperties"], json!(false));
        assert_eq!(transforms.len(), 1);
        match &transforms[0] {
            Transform::MapToArray { path, key_field } => {
                assert_eq!(path, "#");
                assert_eq!(key_field, "key");
            }
            other => panic!("expected MapToArray, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Test 2: Map with complex value schema
    // -----------------------------------------------------------------------
    #[test]
    fn test_map_with_complex_value() {
        let input = json!({
            "type": "object",
            "additionalProperties": {
                "type": "object",
                "properties": {
                    "host": { "type": "string" },
                    "port": { "type": "integer" }
                }
            }
        });

        let (output, transforms) = run(input);

        assert_eq!(output["type"], "array");
        let value_schema = &output["items"]["properties"]["value"];
        assert_eq!(value_schema["type"], "object");
        assert_eq!(
            value_schema["properties"]["host"],
            json!({"type": "string"})
        );
        assert_eq!(
            value_schema["properties"]["port"],
            json!({"type": "integer"})
        );
        assert_eq!(transforms.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Test 3: Nested map (Map<Map<T>>) — recursive transpilation
    // -----------------------------------------------------------------------
    #[test]
    fn test_nested_map() {
        let input = json!({
            "type": "object",
            "additionalProperties": {
                "type": "object",
                "additionalProperties": { "type": "integer" }
            }
        });

        let (output, transforms) = run(input);

        // Outer level: array
        assert_eq!(output["type"], "array");
        // Inner level: value is also an array
        let inner = &output["items"]["properties"]["value"];
        assert_eq!(inner["type"], "array");
        assert_eq!(
            inner["items"]["properties"]["value"],
            json!({"type": "integer"})
        );
        // Two MapToArray transforms
        assert_eq!(transforms.len(), 2);
    }

    // -----------------------------------------------------------------------
    // Test 4: Description + title preserved on output array
    // -----------------------------------------------------------------------
    #[test]
    fn test_metadata_preserved() {
        let input = json!({
            "type": "object",
            "description": "HTTP headers",
            "title": "Headers",
            "additionalProperties": { "type": "string" }
        });

        let (output, transforms) = run(input);

        assert_eq!(output["type"], "array");
        assert_eq!(output["description"], "HTTP headers");
        assert_eq!(output["title"], "Headers");
        assert_eq!(transforms.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Test 5: Not-a-map — object with properties + AP: false
    // -----------------------------------------------------------------------
    #[test]
    fn test_not_a_map() {
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "additionalProperties": false
        });

        let (output, transforms) = run(input.clone());

        assert_eq!(output, input);
        assert_eq!(transforms.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 6: additionalProperties: true — bool, not schema → skip
    // -----------------------------------------------------------------------
    #[test]
    fn test_additional_properties_true() {
        let input = json!({
            "type": "object",
            "additionalProperties": true
        });

        let (output, transforms) = run(input.clone());

        assert_eq!(output, input);
        assert_eq!(transforms.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 7: Mixed object — properties + additionalProperties: Schema
    // -----------------------------------------------------------------------
    #[test]
    fn test_mixed_object() {
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "additionalProperties": { "type": "integer" }
        });

        let (output, transforms) = run(input);

        // Original property preserved.
        assert_eq!(output["properties"]["name"], json!({"type": "string"}));

        // _additional property added as array.
        let additional = &output["properties"]["_additional"];
        assert_eq!(additional["type"], "array");
        assert_eq!(
            additional["items"]["properties"]["value"],
            json!({"type": "integer"})
        );

        // additionalProperties removed.
        assert!(output.get("additionalProperties").is_none());

        // Two transforms: ExtractAdditionalProperties + MapToArray.
        assert_eq!(transforms.len(), 2);
        match &transforms[0] {
            Transform::ExtractAdditionalProperties {
                path,
                property_name,
            } => {
                assert_eq!(path, "#");
                assert_eq!(property_name, "_additional");
            }
            other => panic!("expected ExtractAdditionalProperties, got: {:?}", other),
        }
        match &transforms[1] {
            Transform::MapToArray { path, key_field } => {
                assert_eq!(path, "#/properties/_additional");
                assert_eq!(key_field, "key");
            }
            other => panic!("expected MapToArray, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Test 8: Gemini skip — target: Gemini → unchanged
    // -----------------------------------------------------------------------
    #[test]
    fn test_gemini_skip() {
        let input = json!({
            "type": "object",
            "additionalProperties": { "type": "string" }
        });

        let config = ConvertOptions {
            target: Target::Gemini,
            ..ConvertOptions::default()
        };

        let result = transpile_dictionaries(&input, &config).unwrap();

        assert_eq!(result.schema, input);
        assert_eq!(result.transforms.len(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 9: Depth guard — deeply nested schema exceeding max_depth
    // -----------------------------------------------------------------------
    #[test]
    fn test_depth_guard() {
        let input = json!({
            "type": "object",
            "additionalProperties": {
                "type": "object",
                "additionalProperties": {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                }
            }
        });

        let config = ConvertOptions {
            max_depth: 2,
            ..ConvertOptions::default()
        };

        let result = transpile_dictionaries(&input, &config);
        let err = result.unwrap_err();
        match err {
            ConvertError::RecursionDepthExceeded { max_depth, .. } => {
                assert_eq!(max_depth, config.max_depth);
            }
            other => panic!("expected RecursionDepthExceeded, got: {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Test 10: Idempotent — running output through pass again produces no changes
    // -----------------------------------------------------------------------
    #[test]
    fn test_idempotent() {
        let input = json!({
            "type": "object",
            "additionalProperties": { "type": "string" }
        });

        let (first_output, first_transforms) = run(input);
        let (second_output, second_transforms) = run(first_output.clone());

        assert_eq!(second_output, first_output);
        assert_eq!(second_transforms.len(), 0);
        assert!(!first_transforms.is_empty());
    }
}
