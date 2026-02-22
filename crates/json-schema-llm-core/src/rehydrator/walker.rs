//! Data walker — schema-path → data navigation for transform application.
//!
//! Traverses data using schema path segments, handling structural keywords
//! (skip single/pair), array iteration, numeric indexing, and property navigation.

use std::collections::HashMap;

use regex::Regex;
use serde_json::Value;

use super::{SKIP_PAIR, SKIP_SINGLE};
use crate::codec::Transform;
use crate::error::ConvertError;

use super::transforms::execute_transform;

/// Recursively walk the data following schema path segments and apply the transform
/// at the terminal node.
pub(super) fn apply_transform(
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Helper: create an empty regex cache.
    fn empty_cache() -> HashMap<String, Result<Regex, String>> {
        HashMap::new()
    }

    // -----------------------------------------------------------------------
    // Skip-pair keywords: dependentSchemas, $defs, definitions
    // -----------------------------------------------------------------------

    #[test]
    fn skip_pair_dependent_schemas() {
        // dependentSchemas/foo should be skipped, landing on the transform target
        let mut data = json!({"name": "Alice"});
        let transform = Transform::NullableOptional {
            path: String::new(),
            original_required: false,
        };
        let path = &["dependentSchemas", "foo", "properties", "name"];
        apply_transform(&mut data, path, &transform, &empty_cache()).unwrap();
        // name was null-optional and null → removed, but "Alice" is non-null → kept
        assert_eq!(data, json!({"name": "Alice"}));
    }

    #[test]
    fn skip_pair_defs() {
        // $defs is a skip-pair keyword — path should skip $defs + name
        let mut data = json!({"x": null});
        let transform = Transform::NullableOptional {
            path: String::new(),
            original_required: false,
        };
        let path = &["$defs", "Thing", "properties", "x"];
        apply_transform(&mut data, path, &transform, &empty_cache()).unwrap();
        // x was null and not originally required → removed
        assert_eq!(data, json!({}));
    }

    // -----------------------------------------------------------------------
    // Unknown segment: forward-compat (silent skip)
    // -----------------------------------------------------------------------

    #[test]
    fn unknown_segment_skips_silently() {
        let mut data = json!({"a": 1});
        let original = data.clone();
        let transform = Transform::NullableOptional {
            path: String::new(),
            original_required: false,
        };
        let path = &["futureKeyword", "properties", "a"];
        // futureKeyword is unknown — should return Ok without modifying data
        let result = apply_transform(&mut data, path, &transform, &empty_cache());
        assert!(result.is_ok());
        assert_eq!(data, original);
    }

    // -----------------------------------------------------------------------
    // RootObjectWrapper error paths
    // -----------------------------------------------------------------------

    #[test]
    fn root_object_wrapper_missing_key_errors() {
        let mut data = json!({"wrong_key": {"a": 1}});
        let transform = Transform::RootObjectWrapper {
            path: String::new(),
            wrapper_key: "expected_key".to_string(),
        };
        let result = apply_transform(&mut data, &[], &transform, &empty_cache());
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("expected_key"));
    }

    #[test]
    fn root_object_wrapper_non_object_errors() {
        let mut data = json!("just a string");
        let transform = Transform::RootObjectWrapper {
            path: String::new(),
            wrapper_key: "wrapper".to_string(),
        };
        let result = apply_transform(&mut data, &[], &transform, &empty_cache());
        assert!(result.is_err());
    }

    #[test]
    fn root_object_wrapper_strips_extra_keys() {
        let mut data = json!({"wrapper": {"inner": 42}, "leaked": true});
        let transform = Transform::RootObjectWrapper {
            path: String::new(),
            wrapper_key: "wrapper".to_string(),
        };
        apply_transform(&mut data, &[], &transform, &empty_cache()).unwrap();
        // Should unwrap to inner value, stripping "leaked"
        assert_eq!(data, json!({"inner": 42}));
    }
}
