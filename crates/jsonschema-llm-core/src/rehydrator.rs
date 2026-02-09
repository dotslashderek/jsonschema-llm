//! Rehydrator — reconstructs original data shape from LLM output using the codec.
//!
//! Uses a DataWalker-style path traversal that handles both data-bearing segments
//! (`properties`, `items`) and schema-structural segments (`anyOf`, `oneOf`, etc.)
//! which are skipped during data navigation.

use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codec::{Codec, Transform};
use crate::codec_warning::{Warning, WarningKind};
use crate::error::ConvertError;
use crate::schema_utils::{escape_pointer_segment, split_path};

/// Result of rehydration, including the restored data and any warnings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RehydrateResult {
    /// The rehydrated data in the original schema shape.
    pub data: Value,
    /// Warnings about dropped constraint violations.
    pub warnings: Vec<Warning>,
}

/// Rehydrate LLM output using the codec sidecar.
///
/// Applies transforms in REVERSE order (LIFO) to undo the stack of changes,
/// then validates dropped constraints and collects warnings.
pub fn rehydrate(data: &Value, codec: &Codec) -> Result<RehydrateResult, ConvertError> {
    let mut result = data.clone();

    // Pre-compile all patternProperties regexes from transform and constraint paths
    let regex_cache = build_pattern_properties_cache(codec);

    for transform in codec.transforms.iter().rev() {
        let path_str = match transform {
            Transform::MapToArray { path, .. } => path,
            Transform::JsonStringParse { path } => path,
            Transform::NullableOptional { path, .. } => path,
            Transform::DiscriminatorAnyOf { path, .. } => path,
            Transform::ExtractAdditionalProperties { path, .. } => path,
            Transform::RecursiveInflate { path, .. } => path,
        };

        let segments = split_path(path_str);
        let seg_refs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();

        tracing::debug!(path = %path_str, "applying transform");
        apply_transform(&mut result, &seg_refs, transform, &regex_cache)?;
    }

    let warnings = validate_constraints(&result, codec, &regex_cache);

    Ok(RehydrateResult {
        data: result,
        warnings,
    })
}

/// Pre-scan transform and constraint paths for patternProperties segments
/// and compile their regex patterns into a reusable cache.
fn build_pattern_properties_cache(codec: &Codec) -> HashMap<String, Regex> {
    let mut cache = HashMap::new();

    // Scan transform paths
    let transform_paths = codec.transforms.iter().map(|t| match t {
        Transform::MapToArray { path, .. } => path.as_str(),
        Transform::JsonStringParse { path } => path.as_str(),
        Transform::NullableOptional { path, .. } => path.as_str(),
        Transform::DiscriminatorAnyOf { path, .. } => path.as_str(),
        Transform::ExtractAdditionalProperties { path, .. } => path.as_str(),
        Transform::RecursiveInflate { path, .. } => path.as_str(),
    });

    // Scan constraint paths
    let constraint_paths = codec.dropped_constraints.iter().map(|dc| dc.path.as_str());

    for path in transform_paths.chain(constraint_paths) {
        let segments = split_path(path);
        for window in segments.windows(2) {
            if window[0] == "patternProperties" {
                let pattern = &window[1];
                if !cache.contains_key(pattern.as_str()) {
                    match Regex::new(pattern) {
                        Ok(re) => {
                            cache.insert(pattern.clone(), re);
                        }
                        Err(e) => {
                            tracing::warn!(
                                pattern = %pattern,
                                error = %e,
                                "invalid patternProperties regex, will skip at usage sites"
                            );
                        }
                    }
                }
            }
        }
    }

    // Also pre-compile constraint `pattern` values (existing behavior from validate_constraints)
    for dc in &codec.dropped_constraints {
        if dc.constraint == "pattern" {
            if let Some(pat) = dc.value.as_str() {
                if !cache.contains_key(pat) {
                    match Regex::new(pat) {
                        Ok(re) => {
                            cache.insert(pat.to_string(), re);
                        }
                        Err(e) => {
                            tracing::warn!(
                                pattern = %pat,
                                error = %e,
                                "invalid regex in dropped constraint pattern — will emit ConstraintUnevaluable warning"
                            );
                        }
                    }
                }
            }
        }
    }

    cache
}

// ---------------------------------------------------------------------------
// Schema-path → data navigation (DataWalker)
// ---------------------------------------------------------------------------

/// Schema-structural keywords that should be skipped (keyword only).
const SKIP_SINGLE: &[&str] = &[
    "additionalProperties",
    "unevaluatedProperties",
    "unevaluatedItems",
    "contains",
    "propertyNames",
    "not",
    "if",
    "then",
    "else",
    "prefixItems",
];

/// Schema-structural keywords that should skip keyword + next segment.
const SKIP_PAIR: &[&str] = &[
    "anyOf",
    "oneOf",
    "allOf",
    "$defs",
    "definitions",
    "dependentSchemas",
    "patternProperties",
];

fn apply_transform(
    data: &mut Value,
    path_parts: &[&str],
    transform: &Transform,
    regex_cache: &HashMap<String, Regex>,
) -> Result<(), ConvertError> {
    // End of path — execute the transform
    if path_parts.is_empty() {
        tracing::trace!("reached end of path, executing transform");
        return execute_transform(data, transform);
    }

    let segment = path_parts[0];
    let rest = &path_parts[1..];

    // 1. Schema-structural: skip keyword only
    if SKIP_SINGLE.contains(&segment) {
        tracing::trace!(segment, "skipping schema-structural keyword");
        return apply_transform(data, rest, transform, regex_cache);
    }

    // 2. Schema-structural: skip keyword + next segment (index/name)
    if SKIP_PAIR.contains(&segment) {
        tracing::trace!(segment, "skipping schema-structural keyword pair");
        // Skip the keyword and the following segment (e.g. "anyOf" + "0")
        if rest.is_empty() {
            tracing::trace!(
                segment,
                "incomplete schema-structural keyword pair, stopping"
            );
            return Ok(());
        }
        let skip_to = &rest[1..];

        // Special case: patternProperties iterates matching object values
        if segment == "patternProperties" {
            if let Some(pattern) = rest.first() {
                if let Some(re) = regex_cache.get(*pattern) {
                    if let Some(obj) = data.as_object_mut() {
                        for (key, val) in obj.iter_mut() {
                            if re.is_match(key) {
                                apply_transform(val, skip_to, transform, regex_cache)?;
                            }
                        }
                    }
                } else {
                    // Cache miss means invalid regex — already warned during cache build
                    tracing::debug!(
                        pattern = %pattern,
                        "patternProperties regex not in cache (invalid?), skipping transform"
                    );
                }
            }
            return Ok(());
        }

        return apply_transform(data, skip_to, transform, regex_cache);
    }

    // 3. Array iteration: "items"
    if segment == "items" {
        if let Some(arr) = data.as_array_mut() {
            for item in arr {
                apply_transform(item, rest, transform, regex_cache)?;
            }
        }
        return Ok(());
    }

    // 4. Numeric index: array[n] for tuple/prefixItems navigation
    if let Ok(index) = segment.parse::<usize>() {
        if let Some(arr) = data.as_array_mut() {
            if let Some(item) = arr.get_mut(index) {
                return apply_transform(item, rest, transform, regex_cache);
            }
        }
        return Ok(());
    }

    // 5. Object navigation: "properties" -> "key"
    if segment == "properties" {
        if let Some(key) = rest.first() {
            let remaining = &rest[1..];

            // SPECIAL CASE: NullableOptional at final hop
            if let Transform::NullableOptional {
                original_required, ..
            } = transform
            {
                if remaining.is_empty() {
                    if !original_required {
                        if let Some(obj) = data.as_object_mut() {
                            if let Some(val) = obj.get(*key) {
                                if val.is_null() {
                                    obj.remove(*key);
                                }
                            }
                        }
                    }
                    return Ok(());
                }
            }

            // Normal navigation into property
            if let Some(obj) = data.as_object_mut() {
                if let Some(child) = obj.get_mut(*key) {
                    return apply_transform(child, remaining, transform, regex_cache);
                }
            }
            return Ok(());
        }
    }

    // Unknown segment — skip silently for forward compatibility
    tracing::trace!(segment, "unknown path segment, skipping");
    Ok(())
}

/// Execute a value-level transform at the current data node.
fn execute_transform(data: &mut Value, transform: &Transform) -> Result<(), ConvertError> {
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
            // Handled in the navigation step.
        }
        Transform::DiscriminatorAnyOf { .. } => {
            // No-op
        }
        Transform::RecursiveInflate { .. } => {
            parse_json_string(data)?;
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

// ---------------------------------------------------------------------------
// Dropped constraint validation
// ---------------------------------------------------------------------------

/// Advisory-only constraints where we just warn that they were dropped.
const ADVISORY_CONSTRAINTS: &[&str] = &["if", "then", "else"];

/// Validate dropped constraints against the rehydrated data.
///
/// Uses the pre-compiled regex cache for pattern matching. Walks each
/// constraint path to locate data nodes and check violations.
fn validate_constraints(
    data: &Value,
    codec: &Codec,
    regex_cache: &HashMap<String, Regex>,
) -> Vec<Warning> {
    if codec.dropped_constraints.is_empty() {
        return Vec::new();
    }

    let mut warnings = Vec::new();

    // Emit warnings for invalid/non-string pattern constraints (cache miss = invalid)
    for dc in &codec.dropped_constraints {
        if dc.constraint == "pattern" {
            if let Some(pat) = dc.value.as_str() {
                if !regex_cache.contains_key(pat) {
                    // Pattern was not cached — means it was invalid during cache build
                    // Re-compile to get error detail for the warning message
                    let error_detail = match Regex::new(pat) {
                        Ok(_) => "unknown error".to_string(),
                        Err(e) => e.to_string(),
                    };
                    warnings.push(Warning {
                        data_path: "/".to_string(),
                        schema_path: dc.path.clone(),
                        kind: WarningKind::ConstraintUnevaluable {
                            constraint: "pattern".to_string(),
                        },
                        message: format!(
                            "constraint 'pattern' ({}) has invalid regex and cannot be validated: {}",
                            pat, error_detail
                        ),
                    });
                }
            } else {
                // Non-string pattern value — cannot evaluate
                warnings.push(Warning {
                    data_path: "/".to_string(),
                    schema_path: dc.path.clone(),
                    kind: WarningKind::ConstraintUnevaluable {
                        constraint: "pattern".to_string(),
                    },
                    message: format!(
                        "constraint 'pattern' value ({}) is not a string and cannot be validated",
                        dc.value
                    ),
                });
            }
        }
    }

    for dc in &codec.dropped_constraints {
        // Advisory constraints — just note they were dropped
        if ADVISORY_CONSTRAINTS.contains(&dc.constraint.as_str()) {
            warnings.push(Warning {
                data_path: "/".to_string(),
                schema_path: dc.path.clone(),
                kind: WarningKind::ConstraintUnevaluable {
                    constraint: dc.constraint.clone(),
                },
                message: format!(
                    "constraint '{}' was dropped during compilation and cannot be validated",
                    dc.constraint
                ),
            });
            continue;
        }

        // Skip traversal for invalid pattern constraints (already warned above)
        if dc.constraint == "pattern" {
            if let Some(pat) = dc.value.as_str() {
                if !regex_cache.contains_key(pat) {
                    continue;
                }
            }
        }

        // Locate data nodes for this constraint's path
        let mut nodes = Vec::new();
        locate_data_nodes(
            data,
            &split_path(&dc.path),
            0,
            String::new(),
            &mut nodes,
            &mut warnings,
            &dc.path,
            regex_cache,
        );

        if nodes.is_empty() {
            // Path didn't resolve — not necessarily an error (data may be absent)
            continue;
        }

        for (data_path, value) in &nodes {
            if let Some(warning) = check_constraint(value, &dc.constraint, &dc.value, regex_cache) {
                warnings.push(Warning {
                    data_path: if data_path.is_empty() {
                        "/".to_string()
                    } else {
                        data_path.clone()
                    },
                    schema_path: dc.path.clone(),
                    kind: WarningKind::ConstraintViolation {
                        constraint: dc.constraint.clone(),
                    },
                    message: warning,
                });
            }
        }
    }

    warnings
}

/// Recursively locate data nodes matching a schema path (read-only).
///
/// Collects `(data_path, &Value)` tuples for each data node the schema path resolves to.
/// When `warnings` is provided, path-resolution issues (e.g. invalid regex) are surfaced
/// as `ConstraintUnevaluable` warnings rather than silently skipped.
#[allow(clippy::too_many_arguments)]
fn locate_data_nodes<'a>(
    data: &'a Value,
    segments: &[String],
    pos: usize,
    current_data_path: String,
    out: &mut Vec<(String, &'a Value)>,
    warnings: &mut Vec<Warning>,
    schema_path: &str,
    regex_cache: &HashMap<String, Regex>,
) {
    if pos >= segments.len() {
        out.push((current_data_path, data));
        return;
    }

    let segment = segments[pos].as_str();

    // Schema-structural: skip single
    if SKIP_SINGLE.contains(&segment) {
        locate_data_nodes(
            data,
            segments,
            pos + 1,
            current_data_path,
            out,
            warnings,
            schema_path,
            regex_cache,
        );
        return;
    }

    // Schema-structural: skip pair
    if SKIP_PAIR.contains(&segment) {
        if segment == "patternProperties" {
            if let Some(obj) = data.as_object() {
                // Extract pattern from the next segment; bail with warning if missing
                let Some(pattern_segment) = segments.get(pos + 1) else {
                    tracing::warn!(
                        "missing regex segment after patternProperties in schema path, skipping"
                    );
                    warnings.push(Warning {
                        data_path: if current_data_path.is_empty() {
                            "/".to_string()
                        } else {
                            current_data_path.clone()
                        },
                        schema_path: schema_path.to_string(),
                        kind: WarningKind::ConstraintUnevaluable {
                            constraint: "patternProperties".to_string(),
                        },
                        message: "missing regex segment after patternProperties in schema path; constraint cannot be evaluated".to_string(),
                    });
                    return;
                };
                let pattern = pattern_segment.as_str();

                if let Some(re) = regex_cache.get(pattern) {
                    for (key, val) in obj {
                        if re.is_match(key) {
                            let child_path =
                                format!("{}/{}", current_data_path, escape_pointer_segment(key));
                            locate_data_nodes(
                                val,
                                segments,
                                pos + 2, // patternProperties + pattern consumed
                                child_path,
                                out,
                                warnings,
                                schema_path,
                                regex_cache,
                            );
                        }
                    }
                } else {
                    // Cache miss = invalid regex, already warned during cache build
                    tracing::warn!(
                        pattern,
                        "patternProperties regex not in cache (invalid?), skipping constraint path"
                    );
                    warnings.push(Warning {
                        data_path: if current_data_path.is_empty() {
                            "/".to_string()
                        } else {
                            current_data_path.clone()
                        },
                        schema_path: schema_path.to_string(),
                        kind: WarningKind::ConstraintUnevaluable {
                            constraint: "patternProperties".to_string(),
                        },
                        message: format!(
                            "patternProperties regex '{}' is invalid and cannot be evaluated",
                            pattern
                        ),
                    });
                }
            }
            return;
        }

        if pos + 1 >= segments.len() {
            // Missing pair segment -> stop traversal to avoid incorrect targeting
            return;
        }
        let next_pos = pos + 2;

        locate_data_nodes(
            data,
            segments,
            next_pos,
            current_data_path,
            out,
            warnings,
            schema_path,
            regex_cache,
        );
        return;
    }

    // items → iterate array
    if segment == "items" {
        if let Some(arr) = data.as_array() {
            for (i, item) in arr.iter().enumerate() {
                let child_path = format!("{}/{}", current_data_path, i);
                locate_data_nodes(
                    item,
                    segments,
                    pos + 1,
                    child_path,
                    out,
                    warnings,
                    schema_path,
                    regex_cache,
                );
            }
        }
        return;
    }

    // Numeric index
    if let Ok(index) = segment.parse::<usize>() {
        if let Some(arr) = data.as_array() {
            if let Some(item) = arr.get(index) {
                let child_path = format!("{}/{}", current_data_path, index);
                locate_data_nodes(
                    item,
                    segments,
                    pos + 1,
                    child_path,
                    out,
                    warnings,
                    schema_path,
                    regex_cache,
                );
            }
        }
        return;
    }

    // properties → navigate into object
    if segment == "properties" {
        if let Some(key) = segments.get(pos + 1) {
            if let Some(obj) = data.as_object() {
                if let Some(child) = obj.get(key.as_str()) {
                    let child_path =
                        format!("{}/{}", current_data_path, escape_pointer_segment(key));
                    locate_data_nodes(
                        child,
                        segments,
                        pos + 2,
                        child_path,
                        out,
                        warnings,
                        schema_path,
                        regex_cache,
                    );
                }
            }
        }
    }
}

/// Check a single constraint against a value. Returns `Some(message)` on violation.
fn check_constraint(
    value: &Value,
    constraint: &str,
    expected: &Value,
    regex_cache: &HashMap<String, Regex>,
) -> Option<String> {
    match constraint {
        "pattern" => {
            let s = value.as_str()?;
            let pat = expected.as_str()?;
            let re = regex_cache.get(pat)?;
            if !re.is_match(s) {
                Some(format!("value {:?} does not match pattern {:?}", s, pat))
            } else {
                None
            }
        }
        "minimum" => {
            if let (Some(act), Some(exp)) = (value.as_i64(), expected.as_i64()) {
                if act < exp {
                    return Some(format!("value {} is less than minimum {}", act, exp));
                }
            } else if let (Some(act), Some(exp)) = (value.as_u64(), expected.as_u64()) {
                if act < exp {
                    return Some(format!("value {} is less than minimum {}", act, exp));
                }
            } else {
                let actual = value.as_f64()?;
                let bound = expected.as_f64()?;
                if actual < bound {
                    return Some(format!("value {} is less than minimum {}", actual, bound));
                }
            }
            None
        }
        "maximum" => {
            if let (Some(act), Some(exp)) = (value.as_i64(), expected.as_i64()) {
                if act > exp {
                    return Some(format!("value {} exceeds maximum {}", act, exp));
                }
            } else if let (Some(act), Some(exp)) = (value.as_u64(), expected.as_u64()) {
                if act > exp {
                    return Some(format!("value {} exceeds maximum {}", act, exp));
                }
            } else {
                let actual = value.as_f64()?;
                let bound = expected.as_f64()?;
                if actual > bound {
                    return Some(format!("value {} exceeds maximum {}", actual, bound));
                }
            }
            None
        }
        "exclusiveMinimum" => {
            if let (Some(act), Some(exp)) = (value.as_i64(), expected.as_i64()) {
                if act <= exp {
                    return Some(format!(
                        "value {} is not greater than exclusive minimum {}",
                        act, exp
                    ));
                }
            } else if let (Some(act), Some(exp)) = (value.as_u64(), expected.as_u64()) {
                if act <= exp {
                    return Some(format!(
                        "value {} is not greater than exclusive minimum {}",
                        act, exp
                    ));
                }
            } else {
                let actual = value.as_f64()?;
                let bound = expected.as_f64()?;
                if actual <= bound {
                    return Some(format!(
                        "value {} is not greater than exclusive minimum {}",
                        actual, bound
                    ));
                }
            }
            None
        }
        "exclusiveMaximum" => {
            if let (Some(act), Some(exp)) = (value.as_i64(), expected.as_i64()) {
                if act >= exp {
                    return Some(format!(
                        "value {} is not less than exclusive maximum {}",
                        act, exp
                    ));
                }
            } else if let (Some(act), Some(exp)) = (value.as_u64(), expected.as_u64()) {
                if act >= exp {
                    return Some(format!(
                        "value {} is not less than exclusive maximum {}",
                        act, exp
                    ));
                }
            } else {
                let actual = value.as_f64()?;
                let bound = expected.as_f64()?;
                if actual >= bound {
                    return Some(format!(
                        "value {} is not less than exclusive maximum {}",
                        actual, bound
                    ));
                }
            }
            None
        }
        "minLength" => {
            let s = value.as_str()?;
            let bound = expected.as_u64()? as usize;
            let char_count = s.chars().count();
            if char_count < bound {
                Some(format!(
                    "string length {} is less than minLength {}",
                    char_count, bound
                ))
            } else {
                None
            }
        }
        "maxLength" => {
            let s = value.as_str()?;
            let bound = expected.as_u64()? as usize;
            let char_count = s.chars().count();
            if char_count > bound {
                Some(format!(
                    "string length {} exceeds maxLength {}",
                    char_count, bound
                ))
            } else {
                None
            }
        }
        "minItems" => {
            let arr = value.as_array()?;
            let bound = expected.as_u64()? as usize;
            if arr.len() < bound {
                Some(format!(
                    "array length {} is less than minItems {}",
                    arr.len(),
                    bound
                ))
            } else {
                None
            }
        }
        "maxItems" => {
            let arr = value.as_array()?;
            let bound = expected.as_u64()? as usize;
            if arr.len() > bound {
                Some(format!(
                    "array length {} exceeds maxItems {}",
                    arr.len(),
                    bound
                ))
            } else {
                None
            }
        }
        _ => None,
    }
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
        assert_eq!(result.data, json!({"fixed": 1}));

        // Case B: Present value -> should be kept
        let data_present = json!({
            "fixed": 1,
            "optional": "kept"
        });
        let result_present = rehydrate(&data_present, &codec).unwrap();
        assert_eq!(result_present.data, json!({"fixed": 1, "optional": "kept"}));
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
            result.data,
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
            result.data,
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
            result.data,
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
        assert_eq!(result.data["list"][0]["data"], json!({"id": 1}));
        assert_eq!(result.data["list"][1]["data"], json!({"id": 2}));
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
            result.data,
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
        assert_eq!(result.data, data);
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
        assert_eq!(result.data["map"]["dup"], json!(2));
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
        assert!(result.data["outer"].get("inner").is_none());
        assert_eq!(result.data["outer"]["config"], json!({"x": 1}));
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
        assert!(result.data["map"].is_array());
        assert_eq!(result.data["map"].as_array().unwrap().len(), 2);
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
        assert_eq!(result.data["_extra"], json!("not an object"));
        assert_eq!(result.data["fixed"], json!("keep"));
    }

    // Test 13: Originally required nullable keeps null value
    #[test]
    fn test_strip_nullable_required_preserves_null() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::NullableOptional {
            path: "#/properties/required_field".to_string(),
            original_required: true,
        });

        let data = json!({
            "other": 1,
            "required_field": null
        });

        let result = rehydrate(&data, &codec).unwrap();
        // Required field should keep its null value
        assert_eq!(result.data["required_field"], json!(null));
        assert_eq!(result.data["other"], json!(1));
    }

    // --- Composition path traversal tests ---

    // Test 14: Transform through anyOf/0
    #[test]
    fn test_anyof_skip() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::JsonStringParse {
            path: "#/anyOf/0/properties/config".to_string(),
        });

        let data = json!({
            "config": "{\"a\": 1}"
        });

        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result.data, json!({"config": {"a": 1}}));
    }

    // Test 15: Transform through oneOf/1/items
    #[test]
    fn test_oneof_items_skip() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::JsonStringParse {
            path: "#/properties/list/oneOf/1/items/properties/data".to_string(),
        });

        let data = json!({
            "list": [
                {"data": "{\"x\": true}"},
                {"data": "{\"x\": false}"}
            ]
        });

        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result.data["list"][0]["data"], json!({"x": true}));
        assert_eq!(result.data["list"][1]["data"], json!({"x": false}));
    }

    // Test 16: Numeric index for tuple/prefixItems
    #[test]
    fn test_numeric_index() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::JsonStringParse {
            path: "#/prefixItems/1/properties/config".to_string(),
        });

        let data = json!([
            {"config": "kept as string"},
            {"config": "{\"parsed\": true}"}
        ]);

        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result.data[0]["config"], json!("kept as string"));
        assert_eq!(result.data[1]["config"], json!({"parsed": true}));
    }

    // Test 17: RFC 6901 escaped key in path
    #[test]
    fn test_rfc6901_escaped_key() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::NullableOptional {
            path: "#/properties/a~1b".to_string(), // a/b
            original_required: false,
        });

        let data = json!({
            "a/b": null,
            "other": 1
        });

        let result = rehydrate(&data, &codec).unwrap();
        assert!(result.data.get("a/b").is_none());
        assert_eq!(result.data["other"], json!(1));
    }

    // --- Dropped constraint validation tests ---

    // Test 18: Pattern violation warning
    #[test]
    fn test_pattern_violation_warning() {
        use crate::codec::DroppedConstraint;
        let mut codec = Codec::new();
        codec.dropped_constraints.push(DroppedConstraint {
            path: "#/properties/email".to_string(),
            constraint: "pattern".to_string(),
            value: json!("^[a-z]+@[a-z]+\\.[a-z]+$"),
        });

        let data = json!({"email": "NOT_AN_EMAIL"});
        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].data_path, "/email");
        assert!(result.warnings[0]
            .message
            .contains("does not match pattern"));
    }

    // Test 19: Pattern match — no warning
    #[test]
    fn test_pattern_match_no_warning() {
        use crate::codec::DroppedConstraint;
        let mut codec = Codec::new();
        codec.dropped_constraints.push(DroppedConstraint {
            path: "#/properties/email".to_string(),
            constraint: "pattern".to_string(),
            value: json!("^[a-z]+@[a-z]+\\.[a-z]+$"),
        });

        let data = json!({"email": "test@example.com"});
        let result = rehydrate(&data, &codec).unwrap();
        assert!(result.warnings.is_empty());
    }

    // Test 20: Numeric bounds warning
    #[test]
    fn test_minimum_violation_warning() {
        use crate::codec::DroppedConstraint;
        let mut codec = Codec::new();
        codec.dropped_constraints.push(DroppedConstraint {
            path: "#/properties/age".to_string(),
            constraint: "minimum".to_string(),
            value: json!(18),
        });

        let data = json!({"age": 15});
        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].message.contains("less than minimum"));
    }

    // Test 21: Maximum pass — no warning
    #[test]
    fn test_maximum_pass_no_warning() {
        use crate::codec::DroppedConstraint;
        let mut codec = Codec::new();
        codec.dropped_constraints.push(DroppedConstraint {
            path: "#/properties/score".to_string(),
            constraint: "maximum".to_string(),
            value: json!(100),
        });

        let data = json!({"score": 99});
        let result = rehydrate(&data, &codec).unwrap();
        assert!(result.warnings.is_empty());
    }

    // Test 22: maxLength violation
    #[test]
    fn test_maxlength_warning() {
        use crate::codec::DroppedConstraint;
        let mut codec = Codec::new();
        codec.dropped_constraints.push(DroppedConstraint {
            path: "#/properties/name".to_string(),
            constraint: "maxLength".to_string(),
            value: json!(3),
        });

        let data = json!({"name": "toolong"});
        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].message.contains("exceeds maxLength"));
    }

    // Test 23: Advisory if/then/else warning
    #[test]
    fn test_advisory_warning() {
        use crate::codec::DroppedConstraint;
        let mut codec = Codec::new();
        codec.dropped_constraints.push(DroppedConstraint {
            path: "#".to_string(),
            constraint: "if".to_string(),
            value: json!({"properties": {"type": {"const": "premium"}}}),
        });

        let data = json!({"type": "premium"});
        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0]
            .message
            .contains("dropped during compilation"));
    }

    // Test 24: Warning data path through array items
    #[test]
    fn test_warning_data_path_in_array() {
        use crate::codec::DroppedConstraint;
        let mut codec = Codec::new();
        codec.dropped_constraints.push(DroppedConstraint {
            path: "#/properties/users/items/properties/email".to_string(),
            constraint: "pattern".to_string(),
            value: json!("^.+@.+$"),
        });

        let data = json!({
            "users": [
                {"email": "good@test.com"},
                {"email": "bad"},
                {"email": "also@ok.net"}
            ]
        });

        let result = rehydrate(&data, &codec).unwrap();
        // Only users/1/email should fail
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].data_path, "/users/1/email");
    }

    // Test 25: Empty codec → no warnings
    #[test]
    fn test_no_constraints_no_warnings() {
        let codec = Codec::new();
        let data = json!({"any": "data"});
        let result = rehydrate(&data, &codec).unwrap();
        assert!(result.warnings.is_empty());
    }

    // Test 26: Constraint on missing data → no warning
    #[test]
    fn test_constraint_on_missing_data() {
        use crate::codec::DroppedConstraint;
        let mut codec = Codec::new();
        codec.dropped_constraints.push(DroppedConstraint {
            path: "#/properties/nonexistent".to_string(),
            constraint: "minimum".to_string(),
            value: json!(5),
        });

        let data = json!({"other": 1});
        let result = rehydrate(&data, &codec).unwrap();
        assert!(result.warnings.is_empty());
    }

    // Test 27: patternProperties constraint only warns for matching keys
    #[test]
    fn test_warning_pattern_properties_regex_filter() {
        use crate::codec::DroppedConstraint;
        let mut codec = Codec::new();
        codec.dropped_constraints.push(DroppedConstraint {
            path: "#/patternProperties/^S_".to_string(),
            constraint: "minLength".to_string(),
            value: json!(5),
        });

        let data = json!({
            "S_name": "Al",       // matches ^S_, len 2 < 5 → warning
            "S_code": "ABCDE",   // matches ^S_, len 5 ≥ 5 → no warning
            "other": "XY"        // does NOT match ^S_ → no warning
        });

        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].data_path, "/S_name");
    }

    // Test 28: Invalid regex pattern emits ConstraintUnevaluable warning
    #[test]
    fn test_invalid_regex_constraint_unevaluable() {
        use crate::codec::DroppedConstraint;
        let mut codec = Codec::new();
        codec.dropped_constraints.push(DroppedConstraint {
            path: "#/properties/code".to_string(),
            constraint: "pattern".to_string(),
            value: json!("[invalid"), // unclosed bracket = invalid regex
        });

        let data = json!({"code": "anything"});
        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].data_path, "/");
        assert_eq!(result.warnings[0].schema_path, "#/properties/code");
        assert!(result.warnings[0].message.contains("invalid regex"));
        assert!(
            matches!(&result.warnings[0].kind, WarningKind::ConstraintUnevaluable { constraint } if constraint == "pattern")
        );
    }

    // Test 29: Non-string pattern value emits ConstraintUnevaluable warning
    #[test]
    fn test_non_string_pattern_constraint_unevaluable() {
        use crate::codec::DroppedConstraint;
        let mut codec = Codec::new();
        codec.dropped_constraints.push(DroppedConstraint {
            path: "#/properties/code".to_string(),
            constraint: "pattern".to_string(),
            value: json!(42), // not a string
        });

        let data = json!({"code": "anything"});
        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].data_path, "/");
        assert!(result.warnings[0].message.contains("is not a string"));
        assert!(
            matches!(&result.warnings[0].kind, WarningKind::ConstraintUnevaluable { constraint } if constraint == "pattern")
        );
    }

    // Test 30: Missing regex segment after patternProperties
    #[test]
    fn test_missing_pattern_properties_segment() {
        use crate::codec::DroppedConstraint;
        let mut codec = Codec::new();
        // Path ends at patternProperties without a regex segment
        codec.dropped_constraints.push(DroppedConstraint {
            path: "#/patternProperties".to_string(),
            constraint: "minLength".to_string(),
            value: json!(5),
        });

        let data = json!({"any": "value"});
        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].schema_path, "#/patternProperties");
        assert!(result.warnings[0].message.contains("missing regex segment"));
        assert!(
            matches!(&result.warnings[0].kind, WarningKind::ConstraintUnevaluable { constraint } if constraint == "patternProperties")
        );
    }

    // Test 31: Additional numeric and length constraints
    #[test]
    fn test_additional_numeric_constraints() {
        use crate::codec::DroppedConstraint;
        let mut codec = Codec::new();

        let constraints = vec![
            ("#/properties/ex_min", "exclusiveMinimum", json!(10)),
            ("#/properties/ex_max", "exclusiveMaximum", json!(20)),
            ("#/properties/min_len", "minLength", json!(5)),
            ("#/properties/min_items", "minItems", json!(2)),
            ("#/properties/max_items", "maxItems", json!(2)),
        ];

        for (path, constraint, value) in constraints {
            codec.dropped_constraints.push(DroppedConstraint {
                path: path.to_string(),
                constraint: constraint.to_string(),
                value,
            });
        }

        let data = json!({
            "ex_min": 10,        // Fail: 10 is not > 10
            "ex_max": 20,        // Fail: 20 is not < 20
            "min_len": "fail",   // Fail: len 4 < 5
            "min_items": [1],    // Fail: len 1 < 2
            "max_items": [1, 2, 3] // Fail: len 3 > 2
        });

        let result = rehydrate(&data, &codec).unwrap();
        assert_eq!(result.warnings.len(), 5);

        let msgs: Vec<&str> = result.warnings.iter().map(|w| w.message.as_str()).collect();
        assert!(msgs
            .iter()
            .any(|m| m.contains("not greater than exclusive minimum")));
        assert!(msgs
            .iter()
            .any(|m| m.contains("not less than exclusive maximum")));
        assert!(msgs.iter().any(|m| m.contains("less than minLength")));
        assert!(msgs.iter().any(|m| m.contains("less than minItems")));
        assert!(msgs.iter().any(|m| m.contains("exceeds maxItems")));
    }

    // Test: RecursiveInflate rehydration round-trip
    #[test]
    fn test_recursive_inflate_rehydration() {
        let mut codec = Codec::new();
        codec.transforms.push(Transform::RecursiveInflate {
            path: "#/properties/child".to_string(),
            original_ref: "#/$defs/Node".to_string(),
        });

        // Simulate LLM output where the recursive child is a JSON-encoded string
        let data = json!({
            "value": 42,
            "child": "{\"value\": 99}"
        });

        let result = rehydrate(&data, &codec).unwrap();

        // The JSON string should be parsed back into an object
        assert_eq!(result.data["value"], 42);
        assert_eq!(result.data["child"]["value"], 99);
        assert!(
            result.data["child"].is_object(),
            "child should be an object after rehydration"
        );
    }
}
