//! Pass 1: Composition Compilation (`allOf` merge)
//!
//! Merges `allOf` sub-schemas into a single flat object schema.
//! Handles logical intersection of types, constraints, and properties.
//!
//! Merge semantics follow JSON Schema's `allOf` definition: a value must
//! satisfy ALL sub-schemas simultaneously. Constraints tighten (not loosen),
//! types narrow (not widen), and strictness wins.

use crate::codec::DroppedConstraint;
use crate::config::ConvertOptions;
use crate::error::ConvertError;
use crate::schema_utils::build_path;
use serde_json::{Map, Value};
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Apply composition compilation to a schema.
///
/// Recursively walks schema objects reachable via `properties`, `items`,
/// `anyOf`/`oneOf`/`allOf`, and `additionalProperties`. Where `allOf` is
/// found, all sub-schemas are merged into one flat object.
pub fn compile_composition(
    schema: &Value,
    config: &ConvertOptions,
) -> Result<(Value, Vec<DroppedConstraint>), ConvertError> {
    let mut dropped = Vec::new();
    let result = walk(schema.clone(), "#", 0, config, &mut dropped)?;
    Ok((result, dropped))
}

// ---------------------------------------------------------------------------
// Recursive walker
// ---------------------------------------------------------------------------

fn walk(
    schema: Value,
    path: &str,
    depth: usize,
    config: &ConvertOptions,
    dropped: &mut Vec<DroppedConstraint>,
) -> Result<Value, ConvertError> {
    if depth > config.max_depth {
        return Err(ConvertError::RecursionDepthExceeded {
            path: path.to_string(),
            max_depth: config.max_depth,
        });
    }

    let Value::Object(mut obj) = schema else {
        return Ok(schema);
    };

    // --- 1. Recurse into children FIRST (bottom-up) ---
    // Cross-reference: keyword list should match schema_utils::recurse_into_children.
    // P1 cannot use the shared helper because it consumes values and handles
    // allOf by merging siblings.

    // properties
    if let Some(Value::Object(props)) = obj.remove("properties") {
        let mut new_props = Map::new();
        for (key, val) in props {
            let child_path = build_path(path, &["properties", &key]);
            new_props.insert(key, walk(val, &child_path, depth + 1, config, dropped)?);
        }
        obj.insert("properties".to_string(), Value::Object(new_props));
    }

    // patternProperties
    if let Some(Value::Object(props)) = obj.remove("patternProperties") {
        let mut new_props = Map::new();
        for (key, val) in props {
            let child_path = build_path(path, &["patternProperties", &key]);
            new_props.insert(key, walk(val, &child_path, depth + 1, config, dropped)?);
        }
        obj.insert("patternProperties".to_string(), Value::Object(new_props));
    }

    // $defs / definitions / dependentSchemas (map-of-schemas)
    for keyword in ["$defs", "definitions", "dependentSchemas"] {
        if let Some(Value::Object(defs)) = obj.remove(keyword) {
            let mut new_defs = Map::new();
            for (key, val) in defs {
                let child_path = build_path(path, &[keyword, &key]);
                new_defs.insert(key, walk(val, &child_path, depth + 1, config, dropped)?);
            }
            obj.insert(keyword.to_string(), Value::Object(new_defs));
        }
    }

    // items (object or array form)
    if let Some(items) = obj.remove("items") {
        match items {
            Value::Object(_) => {
                let child_path = build_path(path, &["items"]);
                obj.insert(
                    "items".to_string(),
                    walk(items, &child_path, depth + 1, config, dropped)?,
                );
            }
            Value::Array(arr) => {
                let mut walked = Vec::with_capacity(arr.len());
                for (i, item) in arr.into_iter().enumerate() {
                    let child_path = build_path(path, &["items", &i.to_string()]);
                    walked.push(walk(item, &child_path, depth + 1, config, dropped)?);
                }
                obj.insert("items".to_string(), Value::Array(walked));
            }
            other => {
                obj.insert("items".to_string(), other);
            }
        }
    }

    // anyOf / oneOf
    for keyword in &["anyOf", "oneOf"] {
        if let Some(Value::Array(variants)) = obj.remove(*keyword) {
            let mut new_variants = Vec::new();
            for (i, v) in variants.into_iter().enumerate() {
                let child_path = build_path(path, &[keyword, &i.to_string()]);
                new_variants.push(walk(v, &child_path, depth + 1, config, dropped)?);
            }
            obj.insert(keyword.to_string(), Value::Array(new_variants));
        }
    }

    // Single-schema keywords: additionalProperties, unevaluatedProperties,
    // propertyNames, unevaluatedItems, contains, not, if, then, else
    for keyword in [
        "additionalProperties",
        "unevaluatedProperties",
        "propertyNames",
        "unevaluatedItems",
        "contains",
        "not",
        "if",
        "then",
        "else",
    ] {
        if let Some(val) = obj.remove(keyword) {
            if val.is_object() {
                let child_path = build_path(path, &[keyword]);
                obj.insert(
                    keyword.to_string(),
                    walk(val, &child_path, depth + 1, config, dropped)?,
                );
            } else {
                obj.insert(keyword.to_string(), val);
            }
        }
    }

    // prefixItems (array-of-schemas)
    if let Some(Value::Array(items)) = obj.remove("prefixItems") {
        let mut walked = Vec::with_capacity(items.len());
        for (i, item) in items.into_iter().enumerate() {
            let child_path = build_path(path, &["prefixItems", &i.to_string()]);
            walked.push(walk(item, &child_path, depth + 1, config, dropped)?);
        }
        obj.insert("prefixItems".to_string(), Value::Array(walked));
    }

    // allOf — recurse into sub-schemas before merging
    if let Some(Value::Array(sub_schemas)) = obj.remove("allOf") {
        let mut walked = Vec::new();
        for (i, sub) in sub_schemas.into_iter().enumerate() {
            let child_path = build_path(path, &["allOf", &i.to_string()]);
            walked.push(walk(sub, &child_path, depth + 1, config, dropped)?);
        }

        // Collect sibling keywords as an implicit first sub-schema
        let siblings = Value::Object(obj);
        let mut all = vec![siblings];
        all.extend(walked);

        return merge_allof(all, path, dropped);
    }

    Ok(Value::Object(obj))
}

// ---------------------------------------------------------------------------
// allOf merge
// ---------------------------------------------------------------------------

/// Fold N sub-schemas into one via pairwise merge.
fn merge_allof(
    schemas: Vec<Value>,
    path: &str,
    dropped: &mut Vec<DroppedConstraint>,
) -> Result<Value, ConvertError> {
    let mut iter = schemas.into_iter();
    let mut acc = iter.next().unwrap_or(Value::Object(Map::new()));
    for overlay in iter {
        acc = merge_two(acc, overlay, path, dropped)?;
    }
    Ok(acc)
}

/// Merge two schema objects. `overlay` keys take precedence on last-wins fields.
fn merge_two(
    base: Value,
    overlay: Value,
    path: &str,
    dropped: &mut Vec<DroppedConstraint>,
) -> Result<Value, ConvertError> {
    // Boolean schema `false` is the "impossible" schema — absorbs everything.
    if base == Value::Bool(false) || overlay == Value::Bool(false) {
        return Ok(Value::Bool(false));
    }
    let base_obj = match base {
        Value::Object(m) => m,
        _ => return Ok(overlay),
    };
    let overlay_obj = match overlay {
        Value::Object(m) => m,
        _ => return Ok(Value::Object(base_obj)),
    };

    let mut result = base_obj;

    for (k, v) in overlay_obj {
        match k.as_str() {
            // --- Recursive merge: properties ---
            "properties" => {
                merge_properties(&mut result, v, path, dropped)?;
            }

            // --- Union: required ---
            "required" => {
                merge_required(&mut result, v);
            }

            // --- Intersection: type ---
            "type" => {
                intersect_type(&mut result, v, path)?;
            }

            // --- Conflict: const ---
            "const" => {
                if let Some(existing) = result.get("const") {
                    if *existing != v {
                        return Err(ConvertError::SchemaError {
                            path: path.to_string(),
                            message: format!(
                                "allOf const conflict: cannot merge {:?} with {:?}",
                                existing, v
                            ),
                        });
                    }
                }
                result.insert("const".to_string(), v);
            }

            // --- Concatenate: description ---
            "description" => {
                merge_description(&mut result, v);
            }

            // --- Intersection: enum ---
            "enum" => {
                intersect_enum(&mut result, v);
            }

            // --- Tighten: numeric lower bounds ---
            "minimum" | "exclusiveMinimum" | "minLength" | "minItems" | "minProperties" => {
                tighten_min(&mut result, &k, v);
            }

            // --- Tighten: numeric upper bounds ---
            "maximum" | "exclusiveMaximum" | "maxLength" | "maxItems" | "maxProperties" => {
                tighten_max(&mut result, &k, v);
            }

            // --- Strictness intersection: additionalProperties ---
            "additionalProperties" => {
                merge_additional_properties(&mut result, v, path, dropped)?;
            }

            // --- Recursive merge: items ---
            "items" => {
                merge_items(&mut result, v, path, dropped)?;
            }

            // --- Drop: if/then/else ---
            "if" | "then" | "else" => {
                dropped.push(DroppedConstraint {
                    path: path.to_string(),
                    constraint: k,
                    value: v,
                });
            }

            // --- Last-wins: everything else ---
            _ => {
                result.insert(k, v);
            }
        }
    }

    // Strip any if/then/else that survived from the base schema
    for kw in &["if", "then", "else"] {
        if let Some(v) = result.remove(*kw) {
            dropped.push(DroppedConstraint {
                path: path.to_string(),
                constraint: kw.to_string(),
                value: v,
            });
        }
    }

    Ok(Value::Object(result))
}

// ---------------------------------------------------------------------------
// Merge helpers
// ---------------------------------------------------------------------------

/// Recursively merge `properties`: shared keys merge via `merge_two`, new keys added.
fn merge_properties(
    result: &mut Map<String, Value>,
    overlay_val: Value,
    path: &str,
    dropped: &mut Vec<DroppedConstraint>,
) -> Result<(), ConvertError> {
    let Value::Object(overlay_props) = overlay_val else {
        return Ok(());
    };
    let base_props = result
        .entry("properties")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(base_map) = base_props.as_object_mut() else {
        return Ok(());
    };
    for (prop_key, prop_val) in overlay_props {
        if let Some(existing) = base_map.remove(&prop_key) {
            // Both define this property — full recursive merge
            let prop_path = build_path(path, &["properties", &prop_key]);
            let merged = merge_two(existing, prop_val, &prop_path, dropped)?;
            base_map.insert(prop_key, merged);
        } else {
            base_map.insert(prop_key, prop_val);
        }
    }
    Ok(())
}

/// Union `required` arrays (deduplicated).
fn merge_required(result: &mut Map<String, Value>, overlay_val: Value) {
    let Value::Array(overlay_arr) = overlay_val else {
        return;
    };
    let base_req = result
        .entry("required")
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(base_arr) = base_req.as_array_mut() else {
        return;
    };
    let mut seen: HashSet<String> = base_arr
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    for item in overlay_arr {
        if let Some(s) = item.as_str() {
            if seen.insert(s.to_string()) {
                base_arr.push(item);
            }
        }
    }
}

/// Intersect `type` with subtype awareness.
fn intersect_type(
    result: &mut Map<String, Value>,
    overlay_val: Value,
    path: &str,
) -> Result<(), ConvertError> {
    let Some(existing) = result.get("type") else {
        result.insert("type".to_string(), overlay_val);
        return Ok(());
    };

    let base_type = type_to_string(existing);
    let overlay_type = type_to_string(&overlay_val);

    if let (Some(b), Some(o)) = (&base_type, &overlay_type) {
        if b == o {
            // Same type — keep it
            return Ok(());
        }
        // Subtype compatibility: number ⊃ integer
        if (b == "number" && o == "integer") || (b == "integer" && o == "number") {
            result.insert("type".to_string(), Value::String("integer".to_string()));
            return Ok(());
        }
        return Err(ConvertError::SchemaError {
            path: path.to_string(),
            message: format!("allOf type conflict: cannot merge '{}' with '{}'", b, o),
        });
    }

    // Handle array-form types: intersect the type sets.
    let base_types = type_to_set(existing);
    let overlay_types = type_to_set(&overlay_val);

    if !base_types.is_empty() && !overlay_types.is_empty() {
        // Normalize: expand "number" to include "integer" for subtype compatibility
        let base_expanded = expand_number_subtype(&base_types);
        let overlay_expanded = expand_number_subtype(&overlay_types);
        let raw_intersection: HashSet<String> = base_expanded
            .intersection(&overlay_expanded)
            .cloned()
            .collect();
        // Narrow back: if both "number" and "integer" survived, keep only "integer"
        let intersection: Vec<String> = narrow_number_subtype(&raw_intersection)
            .into_iter()
            .collect();
        if intersection.is_empty() {
            return Err(ConvertError::SchemaError {
                path: path.to_string(),
                message: format!(
                    "allOf type conflict: no common type between {:?} and {:?}",
                    base_types, overlay_types
                ),
            });
        }
        if intersection.len() == 1 {
            result.insert("type".to_string(), Value::String(intersection[0].clone()));
        } else {
            let arr: Vec<Value> = intersection.into_iter().map(Value::String).collect();
            result.insert("type".to_string(), Value::Array(arr));
        }
    } else {
        // Fallback: overlay wins
        result.insert("type".to_string(), overlay_val);
    }
    Ok(())
}

/// Concatenate `description` with newline separator.
fn merge_description(result: &mut Map<String, Value>, overlay_val: Value) {
    if let Some(existing) = result.get("description") {
        if let (Some(base_str), Some(overlay_str)) = (existing.as_str(), overlay_val.as_str()) {
            let combined = format!("{}\n{}", base_str, overlay_str);
            result.insert("description".to_string(), Value::String(combined));
            return;
        }
    }
    result.insert("description".to_string(), overlay_val);
}

/// Intersect `enum` values — keep only values present in ALL schemas.
fn intersect_enum(result: &mut Map<String, Value>, overlay_val: Value) {
    let Value::Array(overlay_arr) = overlay_val else {
        return;
    };
    let Some(Value::Array(base_arr)) = result.get("enum") else {
        result.insert("enum".to_string(), Value::Array(overlay_arr));
        return;
    };
    let intersection: Vec<Value> = base_arr
        .iter()
        .filter(|v| overlay_arr.contains(v))
        .cloned()
        .collect();
    result.insert("enum".to_string(), Value::Array(intersection));
}

/// Tighten a minimum-like bound: max of all values.
fn tighten_min(result: &mut Map<String, Value>, key: &str, overlay_val: Value) {
    if let Some(existing) = result.get(key) {
        if let (Some(base_f), Some(overlay_f)) = (existing.as_f64(), overlay_val.as_f64()) {
            if overlay_f > base_f {
                result.insert(key.to_string(), overlay_val);
            }
            return;
        }
    }
    result.insert(key.to_string(), overlay_val);
}

/// Tighten a maximum-like bound: min of all values.
fn tighten_max(result: &mut Map<String, Value>, key: &str, overlay_val: Value) {
    if let Some(existing) = result.get(key) {
        if let (Some(base_f), Some(overlay_f)) = (existing.as_f64(), overlay_val.as_f64()) {
            if overlay_f < base_f {
                result.insert(key.to_string(), overlay_val);
            }
            return;
        }
    }
    result.insert(key.to_string(), overlay_val);
}

/// Strictness intersection for `additionalProperties`.
///
/// - `false` + anything → `false`  
/// - `true` + `true` → `true`  
/// - schema + schema → recursive merge  
fn merge_additional_properties(
    result: &mut Map<String, Value>,
    overlay_val: Value,
    path: &str,
    dropped: &mut Vec<DroppedConstraint>,
) -> Result<(), ConvertError> {
    let Some(existing) = result.get("additionalProperties").cloned() else {
        result.insert("additionalProperties".to_string(), overlay_val);
        return Ok(());
    };

    // false wins over everything
    if existing == Value::Bool(false) || overlay_val == Value::Bool(false) {
        result.insert("additionalProperties".to_string(), Value::Bool(false));
        return Ok(());
    }

    // Schema + true: keep the stricter schema (true = allow anything)
    if existing.is_object() && overlay_val == Value::Bool(true) {
        return Ok(());
    }
    if overlay_val.is_object() && existing == Value::Bool(true) {
        result.insert("additionalProperties".to_string(), overlay_val);
        return Ok(());
    }

    // Both schemas → recursive merge
    if existing.is_object() && overlay_val.is_object() {
        let child_path = build_path(path, &["additionalProperties"]);
        let merged = merge_two(existing, overlay_val, &child_path, dropped)?;
        result.insert("additionalProperties".to_string(), merged);
        return Ok(());
    }

    // Otherwise overlay wins (true + true)
    result.insert("additionalProperties".to_string(), overlay_val);
    Ok(())
}

/// Recursive merge for `items`.
fn merge_items(
    result: &mut Map<String, Value>,
    overlay_val: Value,
    path: &str,
    dropped: &mut Vec<DroppedConstraint>,
) -> Result<(), ConvertError> {
    let Some(existing) = result.remove("items") else {
        result.insert("items".to_string(), overlay_val);
        return Ok(());
    };

    if existing.is_object() && overlay_val.is_object() {
        let child_path = build_path(path, &["items"]);
        let merged = merge_two(existing, overlay_val, &child_path, dropped)?;
        result.insert("items".to_string(), merged);
    } else {
        // Non-object items: overlay wins
        result.insert("items".to_string(), overlay_val);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Extract a single type string from a `type` value (handles both string and array).
fn type_to_string(val: &Value) -> Option<String> {
    match val {
        Value::String(s) => Some(s.clone()),
        Value::Array(arr) if arr.len() == 1 => arr[0].as_str().map(String::from),
        _ => None,
    }
}

/// Extract all type strings as a set for intersection logic.
fn type_to_set(val: &Value) -> HashSet<String> {
    match val {
        Value::String(s) => {
            let mut set = HashSet::new();
            set.insert(s.clone());
            set
        }
        Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => HashSet::new(),
    }
}

/// Expand type set: if it contains "number", also include "integer" (subtype).
fn expand_number_subtype(types: &HashSet<String>) -> HashSet<String> {
    let mut expanded = types.clone();
    if expanded.contains("number") {
        expanded.insert("integer".to_string());
    }
    expanded
}

/// Narrow type set: if both "number" and "integer" are present, keep only "integer".
fn narrow_number_subtype(types: &HashSet<String>) -> HashSet<String> {
    let mut narrowed = types.clone();
    if narrowed.contains("number") && narrowed.contains("integer") {
        narrowed.remove("number");
    }
    narrowed
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(schema: Value) -> (Value, Vec<DroppedConstraint>) {
        let config = ConvertOptions::default();
        compile_composition(&schema, &config).expect("should not error")
    }

    // -----------------------------------------------------------------------
    // 1. Simple two-schema allOf (base + extension)
    // -----------------------------------------------------------------------
    #[test]
    fn test_simple_merge() {
        let input = json!({
            "allOf": [
                {
                    "type": "object",
                    "properties": { "id": { "type": "string" } },
                    "required": ["id"]
                },
                {
                    "type": "object",
                    "properties": { "name": { "type": "string" } },
                    "required": ["name"]
                }
            ]
        });

        let (output, _) = run(input);

        assert_eq!(output["type"], "object");
        assert_eq!(output["properties"]["id"], json!({"type": "string"}));
        assert_eq!(output["properties"]["name"], json!({"type": "string"}));

        let req: Vec<&str> = output["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(req.contains(&"id"));
        assert!(req.contains(&"name"));
        assert!(output.get("allOf").is_none());
    }

    // -----------------------------------------------------------------------
    // 2. Three-way allOf
    // -----------------------------------------------------------------------
    #[test]
    fn test_three_way_merge() {
        let input = json!({
            "allOf": [
                { "type": "object", "properties": { "a": { "type": "string" } } },
                { "properties": { "b": { "type": "integer" } } },
                { "properties": { "c": { "type": "boolean" } }, "required": ["c"] }
            ]
        });

        let (output, _) = run(input);

        assert_eq!(output["properties"]["a"], json!({"type": "string"}));
        assert_eq!(output["properties"]["b"], json!({"type": "integer"}));
        assert_eq!(output["properties"]["c"], json!({"type": "boolean"}));
        assert_eq!(output["required"], json!(["c"]));
    }

    // -----------------------------------------------------------------------
    // 3. Shared-key recursive merge
    // -----------------------------------------------------------------------
    #[test]
    fn test_shared_key_recursive_merge() {
        let input = json!({
            "allOf": [
                {
                    "type": "object",
                    "properties": {
                        "age": { "type": "integer", "minimum": 0 }
                    }
                },
                {
                    "type": "object",
                    "properties": {
                        "age": { "type": "integer", "maximum": 150 }
                    }
                }
            ]
        });

        let (output, _) = run(input);

        let age = &output["properties"]["age"];
        assert_eq!(age["type"], "integer");
        assert_eq!(age["minimum"], 0);
        assert_eq!(age["maximum"], 150);
    }

    // -----------------------------------------------------------------------
    // 4. Type conflict → error
    // -----------------------------------------------------------------------
    #[test]
    fn test_type_conflict_error() {
        let input = json!({
            "allOf": [
                { "type": "string" },
                { "type": "integer" }
            ]
        });

        let config = ConvertOptions::default();
        let result = compile_composition(&input, &config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("type conflict"));
    }

    // -----------------------------------------------------------------------
    // 5. Type narrowing (number + integer → integer)
    // -----------------------------------------------------------------------
    #[test]
    fn test_type_narrowing() {
        let input = json!({
            "allOf": [
                { "type": "number", "minimum": 0 },
                { "type": "integer" }
            ]
        });

        let (output, _) = run(input);
        assert_eq!(output["type"], "integer");
        assert_eq!(output["minimum"], 0);
    }

    // -----------------------------------------------------------------------
    // 6. allOf nested inside properties
    // -----------------------------------------------------------------------
    #[test]
    fn test_nested_in_properties() {
        let input = json!({
            "type": "object",
            "properties": {
                "address": {
                    "allOf": [
                        { "type": "object", "properties": { "street": { "type": "string" } } },
                        { "properties": { "city": { "type": "string" } } }
                    ]
                }
            }
        });

        let (output, _) = run(input);

        let addr = &output["properties"]["address"];
        assert_eq!(addr["properties"]["street"], json!({"type": "string"}));
        assert_eq!(addr["properties"]["city"], json!({"type": "string"}));
        assert!(addr.get("allOf").is_none());
    }

    // -----------------------------------------------------------------------
    // 7. allOf + sibling keywords
    // -----------------------------------------------------------------------
    #[test]
    fn test_allof_with_siblings() {
        let input = json!({
            "description": "A person",
            "allOf": [
                {
                    "type": "object",
                    "properties": { "name": { "type": "string" } }
                },
                {
                    "description": "with age",
                    "properties": { "age": { "type": "integer" } }
                }
            ]
        });

        let (output, _) = run(input);

        // Description concatenated: "A person" + "with age"
        let desc = output["description"].as_str().unwrap();
        assert!(desc.contains("A person"));
        assert!(desc.contains("with age"));
        assert_eq!(output["properties"]["name"], json!({"type": "string"}));
        assert_eq!(output["properties"]["age"], json!({"type": "integer"}));
    }

    // -----------------------------------------------------------------------
    // 8. if/then/else dropped
    // -----------------------------------------------------------------------
    #[test]
    fn test_if_then_else_dropped() {
        let input = json!({
            "allOf": [
                { "type": "object", "properties": { "x": { "type": "string" } } },
                {
                    "if": { "properties": { "x": { "const": "a" } } },
                    "then": { "required": ["y"] },
                    "else": { "required": ["z"] }
                }
            ]
        });

        let (output, dropped) = run(input);

        // if/then/else should be dropped
        assert!(output.get("if").is_none());
        assert!(output.get("then").is_none());
        assert!(output.get("else").is_none());

        // Dropped constraints recorded
        let constraints: Vec<&str> = dropped.iter().map(|d| d.constraint.as_str()).collect();
        assert!(constraints.contains(&"if"));
        assert!(constraints.contains(&"then"));
        assert!(constraints.contains(&"else"));
    }

    // -----------------------------------------------------------------------
    // 9. Constraint narrowing
    // -----------------------------------------------------------------------
    #[test]
    fn test_constraint_narrowing() {
        let input = json!({
            "allOf": [
                { "type": "integer", "minimum": 0, "maximum": 100 },
                { "type": "integer", "minimum": 10, "maximum": 50 }
            ]
        });

        let (output, _) = run(input);

        assert_eq!(output["minimum"], 10); // max of minima
        assert_eq!(output["maximum"], 50); // min of maxima
    }

    // -----------------------------------------------------------------------
    // 10. additionalProperties strictness
    // -----------------------------------------------------------------------
    #[test]
    fn test_additional_properties_strictness() {
        let input = json!({
            "allOf": [
                {
                    "type": "object",
                    "properties": { "id": { "type": "string" } },
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "properties": { "name": { "type": "string" } },
                    "additionalProperties": true
                }
            ]
        });

        let (output, _) = run(input);

        // false should win (strictness intersection)
        assert_eq!(output["additionalProperties"], json!(false));
    }

    // -----------------------------------------------------------------------
    // 11. Idempotent — flat schema unchanged
    // -----------------------------------------------------------------------
    #[test]
    fn test_idempotent() {
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            },
            "required": ["name"]
        });

        let (output, dropped) = run(input.clone());

        assert_eq!(output, input);
        assert!(dropped.is_empty());
    }

    // -----------------------------------------------------------------------
    // 12. Boolean schema `false` absorbs everything
    // -----------------------------------------------------------------------
    #[test]
    fn test_boolean_false_absorbs() {
        let input = json!({
            "allOf": [
                false,
                { "type": "object", "properties": { "x": { "type": "string" } } }
            ]
        });

        let (output, _) = run(input);
        assert_eq!(output, json!(false));
    }

    // -----------------------------------------------------------------------
    // 13. const conflict → error
    // -----------------------------------------------------------------------
    #[test]
    fn test_const_conflict_error() {
        let input = json!({
            "allOf": [
                { "const": "a" },
                { "const": "b" }
            ]
        });

        let config = ConvertOptions::default();
        let result = compile_composition(&input, &config);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("const conflict"));
    }

    // -----------------------------------------------------------------------
    // 14. AP: true does not overwrite schema
    // -----------------------------------------------------------------------
    #[test]
    fn test_ap_true_preserves_schema() {
        let input = json!({
            "allOf": [
                {
                    "type": "object",
                    "additionalProperties": { "type": "integer" }
                },
                {
                    "type": "object",
                    "additionalProperties": true
                }
            ]
        });

        let (output, _) = run(input);
        assert_eq!(output["additionalProperties"], json!({"type": "integer"}));
    }

    // -----------------------------------------------------------------------
    // 15. Type array intersection: "string" ∩ ["string", "null"] = "string"
    // -----------------------------------------------------------------------
    #[test]
    fn test_type_array_intersection() {
        let input = json!({
            "allOf": [
                { "type": "string" },
                { "type": ["string", "null"] }
            ]
        });

        let (output, _) = run(input);
        assert_eq!(output["type"], "string");
    }

    // -----------------------------------------------------------------------
    // 16. AP symmetry: true + schema (reversed order) keeps schema
    // -----------------------------------------------------------------------
    #[test]
    fn test_ap_true_schema_reversed() {
        let input = json!({
            "allOf": [
                {
                    "type": "object",
                    "additionalProperties": true
                },
                {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                }
            ]
        });

        let (output, _) = run(input);
        assert_eq!(output["additionalProperties"], json!({"type": "string"}));
    }

    // -----------------------------------------------------------------------
    // 17. Number ∩ [integer, null] — subtype-aware array intersection
    // -----------------------------------------------------------------------
    #[test]
    fn test_number_integer_array_intersection() {
        let input = json!({
            "allOf": [
                { "type": "number" },
                { "type": ["integer", "null"] }
            ]
        });

        let (output, _) = run(input);
        assert_eq!(output["type"], "integer");
    }

    // -----------------------------------------------------------------------
    // 18. if/then/else in base (siblings) are stripped
    // -----------------------------------------------------------------------
    #[test]
    fn test_if_then_else_base_stripped() {
        let input = json!({
            "if": { "properties": { "x": { "const": 1 } } },
            "then": { "required": ["y"] },
            "else": { "required": ["z"] },
            "allOf": [
                { "type": "object" }
            ]
        });

        let (output, dropped) = run(input);
        assert!(output.get("if").is_none());
        assert!(output.get("then").is_none());
        assert!(output.get("else").is_none());
        assert!(dropped.iter().any(|d| d.constraint == "if"));
        assert!(dropped.iter().any(|d| d.constraint == "then"));
        assert!(dropped.iter().any(|d| d.constraint == "else"));
    }
}
