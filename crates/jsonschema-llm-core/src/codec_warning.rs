//! Warning types emitted during rehydration when dropped constraints are violated.

use serde::{Deserialize, Serialize};

/// A warning emitted when a dropped constraint is violated by LLM output.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// A codec path could not be resolved in the output data.
    PathNotFound,
}
