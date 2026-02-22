//! Unified JSON Schema AST traversal via the fold pattern.
//!
//! Provides [`SchemaFolder`] — a trait that passes implement to transform
//! schema trees — and [`fold`] — a driver that walks the AST, delegating
//! to the folder at each node. All schema-bearing keyword knowledge lives
//! here; adding a new keyword is a single-file change.

use serde_json::{Map, Value};

use crate::error::ConvertError;
use crate::schema_utils::build_path;

// ---------------------------------------------------------------------------
// Keyword lists — single source of truth
// ---------------------------------------------------------------------------

/// Keywords whose values are maps of schemas (each map entry is a sub-schema).
pub(crate) const MAP_KEYWORDS: &[&str] = &[
    "properties",
    "patternProperties",
    "$defs",
    "definitions",
    "dependentSchemas",
];

/// Keywords whose values are single sub-schemas (when the value is an object).
pub(crate) const SINGLE_KEYWORDS: &[&str] = &[
    "additionalProperties",
    "unevaluatedProperties",
    "propertyNames",
    "unevaluatedItems",
    "contains",
    "not",
    "if",
    "then",
    "else",
    "additionalItems",
];

/// Keywords whose values are arrays of sub-schemas.
pub(crate) const ARRAY_KEYWORDS: &[&str] = &["anyOf", "oneOf", "allOf", "prefixItems"];

// ---------------------------------------------------------------------------
// Fold action
// ---------------------------------------------------------------------------

/// Instruction returned by [`SchemaFolder::fold_schema`] to control traversal.
#[derive(Debug)]
pub enum FoldAction {
    /// Continue folding into children of this node.
    Continue(Value),
    /// Replace this node entirely — skip child traversal.
    Replace(Value),
}

// ---------------------------------------------------------------------------
// SchemaFolder trait
// ---------------------------------------------------------------------------

/// A fold-based schema transformer.
///
/// Implementations receive each schema node via [`fold_schema`] and return a
/// [`FoldAction`] controlling whether the driver recurses into children.
///
/// The [`fold`] driver handles all keyword enumeration — passes never need to
/// know which JSON keys contain sub-schemas.
pub trait SchemaFolder {
    type Error: From<ConvertError>;

    /// Called for each schema node BEFORE recursing into children.
    ///
    /// - `schema` — the current node (owned).
    /// - `path`   — JSON Pointer to this node.
    /// - `depth`  — recursion depth (incremented per child level).
    ///
    /// Return [`FoldAction::Continue`] to recurse into children, or
    /// [`FoldAction::Replace`] to short-circuit child traversal.
    fn fold_schema(
        &mut self,
        schema: Value,
        path: &str,
        depth: usize,
    ) -> Result<FoldAction, Self::Error>;
}

// ---------------------------------------------------------------------------
// Fold driver
// ---------------------------------------------------------------------------

/// Drive a [`SchemaFolder`] over a JSON Schema AST.
///
/// 1. Calls `folder.fold_schema()` on the current node.
/// 2. If [`FoldAction::Replace`], returns the replacement immediately.
/// 3. If [`FoldAction::Continue`] and the value is an object, recurses into
///    all schema-bearing keywords.
/// 4. Returns the rebuilt tree.
pub fn fold<F: SchemaFolder>(
    schema: Value,
    folder: &mut F,
    path: &str,
    depth: usize,
) -> Result<Value, F::Error> {
    let action = folder.fold_schema(schema, path, depth)?;

    match action {
        FoldAction::Replace(v) => Ok(v),
        FoldAction::Continue(v) => fold_children(v, folder, path, depth),
    }
}

/// Recurse into the schema-bearing children of a value.
///
/// Non-object values pass through unchanged.
fn fold_children<F: SchemaFolder>(
    value: Value,
    folder: &mut F,
    path: &str,
    depth: usize,
) -> Result<Value, F::Error> {
    let mut obj = match value {
        Value::Object(obj) => obj,
        other => return Ok(other),
    };

    // --- Map-of-schemas keywords ---
    for keyword in MAP_KEYWORDS {
        if let Some(val) = obj.remove(*keyword) {
            match val {
                Value::Object(map) => {
                    let mut new_map = Map::new();
                    for (key, val) in map {
                        let child_path = build_path(path, &[keyword, &key]);
                        new_map.insert(key, fold(val, folder, &child_path, depth + 1)?);
                    }
                    obj.insert(keyword.to_string(), Value::Object(new_map));
                }
                other => {
                    obj.insert(keyword.to_string(), other);
                }
            }
        }
    }

    // --- Single-schema keywords ---
    for keyword in SINGLE_KEYWORDS {
        if let Some(val) = obj.remove(*keyword) {
            if val.is_object() {
                let child_path = build_path(path, &[keyword]);
                let walked = fold(val, folder, &child_path, depth + 1)?;
                obj.insert(keyword.to_string(), walked);
            } else {
                // Not a schema (e.g. `additionalProperties: false`) — preserve
                obj.insert(keyword.to_string(), val);
            }
        }
    }

    // --- Array-of-schemas keywords ---
    for keyword in ARRAY_KEYWORDS {
        if let Some(val) = obj.remove(*keyword) {
            match val {
                Value::Array(variants) => {
                    let mut walked = Vec::with_capacity(variants.len());
                    for (i, variant) in variants.into_iter().enumerate() {
                        let child_path = build_path(path, &[keyword, &i.to_string()]);
                        walked.push(fold(variant, folder, &child_path, depth + 1)?);
                    }
                    obj.insert(keyword.to_string(), Value::Array(walked));
                }
                other => {
                    obj.insert(keyword.to_string(), other);
                }
            }
        }
    }

    // --- `items` (object or array form) ---
    if let Some(items) = obj.remove("items") {
        match items {
            Value::Object(_) => {
                let child_path = build_path(path, &["items"]);
                let walked = fold(items, folder, &child_path, depth + 1)?;
                obj.insert("items".to_string(), walked);
            }
            Value::Array(arr) => {
                // Tuple validation (Draft 4-7 array form)
                let mut walked = Vec::with_capacity(arr.len());
                for (i, item) in arr.into_iter().enumerate() {
                    let child_path = build_path(path, &["items", &i.to_string()]);
                    walked.push(fold(item, folder, &child_path, depth + 1)?);
                }
                obj.insert("items".to_string(), Value::Array(walked));
            }
            other => {
                // `items: true/false` — preserve
                obj.insert("items".to_string(), other);
            }
        }
    }

    Ok(Value::Object(obj))
}

// ---------------------------------------------------------------------------
// Convenience: identity folder (passthrough)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub struct IdentityFolder;

#[cfg(test)]
impl SchemaFolder for IdentityFolder {
    type Error = ConvertError;

    fn fold_schema(
        &mut self,
        schema: Value,
        _path: &str,
        _depth: usize,
    ) -> Result<FoldAction, Self::Error> {
        Ok(FoldAction::Continue(schema))
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // -----------------------------------------------------------------------
    // Test 1: Identity fold preserves schema
    // -----------------------------------------------------------------------
    #[test]
    fn test_identity_fold_preserves_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            },
            "required": ["name"]
        });

        let mut folder = IdentityFolder;
        let result = fold(schema.clone(), &mut folder, "#", 0).unwrap();
        assert_eq!(result, schema);
    }

    // -----------------------------------------------------------------------
    // Test 2: Fold visits all keyword categories
    // -----------------------------------------------------------------------
    #[test]
    fn test_fold_visits_all_keywords() {
        // Build a schema that uses every keyword category
        let schema = json!({
            "type": "object",
            "properties": {
                "a": { "type": "string" }
            },
            "patternProperties": {
                "^x-": { "type": "string" }
            },
            "$defs": {
                "D": { "type": "integer" }
            },
            "dependentSchemas": {
                "foo": { "type": "object" }
            },
            "additionalProperties": { "type": "string" },
            "unevaluatedProperties": { "type": "boolean" },
            "not": { "type": "null" },
            "if": { "type": "object" },
            "then": { "type": "string" },
            "else": { "type": "integer" },
            "contains": { "type": "number" },
            "propertyNames": { "type": "string" },
            "anyOf": [{ "type": "string" }, { "type": "integer" }],
            "oneOf": [{ "type": "boolean" }],
            "allOf": [{ "type": "object" }],
            "prefixItems": [{ "type": "string" }],
            "items": { "type": "number" }
        });

        let count = AtomicUsize::new(0);

        struct CountFolder<'a> {
            count: &'a AtomicUsize,
        }
        impl SchemaFolder for CountFolder<'_> {
            type Error = ConvertError;
            fn fold_schema(
                &mut self,
                schema: Value,
                _path: &str,
                _depth: usize,
            ) -> Result<FoldAction, Self::Error> {
                self.count.fetch_add(1, Ordering::Relaxed);
                Ok(FoldAction::Continue(schema))
            }
        }

        let mut folder = CountFolder { count: &count };
        let result = fold(schema.clone(), &mut folder, "#", 0).unwrap();

        // Schema is preserved
        assert_eq!(result, schema);

        // Root + all children visited:
        // 1 root + 1 properties.a + 1 patternProperties.^x- +
        // 1 $defs.D + 1 dependentSchemas.foo +
        // 1 additionalProperties + 1 unevaluatedProperties +
        // 1 not + 1 if + 1 then + 1 else + 1 contains + 1 propertyNames +
        // 2 anyOf + 1 oneOf + 1 allOf + 1 prefixItems + 1 items = 19
        assert_eq!(count.load(Ordering::Relaxed), 19);
    }

    // -----------------------------------------------------------------------
    // Test 3: FoldAction::Replace short-circuits children
    // -----------------------------------------------------------------------
    #[test]
    fn test_replace_skips_children() {
        let schema = json!({
            "type": "object",
            "properties": {
                "deep": {
                    "type": "object",
                    "properties": {
                        "nested": { "type": "string" }
                    }
                }
            }
        });

        let count = AtomicUsize::new(0);

        struct ReplaceAtDepth2<'a> {
            count: &'a AtomicUsize,
        }
        impl SchemaFolder for ReplaceAtDepth2<'_> {
            type Error = ConvertError;
            fn fold_schema(
                &mut self,
                schema: Value,
                _path: &str,
                depth: usize,
            ) -> Result<FoldAction, Self::Error> {
                self.count.fetch_add(1, Ordering::Relaxed);
                if depth >= 2 {
                    Ok(FoldAction::Replace(
                        json!({ "type": "string", "replaced": true }),
                    ))
                } else {
                    Ok(FoldAction::Continue(schema))
                }
            }
        }

        let mut folder = ReplaceAtDepth2 { count: &count };
        let result = fold(schema, &mut folder, "#", 0).unwrap();

        // Should visit: root (0), properties.deep (1), properties.deep.properties.nested (2, replaced)
        assert_eq!(count.load(Ordering::Relaxed), 3);
        assert_eq!(
            result["properties"]["deep"]["properties"]["nested"]["replaced"],
            json!(true)
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: items array form (tuple validation)
    // -----------------------------------------------------------------------
    #[test]
    fn test_items_array_form() {
        let schema = json!({
            "items": [
                { "type": "string" },
                { "type": "integer" }
            ]
        });

        let count = AtomicUsize::new(0);

        struct Counter<'a> {
            count: &'a AtomicUsize,
        }
        impl SchemaFolder for Counter<'_> {
            type Error = ConvertError;
            fn fold_schema(
                &mut self,
                schema: Value,
                _path: &str,
                _depth: usize,
            ) -> Result<FoldAction, Self::Error> {
                self.count.fetch_add(1, Ordering::Relaxed);
                Ok(FoldAction::Continue(schema))
            }
        }

        let mut folder = Counter { count: &count };
        let result = fold(schema.clone(), &mut folder, "#", 0).unwrap();
        assert_eq!(result, schema);
        // Root + 2 items = 3
        assert_eq!(count.load(Ordering::Relaxed), 3);
    }

    // -----------------------------------------------------------------------
    // Test 5: Non-object values pass through
    // -----------------------------------------------------------------------
    #[test]
    fn test_non_object_passthrough() {
        let schema = json!(true);
        let mut folder = IdentityFolder;
        let result = fold(schema.clone(), &mut folder, "#", 0).unwrap();
        assert_eq!(result, schema);
    }

    // -----------------------------------------------------------------------
    // Test 6: additionalProperties: false preserved (not treated as schema)
    // -----------------------------------------------------------------------
    #[test]
    fn test_additional_properties_false_preserved() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false
        });

        let mut folder = IdentityFolder;
        let result = fold(schema.clone(), &mut folder, "#", 0).unwrap();
        assert_eq!(result, schema);
    }
}
