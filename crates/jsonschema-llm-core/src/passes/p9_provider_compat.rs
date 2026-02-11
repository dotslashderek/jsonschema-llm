//! Pass 9 — Provider compatibility transforms for OpenAI Strict Mode.
//!
//! Runs **after** all other passes (the schema is already normalized, refs resolved,
//! strict-sealed, etc.) and both transforms and emits advisory `ProviderCompatError`s
//! for constructs that would be rejected by the target provider.
//!
//! Active only when `target == OpenaiStrict && mode == Strict`.
//!
//! ## Checks
//!
//! | Issue | Check                  | Kind       |
//! | ----- | ---------------------- | ---------- |
//! | #94   | Root type enforcement  | Transform  |
//! | #95   | Depth budget           | Diagnostic |
//! | #96   | Enum homogeneity       | Transform  |
//! | #97   | Boolean / empty schema | Transform  |

use crate::codec::Transform;
use crate::config::{ConvertOptions, Mode, Target};
use crate::error::ProviderCompatError;
use crate::schema_utils::build_path;
use serde_json::{json, Value};

/// OpenAI Strict Mode maximum nesting depth.
const OPENAI_MAX_DEPTH: usize = 5;

/// Hard guard against infinite recursion in traversal.
const HARD_RECURSION_LIMIT: usize = 100;

/// Result of provider compatibility checks.
pub struct ProviderCompatResult {
    /// The (possibly modified) schema — root may have been wrapped.
    pub schema: Value,
    /// New transforms produced (e.g. `RootObjectWrapper`).
    pub transforms: Vec<Transform>,
    /// Advisory errors for provider-incompatible constructs.
    pub errors: Vec<ProviderCompatError>,
}

/// Run all provider compatibility checks on the post-pipeline schema.
///
/// Returns the (potentially wrapped) schema, any new transforms, and
/// advisory errors.
pub fn check_provider_compat(schema: &Value, config: &ConvertOptions) -> ProviderCompatResult {
    match config.target {
        Target::OpenaiStrict if config.mode == Mode::Strict => {
            let mut errors = Vec::new();
            let mut transforms = Vec::new();

            // ── Check 1: Root type enforcement (#94) ──────────────────
            let mut schema = check_root_type(schema, config.target, &mut errors, &mut transforms);

            // ── Checks 2–4: Single-pass mutating visitor (#95, #96, #97)
            let max_depth_observed = {
                let mut visitor = CompatVisitor {
                    errors: &mut errors,
                    transforms: &mut transforms,
                    target: config.target,
                    max_depth_observed: 0,
                };
                visitor.visit(&mut schema, "#", 0, 0);
                visitor.max_depth_observed
            };

            // Emit a single aggregated DepthBudgetExceeded if needed
            if max_depth_observed > OPENAI_MAX_DEPTH {
                errors.push(ProviderCompatError::DepthBudgetExceeded {
                    actual_depth: max_depth_observed,
                    max_depth: OPENAI_MAX_DEPTH,
                    target: config.target,
                    hint: format!(
                        "Schema nesting depth {} exceeds OpenAI Strict Mode limit of {}.",
                        max_depth_observed, OPENAI_MAX_DEPTH,
                    ),
                });
            }

            ProviderCompatResult {
                schema,
                transforms,
                errors,
            }
        }
        _ => ProviderCompatResult {
            schema: schema.clone(),
            transforms: vec![],
            errors: vec![],
        },
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Check 1: Root type enforcement (#94)
// ═══════════════════════════════════════════════════════════════════════════

/// Wraps non-object roots in `{ type: object, properties: { result: <original> }, ... }`.
fn check_root_type(
    schema: &Value,
    target: Target,
    errors: &mut Vec<ProviderCompatError>,
    transforms: &mut Vec<Transform>,
) -> Value {
    let root_type = schema.get("type").and_then(|v| v.as_str()).unwrap_or("");

    if root_type == "object" {
        return schema.clone();
    }

    let actual_type = if root_type.is_empty() {
        "unspecified".to_string()
    } else {
        root_type.to_string()
    };

    errors.push(ProviderCompatError::RootTypeIncompatible {
        actual_type: actual_type.clone(),
        target,
        hint: format!(
            "Schema root type '{}' is not 'object'. Wrapping in {{ \"result\": <original> }}.",
            actual_type,
        ),
    });

    transforms.push(Transform::RootObjectWrapper {
        path: "#".to_string(),
        wrapper_key: "result".to_string(),
    });

    // Build the wrapper schema
    json!({
        "type": "object",
        "properties": {
            "result": schema,
        },
        "required": ["result"],
        "additionalProperties": false,
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// Checks 2–4: Single-pass CompatVisitor
//
// NOTE: When adding keywords here, also update `schema_utils::recurse_into_children`.
// Conversely, when adding keywords to `recurse_into_children`, update this visitor.
// ═══════════════════════════════════════════════════════════════════════════

struct CompatVisitor<'a> {
    errors: &'a mut Vec<ProviderCompatError>,
    transforms: &'a mut Vec<Transform>,
    target: Target,
    max_depth_observed: usize,
}

impl CompatVisitor<'_> {
    /// Recursively visit and **mutate** a schema node.
    ///
    /// Uses two depth counters:
    /// - `recursion_depth`: **always** increments — guards against stack overflow
    ///   via `HARD_RECURSION_LIMIT`.
    /// - `semantic_depth`: increments only on data-shape edges (`properties`, `items`,
    ///   `additionalProperties`, etc.) — used for `OPENAI_MAX_DEPTH` budget.
    ///
    /// - #95 Depth budget: diagnostic only (tracks max semantic depth)
    /// - #96 Enum homogeneity: **transforms** mixed enums → all strings + codec entry
    /// - #97 Boolean/empty schemas: **transforms** → opaque string or sealed empty object
    fn visit(
        &mut self,
        schema: &mut Value,
        path: &str,
        recursion_depth: usize,
        semantic_depth: usize,
    ) {
        // Hard recursion guard (always uses recursion_depth)
        if recursion_depth > HARD_RECURSION_LIMIT {
            return;
        }

        // ── #97: Boolean schema transform ──────────────────────────
        if let Some(b) = schema.as_bool() {
            self.errors.push(ProviderCompatError::UnconstrainedSchema {
                path: path.to_string(),
                schema_kind: format!("boolean({})", b),
                target: self.target,
                hint: format!("Boolean schema '{}' replaced with opaque string.", b),
            });

            *schema = json!({
                "type": "string",
                "description": "A JSON-encoded string representing the object. Parse with JSON.parse() after generation."
            });
            self.transforms.push(Transform::JsonStringParse {
                path: path.to_string(),
            });
            return;
        }

        match schema.as_object() {
            Some(_) => {}
            None => return,
        };

        // ── #95 Depth budget (diagnostic, uses semantic_depth) ─────
        if semantic_depth > self.max_depth_observed {
            self.max_depth_observed = semantic_depth;
        }

        // ── #96 Enum homogeneity (transform) ──────────────────────
        fix_enum_homogeneity(schema, path, self.target, self.errors, self.transforms);

        // ── #97 Unconstrained sub-schemas (transform) ─────────────
        if path != "#" {
            if let Some(obj) = schema.as_object() {
                if is_unconstrained(obj) {
                    self.errors.push(ProviderCompatError::UnconstrainedSchema {
                        path: path.to_string(),
                        schema_kind: "empty".to_string(),
                        target: self.target,
                        hint: "Unconstrained schema replaced with opaque string.".into(),
                    });

                    *schema = json!({
                        "type": "string",
                        "description": "A JSON-encoded string representing the object. Parse with JSON.parse() after generation."
                    });
                    self.transforms.push(Transform::JsonStringParse {
                        path: path.to_string(),
                    });
                    return;
                }
            }
        }

        // ── Recurse into children ──────────────────────────────────
        // Data-shape keywords increment semantic_depth.
        // Non-data-shape keywords (combinators, conditionals, defs) do not.
        // All keywords always increment recursion_depth.

        let rd = recursion_depth + 1; // next recursion depth (always +1)
        let sd_data = semantic_depth + 1; // next semantic depth for data-shape edges
        let sd_same = semantic_depth; // unchanged semantic depth for non-data edges

        // ── Data-shape: map-of-schemas ─────────────────────────────
        // properties, patternProperties
        for keyword in &["properties", "patternProperties"] {
            let keys: Vec<String> = schema
                .get(*keyword)
                .and_then(|v| v.as_object())
                .map(|props| props.keys().cloned().collect())
                .unwrap_or_default();
            for key in &keys {
                let child_path = build_path(path, &[keyword, key]);
                if let Some(child) = schema.get_mut(*keyword).and_then(|p| p.get_mut(key)) {
                    self.visit(child, &child_path, rd, sd_data);
                }
            }
        }

        // ── Data-shape: single-schema ──────────────────────────────
        // additionalProperties, unevaluatedProperties, unevaluatedItems, contains
        // Note: only recurse if an object. Boolean values (e.g. `additionalProperties: false`)
        // are intentional constraints, not unconstrained sub-schemas.
        for keyword in &[
            "additionalProperties",
            "unevaluatedProperties",
            "unevaluatedItems",
            "contains",
        ] {
            let is_obj = schema.get(*keyword).map(|v| v.is_object()).unwrap_or(false);
            if is_obj {
                let child_path = build_path(path, &[keyword]);
                if let Some(child) = schema.get_mut(*keyword) {
                    self.visit(child, &child_path, rd, sd_data);
                }
            }
        }

        // ── Data-shape: items (single-schema, tuple array, or boolean) ──
        // Handle `items: {schema}`, `items: [{schema}, ...]`, and `items: true|false`
        // (cf. schema_utils::recurse_into_children for the canonical list)
        {
            let items_kind = schema.get("items").map(|v| {
                if v.is_object() {
                    1u8 // single schema
                } else if v.is_array() {
                    2 // tuple array
                } else if v.is_boolean() {
                    3 // boolean schema (true = unconstrained, false = deny)
                } else {
                    0
                }
            });
            match items_kind {
                Some(1) | Some(3) => {
                    let child_path = build_path(path, &["items"]);
                    if let Some(child) = schema.get_mut("items") {
                        self.visit(child, &child_path, rd, sd_data);
                    }
                }
                Some(2) => {
                    let count = schema
                        .get("items")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    for i in 0..count {
                        let child_path = build_path(path, &["items", &i.to_string()]);
                        if let Some(child) = schema.get_mut("items").and_then(|p| p.get_mut(i)) {
                            self.visit(child, &child_path, rd, sd_data);
                        }
                    }
                }
                _ => {}
            }
        }

        // ── Data-shape: array-of-schemas ───────────────────────────
        // prefixItems
        {
            let count = schema
                .get("prefixItems")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            for i in 0..count {
                let child_path = build_path(path, &["prefixItems", &i.to_string()]);
                if let Some(child) = schema.get_mut("prefixItems").and_then(|p| p.get_mut(i)) {
                    self.visit(child, &child_path, rd, sd_data);
                }
            }
        }

        // ── Non-data-shape: array-of-schemas (combinators) ────────
        // anyOf, oneOf, allOf
        for keyword in &["anyOf", "oneOf", "allOf"] {
            let count = schema
                .get(*keyword)
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            for i in 0..count {
                let child_path = build_path(path, &[keyword, &i.to_string()]);
                if let Some(child) = schema.get_mut(*keyword).and_then(|v| v.get_mut(i)) {
                    self.visit(child, &child_path, rd, sd_same);
                }
            }
        }

        // ── Non-data-shape: single-schema (conditionals, negation) ─
        // if, then, else, not, propertyNames
        for keyword in &["if", "then", "else", "not", "propertyNames"] {
            let has = schema
                .get(*keyword)
                .map(|v| v.is_object() || v.is_boolean())
                .unwrap_or(false);
            if has {
                let child_path = build_path(path, &[keyword]);
                if let Some(child) = schema.get_mut(*keyword) {
                    self.visit(child, &child_path, rd, sd_same);
                }
            }
        }

        // ── Non-data-shape: map-of-schemas ────────────────────────
        // $defs, definitions, dependentSchemas
        for keyword in &["$defs", "definitions", "dependentSchemas"] {
            if let Some(defs) = schema.get(*keyword).and_then(|v| v.as_object()) {
                let keys: Vec<String> = defs.keys().cloned().collect();
                let _ = defs;
                for key in &keys {
                    let child_path = build_path(path, &[keyword, key]);
                    if let Some(child) = schema.get_mut(*keyword).and_then(|v| v.get_mut(key)) {
                        self.visit(child, &child_path, rd, sd_same);
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Transform helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Detect mixed-type enums and stringify all values in-place.
///
/// Produces an `EnumStringify` codec entry so the rehydrator can reverse it.
fn fix_enum_homogeneity(
    schema: &mut Value,
    path: &str,
    target: Target,
    errors: &mut Vec<ProviderCompatError>,
    transforms: &mut Vec<Transform>,
) {
    let enum_vals = match schema.get("enum").and_then(|v| v.as_array()) {
        Some(vals) if !vals.is_empty() => vals,
        _ => return,
    };

    let mut types = std::collections::BTreeSet::new();
    for v in enum_vals {
        types.insert(json_type_name(v));
    }

    if types.len() <= 1 {
        return; // Homogeneous — nothing to fix
    }

    let types_found: Vec<String> = types.into_iter().map(|s| s.to_string()).collect();
    let original_values: Vec<Value> = enum_vals.clone();

    // Stringify all values and deduplicate (e.g. [1, "1"] → ["1"] not ["1", "1"])
    let mut seen = std::collections::HashSet::new();
    let stringified: Vec<Value> = original_values
        .iter()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .filter(|s| seen.insert(s.clone()))
        .map(Value::String)
        .collect();

    // Replace enum in-place
    if let Some(obj) = schema.as_object_mut() {
        obj.insert("enum".to_string(), Value::Array(stringified));
        // Ensure type is string since all values are now strings
        obj.insert("type".to_string(), json!("string"));
    }

    errors.push(ProviderCompatError::MixedEnumTypes {
        path: path.to_string(),
        types_found,
        target,
        hint: "Mixed-type enum values normalized to strings.".into(),
    });

    transforms.push(Transform::EnumStringify {
        path: path.to_string(),
        original_values,
    });
}

/// Returns the JSON type name for a value.
fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Returns true if a schema object is unconstrained:
///   - it is completely empty, or
///   - it only contains metadata / documentation keywords and the structural
///     keywords injected by `p6_strict` (e.g. `additionalProperties`, `required`).
///
/// Any other keyword is treated as potentially constraining, so the schema will
/// not be considered unconstrained even if we do not explicitly recognise it.
fn is_unconstrained(obj: &serde_json::Map<String, Value>) -> bool {
    // Quick check: truly empty
    if obj.is_empty() {
        return true;
    }

    // Keywords that are purely metadata / documentation and do not constrain
    // the set of acceptable instances.
    const METADATA_KEYWORDS: &[&str] = &[
        "title",
        "description",
        "$schema",
        "$id",
        "$anchor",
        "$comment",
        "examples",
        "default",
        "deprecated",
        "readOnly",
        "writeOnly",
        "contentMediaType",
        "contentEncoding",
        "contentSchema",
    ];

    // If we see any key that is not known-metadata, treat the schema as
    // constrained.
    for key in obj.keys() {
        let k = key.as_str();
        if METADATA_KEYWORDS.contains(&k) {
            continue;
        }
        // Any other keyword (including all typical constraint keywords like
        // `type`, `properties`, `items`, `enum`, `minimum`, `contains`,
        // `minProperties`, `required`, `additionalProperties`, etc.) is treated
        // as constraining.
        return false;
    }

    // Only metadata keywords were present.
    true
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

    // ── Root type ──────────────────────────────────────────────
    #[test]
    fn object_root_unchanged() {
        let schema = json!({"type": "object", "properties": {"x": {"type": "string"}}});
        let r = check_provider_compat(&schema, &opts());
        assert!(r.transforms.is_empty());
        assert!(r
            .errors
            .iter()
            .all(|e| !matches!(e, ProviderCompatError::RootTypeIncompatible { .. })));
    }

    #[test]
    fn array_root_wrapped() {
        let schema = json!({"type": "array", "items": {"type": "string"}});
        let r = check_provider_compat(&schema, &opts());
        assert_eq!(r.transforms.len(), 1);
        assert_eq!(r.schema.get("type").unwrap(), "object");
        assert!(r.schema.pointer("/properties/result/type").unwrap() == "array");
    }

    #[test]
    fn string_root_wrapped() {
        let schema = json!({"type": "string"});
        let r = check_provider_compat(&schema, &opts());
        assert_eq!(r.transforms.len(), 1);
        assert!(r.schema.pointer("/properties/result/type").unwrap() == "string");
    }

    #[test]
    fn missing_type_wrapped() {
        let schema = json!({"description": "no type"});
        let r = check_provider_compat(&schema, &opts());
        // Root wrap + inner unconstrained → opaque string = 2 transforms
        assert_eq!(r.transforms.len(), 2);
        assert_eq!(r.schema.get("type").unwrap(), "object");
        // The inner schema should be an opaque string now
        let result_schema = &r.schema["properties"]["result"];
        assert_eq!(result_schema["type"], "string");
    }

    // ── Depth budget ──────────────────────────────────────────
    #[test]
    fn shallow_no_error() {
        let schema = json!({"type": "object", "properties": {"a": {"type": "string"}}});
        let r = check_provider_compat(&schema, &opts());
        assert!(r
            .errors
            .iter()
            .all(|e| !matches!(e, ProviderCompatError::DepthBudgetExceeded { .. })));
    }

    #[test]
    fn deep_emits_error() {
        // Build 7 levels deep
        let mut inner = json!({"type": "string"});
        for i in (0..7).rev() {
            inner = json!({"type": "object", "properties": {format!("l{i}"): inner}});
        }
        let r = check_provider_compat(&inner, &opts());
        let depth_errs: Vec<_> = r
            .errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::DepthBudgetExceeded { .. }))
            .collect();
        assert!(
            !depth_errs.is_empty(),
            "should have at least one depth error"
        );
    }

    // ── Enum homogeneity ──────────────────────────────────────
    #[test]
    fn homo_enum_clean() {
        let schema = json!({"type": "object", "properties": {"c": {"enum": ["a", "b"]}}});
        let r = check_provider_compat(&schema, &opts());
        assert!(r
            .errors
            .iter()
            .all(|e| !matches!(e, ProviderCompatError::MixedEnumTypes { .. })));
    }

    #[test]
    fn mixed_enum_error() {
        let schema = json!({"type": "object", "properties": {"c": {"enum": ["a", 1]}}});
        let r = check_provider_compat(&schema, &opts());
        let enum_errs: Vec<_> = r
            .errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::MixedEnumTypes { .. }))
            .collect();
        assert_eq!(enum_errs.len(), 1);
    }

    // ── Boolean / empty schemas ───────────────────────────────
    #[test]
    fn typed_no_unconstrained() {
        let schema = json!({"type": "object", "properties": {"x": {"type": "string"}}});
        let r = check_provider_compat(&schema, &opts());
        assert!(r
            .errors
            .iter()
            .all(|e| !matches!(e, ProviderCompatError::UnconstrainedSchema { .. })));
    }

    #[test]
    fn empty_sub_schema_flagged() {
        let schema = json!({"type": "object", "properties": {"x": {}}});
        let r = check_provider_compat(&schema, &opts());
        let uc_errs: Vec<_> = r
            .errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. }))
            .collect();
        assert!(!uc_errs.is_empty());
    }

    // ── Gate: non-OpenAI passthrough ──────────────────────────
    #[test]
    fn gemini_passthrough() {
        let schema = json!({"type": "array"});
        let mut o = opts();
        o.target = Target::Gemini;
        let r = check_provider_compat(&schema, &o);
        assert!(r.errors.is_empty());
        assert!(r.transforms.is_empty());
    }

    // ── Boolean false schema ──────────────────────────────────
    #[test]
    fn false_schema_becomes_opaque_string() {
        let schema = json!({"type": "object", "properties": {"deny": false}});
        let r = check_provider_compat(&schema, &opts());
        // Should flag as unconstrained
        let uc_errs: Vec<_> = r
            .errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. }))
            .collect();
        assert!(
            !uc_errs.is_empty(),
            "false schema should trigger UnconstrainedSchema"
        );
        // Should produce a JsonStringParse transform
        assert!(
            r.transforms
                .iter()
                .any(|t| matches!(t, Transform::JsonStringParse { .. })),
            "false schema should produce JsonStringParse transform"
        );
    }

    // ── Enum collision dedup ──────────────────────────────────
    #[test]
    fn enum_collision_deduplicates() {
        // [1, "1"] should stringify to ["1"] (deduplicated), not ["1", "1"]
        let schema = json!({"type": "object", "properties": {"v": {"enum": [1, "1"]}}});
        let r = check_provider_compat(&schema, &opts());
        let enum_vals = r.schema["properties"]["v"]["enum"]
            .as_array()
            .expect("enum should be an array");
        assert_eq!(
            enum_vals.len(),
            1,
            "colliding values should be deduplicated"
        );
        assert_eq!(enum_vals[0], json!("1"));
    }

    // ── #109 Keyword recursion ────────────────────────────────
    #[test]
    fn visitor_recurses_pattern_properties() {
        // patternProperties values should be visited for mixed enum detection
        let mut schema = json!({
            "type": "object",
            "patternProperties": {
                "^x-": { "enum": ["a", 1] }
            }
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        let enum_errs: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::MixedEnumTypes { .. }))
            .collect();
        assert_eq!(
            enum_errs.len(),
            1,
            "mixed enum inside patternProperties should be detected"
        );
    }

    #[test]
    fn visitor_recurses_dependent_schemas() {
        // dependentSchemas values should be visited for unconstrained detection
        let mut schema = json!({
            "type": "object",
            "dependentSchemas": {
                "foo": {}
            }
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        let uc_errs: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. }))
            .collect();
        assert!(
            !uc_errs.is_empty(),
            "empty schema inside dependentSchemas should trigger UnconstrainedSchema"
        );
    }

    // ── #111 Depth semantics ──────────────────────────────────
    #[test]
    fn visitor_combinator_no_depth_increment() {
        // anyOf/oneOf/allOf should NOT increment semantic depth (max_depth_observed)
        let mut schema = json!({
            "type": "object",
            "anyOf": [{
                "type": "object",
                "oneOf": [{
                    "type": "string"
                }]
            }]
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        assert_eq!(
            visitor.max_depth_observed, 0,
            "combinators should not increment semantic depth, got: {}",
            visitor.max_depth_observed
        );
    }

    #[test]
    fn visitor_data_shape_keywords_increment_depth() {
        // properties/items/additionalProperties should increment semantic depth
        let mut schema = json!({
            "type": "object",
            "properties": {
                "child": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    }
                }
            }
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        assert_eq!(
            visitor.max_depth_observed, 2,
            "properties → items should yield semantic depth 2, got: {}",
            visitor.max_depth_observed
        );
    }

    // ── Tuple items traversal (Gemini review finding) ────────
    #[test]
    fn visitor_recurses_tuple_items() {
        // Array-form items (tuple validation) should be traversed for mixed enum detection
        let mut schema = json!({
            "type": "array",
            "items": [
                { "type": "string" },
                { "enum": ["a", 1] }
            ]
        });
        let mut errors = Vec::new();
        let mut transforms = Vec::new();
        let mut visitor = CompatVisitor {
            errors: &mut errors,
            transforms: &mut transforms,
            target: Target::OpenaiStrict,
            max_depth_observed: 0,
        };
        visitor.visit(&mut schema, "#", 0, 0);
        let enum_errs: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, ProviderCompatError::MixedEnumTypes { .. }))
            .collect();
        assert!(
            !enum_errs.is_empty(),
            "mixed enum inside tuple items should be detected"
        );
    }
}
