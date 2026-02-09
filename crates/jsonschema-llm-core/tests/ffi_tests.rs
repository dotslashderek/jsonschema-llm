//! FFI Facade tests — Error codes (#51) and JSON-string bridge (#53).
//!
//! These tests are written FIRST (TDD gate) before any implementation.

use jsonschema_llm_core::{
    convert, convert_json, rehydrate_json, ConvertError, ConvertOptions, ErrorCode,
};

// ---------------------------------------------------------------------------
// Issue #51 — Error Code Mapping
// ---------------------------------------------------------------------------

#[test]
fn test_error_code_json_parse() {
    let err = ConvertError::JsonError(serde_json::from_str::<serde_json::Value>("{{").unwrap_err());
    assert_eq!(err.error_code(), ErrorCode::JsonParseError);
}

#[test]
fn test_error_code_schema_error() {
    let err = ConvertError::SchemaError {
        path: "#/properties/name".to_string(),
        message: "invalid type".to_string(),
    };
    assert_eq!(err.error_code(), ErrorCode::SchemaError);
}

#[test]
fn test_error_code_recursion_exceeded() {
    let err = ConvertError::RecursionDepthExceeded {
        path: "#/definitions/Node".to_string(),
        max_depth: 50,
    };
    assert_eq!(err.error_code(), ErrorCode::RecursionDepthExceeded);
}

#[test]
fn test_error_code_unsupported_feature() {
    let err = ConvertError::UnsupportedFeature {
        path: "#/properties/data".to_string(),
        feature: "contentEncoding".to_string(),
    };
    assert_eq!(err.error_code(), ErrorCode::UnsupportedFeature);
}

#[test]
fn test_error_code_unresolvable_ref() {
    let err = ConvertError::UnresolvableRef {
        path: "#/properties/link".to_string(),
        reference: "#/definitions/Missing".to_string(),
    };
    assert_eq!(err.error_code(), ErrorCode::UnresolvableRef);
}

#[test]
fn test_error_code_rehydration() {
    let err = ConvertError::RehydrationError("parse failed".to_string());
    assert_eq!(err.error_code(), ErrorCode::RehydrationError);
}

// ---------------------------------------------------------------------------
// Issue #51 — Structured Error JSON
// ---------------------------------------------------------------------------

#[test]
fn test_error_to_json_structure() {
    let err = ConvertError::SchemaError {
        path: "#/properties/name".to_string(),
        message: "invalid type".to_string(),
    };
    let json = err.to_json();
    assert!(json.get("code").unwrap().is_string());
    assert!(json.get("message").unwrap().is_string());
    // path should be present (not null) for SchemaError
    assert!(json.get("path").unwrap().is_string());
    assert_eq!(json["code"].as_str().unwrap(), "schema_error");
}

#[test]
fn test_error_to_json_null_path() {
    // JsonError and RehydrationError have no path context
    let err = ConvertError::JsonError(serde_json::from_str::<serde_json::Value>("{{").unwrap_err());
    let json = err.to_json();
    assert_eq!(json["code"].as_str().unwrap(), "json_parse_error");
    assert!(json.get("path").unwrap().is_null());
}

#[test]
fn test_error_code_serde_roundtrip() {
    let code = ErrorCode::RecursionDepthExceeded;
    let json = serde_json::to_string(&code).unwrap();
    assert_eq!(json, "\"recursion_depth_exceeded\"");
    let deserialized: ErrorCode = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, code);
}

/// Stability contract: all 6 codes must serialize to exact expected strings.
/// If this test breaks, it means a stable API contract was violated.
#[test]
fn test_error_code_stability() {
    let expected = vec![
        (ErrorCode::JsonParseError, "json_parse_error"),
        (ErrorCode::SchemaError, "schema_error"),
        (
            ErrorCode::RecursionDepthExceeded,
            "recursion_depth_exceeded",
        ),
        (ErrorCode::UnsupportedFeature, "unsupported_feature"),
        (ErrorCode::UnresolvableRef, "unresolvable_ref"),
        (ErrorCode::RehydrationError, "rehydration_error"),
    ];

    for (code, expected_str) in &expected {
        let serialized = serde_json::to_value(code).unwrap();
        assert_eq!(
            serialized.as_str().unwrap(),
            *expected_str,
            "ErrorCode::{:?} serialized to {:?}, expected {:?}",
            code,
            serialized,
            expected_str
        );
    }
}

// ---------------------------------------------------------------------------
// Issue #53 — JSON-String Bridge: convert_json
// ---------------------------------------------------------------------------

#[test]
fn test_convert_json_valid() {
    let schema =
        r#"{"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}"#;
    let options = r#"{"target": "openai-strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#;

    let result = convert_json(schema, options);
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result);

    let parsed: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert!(parsed.get("schema").is_some(), "Missing 'schema' key");
    assert!(parsed.get("codec").is_some(), "Missing 'codec' key");
}

#[test]
fn test_convert_json_invalid_schema() {
    let result = convert_json(
        "not valid json",
        r#"{"target": "openai-strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#,
    );
    assert!(result.is_err());

    let err_json: serde_json::Value = serde_json::from_str(&result.unwrap_err()).unwrap();
    assert_eq!(err_json["code"].as_str().unwrap(), "json_parse_error");
    assert!(err_json.get("message").is_some());
}

#[test]
fn test_convert_json_invalid_options() {
    let schema = r#"{"type": "object"}"#;
    let result = convert_json(schema, "not valid json");
    assert!(result.is_err());

    let err_json: serde_json::Value = serde_json::from_str(&result.unwrap_err()).unwrap();
    assert_eq!(err_json["code"].as_str().unwrap(), "json_parse_error");
}

// ---------------------------------------------------------------------------
// Issue #53 — JSON-String Bridge: rehydrate_json
// ---------------------------------------------------------------------------

#[test]
fn test_rehydrate_json_valid() {
    let data = r#"{"name": "Alice"}"#;
    let codec = r#"{"$schema": "https://jsonschema-llm.dev/codec/v1", "transforms": [], "dropped_constraints": []}"#;

    let result = rehydrate_json(data, codec);
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result);

    let parsed: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert!(parsed.get("data").is_some(), "Missing 'data' key");
    assert!(parsed.get("warnings").is_some(), "Missing 'warnings' key");
}

#[test]
fn test_rehydrate_json_invalid_data() {
    let codec = r#"{"$schema": "https://jsonschema-llm.dev/codec/v1", "transforms": [], "dropped_constraints": []}"#;
    let result = rehydrate_json("not valid json", codec);
    assert!(result.is_err());

    let err_json: serde_json::Value = serde_json::from_str(&result.unwrap_err()).unwrap();
    assert_eq!(err_json["code"].as_str().unwrap(), "json_parse_error");
}

#[test]
fn test_rehydrate_json_invalid_codec() {
    let data = r#"{"name": "Alice"}"#;
    let result = rehydrate_json(data, "not valid json");
    assert!(result.is_err());

    let err_json: serde_json::Value = serde_json::from_str(&result.unwrap_err()).unwrap();
    assert_eq!(err_json["code"].as_str().unwrap(), "json_parse_error");
}

// ---------------------------------------------------------------------------
// Issue #53 — Full Roundtrip
// ---------------------------------------------------------------------------

#[test]
fn test_roundtrip_json_bridge() {
    let schema = r#"{"type": "object", "properties": {"name": {"type": "string"}, "age": {"type": "integer"}}, "required": ["name"]}"#;
    let options = r#"{"target": "openai-strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#;

    // Step 1: Convert via JSON bridge
    let convert_result = convert_json(schema, options).expect("convert_json should succeed");
    let convert_parsed: serde_json::Value = serde_json::from_str(&convert_result).unwrap();

    // Step 2: Extract codec from conversion result
    let codec_json = serde_json::to_string(&convert_parsed["codec"]).unwrap();

    // Step 3: Create some LLM-like output data
    let llm_output = r#"{"name": "Ada", "age": 36}"#;

    // Step 4: Rehydrate via JSON bridge
    let rehydrate_result =
        rehydrate_json(llm_output, &codec_json).expect("rehydrate_json should succeed");
    let rehydrate_parsed: serde_json::Value = serde_json::from_str(&rehydrate_result).unwrap();

    // Verify rehydrated data matches original
    assert_eq!(rehydrate_parsed["data"]["name"], "Ada");
    assert_eq!(rehydrate_parsed["data"]["age"], 36);
}

// ---------------------------------------------------------------------------
// Issue #53 — Malformed JSON Corpus
// ---------------------------------------------------------------------------

#[test]
fn test_convert_json_malformed_corpus() {
    let malformed_inputs = vec![
        "",               // empty string
        "{",              // incomplete
        "null",           // valid JSON but not an object
        "[1, 2, 3]",      // array, not object
        "\"just a str\"", // string, not object
    ];
    let valid_options = r#"{"target": "openai-strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#;

    for input in malformed_inputs {
        let result = convert_json(input, valid_options);
        // All should either error at parse or at schema validation — never panic
        if let Err(err_str) = &result {
            // Error should be valid JSON
            let err_json: serde_json::Value = serde_json::from_str(err_str)
                .unwrap_or_else(|_| panic!("Error string is not valid JSON: {}", err_str));
            assert!(err_json.get("code").is_some(), "Missing 'code' in error");
            assert!(
                err_json.get("message").is_some(),
                "Missing 'message' in error"
            );
        }
        // Ok is also acceptable for valid-but-unusual JSON schemas
    }
}

// ---------------------------------------------------------------------------
// Codex review — Semantic deserialization error coverage
// ---------------------------------------------------------------------------

/// Valid JSON but missing required fields → error must still be structured JSON
#[test]
fn test_convert_json_missing_options_fields() {
    let schema = r#"{"type": "object"}"#;
    // Valid JSON object, but missing required fields for ConvertOptions
    let result = convert_json(schema, "{}");
    assert!(result.is_err());

    let err_json: serde_json::Value =
        serde_json::from_str(&result.unwrap_err()).expect("Error string must be valid JSON");
    assert_eq!(err_json["code"].as_str().unwrap(), "json_parse_error");
    assert!(err_json.get("message").is_some());
}

/// Valid JSON but invalid enum value → error must still be structured JSON
#[test]
fn test_convert_json_invalid_target_enum() {
    let schema = r#"{"type": "object"}"#;
    let options = r#"{"target": "nonexistent-provider", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#;
    let result = convert_json(schema, options);
    assert!(result.is_err());

    let err_json: serde_json::Value =
        serde_json::from_str(&result.unwrap_err()).expect("Error string must be valid JSON");
    assert_eq!(err_json["code"].as_str().unwrap(), "json_parse_error");
}

/// Valid JSON but invalid codec shape → error must still be structured JSON
#[test]
fn test_rehydrate_json_wrong_codec_shape() {
    let data = r#"{"name": "Alice"}"#;
    // Valid JSON object but not a valid Codec structure
    let result = rehydrate_json(data, "{}");
    assert!(result.is_err());

    let err_json: serde_json::Value =
        serde_json::from_str(&result.unwrap_err()).expect("Error string must be valid JSON");
    assert_eq!(err_json["code"].as_str().unwrap(), "json_parse_error");
}
