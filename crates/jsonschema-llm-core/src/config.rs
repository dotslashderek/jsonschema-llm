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
