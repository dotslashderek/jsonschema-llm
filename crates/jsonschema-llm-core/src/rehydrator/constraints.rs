//! Constraint validation and enforcement — checks and fixes dropped constraints.
//!
//! Contains both advisory validation (read-only warning generation) and
//! active enforcement (in-place data modification for enforceable constraints).
//! Also houses `locate_data_nodes` and `collect_data_paths` for schema-path
//! → data-node resolution, since they are exclusively used by constraint logic.

use std::collections::HashMap;

use regex::Regex;
use serde_json::{json, Value};

use super::{SKIP_PAIR, SKIP_SINGLE};
use crate::codec::Codec;
use crate::codec_warning::{Warning, WarningKind};
use crate::schema_utils::{escape_pointer_segment, split_path};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Advisory-only constraints where we just warn that they were dropped.
const ADVISORY_CONSTRAINTS: &[&str] = &["if", "then", "else"];

/// Constraints that can be automatically enforced by modifying data.
const ENFORCEABLE_CONSTRAINTS: &[&str] = &[
    "maximum",
    "minimum",
    "exclusiveMaximum",
    "exclusiveMinimum",
    "maxLength",
    "maxItems",
];

// ---------------------------------------------------------------------------
// Validation (read-only)
// ---------------------------------------------------------------------------

/// Validate dropped constraints against the rehydrated data.
///
/// Uses the pre-compiled regex cache for pattern matching. Walks each
/// constraint path to locate data nodes and check violations.
pub(crate) fn validate_constraints(
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
        let segments = split_path(&dc.path);
        let ctx = TraversalContext {
            segments: &segments,
            schema_path: &dc.path,
            regex_cache,
        };
        let mut nodes = Vec::new();
        locate_data_nodes(data, 0, String::new(), &mut nodes, &mut warnings, &ctx);

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

// ---------------------------------------------------------------------------
// Enforcement (mutable)
// ---------------------------------------------------------------------------

/// Enforce dropped constraints by modifying data in-place.
///
/// For constraints where we can safely fix violations without data loss:
/// - `maximum` / `minimum`: clamp numeric values
/// - `exclusiveMaximum` / `exclusiveMinimum`: clamp to boundary ± 1
/// - `maxLength`: truncate strings
/// - `maxItems`: truncate arrays
///
/// Returns warnings for each enforcement action taken.
pub(crate) fn enforce_constraints(
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

// ---------------------------------------------------------------------------
// Data node resolution
// ---------------------------------------------------------------------------

/// Bundles the immutable "environment" parameters threaded through every
/// `locate_data_nodes` recursive call.
///
/// Separating these from the mutable `warnings` and `out` parameters avoids
/// borrow-checker conflicts and keeps the function signature lean.
struct TraversalContext<'c> {
    segments: &'c [String],
    schema_path: &'c str,
    regex_cache: &'c HashMap<String, Result<Regex, String>>,
}

/// Recursively locate data nodes matching a schema path (read-only).
///
/// Collects `(data_path, &Value)` tuples for each data node the schema path resolves to.
/// When `warnings` is provided, path-resolution issues (e.g. invalid regex) are surfaced
/// as `ConstraintUnevaluable` warnings rather than silently skipped.
fn locate_data_nodes<'d>(
    data: &'d Value,
    pos: usize,
    current_data_path: String,
    out: &mut Vec<(String, &'d Value)>,
    warnings: &mut Vec<Warning>,
    ctx: &TraversalContext<'_>,
) {
    if pos >= ctx.segments.len() {
        out.push((current_data_path, data));
        return;
    }

    let segment = ctx.segments[pos].as_str();

    // Schema-structural: skip single
    if SKIP_SINGLE.contains(&segment) {
        locate_data_nodes(data, pos + 1, current_data_path, out, warnings, ctx);
        return;
    }

    // Schema-structural: skip pair
    if SKIP_PAIR.contains(&segment) {
        if segment == "patternProperties" {
            if let Some(obj) = data.as_object() {
                // Extract pattern from the next segment; bail with warning if missing
                let Some(pattern_segment) = ctx.segments.get(pos + 1) else {
                    tracing::warn!(
                        "missing regex segment after patternProperties in schema path, skipping"
                    );
                    warnings.push(Warning {
                        data_path: if current_data_path.is_empty() {
                            "/".to_string()
                        } else {
                            current_data_path.clone()
                        },
                        schema_path: ctx.schema_path.to_string(),
                        kind: WarningKind::ConstraintUnevaluable {
                            constraint: "patternProperties".to_string(),
                        },
                        message: "missing regex segment after patternProperties in schema path; constraint cannot be evaluated".to_string(),
                    });
                    return;
                };
                let pattern = pattern_segment.as_str();

                match ctx.regex_cache.get(pattern) {
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
                                    pos + 2, // patternProperties + pattern consumed
                                    child_path,
                                    out,
                                    warnings,
                                    ctx,
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
                            schema_path: ctx.schema_path.to_string(),
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
                            schema_path: ctx.schema_path.to_string(),
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

        if pos + 1 >= ctx.segments.len() {
            // Missing pair segment -> stop traversal to avoid incorrect targeting
            return;
        }

        locate_data_nodes(data, pos + 2, current_data_path, out, warnings, ctx);
        return;
    }

    // items → iterate array
    if segment == "items" {
        if let Some(arr) = data.as_array() {
            for (i, item) in arr.iter().enumerate() {
                let child_path = format!("{}/{}", current_data_path, i);
                locate_data_nodes(item, pos + 1, child_path, out, warnings, ctx);
            }
        }
        return;
    }

    // Numeric index
    if let Ok(index) = segment.parse::<usize>() {
        if let Some(arr) = data.as_array() {
            if let Some(item) = arr.get(index) {
                let child_path = format!("{}/{}", current_data_path, index);
                locate_data_nodes(item, pos + 1, child_path, out, warnings, ctx);
            }
        }
        return;
    }

    // properties → navigate into object
    if segment == "properties" {
        if let Some(key) = ctx.segments.get(pos + 1) {
            if let Some(obj) = data.as_object() {
                if let Some(child) = obj.get(key.as_str()) {
                    let child_path =
                        format!("{}/{}", current_data_path, escape_pointer_segment(key));
                    locate_data_nodes(child, pos + 2, child_path, out, warnings, ctx);
                }
            }
        }
    }
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

// ---------------------------------------------------------------------------
// Navigation helper
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Constraint checking (read-only)
// ---------------------------------------------------------------------------

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
// Constraint enforcement (in-place modification)
// ---------------------------------------------------------------------------

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -----------------------------------------------------------------------
    // navigate_to_mut
    // -----------------------------------------------------------------------

    #[test]
    fn navigate_to_mut_empty_pointer_returns_root() {
        let mut data = json!({"a": 1});
        let result = navigate_to_mut(&mut data, "");
        assert_eq!(result, Some(&mut json!({"a": 1})));
    }

    #[test]
    fn navigate_to_mut_deep_object() {
        let mut data = json!({"a": {"b": {"c": 42}}});
        let result = navigate_to_mut(&mut data, "/a/b/c");
        assert_eq!(result, Some(&mut json!(42)));
    }

    #[test]
    fn navigate_to_mut_array_index() {
        let mut data = json!({"items": [10, 20, 30]});
        let result = navigate_to_mut(&mut data, "/items/1");
        assert_eq!(result, Some(&mut json!(20)));
    }

    #[test]
    fn navigate_to_mut_non_numeric_on_array_returns_none() {
        let mut data = json!([1, 2, 3]);
        let result = navigate_to_mut(&mut data, "/foo");
        assert!(result.is_none());
    }

    #[test]
    fn navigate_to_mut_missing_key_returns_none() {
        let mut data = json!({"a": 1});
        let result = navigate_to_mut(&mut data, "/b");
        assert!(result.is_none());
    }

    #[test]
    fn navigate_to_mut_rfc6901_escaped_key() {
        let mut data = json!({"a/b": 99});
        let result = navigate_to_mut(&mut data, "/a~1b");
        assert_eq!(result, Some(&mut json!(99)));
    }

    // -----------------------------------------------------------------------
    // collect_data_paths (isolated — previously only tested indirectly)
    // -----------------------------------------------------------------------

    #[test]
    fn collect_data_paths_simple_property() {
        let data = json!({"name": "Alice"});
        let segments: Vec<String> = ["properties", "name"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut out = Vec::new();
        let cache = HashMap::new();
        collect_data_paths(&data, &segments, 0, String::new(), &mut out, &cache);
        assert_eq!(out, vec!["/name"]);
    }

    #[test]
    fn collect_data_paths_items_iteration() {
        let data = json!({"list": [1, 2, 3]});
        let segments: Vec<String> = ["properties", "list", "items"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut out = Vec::new();
        let cache = HashMap::new();
        collect_data_paths(&data, &segments, 0, String::new(), &mut out, &cache);
        assert_eq!(out, vec!["/list/0", "/list/1", "/list/2"]);
    }

    #[test]
    fn collect_data_paths_skip_pair() {
        // anyOf is a skip-pair keyword, so it should skip anyOf + "0", landing on properties/x
        let data = json!({"x": 5});
        let segments: Vec<String> = ["anyOf", "0", "properties", "x"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut out = Vec::new();
        let cache = HashMap::new();
        collect_data_paths(&data, &segments, 0, String::new(), &mut out, &cache);
        assert_eq!(out, vec!["/x"]);
    }

    #[test]
    fn collect_data_paths_missing_property_yields_empty() {
        let data = json!({"a": 1});
        let segments: Vec<String> = ["properties", "missing"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut out = Vec::new();
        let cache = HashMap::new();
        collect_data_paths(&data, &segments, 0, String::new(), &mut out, &cache);
        assert!(out.is_empty());
    }

    #[test]
    fn collect_data_paths_pattern_properties() {
        let data = json!({"foo_1": "a", "foo_2": "b", "bar": "c"});
        let segments: Vec<String> = ["patternProperties", "^foo_"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut cache = HashMap::new();
        cache.insert("^foo_".to_string(), Ok(Regex::new("^foo_").unwrap()));
        let mut out = Vec::new();
        collect_data_paths(&data, &segments, 0, String::new(), &mut out, &cache);
        out.sort();
        assert_eq!(out, vec!["/foo_1", "/foo_2"]);
    }

    // -----------------------------------------------------------------------
    // enforce_single_constraint
    // -----------------------------------------------------------------------

    #[test]
    fn enforce_max_length_truncates() {
        let mut value = json!("hello world");
        let result = enforce_single_constraint(&mut value, "maxLength", &json!(5));
        assert!(result.is_some());
        assert_eq!(value.as_str().unwrap().chars().count(), 5);
        assert_eq!(value, json!("hello"));
    }

    #[test]
    fn enforce_max_length_within_bounds_no_op() {
        let mut value = json!("hi");
        let result = enforce_single_constraint(&mut value, "maxLength", &json!(10));
        assert!(result.is_none());
        assert_eq!(value, json!("hi"));
    }

    #[test]
    fn enforce_max_items_truncates() {
        let mut value = json!([1, 2, 3, 4, 5]);
        let result = enforce_single_constraint(&mut value, "maxItems", &json!(3));
        assert!(result.is_some());
        assert_eq!(value, json!([1, 2, 3]));
    }

    #[test]
    fn enforce_max_items_within_bounds_no_op() {
        let mut value = json!([1, 2]);
        let result = enforce_single_constraint(&mut value, "maxItems", &json!(5));
        assert!(result.is_none());
        assert_eq!(value, json!([1, 2]));
    }

    #[test]
    fn enforce_maximum_clamps_integer() {
        let mut value = json!(150);
        let result = enforce_single_constraint(&mut value, "maximum", &json!(100));
        assert!(result.is_some());
        assert_eq!(value, json!(100));
    }

    #[test]
    fn enforce_minimum_clamps_integer() {
        let mut value = json!(5);
        let result = enforce_single_constraint(&mut value, "minimum", &json!(10));
        assert!(result.is_some());
        assert_eq!(value, json!(10));
    }

    #[test]
    fn enforce_unknown_constraint_returns_none() {
        let mut value = json!("anything");
        let result = enforce_single_constraint(&mut value, "unknownConstraint", &json!(42));
        assert!(result.is_none());
    }
}
