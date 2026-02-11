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
            let mut schema =
                check_root_type(schema, config.target, &mut errors, &mut transforms);

            // ── Checks 2–4: Single-pass mutating visitor (#95, #96, #97)
            let max_depth_observed = {
                let mut visitor = CompatVisitor {
                    errors: &mut errors,
                    transforms: &mut transforms,
                    target: config.target,
                    max_depth_observed: 0,
                };
                visitor.visit(&mut schema, "#", 0);
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
    let root_type = schema
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("");

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
    /// - #95 Depth budget: diagnostic only (tracks max depth)
    /// - #96 Enum homogeneity: **transforms** mixed enums → all strings + codec entry
    /// - #97 Boolean/empty schemas: **transforms** → opaque string or sealed empty object
    fn visit(&mut self, schema: &mut Value, path: &str, depth: usize) {
        // Hard recursion guard
        if depth > HARD_RECURSION_LIMIT {
            return;
        }

        // ── #97: Boolean schema transform ──────────────────────────
        if let Some(b) = schema.as_bool() {
            self.errors.push(ProviderCompatError::UnconstrainedSchema {
                path: path.to_string(),
                schema_kind: format!("boolean({})", b),
                target: self.target,
                hint: format!(
                    "Boolean schema '{}' replaced with {}.",
                    b,
                    if b {
                        "opaque string (accepts any JSON-encoded value)"
                    } else {
                        "sealed empty object (rejects all values)"
                    }
                ),
            });

            if b {
                // `true` → opaque string (same as p4 stringification)
                *schema = json!({
                    "type": "string",
                    "description": "A JSON-encoded string representing the object. Parse with JSON.parse() after generation."
                });
                self.transforms.push(Transform::JsonStringParse {
                    path: path.to_string(),
                });
            } else {
                // `false` → sealed empty object (unsatisfiable but structurally valid)
                *schema = json!({
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": false
                });
            }
            return;
        }

        match schema.as_object() {
            Some(_) => {},
            None => return,
        };

        // ── #95 Depth budget (diagnostic) ──────────────────────────
        if depth > self.max_depth_observed {
            self.max_depth_observed = depth;
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

                    // Replace with opaque string (same as p4)
                    *schema = json!({
                        "type": "string",
                        "description": "A JSON-encoded string representing the object. Parse with JSON.parse() after generation."
                    });
                    self.transforms.push(Transform::JsonStringParse {
                        path: path.to_string(),
                    });
                    return; // No children to recurse into
                }
            }
        }

        // ── Recurse into children ──────────────────────────────────

        // properties
        if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
            let keys: Vec<String> = props.keys().cloned().collect();
            let _ = props;
            for key in &keys {
                let child_path = build_path(path, &["properties", key]);
                // Temporarily take the child, visit it, put it back
                if let Some(child) = schema.get_mut("properties").and_then(|p| p.get_mut(key)) {
                    self.visit(child, &child_path, depth + 1);
                }
            }
        }

        // items (single schema)
        {
            let has_items = schema.get("items").map(|v| v.is_object() || v.is_boolean()).unwrap_or(false);
            if has_items {
                let child_path = build_path(path, &["items"]);
                if let Some(child) = schema.get_mut("items") {
                    self.visit(child, &child_path, depth + 1);
                }
            }
        }

        // prefixItems (tuple)
        {
            let count = schema.get("prefixItems").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
            for i in 0..count {
                let child_path = build_path(path, &["prefixItems", &i.to_string()]);
                if let Some(child) = schema.get_mut("prefixItems").and_then(|p| p.get_mut(i)) {
                    self.visit(child, &child_path, depth + 1);
                }
            }
        }

        // additionalProperties (if it's a schema object)
        {
            let has_ap = schema.get("additionalProperties").map(|v| v.is_object()).unwrap_or(false);
            if has_ap {
                let child_path = build_path(path, &["additionalProperties"]);
                if let Some(child) = schema.get_mut("additionalProperties") {
                    self.visit(child, &child_path, depth + 1);
                }
            }
        }

        // anyOf / oneOf / allOf
        for keyword in &["anyOf", "oneOf", "allOf"] {
            let count = schema.get(*keyword).and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
            for i in 0..count {
                let child_path = build_path(path, &[keyword, &i.to_string()]);
                if let Some(child) = schema.get_mut(*keyword).and_then(|v| v.get_mut(i)) {
                    self.visit(child, &child_path, depth + 1);
                }
            }
        }

        // $defs / definitions
        for keyword in &["$defs", "definitions"] {
            if let Some(defs) = schema.get(*keyword).and_then(|v| v.as_object()) {
                let keys: Vec<String> = defs.keys().cloned().collect();
                let _ = defs;
                for key in &keys {
                    let child_path = build_path(path, &[keyword, key]);
                    if let Some(child) = schema.get_mut(*keyword).and_then(|v| v.get_mut(key)) {
                        self.visit(child, &child_path, depth + 1);
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

    // Stringify all values
    let stringified: Vec<Value> = original_values
        .iter()
        .map(|v| match v {
            Value::String(s) => Value::String(s.clone()),
            other => Value::String(other.to_string()),
        })
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

/// Returns true if a schema object is unconstrained (empty or only structural keywords
/// added by p6_strict like `additionalProperties` and `required`).
fn is_unconstrained(obj: &serde_json::Map<String, Value>) -> bool {
    // Quick check: truly empty
    if obj.is_empty() {
        return true;
    }

    // Keywords that indicate the schema has actual content constraints
    const CONTENT_KEYWORDS: &[&str] = &[
        "type",
        "properties",
        "items",
        "prefixItems",
        "enum",
        "const",
        "anyOf",
        "oneOf",
        "allOf",
        "$ref",
        "not",
        "if",
        "then",
        "else",
        "pattern",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        "format",
    ];

    // Keywords that are structural (added by p6) and don't imply content constraints
    //   - additionalProperties: sealing
    //   - required: empty required array on sealed empty object
    //   - description: metadata only
    //   - title: metadata only
    //   - $schema: metadata only
    !obj.keys().any(|k| CONTENT_KEYWORDS.contains(&k.as_str()))
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
        assert!(r.errors.iter().all(|e| !matches!(e, ProviderCompatError::RootTypeIncompatible { .. })));
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
        assert!(r.errors.iter().all(|e| !matches!(e, ProviderCompatError::DepthBudgetExceeded { .. })));
    }

    #[test]
    fn deep_emits_error() {
        // Build 7 levels deep
        let mut inner = json!({"type": "string"});
        for i in (0..7).rev() {
            inner = json!({"type": "object", "properties": {format!("l{i}"): inner}});
        }
        let r = check_provider_compat(&inner, &opts());
        let depth_errs: Vec<_> = r.errors.iter().filter(|e| matches!(e, ProviderCompatError::DepthBudgetExceeded { .. })).collect();
        assert!(!depth_errs.is_empty(), "should have at least one depth error");
    }

    // ── Enum homogeneity ──────────────────────────────────────
    #[test]
    fn homo_enum_clean() {
        let schema = json!({"type": "object", "properties": {"c": {"enum": ["a", "b"]}}});
        let r = check_provider_compat(&schema, &opts());
        assert!(r.errors.iter().all(|e| !matches!(e, ProviderCompatError::MixedEnumTypes { .. })));
    }

    #[test]
    fn mixed_enum_error() {
        let schema = json!({"type": "object", "properties": {"c": {"enum": ["a", 1]}}});
        let r = check_provider_compat(&schema, &opts());
        let enum_errs: Vec<_> = r.errors.iter().filter(|e| matches!(e, ProviderCompatError::MixedEnumTypes { .. })).collect();
        assert_eq!(enum_errs.len(), 1);
    }

    // ── Boolean / empty schemas ───────────────────────────────
    #[test]
    fn typed_no_unconstrained() {
        let schema = json!({"type": "object", "properties": {"x": {"type": "string"}}});
        let r = check_provider_compat(&schema, &opts());
        assert!(r.errors.iter().all(|e| !matches!(e, ProviderCompatError::UnconstrainedSchema { .. })));
    }

    #[test]
    fn empty_sub_schema_flagged() {
        let schema = json!({"type": "object", "properties": {"x": {}}});
        let r = check_provider_compat(&schema, &opts());
        let uc_errs: Vec<_> = r.errors.iter().filter(|e| matches!(e, ProviderCompatError::UnconstrainedSchema { .. })).collect();
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
}
