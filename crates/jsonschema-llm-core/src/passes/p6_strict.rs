//! Pass 6: Strict Mode Enforcement
//! additionalProperties: false, all props required, optionals → anyOf [T, null]

use serde_json::Value;

pub fn enforce_strict(_schema: &Value) -> Value {
    todo!()
}
