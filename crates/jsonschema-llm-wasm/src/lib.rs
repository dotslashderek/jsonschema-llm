//! WASM bindings for jsonschema-llm.
//!
//! Exposes `convert` and `rehydrate` via `wasm-bindgen` for use from
//! JavaScript / TypeScript. Uses `serde-wasm-bindgen` for JS ↔
//! serde_json::Value marshalling.

use wasm_bindgen::prelude::*;

/// Initialize WASM module — sets up panic hook for better error messages.
///
/// Called automatically when the WASM module loads (`#[wasm_bindgen(start)]`).
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Convert a JSON Schema into an LLM-compatible structured output schema.
///
/// Accepts a JS object (schema) and a JS object (options).
/// Returns a JS object: `{ schema, codec }`.
///
/// **Stub** — full implementation in #56.
#[wasm_bindgen]
pub fn convert(schema: JsValue, options: JsValue) -> Result<JsValue, JsError> {
    let schema: serde_json::Value = serde_wasm_bindgen::from_value(schema)?;
    let options: jsonschema_llm_core::ConvertOptions = serde_wasm_bindgen::from_value(options)?;

    let result = jsonschema_llm_core::convert(&schema, &options)
        .map_err(|e| JsError::new(&e.to_string()))?;

    Ok(serde_wasm_bindgen::to_value(&result)?)
}

/// Rehydrate LLM output back to the original schema shape.
///
/// Accepts a JS object (data) and a JS object (codec).
/// Returns a JS object: `{ data, warnings }`.
///
/// **Stub** — full implementation in #56.
#[wasm_bindgen]
pub fn rehydrate(data: JsValue, codec: JsValue) -> Result<JsValue, JsError> {
    let data: serde_json::Value = serde_wasm_bindgen::from_value(data)?;
    let codec: jsonschema_llm_core::Codec = serde_wasm_bindgen::from_value(codec)?;

    let result =
        jsonschema_llm_core::rehydrate(&data, &codec).map_err(|e| JsError::new(&e.to_string()))?;

    Ok(serde_wasm_bindgen::to_value(&result)?)
}
