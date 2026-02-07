//! Rehydrator — reconstructs original data shape from LLM output using the codec.

use crate::codec::Codec;
use crate::error::ConvertError;
use serde_json::Value;

/// Rehydrate LLM output using the codec sidecar.
///
/// Applies transforms in reverse:
/// 1. Strip nullable optionals (null → remove key)
/// 2. Restore maps from arrays ({key, value}[] → {k: v})
/// 3. Parse JSON strings back to objects
pub fn rehydrate(data: &Value, codec: &Codec) -> Result<Value, ConvertError> {
    let mut result = data.clone();

    for transform in &codec.transforms {
        match transform {
            crate::codec::Transform::NullableOptional { path, .. } => {
                strip_nullable(&mut result, path);
            }
            crate::codec::Transform::MapToArray { path, key_field } => {
                restore_map(&mut result, path, key_field);
            }
            crate::codec::Transform::JsonStringParse { path } => {
                parse_json_string(&mut result, path);
            }
            crate::codec::Transform::DiscriminatorAnyOf { .. } => {
                // No action needed — anyOf data comes back structurally correct
            }
        }
    }

    Ok(result)
}

fn strip_nullable(_data: &mut Value, _path: &str) {
    todo!()
}

fn restore_map(_data: &mut Value, _path: &str, _key_field: &str) {
    todo!()
}

fn parse_json_string(_data: &mut Value, _path: &str) {
    todo!()
}
