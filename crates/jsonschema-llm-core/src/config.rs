//! Configuration for schema conversion.

use serde::{Deserialize, Serialize};

/// Target LLM provider for schema conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    /// OpenAI Strict Mode — most restrictive, all passes applied.
    OpenaiStrict,
    /// Google Gemini — relaxed, some passes skipped.
    Gemini,
    /// Anthropic Claude — moderate restrictions.
    Claude,
}

/// Options for schema conversion.
///
/// ## Serialization Format
///
/// Fields are serialized in `kebab-case` (e.g., `max-depth`, `recursion-limit`).
/// This naming convention is part of the public API contract for FFI and config files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ConvertOptions {
    /// Target provider. Default: OpenAI Strict.
    pub target: Target,
    /// Maximum traversal depth for Pass 0 ref resolution (stack overflow guard).
    pub max_depth: usize,
    /// Maximum number of times a recursive type may be inlined before
    /// being replaced with an opaque JSON-string placeholder (Pass 5).
    /// Default: 3. Keep low to avoid exponential schema expansion.
    pub recursion_limit: usize,
    /// Polymorphism strategy override.
    pub polymorphism: PolymorphismStrategy,
}

/// Strategy for handling oneOf/anyOf polymorphism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolymorphismStrategy {
    /// Rewrite oneOf → anyOf (default, recommended).
    AnyOf,
    /// Flatten all variants into a single object with nullable fields.
    /// Not recommended — can cause discriminator hallucination.
    Flatten,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            target: Target::OpenaiStrict,
            max_depth: 50,
            recursion_limit: 3,
            polymorphism: PolymorphismStrategy::AnyOf,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_options_serde_round_trip() {
        let opts = ConvertOptions {
            target: Target::Gemini,
            max_depth: 100,
            recursion_limit: 5,
            polymorphism: PolymorphismStrategy::Flatten,
        };

        // Serialize to JSON
        let json = serde_json::to_string(&opts).unwrap();

        // Verify kebab-case field names are in the JSON
        assert!(json.contains("\"max-depth\""));
        assert!(json.contains("\"recursion-limit\""));
        assert!(json.contains("\"gemini\""));
        assert!(json.contains("\"flatten\""));

        // Deserialize back
        let deserialized: ConvertOptions = serde_json::from_str(&json).unwrap();

        // Verify round-trip preserved values
        assert_eq!(deserialized.target, Target::Gemini);
        assert_eq!(deserialized.max_depth, 100);
        assert_eq!(deserialized.recursion_limit, 5);
        assert_eq!(deserialized.polymorphism, PolymorphismStrategy::Flatten);
    }
}
