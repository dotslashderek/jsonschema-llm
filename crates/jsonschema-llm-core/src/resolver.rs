//! Centralized JSON Schema reference resolution engine.
//!
//! [`ResolverEngine`] is the single point of truth for resolving `$ref` values.
//! It encapsulates:
//! - The anchor map (`$anchor` → JSON Pointer)
//! - The root `$id` base URI
//! - The root schema document (for pointer lookups)
//!
//! This consolidates the scattered ref resolution logic from `extract.rs`,
//! `p0_normalize.rs`, and the former `anchor_utils.rs` into one struct.

use std::collections::HashMap;

use serde_json::Value;
use url::Url;

use crate::error::ConvertError;

// Re-export types that consumers need
pub(crate) use crate::anchor_utils::ResolvedRef;

// ---------------------------------------------------------------------------
// ResolverEngine
// ---------------------------------------------------------------------------

/// Centralized reference resolution engine for a JSON Schema document.
///
/// Create one per schema document, then call [`resolve`] to resolve `$ref`
/// strings into JSON Pointer paths.
pub(crate) struct ResolverEngine {
    anchor_map: HashMap<String, String>,
    base_uri: Url,
}

impl ResolverEngine {
    /// Build a resolver for the given schema document.
    ///
    /// * Scans for `$anchor` declarations (respecting `$id` base URI scoping)
    /// * Extracts the root `$id` as the base URI (falls back to default)
    pub fn new(schema: &Value) -> Result<Self, ConvertError> {
        let default_base = crate::anchor_utils::default_base_uri();
        let base_uri = if let Some(id_val) = schema.get("$id").and_then(Value::as_str) {
            default_base.join(id_val).unwrap_or(default_base)
        } else {
            default_base
        };
        let default_anchor_base = crate::anchor_utils::default_base_uri();
        let anchor_map = crate::anchor_utils::build_anchor_map(schema, Some(&default_anchor_base))?;
        Ok(Self {
            anchor_map,
            base_uri,
        })
    }

    /// Resolve a `$ref` string against the current base URI.
    ///
    /// - JSON Pointer refs (`#/...`, `#`) pass through as `Pointer`.
    /// - Anchor-style refs (`#anchor`) are looked up in the anchor map.
    /// - Unresolvable refs (external URLs, unknown anchors) → `Unresolvable`.
    pub fn resolve(&self, ref_str: &str, current_base: &Url) -> ResolvedRef {
        crate::anchor_utils::resolve_ref(ref_str, current_base, &self.anchor_map)
    }

    /// The root base URI for this schema document.
    pub fn base_uri(&self) -> &Url {
        &self.base_uri
    }

    /// The anchor map for this schema document.
    #[allow(dead_code)]
    pub fn anchor_map(&self) -> &HashMap<String, String> {
        &self.anchor_map
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_resolver_engine_new_no_id() {
        let schema = json!({
            "$defs": {
                "Foo": {
                    "$anchor": "foo-anchor",
                    "type": "string"
                }
            }
        });

        let engine = ResolverEngine::new(&schema).unwrap();
        assert!(engine
            .base_uri()
            .as_str()
            .starts_with("file:///schema.json"));

        // Resolve anchor ref
        let result = engine.resolve("#foo-anchor", engine.base_uri());
        assert_eq!(result, ResolvedRef::Pointer("#/$defs/Foo".to_string()));
    }

    #[test]
    fn test_resolver_engine_with_root_id() {
        let schema = json!({
            "$id": "https://example.com/schema",
            "$defs": {
                "Bar": {
                    "$anchor": "bar-anchor",
                    "type": "integer"
                }
            }
        });

        let engine = ResolverEngine::new(&schema).unwrap();
        assert_eq!(engine.base_uri().as_str(), "https://example.com/schema");

        let result = engine.resolve("#bar-anchor", engine.base_uri());
        assert_eq!(result, ResolvedRef::Pointer("#/$defs/Bar".to_string()));
    }

    #[test]
    fn test_resolver_json_pointer_passthrough() {
        let schema = json!({ "type": "object" });
        let engine = ResolverEngine::new(&schema).unwrap();

        let result = engine.resolve("#/$defs/Foo", engine.base_uri());
        assert_eq!(result, ResolvedRef::Pointer("#/$defs/Foo".to_string()));
    }

    #[test]
    fn test_resolver_external_ref_unresolvable() {
        let schema = json!({ "type": "object" });
        let engine = ResolverEngine::new(&schema).unwrap();

        let result = engine.resolve("https://external.com/schema", engine.base_uri());
        match result {
            ResolvedRef::Unresolvable(s) => {
                assert_eq!(s, "https://external.com/schema");
            }
            _ => panic!("Expected Unresolvable"),
        }
    }
}
