//! Warning types emitted during rehydration when dropped constraints are violated.
//!
//! These warnings are **advisory** — they indicate that the LLM output may not
//! conform to constraints that were removed during schema conversion (e.g. regex
//! patterns, numeric bounds, string lengths). Callers should inspect warnings to
//! detect potentially hallucinated or out-of-spec data and decide whether to
//! reject, retry, or accept the output.

use serde::{Deserialize, Serialize};

/// A warning emitted when a dropped constraint is violated by LLM output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    /// Data location where the violation occurred (e.g. "/users/0/email").
    pub data_path: String,
    /// Schema location for debugging (e.g. "#/properties/users/items/properties/email").
    pub schema_path: String,
    /// Classification of the warning.
    pub kind: WarningKind,
    /// Human-readable description of the violation.
    pub message: String,
}

/// Classification of rehydration warnings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WarningKind {
    /// A dropped constraint was violated by the LLM output.
    ConstraintViolation {
        /// The constraint keyword that was violated (e.g. "pattern", "minimum").
        constraint: String,
    },
    /// A dropped constraint could not be evaluated (e.g. invalid regex pattern).
    ConstraintUnevaluable {
        /// The constraint keyword that could not be evaluated.
        constraint: String,
    },
    /// A codec path could not be resolved in the output data.
    PathNotFound,
}
