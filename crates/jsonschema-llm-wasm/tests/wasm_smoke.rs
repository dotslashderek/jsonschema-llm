//! In-crate smoke tests for the WASM boundary layer.
//!
//! These run under `wasm32-unknown-unknown` via `wasm-pack test --node`
//! and validate the WASM API contract at the JsValue level.

use serde::Serialize;
use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_node_experimental);

use jsonschema_llm_wasm::{convert, rehydrate};

// ---------------------------------------------------------------------------
// Helper: parse a JsValue as a serde_json::Value for assertions
// ---------------------------------------------------------------------------

fn js_to_json(val: &JsValue) -> serde_json::Value {
    serde_wasm_bindgen::from_value(val.clone()).expect("JsValue → serde_json::Value")
}

fn schema_js() -> JsValue {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "integer" }
        },
        "required": ["name", "age"]
    });
    serde_wasm_bindgen::to_value(&schema).unwrap()
}

fn options_js(target: &str) -> JsValue {
    let opts = serde_json::json!({ "target": target });
    serde_wasm_bindgen::to_value(&opts).unwrap()
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn test_convert_returns_envelope() {
    let result = convert(schema_js(), options_js("openai-strict")).unwrap();
    let json = js_to_json(&result);

    assert_eq!(
        json["apiVersion"], "1.0",
        "envelope must include apiVersion"
    );
    assert!(
        json["schema"].is_object(),
        "envelope must include schema object"
    );
    assert!(
        json["codec"].is_object(),
        "envelope must include codec object"
    );
}

#[wasm_bindgen_test]
fn test_convert_with_default_options() {
    // options = undefined → should use ConvertOptions::default()
    let result = convert(schema_js(), JsValue::UNDEFINED).unwrap();
    let json = js_to_json(&result);

    assert_eq!(json["apiVersion"], "1.0");
    assert!(json["schema"].is_object());
}

#[wasm_bindgen_test]
fn test_convert_with_null_options() {
    let result = convert(schema_js(), JsValue::NULL).unwrap();
    let json = js_to_json(&result);

    assert_eq!(json["apiVersion"], "1.0");
    assert!(json["schema"].is_object());
}

#[wasm_bindgen_test]
fn test_convert_camel_case_options() {
    // JS callers naturally use camelCase
    let opts = serde_json::json!({
        "target": "openai-strict",
        "maxDepth": 10,
        "recursionLimit": 2
    });
    let result = convert(schema_js(), serde_wasm_bindgen::to_value(&opts).unwrap()).unwrap();
    let json = js_to_json(&result);

    assert_eq!(json["apiVersion"], "1.0");
}

#[wasm_bindgen_test]
fn test_convert_all_targets() {
    for target in &["openai-strict", "gemini", "claude"] {
        let result = convert(schema_js(), options_js(target)).unwrap();
        let json = js_to_json(&result);
        assert_eq!(json["apiVersion"], "1.0", "target={target}");
        assert!(json["schema"].is_object(), "target={target} schema");
        assert!(json["codec"].is_object(), "target={target} codec");
    }
}

#[wasm_bindgen_test]
fn test_rehydrate_round_trip() {
    // Convert → get codec → rehydrate sample data
    let result = convert(schema_js(), JsValue::UNDEFINED).unwrap();
    let result_json = js_to_json(&result);

    // Use json_compatible serializer — same as production code produces Objects not Maps
    let serializer = serde_wasm_bindgen::Serializer::json_compatible();
    let codec_js = result_json["codec"].serialize(&serializer).unwrap();

    let sample_data = serde_json::json!({ "name": "Alice", "age": 30 });
    let data_js = serde_wasm_bindgen::to_value(&sample_data).unwrap();

    let rehydrate_result = rehydrate(data_js, codec_js).unwrap();
    let rehydrate_json = js_to_json(&rehydrate_result);

    assert_eq!(rehydrate_json["apiVersion"], "1.0", "rehydrate envelope");
    assert!(rehydrate_json["data"].is_object(), "rehydrate data");
    assert!(rehydrate_json["warnings"].is_array(), "rehydrate warnings");
}

// ---------------------------------------------------------------------------
// Error paths
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn test_convert_serde_error_for_bad_options() {
    // Pass a number as options → can't deserialize as WasmConvertOptions struct
    let bad_opts = JsValue::from_f64(42.0);
    let err = convert(schema_js(), bad_opts).unwrap_err();
    let err_json = js_to_json(&err);

    assert_eq!(err_json["code"], "json_parse_error");
    assert!(err_json["message"].is_string());
}

#[wasm_bindgen_test]
fn test_rehydrate_serde_error_for_bad_codec() {
    // Pass a number as codec → can't deserialize as Codec struct
    let data = serde_wasm_bindgen::to_value(&serde_json::json!({"name": "test"})).unwrap();
    let bad_codec = JsValue::from_f64(42.0);
    let err = rehydrate(data, bad_codec).unwrap_err();
    let err_json = js_to_json(&err);

    assert_eq!(err_json["code"], "json_parse_error");
    assert!(err_json["message"].is_string());
}
