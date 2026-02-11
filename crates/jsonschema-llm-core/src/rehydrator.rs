//! Rehydrator — reconstructs original data shape from LLM output using the codec.
//!
//! Uses a DataWalker-style path traversal that handles both data-bearing segments
//! (`properties`, `items`) and schema-structural segments (`anyOf`, `oneOf`, etc.)
//! which are skipped during data navigation.

use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::codec::{Codec, Transform, CODEC_MAJOR_VERSION};
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
/// then enforces enforceable dropped constraints (clamp/truncate) and validates
/// the rest.
pub fn rehydrate(data: &Value, codec: &Codec) -> Result<RehydrateResult, ConvertError> {
    // Validate codec version — hard-fail on incompatible major version
    validate_codec_version(codec)?;

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
            Transform::RootObjectWrapper { path, .. } => path,
            Transform::EnumStringify { path, .. } => path,
        };

        let segments = split_path(path_str);
        let seg_refs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();

        tracing::debug!(path = %path_str, "applying transform");
        apply_transform(&mut result, &seg_refs, transform, &regex_cache)?;
    }

    // Enforce enforceable constraints (clamp/truncate) before validation
    let mut enforcement_warnings = enforce_constraints(&mut result, codec, &regex_cache);

    // Validate remaining constraints (pattern, min*, etc.) as advisory warnings
    let mut validation_warnings = validate_constraints(&result, codec, &regex_cache);
    enforcement_warnings.append(&mut validation_warnings);

    Ok(RehydrateResult {
        data: result,
        warnings: enforcement_warnings,
    })
}

/// Pre-scan transform and constraint paths for patternProperties segments
/// and compile their regex patterns into a reusable cache.
fn build_pattern_properties_cache(codec: &Codec) -> HashMap<String, Result<Regex, String>> {
    let mut cache = HashMap::new();

    // Extract paths from transforms using match statement
    let transform_paths = codec.transforms.iter().map(|t| match t {
        Transform::MapToArray { path, .. } => path.as_str(),
        Transform::JsonStringParse { path } => path.as_str(),
        Transform::NullableOptional { path, .. } => path.as_str(),
        Transform::DiscriminatorAnyOf { path, .. } => path.as_str(),
        Transform::ExtractAdditionalProperties { path, .. } => path.as_str(),
        Transform::RecursiveInflate { path, .. } => path.as_str(),
        Transform::RootObjectWrapper { path, .. } => path.as_str(),
        Transform::EnumStringify { path, .. } => path.as_str(),
    });
    let constraint_paths = codec.dropped_constraints.iter().map(|dc| dc.path.as_str());

    // Scan for patternProperties patterns
    for path in transform_paths.chain(constraint_paths) {
        let segments = split_path(path);
        for window in segments.windows(2) {
            if window[0] == "patternProperties" {
                let pattern = &window[1];
                if !cache.contains_key(pattern.as_str()) {
                    let result = Regex::new(pattern).map_err(|e| e.to_string());
                    if let Err(ref err) = result {
                        tracing::warn!(
                            pattern = %pattern,
                            error = %err,
                            "invalid patternProperties regex, will skip at usage sites"
                        );
                    }
                    cache.insert(pattern.clone(), result);
                }
            }
        }
    }

    // Also pre-compile constraint `pattern` values (existing behavior from validate_constraints)
    for dc in &codec.dropped_constraints {
        if dc.constraint == "pattern" {
            if let Some(pat) = dc.value.as_str() {
                if !cache.contains_key(pat) {
                    let result = Regex::new(pat).map_err(|e| e.to_string());
                    if let Err(ref err) = result {
                        tracing::warn!(
                            pattern = %pat,
                            error = %err,
                            "invalid regex in dropped constraint pattern — will emit ConstraintUnevaluable warning"
                        );
                    }
                    cache.insert(pat.to_string(), result);
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
    regex_cache: &HashMap<String, Result<Regex, String>>,
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
                match regex_cache.get(*pattern) {
                    Some(Ok(re)) => {
                        if let Some(obj) = data.as_object_mut() {
                            for (key, val) in obj.iter_mut() {
                                if re.is_match(key) {
                                    apply_transform(val, skip_to, transform, regex_cache)?;
                                }
                            }
                        }
                    }
                    Some(Err(_)) | None => {
                        // Invalid regex (Err) or cache miss (None) — already warned during cache build.
                        // Skipping this transform is intentional: transforms are best-effort.
                        tracing::debug!(
                            pattern = %pattern,
                            "patternProperties regex invalid or not in cache, skipping transform"
                        );
                    }
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
    regex_cache: &HashMap<String, Result<Regex, String>>,
) -> Vec<Warning> {
    if codec.dropped_constraints.is_empty() {
        return Vec::new();
    }

    let mut warnings = Vec::new();

    // Emit warnings for invalid/non-string pattern constraints (cache miss = invalid)
    for dc in &codec.dropped_constraints {
        if dc.constraint == "pattern" {
            if let Some(pat) = dc.value.as_str() {
                match regex_cache.get(pat) {
                    Some(Err(err)) => {
                        // Invalid regex (Err contains error string from compilation)
                        warnings.push(Warning {
                            data_path: "/".to_string(),
                            schema_path: dc.path.clone(),
                            kind: WarningKind::ConstraintUnevaluable {
                                constraint: "pattern".to_string(),
                            },
                            message: format!(
                                "constraint 'pattern' ({}) cannot be validated: {}",
                                pat, err
                            ),
                        });
                    }
                    None => {
                        // Cache miss (internal error - should never happen)
                        warnings.push(Warning {
                            data_path: "/".to_string(),
                            schema_path: dc.path.clone(),
                            kind: WarningKind::ConstraintUnevaluable {
                                constraint: "pattern".to_string(),
                            },
                            message: format!(
                                "constraint 'pattern' ({}) cannot be validated: regex missing from cache (internal error)",
                                pat
                            ),
                        });
                    }
                    Some(Ok(_)) => {
                        // Valid regex, no warning needed during validation_constraints pass
                        // (actual validation happens in check_constraint)
                    }
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
    regex_cache: &HashMap<String, Result<Regex, String>>,
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

                match regex_cache.get(pattern) {
                    Some(Ok(re)) => {
                        for (key, val) in obj {
                            if re.is_match(key) {
                                let child_path = format!(
                                    "{}/{}",
                                    current_data_path,
                                    escape_pointer_segment(key)
                                );
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
                    }
                    Some(Err(err)) => {
                        // Invalid regex (Err contains compile error string)
                        tracing::debug!(
                            pattern,
                            error = %err,
                            "patternProperties regex invalid, skipping constraint path"
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
                                "patternProperties regex '{}' cannot be evaluated: {}",
                                pattern, err
                            ),
                        });
                    }
                    None => {
                        // Cache miss (internal error - should never happen)
                        tracing::debug!(
                            pattern,
                            "patternProperties regex not in cache, skipping constraint path"
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
                                "patternProperties regex '{}' cannot be evaluated: regex missing from cache (internal error)",
                                pattern
                            ),
                        });
                    }
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
    regex_cache: &HashMap<String, Result<Regex, String>>,
) -> Option<String> {
    match constraint {
        "pattern" => {
            let s = value.as_str()?;
            let pat = expected.as_str()?;
            let re = regex_cache.get(pat)?.as_ref().ok()?; // Unwrap Result, skip if Err
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

// ---------------------------------------------------------------------------
// Constraint enforcement (clamp/truncate)
// ---------------------------------------------------------------------------

/// Constraints that can be automatically enforced by modifying data.
const ENFORCEABLE_CONSTRAINTS: &[&str] = &[
    "maximum",
    "minimum",
    "exclusiveMaximum",
    "exclusiveMinimum",
    "maxLength",
    "maxItems",
];

/// Enforce dropped constraints by modifying data in-place.
///
/// For constraints where we can safely fix violations without data loss:
/// - `maximum` / `minimum`: clamp numeric values
/// - `exclusiveMaximum` / `exclusiveMinimum`: clamp to boundary ± 1
/// - `maxLength`: truncate strings
/// - `maxItems`: truncate arrays
///
/// Returns warnings for each enforcement action taken.
fn enforce_constraints(
    data: &mut Value,
    codec: &Codec,
    regex_cache: &HashMap<String, Result<Regex, String>>,
) -> Vec<Warning> {
    let mut warnings = Vec::new();

    for dc in &codec.dropped_constraints {
        if !ENFORCEABLE_CONSTRAINTS.contains(&dc.constraint.as_str()) {
            continue;
        }

        // Locate mutable data nodes for this constraint's path
        let segments = split_path(&dc.path);
        let mut data_paths: Vec<String> = Vec::new();
        collect_data_paths(
            data,
            &segments,
            0,
            String::new(),
            &mut data_paths,
            regex_cache,
        );

        for data_path in &data_paths {
            // Navigate to the mutable node using the data path
            let node = match navigate_to_mut(data, data_path) {
                Some(n) => n,
                None => continue,
            };

            if let Some(msg) = enforce_single_constraint(node, &dc.constraint, &dc.value) {
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
                    message: msg,
                });
            }
        }
    }

    warnings
}

/// Collect JSON Pointer data paths for a schema path.
///
/// Similar to `locate_data_nodes` but only collects paths (not references),
/// so we can later navigate mutably.
fn collect_data_paths(
    data: &Value,
    segments: &[String],
    pos: usize,
    current_path: String,
    out: &mut Vec<String>,
    regex_cache: &HashMap<String, Result<Regex, String>>,
) {
    if pos >= segments.len() {
        out.push(current_path);
        return;
    }

    let segment = segments[pos].as_str();

    // Schema-structural: skip single
    if SKIP_SINGLE.contains(&segment) {
        collect_data_paths(data, segments, pos + 1, current_path, out, regex_cache);
        return;
    }

    // Schema-structural: skip pair
    if SKIP_PAIR.contains(&segment) {
        if segment == "patternProperties" {
            if let Some(obj) = data.as_object() {
                if let Some(pattern_segment) = segments.get(pos + 1) {
                    if let Some(Ok(re)) = regex_cache.get(pattern_segment.as_str()) {
                        for (key, val) in obj {
                            if re.is_match(key) {
                                let child_path =
                                    format!("{}/{}", current_path, escape_pointer_segment(key));
                                collect_data_paths(
                                    val,
                                    segments,
                                    pos + 2,
                                    child_path,
                                    out,
                                    regex_cache,
                                );
                            }
                        }
                    }
                }
            }
            return;
        }

        if pos + 1 >= segments.len() {
            return;
        }
        collect_data_paths(data, segments, pos + 2, current_path, out, regex_cache);
        return;
    }

    // items → iterate array
    if segment == "items" {
        if let Some(arr) = data.as_array() {
            for (i, item) in arr.iter().enumerate() {
                let child_path = format!("{}/{}", current_path, i);
                collect_data_paths(item, segments, pos + 1, child_path, out, regex_cache);
            }
        }
        return;
    }

    // Numeric index
    if let Ok(index) = segment.parse::<usize>() {
        if let Some(arr) = data.as_array() {
            if let Some(item) = arr.get(index) {
                let child_path = format!("{}/{}", current_path, index);
                collect_data_paths(item, segments, pos + 1, child_path, out, regex_cache);
            }
        }
        return;
    }

    // properties → navigate into object
    if segment == "properties" {
        if let Some(key) = segments.get(pos + 1) {
            if let Some(obj) = data.as_object() {
                if let Some(child) = obj.get(key.as_str()) {
                    let child_path = format!("{}/{}", current_path, escape_pointer_segment(key));
                    collect_data_paths(child, segments, pos + 2, child_path, out, regex_cache);
                }
            }
        }
    }
}

/// Navigate to a mutable value using a JSON Pointer data path.
fn navigate_to_mut<'a>(data: &'a mut Value, pointer: &str) -> Option<&'a mut Value> {
    if pointer.is_empty() {
        return Some(data);
    }
    // JSON pointer segments are separated by '/'
    let segments: Vec<&str> = pointer.split('/').filter(|s| !s.is_empty()).collect();
    let mut current = data;
    for seg in segments {
        // RFC 6901: unescape ~1 -> /, ~0 -> ~
        let unescaped = seg.replace("~1", "/").replace("~0", "~");
        if current.is_array() {
            if let Ok(idx) = seg.parse::<usize>() {
                current = current.as_array_mut()?.get_mut(idx)?;
            } else {
                return None; // non-numeric segment on array
            }
        } else {
            current = current.as_object_mut()?.get_mut(&unescaped)?;
        }
    }
    Some(current)
}

/// Enforce a single constraint on a value, modifying it in-place if needed.
///
/// Returns `Some(message)` if the value was modified, `None` if no action needed.
fn enforce_single_constraint(
    value: &mut Value,
    constraint: &str,
    expected: &Value,
) -> Option<String> {
    match constraint {
        "maximum" => {
            let bound_f = expected.as_f64()?;
            let actual_f = value.as_f64()?;
            if actual_f > bound_f {
                let msg = format!("value {} exceeded maximum {}; clamped", actual_f, bound_f);
                // Preserve integer type if the bound is integer
                if let Some(bound_i) = expected.as_i64() {
                    *value = Value::Number(serde_json::Number::from(bound_i));
                } else {
                    *value = json!(bound_f);
                }
                return Some(msg);
            }
            None
        }
        "minimum" => {
            let bound_f = expected.as_f64()?;
            let actual_f = value.as_f64()?;
            if actual_f < bound_f {
                let msg = format!("value {} below minimum {}; clamped", actual_f, bound_f);
                if let Some(bound_i) = expected.as_i64() {
                    *value = Value::Number(serde_json::Number::from(bound_i));
                } else {
                    *value = json!(bound_f);
                }
                return Some(msg);
            }
            None
        }
        "exclusiveMaximum" => {
            let bound_f = expected.as_f64()?;
            let actual_f = value.as_f64()?;
            if actual_f >= bound_f {
                // Clamp to bound - 1 for integers, bound - epsilon for floats
                let msg = format!(
                    "value {} not less than exclusive maximum {}; clamped",
                    actual_f, bound_f
                );
                if let Some(bound_i) = expected.as_i64() {
                    if let Some(clamped) = bound_i.checked_sub(1) {
                        *value = Value::Number(serde_json::Number::from(clamped));
                    } else {
                        *value = json!(bound_f - f64::EPSILON);
                    }
                } else {
                    *value = json!(bound_f - f64::EPSILON);
                }
                return Some(msg);
            }
            None
        }
        "exclusiveMinimum" => {
            let bound_f = expected.as_f64()?;
            let actual_f = value.as_f64()?;
            if actual_f <= bound_f {
                let msg = format!(
                    "value {} not greater than exclusive minimum {}; clamped",
                    actual_f, bound_f
                );
                if let Some(bound_i) = expected.as_i64() {
                    if let Some(clamped) = bound_i.checked_add(1) {
                        *value = Value::Number(serde_json::Number::from(clamped));
                    } else {
                        *value = json!(bound_f + f64::EPSILON);
                    }
                } else {
                    *value = json!(bound_f + f64::EPSILON);
                }
                return Some(msg);
            }
            None
        }
        "maxLength" => {
            let bound = expected.as_u64()? as usize;
            if let Some(s) = value.as_str() {
                let char_count = s.chars().count();
                if char_count > bound {
                    let msg = format!(
                        "string length {} exceeded maxLength {}; truncated",
                        char_count, bound
                    );
                    let truncated: String = s.chars().take(bound).collect();
                    *value = Value::String(truncated);
                    return Some(msg);
                }
            }
            None
        }
        "maxItems" => {
            if let Some(arr) = value.as_array() {
                let bound = expected.as_u64()? as usize;
                if arr.len() > bound {
                    let msg = format!(
                        "array length {} exceeded maxItems {}; truncated",
                        arr.len(),
                        bound
                    );
                    let truncated: Vec<Value> = arr.iter().take(bound).cloned().collect();
                    *value = Value::Array(truncated);
                    return Some(msg);
                }
            }
            None
        }
        _ => None,
    }
}

/// Validate the codec version against the expected major version.
///
/// The `$schema` URI is expected to end with `/v{major}` (e.g.
/// `https://jsonschema-llm.dev/codec/v1`). Hard-fails on incompatible
/// major version or malformed URI.
fn validate_codec_version(codec: &Codec) -> Result<(), ConvertError> {
    let uri = &codec.schema;

    // Extract the last path segment after the final '/'
    let version_segment = uri
        .rsplit('/')
        .next()
        .and_then(|seg| seg.strip_prefix('v'))
        .ok_or_else(|| ConvertError::CodecVersionMismatch {
            found: uri.clone(),
            expected: format!("URI ending with /v{}", CODEC_MAJOR_VERSION),
        })?;

    // Parse the major version (tolerates "1", "1.2", etc.)
    let major_str = version_segment.split('.').next().unwrap();
    let major: u32 = major_str
        .parse()
        .map_err(|_| ConvertError::CodecVersionMismatch {
            found: uri.clone(),
            expected: format!("URI ending with /v{}", CODEC_MAJOR_VERSION),
        })?;

    if major != CODEC_MAJOR_VERSION {
        return Err(ConvertError::CodecVersionMismatch {
            found: uri.clone(),
            expected: format!("URI ending with /v{}", CODEC_MAJOR_VERSION),
        });
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
        // Enforcement clamps the value and uses "below minimum" message
        assert!(result.warnings[0].message.contains("below minimum"));
        assert!(result.warnings[0].message.contains("clamped"));
        // Data should be clamped to the minimum
        assert_eq!(result.data["age"], json!(18));
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
        // Enforcement truncates and uses "exceeded maxLength" message
        assert!(result.warnings[0].message.contains("exceeded maxLength"));
        assert!(result.warnings[0].message.contains("truncated"));
        // Data should be truncated to 3 chars
        assert_eq!(result.data["name"], json!("too"));
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
        // The message should include the specific regex compilation error
        assert!(result.warnings[0].message.contains("cannot be validated"));
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
        // 5 warnings: exclusiveMinimum (enforced), exclusiveMaximum (enforced),
        // minLength (advisory), minItems (advisory), maxItems (enforced)
        assert_eq!(result.warnings.len(), 5);

        let msgs: Vec<&str> = result.warnings.iter().map(|w| w.message.as_str()).collect();
        // Enforcement messages for enforceable constraints
        assert!(msgs
            .iter()
            .any(|m| m.contains("not greater than exclusive minimum") && m.contains("clamped")));
        assert!(msgs
            .iter()
            .any(|m| m.contains("not less than exclusive maximum") && m.contains("clamped")));
        // Advisory messages for non-enforceable constraints
        assert!(msgs.iter().any(|m| m.contains("less than minLength")));
        assert!(msgs.iter().any(|m| m.contains("less than minItems")));
        // Enforcement message for maxItems
        assert!(msgs
            .iter()
            .any(|m| m.contains("exceeded maxItems") && m.contains("truncated")));

        // Verify data was enforced
        assert_eq!(result.data["ex_min"], json!(11)); // clamped to exclusive min + 1
        assert_eq!(result.data["ex_max"], json!(19)); // clamped to exclusive max - 1
        assert_eq!(result.data["max_items"], json!([1, 2])); // truncated to 2
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
