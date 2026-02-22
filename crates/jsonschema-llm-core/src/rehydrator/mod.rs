//! Rehydrator — reconstructs original data shape from LLM output using the codec.
//!
//! Uses a DataWalker-style path traversal that handles both data-bearing segments
//! (`properties`, `items`) and schema-structural segments (`anyOf`, `oneOf`, etc.)
//! which are skipped during data navigation.

mod coercion;
mod constraints;
mod transforms;
mod walker;

use std::collections::{HashMap, HashSet};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codec::{Codec, Transform, CODEC_MAJOR_VERSION};
use crate::error::ConvertError;
use crate::schema_utils::split_path;

// Re-export public API items
pub use coercion::coerce_types;
pub(crate) use constraints::{enforce_constraints, validate_constraints};
use walker::apply_transform;

/// Result of rehydration, including the restored data and any warnings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RehydrateResult {
    /// The rehydrated data in the original schema shape.
    pub data: Value,
    /// Warnings about dropped constraint violations.
    pub warnings: Vec<crate::codec_warning::Warning>,
}

/// Schema-structural keywords that should be skipped (keyword only).
pub(super) const SKIP_SINGLE: &[&str] = &[
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
pub(super) const SKIP_PAIR: &[&str] = &[
    "anyOf",
    "oneOf",
    "allOf",
    "$defs",
    "definitions",
    "dependentSchemas",
    "patternProperties",
];

/// Apply codec transforms to rehydrate LLM output back to the original schema shape.
///
/// Applies transforms in REVERSE order (LIFO) to undo the stack of changes
/// recorded during conversion. Does NOT run constraint enforcement or validation;
/// those are orchestrated by the public `rehydrate()` in `lib.rs` after type
/// coercion so that constraints evaluate against correctly-typed values.
pub fn apply_transforms(data: &Value, codec: &Codec) -> Result<RehydrateResult, ConvertError> {
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

    // ── #120: Replay $defs-sourced transforms at RecursiveInflate sites ──
    //
    // The p4 opaque pass records JsonStringParse with $defs-relative paths
    // (e.g. `#/$defs/graph_node/properties/data`). The data walker's SKIP_PAIR
    // logic skips `$defs` + def_name, so these accidentally resolve to the
    // root-level `properties/data` instead of the nested data inside recursive
    // nodes. After RecursiveInflate has expanded JSON strings into objects,
    // replay those JSP transforms at each RI location.
    replay_defs_transforms_at_inflate_sites(&mut result, codec, &regex_cache)?;

    Ok(RehydrateResult {
        data: result,
        warnings: Vec::new(),
    })
}

/// Replay `$defs`-sourced `JsonStringParse` transforms at each `RecursiveInflate` location.
///
/// When the converter records a `JsonStringParse` at `#/$defs/TypeName/properties/field`,
/// the data walker's `SKIP_PAIR` logic for `$defs` causes it to skip `$defs` + `TypeName`
/// and apply the transform at the root level. But the same `field` also exists inside
/// each inline-expanded copy of the recursive type. This function extracts all
/// intermediate recursion boundaries from the RI path and replays the $defs JSP
/// suffix at each one (including the RI terminal itself).
fn replay_defs_transforms_at_inflate_sites(
    data: &mut Value,
    codec: &Codec,
    regex_cache: &HashMap<String, Result<Regex, String>>,
) -> Result<(), ConvertError> {
    // Collect RecursiveInflate paths and their original $ref values
    let inflate_sites: Vec<(&str, &str)> = codec
        .transforms
        .iter()
        .filter_map(|t| match t {
            Transform::RecursiveInflate {
                path, original_ref, ..
            } => Some((path.as_str(), original_ref.as_str())),
            _ => None,
        })
        .collect();

    if inflate_sites.is_empty() {
        return Ok(());
    }

    // Collect $defs-sourced JsonStringParse transforms
    // These have paths like `#/$defs/TypeName/properties/field`
    let defs_jsps: Vec<(&str, String, Vec<String>)> = codec
        .transforms
        .iter()
        .filter_map(|t| match t {
            Transform::JsonStringParse { path } => {
                let segments = split_path(path);
                // Check if path starts with $defs (after # root)
                if segments.len() >= 2 && segments[0] == "$defs" {
                    let def_name = segments[1].clone();
                    // The suffix after $defs/TypeName is the data-relative path
                    let suffix: Vec<String> = segments[2..].to_vec();
                    Some((path.as_str(), def_name, suffix))
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect();

    // Pre-index $defs JSPs by their def name for O(1) lookup per inflate site
    let mut defs_jsps_by_def: HashMap<&str, Vec<(&str, &[String])>> = HashMap::new();
    for (jsp_path, def_name, suffix) in &defs_jsps {
        if !suffix.is_empty() {
            defs_jsps_by_def
                .entry(def_name.as_str())
                .or_default()
                .push((jsp_path, suffix.as_slice()));
        }
    }

    if defs_jsps_by_def.is_empty() {
        return Ok(());
    }

    // Reusable buffer for building concrete paths
    let mut concrete_segments: Vec<String> = Vec::new();
    // Dedup: avoid replaying the same synthetic path across overlapping inflate sites
    let mut seen_paths: HashSet<String> = HashSet::new();

    for (ri_path, original_ref) in &inflate_sites {
        let ri_segments = split_path(ri_path);

        // Extract the def name from original_ref (e.g. "#/$defs/graph_node" → "graph_node")
        let ref_segments = split_path(original_ref);
        let ref_def_name = if ref_segments.len() >= 2 && ref_segments[0] == "$defs" {
            ref_segments[1].as_str()
        } else {
            continue;
        };

        // Look up JSPs for this def type — skip if none
        let matching_jsps = match defs_jsps_by_def.get(ref_def_name) {
            Some(jsps) => jsps,
            None => continue,
        };

        // Find all intermediate "recursion boundaries" in the RI path using
        // shortest-period detection. The RI path is the fully-expanded inline path
        // to the deepest recursive site. We detect the repeating structural unit
        // and emit one prefix per recursion level.
        let replay_prefixes = extract_recursion_prefixes(&ri_segments);

        for (jsp_path, suffix) in matching_jsps {
            for prefix in &replay_prefixes {
                concrete_segments.clear();
                concrete_segments.extend(prefix.iter().cloned());
                concrete_segments.extend(suffix.iter().cloned());

                let synthetic_path = format!("#/{}", concrete_segments.join("/"));

                // Skip if we've already replayed this exact path
                if !seen_paths.insert(synthetic_path.clone()) {
                    continue;
                }

                let seg_refs: Vec<&str> = concrete_segments.iter().map(|s| s.as_str()).collect();
                let synthetic_transform = Transform::JsonStringParse {
                    path: synthetic_path.clone(),
                };

                tracing::debug!(
                    original_jsp = %jsp_path,
                    concrete_path = %synthetic_path,
                    "replaying $defs JSP at recursive expansion site"
                );
                apply_transform(data, &seg_refs, &synthetic_transform, regex_cache)?;
            }
        }
    }

    Ok(())
}

/// Extract all intermediate recursion boundary prefixes from a RecursiveInflate path.
///
/// Given a path like `a/b/c/a/b/c/a/b/c` (where `a/b/c` is the repeating recursion unit),
/// returns `[a/b/c, a/b/c/a/b/c, a/b/c/a/b/c/a/b/c]` — one prefix for each recursion level.
///
/// Uses shortest-period detection to find the repeating unit, avoiding false positives
/// when the terminal segment name appears non-recursively earlier in the path.
fn extract_recursion_prefixes(ri_segments: &[String]) -> Vec<Vec<String>> {
    let n = ri_segments.len();
    if n == 0 {
        return vec![];
    }

    // Detect the shortest period that composes the entire path. For example, for
    // ["a","b","c","a","b","c","a","b","c"] the period is ["a","b","c"].
    let mut period_len = n;
    'outer: for candidate in 1..=n {
        #[allow(unknown_lints)]
        #[allow(clippy::manual_is_multiple_of)]
        if n % candidate != 0 {
            continue;
        }
        for i in 0..n {
            if ri_segments[i] != ri_segments[i % candidate] {
                continue 'outer;
            }
        }
        period_len = candidate;
        break;
    }

    let levels = n / period_len;
    let mut prefixes = Vec::with_capacity(levels);
    for level in 1..=levels {
        let end = level * period_len;
        prefixes.push(ri_segments[..end].to_vec());
    }

    prefixes
}

/// Pre-scan transform and constraint paths for patternProperties segments
/// and compile their regex patterns into a reusable cache.
pub(crate) fn build_pattern_properties_cache(
    codec: &Codec,
) -> HashMap<String, Result<Regex, String>> {
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
    let major_str = version_segment.split('.').next().expect(
        "invariant: str::split('.') always yields at least one element for a non-empty string",
    );
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
    use crate::codec::CODEC_SCHEMA_URI;
    use crate::codec_warning::WarningKind;
    use serde_json::json;

    /// Helper: run the full rehydration pipeline (transforms + constraints)
    /// for unit tests that need constraint behavior. Mirror of `lib.rs::rehydrate()`
    /// without the coercion step (unit tests don't use an original schema).
    fn apply_transforms_with_constraints(
        data: &Value,
        codec: &Codec,
    ) -> Result<RehydrateResult, ConvertError> {
        let mut result = apply_transforms(data, codec)?;
        let regex_cache = build_pattern_properties_cache(codec);
        let enforcement = enforce_constraints(&mut result.data, codec, &regex_cache);
        let validation = validate_constraints(&result.data, codec, &regex_cache);
        result.warnings.extend(enforcement);
        result.warnings.extend(validation);
        Ok(result)
    }

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
        let result = apply_transforms(&data, &codec).unwrap();
        assert_eq!(result.data, json!({"fixed": 1}));

        // Case B: Present value -> should be kept
        let data_present = json!({
            "fixed": 1,
            "optional": "kept"
        });
        let result_present = apply_transforms(&data_present, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec);
        assert!(matches!(result, Err(ConvertError::RehydrationError(_))));
    }

    // Test 5: Combined
    #[test]
    fn test_combined() {
        let mut codec = Codec::new();
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

        let result = apply_transforms(&data, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec).unwrap();
        assert_eq!(result.data["list"][0]["data"], json!({"id": 1}));
        assert_eq!(result.data["list"][1]["data"], json!({"id": 2}));
    }

    // Test 7: Extract Additional Properties
    #[test]
    fn test_extract_ap() {
        let mut codec = Codec::new();
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

        let result = apply_transforms(&data, &codec).unwrap();
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
        let result = apply_transforms(&data, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec).unwrap();
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
        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
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
        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
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
        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
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
        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
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
        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
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
        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
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

        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
        // Only users/1/email should fail
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].data_path, "/users/1/email");
    }

    // Test 25: Empty codec → no warnings
    #[test]
    fn test_no_constraints_no_warnings() {
        let codec = Codec::new();
        let data = json!({"any": "data"});
        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
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
        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
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

        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
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
        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
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
        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
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
        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
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

        let result = apply_transforms_with_constraints(&data, &codec).unwrap();
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

        let result = apply_transforms(&data, &codec).unwrap();

        // The JSON string should be parsed back into an object
        assert_eq!(result.data["value"], 42);
        assert_eq!(result.data["child"]["value"], 99);
        assert!(
            result.data["child"].is_object(),
            "child should be an object after rehydration"
        );
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Type Coercion Tests
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn test_coerce_number_to_string() {
        let schema = json!({"type": "object", "properties": {"name": {"type": "string"}}});
        let mut data = json!({"name": 123});
        let warnings = coerce_types(&mut data, &schema);
        assert_eq!(data["name"], json!("123"));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("coerced number"));
    }

    #[test]
    fn test_coerce_boolean_to_string() {
        let schema = json!({"type": "object", "properties": {"flag": {"type": "string"}}});
        let mut data = json!({"flag": true});
        let warnings = coerce_types(&mut data, &schema);
        assert_eq!(data["flag"], json!("true"));
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn test_coerce_string_to_integer() {
        let schema = json!({"type": "object", "properties": {"age": {"type": "integer"}}});
        let mut data = json!({"age": "42"});
        let warnings = coerce_types(&mut data, &schema);
        assert_eq!(data["age"], json!(42));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("coerced string"));
    }

    #[test]
    fn test_coerce_string_to_number() {
        let schema = json!({"type": "object", "properties": {"score": {"type": "number"}}});
        let mut data = json!({"score": "2.78"});
        let warnings = coerce_types(&mut data, &schema);
        assert!((data["score"].as_f64().unwrap() - 2.78).abs() < f64::EPSILON);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn test_coerce_string_to_boolean() {
        let schema = json!({"type": "object", "properties": {"active": {"type": "boolean"}}});
        let mut data = json!({"active": "false"});
        let warnings = coerce_types(&mut data, &schema);
        assert_eq!(data["active"], json!(false));
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn test_coerce_roundtrip_rejects_non_numeric() {
        let schema = json!({"type": "object", "properties": {"count": {"type": "integer"}}});
        let mut data = json!({"count": "hello"});
        let warnings = coerce_types(&mut data, &schema);
        // Should NOT coerce — "hello" is not parseable as integer
        assert_eq!(data["count"], json!("hello"));
        assert_eq!(warnings.len(), 0);
    }

    #[test]
    fn test_coerce_nested_objects() {
        let schema = json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "age": {"type": "integer"}
                    }
                }
            }
        });
        let mut data = json!({"user": {"id": 456, "age": "30"}});
        let warnings = coerce_types(&mut data, &schema);
        assert_eq!(data["user"]["id"], json!("456"));
        assert_eq!(data["user"]["age"], json!(30));
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn test_coerce_array_items() {
        let schema = json!({
            "type": "array",
            "items": {"type": "string"}
        });
        let mut data = json!([1, "two", 3, true]);
        let warnings = coerce_types(&mut data, &schema);
        assert_eq!(data, json!(["1", "two", "3", "true"]));
        assert_eq!(warnings.len(), 3); // 1, 3, true coerced
    }

    #[test]
    fn test_coerce_null_passthrough() {
        let schema = json!({"type": ["string", "null"]});
        let mut data = json!(null);
        let warnings = coerce_types(&mut data, &schema);
        assert_eq!(data, json!(null));
        assert_eq!(warnings.len(), 0);
    }

    #[test]
    fn test_coerce_no_op_when_types_match() {
        let schema = json!({"type": "object", "properties": {"name": {"type": "string"}}});
        let mut data = json!({"name": "Alice"});
        let warnings = coerce_types(&mut data, &schema);
        assert_eq!(data["name"], json!("Alice"));
        assert_eq!(warnings.len(), 0);
    }

    #[test]
    fn test_coerce_integer_satisfies_number() {
        // An integer value should satisfy a "number" type without coercion
        let schema = json!({"type": "object", "properties": {"val": {"type": "number"}}});
        let mut data = json!({"val": 42});
        let warnings = coerce_types(&mut data, &schema);
        assert_eq!(data["val"], json!(42));
        assert_eq!(warnings.len(), 0);
    }

    #[test]
    fn test_coerce_prefix_items_tuple() {
        let schema = json!({
            "type": "array",
            "prefixItems": [
                {"type": "integer"},
                {"type": "boolean"},
                {"type": "string"}
            ],
            "items": {"type": "string"}
        });
        let mut data = json!(["42", "true", "hello", 99]);
        let warnings = coerce_types(&mut data, &schema);
        // prefixItems[0] → integer: "42" → 42
        assert_eq!(data[0], json!(42));
        // prefixItems[1] → boolean: "true" → true
        assert_eq!(data[1], json!(true));
        // prefixItems[2] → string: "hello" stays "hello"
        assert_eq!(data[2], json!("hello"));
        // items (fallback) → string: 99 → "99"
        assert_eq!(data[3], json!("99"));
        assert_eq!(warnings.len(), 3); // 3 coercions
    }

    // ── #120: JsonStringParse inside RecursiveInflate ──────────────

    #[test]
    fn test_recursive_inflate_then_json_string_parse() {
        let inner_data = json!({"key": "value"});
        let inner_node = json!({
            "id": "child",
            "data": serde_json::to_string(&inner_data).unwrap(),
            "edges": []
        });

        let data = json!({
            "id": "root",
            "data": serde_json::to_string(&inner_data).unwrap(),
            "edges": [
                {
                    "target": serde_json::to_string(&inner_node).unwrap(),
                    "weight": 1.0
                }
            ]
        });

        let codec = Codec {
            schema: CODEC_SCHEMA_URI.to_string(),
            transforms: vec![
                Transform::JsonStringParse {
                    path: "#/properties/data".to_string(),
                },
                Transform::JsonStringParse {
                    path: "#/$defs/graph_node/properties/data".to_string(),
                },
                Transform::RecursiveInflate {
                    path: "#/properties/edges/items/properties/target".to_string(),
                    original_ref: "#/$defs/graph_node".to_string(),
                },
            ],
            dropped_constraints: vec![],
        };

        let result = apply_transforms(&data, &codec).expect("apply_transforms should succeed");

        // Root-level data should be parsed
        assert_eq!(
            result.data["data"],
            json!({"key": "value"}),
            "root data should be parsed from JSON string"
        );

        // Target should be inflated from JSON string to object
        assert!(
            result.data["edges"][0]["target"].is_object(),
            "target should be inflated to an object, got: {:?}",
            result.data["edges"][0]["target"]
        );

        // The nested data inside target should also be parsed from JSON string
        assert_eq!(
            result.data["edges"][0]["target"]["data"],
            json!({"key": "value"}),
            "nested data inside inflated target should be parsed from JSON string"
        );
    }
}
