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
    /// Schema violates a specific constraint of the target provider (e.g. strict mode).
    ProviderCompatFailure,
}

use crate::config::Target;
use std::fmt;

/// Provider compatibility failure — the schema violates a target provider's constraints.
///
/// These are pre-flight checks: they detect issues the provider API would reject,
/// before the API call is made.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderCompatError {
    /// Root schema is not type: object (OpenAI strict requires object root)
    RootTypeIncompatible {
        actual_type: String,
        target: Target,
        hint: String,
    },
    /// Schema nesting exceeds provider limit
    DepthBudgetExceeded {
        actual_depth: usize,
        max_depth: usize,
        target: Target,
        hint: String,
    },
    /// Enum contains mixed types
    MixedEnumTypes {
        path: String,
        types_found: Vec<String>,
        target: Target,
        hint: String,
    },
    /// Boolean/empty schema lacks required constraints
    UnconstrainedSchema {
        path: String,
        schema_kind: String,
        target: Target,
        hint: String,
    },
    /// #246: patternProperties stripped from typed object (lossy constraint drop)
    PatternPropertiesStripped {
        path: String,
        target: Target,
        hint: String,
    },
    /// #246: patternProperties-only schema opaque-stringified
    PatternPropertiesStringified {
        path: String,
        target: Target,
        hint: String,
    },
}

impl fmt::Display for ProviderCompatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderCompatError::RootTypeIncompatible {
                actual_type,
                target: _,
                hint,
            } => write!(
                f,
                "Root schema must be an object (found: {}). {}",
                actual_type, hint
            ),
            ProviderCompatError::DepthBudgetExceeded {
                actual_depth,
                max_depth,
                target: _,
                hint,
            } => write!(
                f,
                "Schema depth {} reaches or exceeds limit of {}. {}",
                actual_depth, max_depth, hint
            ),
            ProviderCompatError::MixedEnumTypes {
                path,
                types_found,
                target: _,
                hint,
            } => write!(
                f,
                "Enum at '{}' has mixed types ({:?}). {}",
                path, types_found, hint
            ),
            ProviderCompatError::UnconstrainedSchema {
                path,
                schema_kind,
                target: _,
                hint,
            } => write!(
                f,
                "{} schema at '{}' is unconstrained. {}",
                schema_kind, path, hint
            ),
            ProviderCompatError::PatternPropertiesStripped {
                path,
                target: _,
                hint,
            } => write!(f, "patternProperties stripped at '{}'. {}", path, hint),
            ProviderCompatError::PatternPropertiesStringified {
                path,
                target: _,
                hint,
            } => write!(
                f,
                "patternProperties schema opaque-stringified at '{}'. {}",
                path, hint
            ),
        }
    }
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

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_error_code_provider_compat_serializes() {
        let code = ErrorCode::ProviderCompatFailure;
        let json = serde_json::to_value(code).unwrap();
        assert_eq!(json, json!("provider_compat_failure"));
    }

    #[test]
    fn test_error_code_provider_compat_round_trips() {
        let code = ErrorCode::ProviderCompatFailure;
        let json_str = serde_json::to_string(&code).unwrap();
        let deserialized: ErrorCode = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized, code);
    }

    #[test]
    fn test_provider_compat_error_root_type_serializes() {
        let err = ProviderCompatError::RootTypeIncompatible {
            actual_type: "array".to_string(),
            target: Target::OpenaiStrict,
            hint: "Wrap in an object".to_string(),
        };

        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], json!("root_type_incompatible"));
        assert_eq!(json["actual_type"], json!("array"));
        assert_eq!(json["target"], json!("openai-strict"));
        assert_eq!(json["hint"], json!("Wrap in an object"));
    }

    #[test]
    fn test_provider_compat_error_depth_budget_serializes() {
        let err = ProviderCompatError::DepthBudgetExceeded {
            actual_depth: 15,
            max_depth: 10,
            target: Target::OpenaiStrict,
            hint: "Reduce nesting".to_string(),
        };

        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], json!("depth_budget_exceeded"));
        assert_eq!(json["actual_depth"], json!(15));
        assert_eq!(json["max_depth"], json!(10));
    }

    #[test]
    fn test_provider_compat_error_mixed_enum_serializes() {
        let err = ProviderCompatError::MixedEnumTypes {
            path: "#/properties/status".to_string(),
            types_found: vec!["string".to_string(), "integer".to_string()],
            target: Target::OpenaiStrict,
            hint: "Use separate fields".to_string(),
        };

        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], json!("mixed_enum_types"));
        assert_eq!(json["path"], json!("#/properties/status"));
        assert_eq!(json["types_found"], json!(["string", "integer"]));
    }

    #[test]
    fn test_provider_compat_error_unconstrained_serializes() {
        let err = ProviderCompatError::UnconstrainedSchema {
            path: "#/properties/meta".to_string(),
            schema_kind: "true".to_string(),
            target: Target::OpenaiStrict,
            hint: "Add type constraint".to_string(),
        };

        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["type"], json!("unconstrained_schema"));
        assert_eq!(json["schema_kind"], json!("true"));
    }

    #[test]
    fn test_provider_compat_error_display() {
        let err = ProviderCompatError::RootTypeIncompatible {
            actual_type: "array".to_string(),
            target: Target::OpenaiStrict,
            hint: "Wrap in an object".to_string(),
        };

        let display = format!("{}", err);
        assert!(
            display.contains("array"),
            "Display should mention actual type"
        );
        assert!(
            display.contains("Wrap in an object"),
            "Display should include hint"
        );
    }

    #[test]
    fn test_provider_compat_error_round_trips() {
        let err = ProviderCompatError::RootTypeIncompatible {
            actual_type: "string".to_string(),
            target: Target::OpenaiStrict,
            hint: "Wrap it".to_string(),
        };

        let json_str = serde_json::to_string(&err).unwrap();
        let deserialized: ProviderCompatError = serde_json::from_str(&json_str).unwrap();

        match deserialized {
            ProviderCompatError::RootTypeIncompatible {
                actual_type,
                target,
                hint,
            } => {
                assert_eq!(actual_type, "string");
                assert_eq!(target, Target::OpenaiStrict);
                assert_eq!(hint, "Wrap it");
            }
            other => panic!("Wrong variant: {:?}", other),
        }
    }
}
