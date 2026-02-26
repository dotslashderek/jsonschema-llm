//! WASM bindings for json-schema-llm.
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

use json_schema_llm_core::{
    ConvertError, ConvertOptions, Mode, PolymorphismStrategy, ProviderCompatError, Target,
    API_VERSION,
};

// ---------------------------------------------------------------------------
// WASM-local DTOs (Anti-Corruption Layer)
// ---------------------------------------------------------------------------

fn is_empty_slice<T>(slice_ref: &&[T]) -> bool {
    slice_ref.is_empty()
}

/// WASM envelope for `convert` results.
/// Injects `apiVersion` and flattens the core `ConvertResult`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WasmConvertResult<'a> {
    api_version: &'static str,
    schema: &'a serde_json::Value,
    codec: &'a json_schema_llm_core::Codec,
    #[serde(skip_serializing_if = "is_empty_slice")]
    provider_compat_errors: &'a [ProviderCompatError],
}

/// WASM envelope for `rehydrate` results.
/// Injects `apiVersion` and flattens the core `RehydrateResult`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WasmRehydrateResult<'a> {
    api_version: &'static str,
    data: &'a serde_json::Value,
    warnings: &'a [json_schema_llm_core::Warning],
}

/// WASM-local options DTO accepting camelCase from JS callers.
///
/// Core `ConvertOptions` uses kebab-case serde, but JS consumers
/// naturally use camelCase. This DTO bridges that gap using
/// `#[serde(alias)]` to accept both conventions.
///
/// NOTE: Keep in sync with `json_schema_llm_core::ConvertOptions`.
/// If core adds new options, update this struct to match.
/// Defaults are sourced from `ConvertOptions::default()` (single source of truth).
#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
struct WasmConvertOptions {
    #[serde(alias = "target")]
    target: Option<Target>,
    #[serde(alias = "mode")]
    mode: Option<Mode>,
    #[serde(alias = "max-depth")]
    max_depth: Option<usize>,
    #[serde(alias = "recursion-limit")]
    recursion_limit: Option<usize>,
    #[serde(alias = "polymorphism")]
    polymorphism: Option<PolymorphismStrategy>,
    #[serde(alias = "skip-components")]
    skip_components: Option<bool>,
}

impl From<WasmConvertOptions> for ConvertOptions {
    fn from(wasm: WasmConvertOptions) -> Self {
        let mut opts = ConvertOptions::default();
        if let Some(target) = wasm.target {
            opts.target = target;
        }
        if let Some(mode) = wasm.mode {
            opts.mode = mode;
        }
        if let Some(max_depth) = wasm.max_depth {
            opts.max_depth = max_depth;
        }
        if let Some(recursion_limit) = wasm.recursion_limit {
            opts.recursion_limit = recursion_limit;
        }
        if let Some(polymorphism) = wasm.polymorphism {
            opts.polymorphism = polymorphism;
        }
        if let Some(skip_components) = wasm.skip_components {
            opts.skip_components = skip_components;
        }
        opts
    }
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

/// Map a `ConvertError` to a structured JS object `{ code, message, path }`.
fn to_structured_js_error(e: &ConvertError) -> JsValue {
    let serializer = Serializer::json_compatible();
    e.to_json().serialize(&serializer).unwrap_or_else(|_| {
        let fallback = serde_json::json!({
            "code": "serialization_error",
            "message": e.to_string(),
            "path": serde_json::Value::Null,
        });
        fallback
            .serialize(&serializer)
            .unwrap_or_else(|_| JsValue::from_str(&e.to_string()))
    })
}

/// Map a `serde_wasm_bindgen` deserialization error to `{ code: "json_parse_error", ... }`.
fn to_serde_js_error(e: serde_wasm_bindgen::Error) -> JsValue {
    let error_obj = serde_json::json!({
        "code": "json_parse_error",
        "message": e.to_string(),
        "path": serde_json::Value::Null,
    });
    let serializer = Serializer::json_compatible();
    error_obj.serialize(&serializer).unwrap_or_else(|_| {
        let fallback = serde_json::json!({
            "code": "serialization_error",
            "message": e.to_string(),
            "path": serde_json::Value::Null,
        });
        fallback
            .serialize(&serializer)
            .unwrap_or_else(|_| JsValue::from_str(&e.to_string()))
    })
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
#[wasm_bindgen(skip_typescript)]
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
        json_schema_llm_core::convert(&schema, &options).map_err(|e| to_structured_js_error(&e))?;

    let bridge = WasmConvertResult {
        api_version: API_VERSION,
        schema: &result.schema,
        codec: &result.codec,
        provider_compat_errors: &result.provider_compat_errors,
    };

    let serializer = Serializer::json_compatible();
    bridge.serialize(&serializer).map_err(to_serde_js_error)
}

/// Rehydrate LLM output back to the original schema shape.
///
/// Accepts a JS object (data), a JS object (codec), and the original
/// JSON Schema (for type coercion).
/// Returns a JS object: `{ apiVersion: "1.0", data, warnings }`.
///
/// On error, throws a structured JS object `{ code, message, path }`.
#[wasm_bindgen(skip_typescript)]
pub fn rehydrate(
    data: JsValue,
    codec: JsValue,
    original_schema: JsValue,
) -> Result<JsValue, JsValue> {
    let data: serde_json::Value =
        serde_wasm_bindgen::from_value(data).map_err(to_serde_js_error)?;
    let codec: json_schema_llm_core::Codec =
        serde_wasm_bindgen::from_value(codec).map_err(to_serde_js_error)?;
    let original_schema: serde_json::Value =
        serde_wasm_bindgen::from_value(original_schema).map_err(to_serde_js_error)?;

    let result = json_schema_llm_core::rehydrate(&data, &codec, &original_schema)
        .map_err(|e| to_structured_js_error(&e))?;

    let bridge = WasmRehydrateResult {
        api_version: API_VERSION,
        data: &result.data,
        warnings: &result.warnings,
    };

    let serializer = Serializer::json_compatible();
    bridge.serialize(&serializer).map_err(to_serde_js_error)
}

// ⚠️ SYNC WARNING: These TypeScript types are hand-authored to match the
// serialized JS shapes produced by serde + Serializer::json_compatible().
// If you modify any of these Rust types, you MUST update the corresponding
// TypeScript definitions below:
//
//   - WasmConvertOptions (this file)      → ConvertOptions
//   - WasmConvertResult (this file)       → ConvertResult
//   - WasmRehydrateResult (this file)     → RehydrateResult
//   - Target (config.rs)                  → Target
//   - PolymorphismStrategy (config.rs)    → PolymorphismStrategy
//   - Codec (codec.rs)                    → Codec
//   - Transform (codec.rs)               → Transform
//   - DroppedConstraint (codec.rs)        → DroppedConstraint
//   - Warning (codec_warning.rs)          → Warning
//   - WarningKind (codec_warning.rs)      → WarningKind
//   - ErrorCode (error.rs)               → ErrorCode
//   - ConvertError.to_json (error.rs)     → StructuredError
#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &str = r#"
export type Target = "openai-strict" | "gemini" | "claude";
export type Mode = "strict" | "permissive";
export type PolymorphismStrategy = "any-of" | "flatten";

export interface ConvertOptions {
  target?: Target;
  mode?: Mode;
  maxDepth?: number;
  recursionLimit?: number;
  polymorphism?: PolymorphismStrategy;
  skipComponents?: boolean;
}

export interface Codec {
  $schema: string;
  transforms: Transform[];
  droppedConstraints: DroppedConstraint[];
}

export type Transform =
  | { type: "map_to_array"; path: string; keyField: string }
  | { type: "json_string_parse"; path: string }
  | { type: "nullable_optional"; path: string; originalRequired: boolean }
  | { type: "discriminator_any_of"; path: string; discriminator: string; variants: string[] }
  | { type: "extract_additional_properties"; path: string; propertyName: string }
  | { type: "recursive_inflate"; path: string; originalRef: string };

export interface DroppedConstraint {
  path: string;
  constraint: string;
  value: unknown;
}

export type ProviderCompatError =
  | { type: "root_type_incompatible"; actual_type: string; target: Target; hint: string }
  | { type: "depth_budget_exceeded"; actual_depth: number; max_depth: number; target: Target; hint: string }
  | { type: "mixed_enum_types"; path: string; types_found: string[]; target: Target; hint: string }
  | { type: "unconstrained_schema"; path: string; schema_kind: string; target: Target; hint: string }
  | { type: "pattern_properties_stripped"; path: string; target: Target; hint: string }
  | { type: "pattern_properties_stringified"; path: string; target: Target; hint: string };

export interface ConvertResult {
  apiVersion: string;
  schema: Record<string, unknown>;
  codec: Codec;
  providerCompatErrors?: ProviderCompatError[];
}

export interface RehydrateResult {
  apiVersion: string;
  data: unknown;
  warnings: Warning[];
}

export type WarningKind =
  | { type: "constraint_violation"; constraint: string }
  | { type: "constraint_unevaluable"; constraint: string }
  | { type: "path_not_found" };

export interface Warning {
  dataPath: string;
  schemaPath: string;
  kind: WarningKind;
  message: string;
}

export type ErrorCode =
  | "json_parse_error"
  | "schema_error"
  | "recursion_depth_exceeded"
  | "unsupported_feature"
  | "unresolvable_ref"
  | "rehydration_error"
  | "codec_version_mismatch"
  | "provider_compat_failure";

export interface StructuredError {
  code: ErrorCode;
  message: string;
  path: string | null;
}

export function convert(
  schema: Record<string, unknown> | boolean,
  options?: ConvertOptions | null
): ConvertResult;

export function rehydrate(
  data: unknown,
  codec: Codec,
  originalSchema: Record<string, unknown> | boolean
): RehydrateResult;
"#;
