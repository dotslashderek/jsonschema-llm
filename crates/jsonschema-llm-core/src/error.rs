//! Error types for schema conversion.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable, machine-readable error codes for FFI consumers.
///
/// These codes form a **stable API contract** — once published, variant names
/// and their serialized `snake_case` strings must never change across versions.
/// Language bindings (WASM, Python, Java) map these to native exception types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    /// JSON (de)serialization error (malformed input, invalid structure, or encoding failure).
    JsonParseError,
    /// Schema structure error (invalid or unsupported schema construct).
    SchemaError,
    /// Maximum traversal depth exceeded during $ref resolution.
    RecursionDepthExceeded,
    /// Schema uses a feature not supported by the target provider.
    UnsupportedFeature,
    /// A $ref could not be resolved within the schema.
    UnresolvableRef,
    /// Error during data rehydration.
    RehydrationError,
    /// Codec version is incompatible with this library version.
    CodecVersionMismatch,
}

#[derive(Debug, Error)]
pub enum ConvertError {
    #[error("JSON (de)serialization error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Schema error at {path}: {message}")]
    SchemaError { path: String, message: String },

    #[error("Recursion depth exceeded at {path} (max: {max_depth})")]
    RecursionDepthExceeded { path: String, max_depth: usize },

    #[error("Unsupported schema feature at {path}: {feature}")]
    UnsupportedFeature { path: String, feature: String },

    #[error("Unresolvable $ref at {path}: {reference}")]
    UnresolvableRef { path: String, reference: String },

    #[error("Rehydration error: {0}")]
    RehydrationError(String),

    #[error("Codec version mismatch: found {found}, expected {expected}")]
    CodecVersionMismatch { found: String, expected: String },
}

impl ConvertError {
    /// Returns the stable error code for this error variant.
    pub fn error_code(&self) -> ErrorCode {
        match self {
            ConvertError::JsonError(_) => ErrorCode::JsonParseError,
            ConvertError::SchemaError { .. } => ErrorCode::SchemaError,
            ConvertError::RecursionDepthExceeded { .. } => ErrorCode::RecursionDepthExceeded,
            ConvertError::UnsupportedFeature { .. } => ErrorCode::UnsupportedFeature,
            ConvertError::UnresolvableRef { .. } => ErrorCode::UnresolvableRef,
            ConvertError::RehydrationError(_) => ErrorCode::RehydrationError,
            ConvertError::CodecVersionMismatch { .. } => ErrorCode::CodecVersionMismatch,
        }
    }

    /// Returns the schema path context, if available.
    ///
    /// Returns `None` for `JsonError` and `RehydrationError` (no path context).
    pub fn path(&self) -> Option<&str> {
        match self {
            ConvertError::JsonError(_) => None,
            ConvertError::SchemaError { path, .. } => Some(path),
            ConvertError::RecursionDepthExceeded { path, .. } => Some(path),
            ConvertError::UnsupportedFeature { path, .. } => Some(path),
            ConvertError::UnresolvableRef { path, .. } => Some(path),
            ConvertError::RehydrationError(_) => None,
            ConvertError::CodecVersionMismatch { .. } => None,
        }
    }

    /// Produces a structured JSON error for FFI consumers.
    ///
    /// Format: `{"code": "...", "message": "...", "path": "..." | null}`
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.error_code(),
            "message": self.to_string(),
            "path": self.path(),
        })
    }
}
