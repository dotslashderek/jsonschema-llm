//! Shared result type for conversion passes.
//!
//! Every pass (p1–p9) returns a `PassResult` containing the transformed schema
//! and any codec metadata (transforms and dropped constraints) produced during
//! the pass. This replaces the per-pass bespoke result structs.

use serde_json::Value;

use crate::codec::{Codec, DroppedConstraint, Transform};

/// Unified result of a single conversion pass.
#[derive(Debug)]
pub struct PassResult {
    /// The transformed schema.
    pub schema: Value,
    /// Codec transforms produced by this pass.
    pub transforms: Vec<Transform>,
    /// Constraints that were dropped during this pass.
    pub dropped_constraints: Vec<DroppedConstraint>,
}

impl PassResult {
    /// Create a result with only a schema (no transforms or dropped constraints).
    pub fn schema_only(schema: Value) -> Self {
        Self {
            schema,
            transforms: Vec::new(),
            dropped_constraints: Vec::new(),
        }
    }

    /// Create a result with a schema and transforms.
    pub fn with_transforms(schema: Value, transforms: Vec<Transform>) -> Self {
        Self {
            schema,
            transforms,
            dropped_constraints: Vec::new(),
        }
    }

    /// Create a result with a schema and dropped constraints.
    pub fn with_dropped(schema: Value, dropped_constraints: Vec<DroppedConstraint>) -> Self {
        Self {
            schema,
            transforms: Vec::new(),
            dropped_constraints,
        }
    }

    /// Merge this pass's codec metadata into a codec accumulator.
    ///
    /// Consumes `self`, moves transforms/constraints into the codec, and
    /// returns the schema for the next pass in the pipeline.
    pub fn merge_into_codec(self, codec: &mut Codec) -> Value {
        codec.transforms.extend(self.transforms);
        codec.dropped_constraints.extend(self.dropped_constraints);
        self.schema
    }
}
