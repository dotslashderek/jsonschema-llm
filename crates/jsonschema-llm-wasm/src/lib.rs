//! WASM bindings for jsonschema-llm.
//!
//! Exposes `convert` and `rehydrate` via `wasm-bindgen` for use from
//! JavaScript / TypeScript. Uses `serde-wasm-bindgen` for JS ↔
//! serde_json::Value marshalling.
//!
//! ## WASM API Contract
//!
//! - Results are wrapped in an `apiVersion: "1.0"` envelope.
//! - Errors are structured JS objects `{ code, message, path }`.
//! - The `options` parameter defaults to `ConvertOptions::default()` when omitted.
//! - Options accept **camelCase** (`maxDepth`) with kebab-case fallback (`max-depth`).

use serde::Serialize;
use wasm_bindgen::prelude::*;

use serde_wasm_bindgen::Serializer;

use jsonschema_llm_core::{
    ConvertError, ConvertOptions, PolymorphismStrategy, Target, API_VERSION,
};

// ---------------------------------------------------------------------------
// WASM-local DTOs (Anti-Corruption Layer)
// ---------------------------------------------------------------------------

/// WASM envelope for `convert` results.
/// Injects `apiVersion` and flattens the core `ConvertResult`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WasmConvertResult<'a> {
    api_version: &'static str,
    schema: &'a serde_json::Value,
    codec: &'a jsonschema_llm_core::Codec,
}

/// WASM envelope for `rehydrate` results.
/// Injects `apiVersion` and flattens the core `RehydrateResult`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WasmRehydrateResult<'a> {
    api_version: &'static str,
    data: &'a serde_json::Value,
    warnings: &'a [jsonschema_llm_core::Warning],
}

/// WASM-local options DTO accepting camelCase from JS callers.
///
/// Core `ConvertOptions` uses kebab-case serde, but JS consumers
/// naturally use camelCase. This DTO bridges that gap using
/// `#[serde(alias)]` to accept both conventions.
///
/// NOTE: Keep in sync with `jsonschema_llm_core::ConvertOptions`.
/// If core adds new options, update this struct to match.
/// Defaults are sourced from `ConvertOptions::default()` (single source of truth).
#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
struct WasmConvertOptions {
    #[serde(alias = "target")]
    target: Option<Target>,
    #[serde(alias = "max-depth")]
    max_depth: Option<usize>,
    #[serde(alias = "recursion-limit")]
    recursion_limit: Option<usize>,
    #[serde(alias = "polymorphism")]
    polymorphism: Option<PolymorphismStrategy>,
}

impl From<WasmConvertOptions> for ConvertOptions {
    fn from(wasm: WasmConvertOptions) -> Self {
        let defaults = ConvertOptions::default();
        ConvertOptions {
            target: wasm.target.unwrap_or(defaults.target),
            max_depth: wasm.max_depth.unwrap_or(defaults.max_depth),
            recursion_limit: wasm.recursion_limit.unwrap_or(defaults.recursion_limit),
            polymorphism: wasm.polymorphism.unwrap_or(defaults.polymorphism),
        }
    }
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

/// Map a `ConvertError` to a structured JS object `{ code, message, path }`.
fn to_structured_js_error(e: &ConvertError) -> JsValue {
    let serializer = Serializer::json_compatible();
    e.to_json()
        .serialize(&serializer)
        .unwrap_or_else(|_| JsValue::from_str(&e.to_string()))
}

/// Map a `serde_wasm_bindgen` deserialization error to `{ code: "json_parse_error", ... }`.
fn to_serde_js_error(e: serde_wasm_bindgen::Error) -> JsValue {
    let error_obj = serde_json::json!({
        "code": "json_parse_error",
        "message": e.to_string(),
        "path": serde_json::Value::Null,
    });
    let serializer = Serializer::json_compatible();
    error_obj
        .serialize(&serializer)
        .unwrap_or_else(|_| JsValue::from_str(&e.to_string()))
}

// ---------------------------------------------------------------------------
// Public WASM API
// ---------------------------------------------------------------------------

/// Initialize WASM module — sets up panic hook for better error messages.
///
/// Called automatically when the WASM module loads (`#[wasm_bindgen(start)]`).
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Convert a JSON Schema into an LLM-compatible structured output schema.
///
/// Accepts a JS object (schema) and an optional JS object (options).
/// Returns a JS object: `{ apiVersion: "1.0", schema, codec }`.
///
/// Options accept camelCase (`maxDepth`) or kebab-case (`max-depth`).
/// If options is `undefined` or `null`, defaults are used.
///
/// On error, throws a structured JS object `{ code, message, path }`.
#[wasm_bindgen]
pub fn convert(schema: JsValue, options: JsValue) -> Result<JsValue, JsValue> {
    let schema: serde_json::Value =
        serde_wasm_bindgen::from_value(schema).map_err(to_serde_js_error)?;

    let options: ConvertOptions = if options.is_undefined() || options.is_null() {
        ConvertOptions::default()
    } else {
        let wasm_opts: WasmConvertOptions =
            serde_wasm_bindgen::from_value(options).map_err(to_serde_js_error)?;
        wasm_opts.into()
    };

    let result =
        jsonschema_llm_core::convert(&schema, &options).map_err(|e| to_structured_js_error(&e))?;

    let bridge = WasmConvertResult {
        api_version: API_VERSION,
        schema: &result.schema,
        codec: &result.codec,
    };

    let serializer = Serializer::json_compatible();
    bridge.serialize(&serializer).map_err(to_serde_js_error)
}

/// Rehydrate LLM output back to the original schema shape.
///
/// Accepts a JS object (data) and a JS object (codec).
/// Returns a JS object: `{ apiVersion: "1.0", data, warnings }`.
///
/// On error, throws a structured JS object `{ code, message, path }`.
#[wasm_bindgen]
pub fn rehydrate(data: JsValue, codec: JsValue) -> Result<JsValue, JsValue> {
    let data: serde_json::Value =
        serde_wasm_bindgen::from_value(data).map_err(to_serde_js_error)?;
    let codec: jsonschema_llm_core::Codec =
        serde_wasm_bindgen::from_value(codec).map_err(to_serde_js_error)?;

    let result =
        jsonschema_llm_core::rehydrate(&data, &codec).map_err(|e| to_structured_js_error(&e))?;

    let bridge = WasmRehydrateResult {
        api_version: API_VERSION,
        data: &result.data,
        warnings: &result.warnings,
    };

    let serializer = Serializer::json_compatible();
    bridge.serialize(&serializer).map_err(to_serde_js_error)
}
