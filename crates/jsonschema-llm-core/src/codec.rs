//! Rehydration codec — metadata for reconstructing original shape from LLM output.

use serde::{Deserialize, Serialize};

/// Codec format version URI constant.
pub const CODEC_SCHEMA_URI: &str = "https://jsonschema-llm.dev/codec/v1";

/// Expected major version of the codec format.
pub const CODEC_MAJOR_VERSION: u32 = 1;

/// A collection of transformation records produced during schema conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Codec {
    /// Schema URI for codec format versioning.
    #[serde(rename = "$schema")]
    pub schema: String,
    /// Ordered list of transforms applied during conversion.
    pub transforms: Vec<Transform>,
    /// Constraints that were dropped during conversion.
    pub dropped_constraints: Vec<DroppedConstraint>,
}

/// A single transformation record.
///
/// The `type` tag uses `snake_case` (e.g. `"map_to_array"`), while variant
/// fields use `camelCase` (e.g. `"keyField"`) for cross-language consistency.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transform {
    MapToArray {
        path: String,
        #[serde(rename = "keyField")]
        key_field: String,
    },
    JsonStringParse {
        path: String,
    },
    NullableOptional {
        path: String,
        #[serde(rename = "originalRequired")]
        original_required: bool,
    },
    DiscriminatorAnyOf {
        path: String,
        discriminator: String,
        variants: Vec<String>,
    },
    ExtractAdditionalProperties {
        path: String,
        #[serde(rename = "propertyName")]
        property_name: String,
    },
    RecursiveInflate {
        path: String,
        #[serde(rename = "originalRef")]
        original_ref: String,
    },
    RootObjectWrapper {
        path: String,
        #[serde(rename = "wrapperKey")]
        wrapper_key: String,
    },
    EnumStringify {
        path: String,
        #[serde(rename = "originalValues")]
        original_values: Vec<serde_json::Value>,
    },
}

/// A constraint that was dropped during conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DroppedConstraint {
    pub path: String,
    pub constraint: String,
    pub value: serde_json::Value,
}

impl Codec {
    pub fn new() -> Self {
        Self {
            schema: "https://jsonschema-llm.dev/codec/v1".to_string(),
            transforms: Vec::new(),
            dropped_constraints: Vec::new(),
        }
    }
}

impl Default for Codec {
    fn default() -> Self {
        Self::new()
    }
}
