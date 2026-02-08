//! Rehydration codec — metadata for reconstructing original shape from LLM output.

use serde::{Deserialize, Serialize};

/// A collection of transformation records produced during schema conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transform {
    MapToArray {
        path: String,
        key_field: String,
    },
    JsonStringParse {
        path: String,
    },
    NullableOptional {
        path: String,
        original_required: bool,
    },
    DiscriminatorAnyOf {
        path: String,
        discriminator: String,
        variants: Vec<String>,
    },
    ExtractAdditionalProperties {
        path: String,
        property_name: String,
    },
    RecursiveInflate {
        path: String,
        original_ref: String,
    },
}

/// A constraint that was dropped during conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
