//! Pass 0: Schema Normalization
//! Resolves $ref, normalizes draft syntax, detects recursive cycles.

use serde_json::Value;

pub fn normalize(_schema: &Value) -> Value {
    todo!()
}
