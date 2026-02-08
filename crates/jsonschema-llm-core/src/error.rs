//! Error types for schema conversion.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConvertError {
    #[error("JSON parsing error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Schema error at {path}: {message}")]
    SchemaError { path: String, message: String },

    #[error("Recursion depth exceeded at {path} (max: {max_depth})")]
    RecursionDepthExceeded { path: String, max_depth: usize },

    #[error("Unsupported schema feature at {path}: {feature}")]
    UnsupportedFeature { path: String, feature: String },

    #[error("Rehydration error: {0}")]
    RehydrationError(String),
}
