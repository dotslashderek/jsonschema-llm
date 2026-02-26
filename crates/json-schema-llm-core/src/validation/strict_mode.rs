//! Offline strict-mode validator for OpenAI structured output schemas.
//!
//! Provides [`validate_strict_mode`] — a read-only auditor that recursively
//! walks a final output schema and reports every known OpenAI strict-mode
//! violation. This function is independent of the p1–p9 pipeline and can
//! be used as a public API by consumers.
//!
//! # Design
//!
//! - **Auditor, not transformer**: reads only, never mutates.
//! - **Independent**: no imports from `passes/`.
//! - **Semantic depth**: tracks data-shape edges (properties, items,
//!   additionalProperties, prefixItems) just like `p9_provider_compat.rs`.
//!   Combinators (anyOf/oneOf/allOf) do NOT increment depth.

use serde_json::Value;

use crate::schema_utils::build_path;
use crate::schema_walker::{ARRAY_KEYWORDS, MAP_KEYWORDS, SINGLE_KEYWORDS};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Machine-readable identifier for each strict-mode rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrictModeRule {
    RootNotObject,
    MissingType,
    MissingAdditionalProperties,
    MissingItems,
    BannedPatternProperties,
    BannedSchemaRef,
    BannedAnchor,
    BannedDynamicRef,
    BannedDynamicAnchor,
    BannedDependentSchemas,
    BannedDependentRequired,
    BannedUnevaluatedProperties,
    BannedUnevaluatedItems,
    BannedContains,
    BannedMinContains,
    BannedMaxContains,
    BannedIf,
    BannedThen,
    BannedElse,
    BannedNot,
    DepthExceeded,
}

/// A single strict-mode violation found during validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrictModeViolation {
    /// JSON Pointer path to the violating node.
    pub path: String,
    /// Machine-readable rule identifier.
    pub rule_id: StrictModeRule,
    /// Human-readable explanation of the violation.
    pub message: String,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// OpenAI strict-mode maximum semantic nesting depth.
/// OpenAI's documented limit is 5 levels of nesting.
/// Mirrors `OPENAI_MAX_DEPTH` in `p9_provider_compat.rs`.
const MAX_SEMANTIC_DEPTH: usize = 5;

/// Banned keywords mapped to their rule IDs.
const BANNED_KEYWORD_RULES: &[(&str, StrictModeRule)] = &[
    ("patternProperties", StrictModeRule::BannedPatternProperties),
    ("$anchor", StrictModeRule::BannedAnchor),
    ("$dynamicRef", StrictModeRule::BannedDynamicRef),
    ("$dynamicAnchor", StrictModeRule::BannedDynamicAnchor),
    ("dependentSchemas", StrictModeRule::BannedDependentSchemas),
    ("dependentRequired", StrictModeRule::BannedDependentRequired),
    (
        "unevaluatedProperties",
        StrictModeRule::BannedUnevaluatedProperties,
    ),
    ("unevaluatedItems", StrictModeRule::BannedUnevaluatedItems),
    ("contains", StrictModeRule::BannedContains),
    ("minContains", StrictModeRule::BannedMinContains),
    ("maxContains", StrictModeRule::BannedMaxContains),
    ("if", StrictModeRule::BannedIf),
    ("then", StrictModeRule::BannedThen),
    ("else", StrictModeRule::BannedElse),
    ("not", StrictModeRule::BannedNot),
];

/// Data-shape keywords that increment semantic depth.
const DATA_SHAPE_KEYWORDS: &[&str] = &[
    "properties",
    "patternProperties",
    "additionalProperties",
    "items",
    "prefixItems",
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Validate a schema against OpenAI strict-mode rules.
///
/// Returns a list of violations. An empty list means the schema is compliant.
///
/// This is a **read-only auditor** — it never mutates the input schema.
pub fn validate_strict_mode(schema: &Value) -> Vec<StrictModeViolation> {
    let mut violations = Vec::new();

    // ── Root-level checks ────────────────────────────────────────
    // OpenAI strict mode requires the root to be type: "object".
    if let Some(obj) = schema.as_object() {
        let root_type = obj.get("type").and_then(|v| v.as_str());
        if root_type != Some("object") {
            violations.push(StrictModeViolation {
                path: "#".to_string(),
                rule_id: StrictModeRule::RootNotObject,
                message: format!(
                    "Root schema must be type 'object', found '{}'",
                    root_type.unwrap_or("none")
                ),
            });
        }
    }

    walk(schema, "#", 0, &mut violations);
    violations
}

// ---------------------------------------------------------------------------
// Internal recursive walker
// ---------------------------------------------------------------------------

/// Recursively walk a schema node, collecting violations.
///
/// - `value`: the current schema node
/// - `path`: JSON Pointer to this node
/// - `semantic_depth`: data-shape nesting depth (NOT incremented by combinators)
/// - `violations`: accumulator for found violations
fn walk(
    value: &Value,
    path: &str,
    semantic_depth: usize,
    violations: &mut Vec<StrictModeViolation>,
) {
    let obj = match value.as_object() {
        Some(o) => o,
        None => return, // Non-object values (strings, bools, arrays) are not schema nodes
    };

    // ── Depth check ──────────────────────────────────────────────
    // Use >= to match p9_provider_compat.rs behavior (depth 5 IS the limit).
    // Exempt primitive leaves — they don't contribute nesting.
    if semantic_depth >= MAX_SEMANTIC_DEPTH && path != "#" {
        let schema_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let is_primitive = matches!(
            schema_type,
            "string" | "integer" | "number" | "boolean" | "null"
        );
        let has_sub_structure = obj.contains_key("properties")
            || obj.contains_key("items")
            || obj.contains_key("additionalProperties")
            || obj.contains_key("anyOf")
            || obj.contains_key("oneOf")
            || obj.contains_key("allOf")
            || obj.contains_key("prefixItems");

        if !(is_primitive && !has_sub_structure) {
            violations.push(StrictModeViolation {
                path: path.to_string(),
                rule_id: StrictModeRule::DepthExceeded,
                message: format!(
                    "Semantic depth {} reaches limit {} at '{}'",
                    semantic_depth, MAX_SEMANTIC_DEPTH, path
                ),
            });
            // Don't recurse further — everything below is also over the limit
            return;
        }
    }

    // ── Banned keywords ──────────────────────────────────────────
    for (keyword, rule) in BANNED_KEYWORD_RULES {
        if obj.contains_key(*keyword) {
            violations.push(StrictModeViolation {
                path: path.to_string(),
                rule_id: rule.clone(),
                message: format!("Banned keyword '{}' found at '{}'", keyword, path),
            });
        }
    }

    // ── Schema-level $ref ────────────────────────────────────────
    // Flag `$ref` unconditionally — the walker only visits schema
    // nodes (not property names), so `$ref` here is always a schema keyword.
    if obj.contains_key("$ref") {
        violations.push(StrictModeViolation {
            path: path.to_string(),
            rule_id: StrictModeRule::BannedSchemaRef,
            message: format!("Schema-level '$ref' found at '{}'", path),
        });
    }

    // ── MissingType ──────────────────────────────────────────────
    // Require `type` on all schema nodes. Exempt:
    // - Bare combinatorial wrappers (anyOf/oneOf/allOf without type)
    // - Schemas with `enum` or `const` (type is implicit from values)
    let has_type = obj.contains_key("type");
    let is_combinator_wrapper = !has_type
        && (obj.contains_key("anyOf") || obj.contains_key("oneOf") || obj.contains_key("allOf"));
    let has_implicit_type = obj.contains_key("enum") || obj.contains_key("const");

    if !has_type && !is_combinator_wrapper && !has_implicit_type && !obj.is_empty() {
        violations.push(StrictModeViolation {
            path: path.to_string(),
            rule_id: StrictModeRule::MissingType,
            message: format!("Schema node at '{}' is missing 'type'", path),
        });
    }

    // ── MissingAdditionalProperties ──────────────────────────────
    let is_object_type = obj.get("type").and_then(|v| v.as_str()) == Some("object");

    if is_object_type {
        let has_ap = obj.get("additionalProperties") == Some(&Value::Bool(false));
        if !has_ap {
            violations.push(StrictModeViolation {
                path: path.to_string(),
                rule_id: StrictModeRule::MissingAdditionalProperties,
                message: format!(
                    "Object at '{}' is missing 'additionalProperties: false'",
                    path
                ),
            });
        }
    }

    // ── MissingItems ─────────────────────────────────────────────
    // OpenAI strict mode requires `items` on array-typed schemas.
    let is_array_type = obj.get("type").and_then(|v| v.as_str()) == Some("array");

    if is_array_type && !obj.contains_key("items") && !obj.contains_key("prefixItems") {
        violations.push(StrictModeViolation {
            path: path.to_string(),
            rule_id: StrictModeRule::MissingItems,
            message: format!("Array at '{}' is missing 'items' keyword", path),
        });
    }

    // ── Recurse into children ────────────────────────────────────
    // Follow the keyword taxonomy from schema_walker.rs.

    // Map-of-schemas keywords (properties, patternProperties, $defs, etc.)
    for keyword in MAP_KEYWORDS {
        if let Some(Value::Object(map)) = obj.get(*keyword) {
            let is_data_shape = DATA_SHAPE_KEYWORDS.contains(keyword);
            let next_depth = if is_data_shape {
                semantic_depth + 1
            } else {
                semantic_depth
            };
            for (key, child) in map {
                let child_path = build_path(path, &[keyword, key]);
                walk(child, &child_path, next_depth, violations);
            }
        }
    }

    // Single-schema keywords (additionalProperties, not, if, etc.)
    for keyword in SINGLE_KEYWORDS {
        if let Some(child) = obj.get(*keyword) {
            if child.is_object() {
                let is_data_shape = DATA_SHAPE_KEYWORDS.contains(keyword);
                let next_depth = if is_data_shape {
                    semantic_depth + 1
                } else {
                    semantic_depth
                };
                let child_path = build_path(path, &[keyword]);
                walk(child, &child_path, next_depth, violations);
            }
        }
    }

    // Array-of-schemas keywords (anyOf, oneOf, allOf, prefixItems)
    for keyword in ARRAY_KEYWORDS {
        if let Some(Value::Array(arr)) = obj.get(*keyword) {
            let is_data_shape = DATA_SHAPE_KEYWORDS.contains(keyword);
            let next_depth = if is_data_shape {
                semantic_depth + 1
            } else {
                semantic_depth
            };
            for (i, child) in arr.iter().enumerate() {
                let child_path = build_path(path, &[keyword, &i.to_string()]);
                walk(child, &child_path, next_depth, violations);
            }
        }
    }

    // items (object or array form)
    if let Some(items) = obj.get("items") {
        match items {
            Value::Object(_) => {
                let child_path = build_path(path, &["items"]);
                walk(items, &child_path, semantic_depth + 1, violations);
            }
            Value::Array(arr) => {
                for (i, child) in arr.iter().enumerate() {
                    let child_path = build_path(path, &["items", &i.to_string()]);
                    walk(child, &child_path, semantic_depth + 1, violations);
                }
            }
            _ => {}
        }
    }
}
