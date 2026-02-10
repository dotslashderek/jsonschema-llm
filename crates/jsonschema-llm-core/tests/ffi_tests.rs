//! FFI Facade tests — Error codes (#51) and JSON-string bridge (#53).
//!
//! These tests are written FIRST (TDD gate) before any implementation.

use jsonschema_llm_core::{convert_json, rehydrate_json, ConvertError, ErrorCode};

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
    let codec = r#"{"$schema": "https://jsonschema-llm.dev/codec/v1", "transforms": [], "droppedConstraints": []}"#;

    let result = rehydrate_json(data, codec);
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result);

    let parsed: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert!(parsed.get("data").is_some(), "Missing 'data' key");
    assert!(parsed.get("warnings").is_some(), "Missing 'warnings' key");
}

#[test]
fn test_rehydrate_json_invalid_data() {
    let codec = r#"{"$schema": "https://jsonschema-llm.dev/codec/v1", "transforms": [], "droppedConstraints": []}"#;
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

// ===========================================================================
// Issue #54 — Binding DTOs + API Versioning (Acceptance Tests)
// ===========================================================================

/// Bridge convert output must include apiVersion field.
#[test]
fn test_convert_json_has_api_version() {
    let schema =
        r#"{"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}"#;
    let options = r#"{"target": "openai-strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#;

    let result = convert_json(schema, options).expect("convert_json should succeed");
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert!(
        parsed.get("apiVersion").is_some(),
        "Bridge output must include 'apiVersion' field"
    );
    assert_eq!(
        parsed["apiVersion"].as_str().unwrap(),
        "1.0",
        "apiVersion must be '1.0'"
    );
}

/// Bridge rehydrate output must include apiVersion field.
#[test]
fn test_rehydrate_json_has_api_version() {
    let data = r#"{"name": "Alice"}"#;
    let codec = r#"{"$schema": "https://jsonschema-llm.dev/codec/v1", "transforms": [], "droppedConstraints": []}"#;

    let result = rehydrate_json(data, codec).expect("rehydrate_json should succeed");
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert!(
        parsed.get("apiVersion").is_some(),
        "Bridge output must include 'apiVersion' field"
    );
    assert_eq!(
        parsed["apiVersion"].as_str().unwrap(),
        "1.0",
        "apiVersion must be '1.0'"
    );
}

/// Stability contract: apiVersion string must be exactly "1.0".
#[test]
fn test_api_version_stability() {
    use jsonschema_llm_core::API_VERSION;
    assert_eq!(API_VERSION, "1.0", "API_VERSION constant must be '1.0'");
}

/// Codec must serialize with camelCase field names.
#[test]
fn test_codec_camel_case_serialization() {
    use jsonschema_llm_core::Codec;
    let codec = Codec::new();
    let json = serde_json::to_string(&codec).unwrap();

    assert!(
        json.contains("\"droppedConstraints\""),
        "Codec field 'dropped_constraints' must serialize as 'droppedConstraints', got: {}",
        json
    );
    assert!(
        !json.contains("\"dropped_constraints\""),
        "Codec must NOT contain snake_case 'dropped_constraints', got: {}",
        json
    );
}

/// Warning must serialize with camelCase field names.
#[test]
fn test_warning_camel_case_serialization() {
    use jsonschema_llm_core::codec_warning::WarningKind;
    use jsonschema_llm_core::Warning;

    let warning = Warning {
        data_path: "/foo".to_string(),
        schema_path: "#/properties/foo".to_string(),
        kind: WarningKind::ConstraintViolation {
            constraint: "minimum".to_string(),
        },
        message: "test".to_string(),
    };
    let json = serde_json::to_string(&warning).unwrap();

    assert!(
        json.contains("\"dataPath\""),
        "Warning field 'data_path' must serialize as 'dataPath', got: {}",
        json
    );
    assert!(
        json.contains("\"schemaPath\""),
        "Warning field 'schema_path' must serialize as 'schemaPath', got: {}",
        json
    );
}

/// Codec version mismatch must produce structured error with specific code.
#[test]
fn test_codec_version_mismatch() {
    let data = r#"{"name": "Alice"}"#;
    // Use a v99 codec version that is incompatible
    let codec = r#"{"$schema": "https://jsonschema-llm.dev/codec/v99", "transforms": [], "droppedConstraints": []}"#;

    let result = rehydrate_json(data, codec);
    assert!(result.is_err(), "Incompatible codec version must fail");

    let err: serde_json::Value = serde_json::from_str(&result.unwrap_err()).unwrap();
    assert_eq!(
        err["code"].as_str().unwrap(),
        "codec_version_mismatch",
        "Error code must be 'codec_version_mismatch'"
    );
}

/// Codec with malformed $schema URI must produce error, not panic.
#[test]
fn test_codec_version_malformed_uri() {
    let data = r#"{"name": "Alice"}"#;
    let codec = r#"{"$schema": "not-a-valid-uri", "transforms": [], "droppedConstraints": []}"#;

    let result = rehydrate_json(data, codec);
    assert!(result.is_err(), "Malformed codec URI must fail");

    let err: serde_json::Value = serde_json::from_str(&result.unwrap_err()).unwrap();
    assert!(err.get("code").is_some(), "Error must have 'code' field");
}

/// ErrorCode::CodecVersionMismatch must serialize to exact string.
#[test]
fn test_error_code_codec_version_mismatch_stability() {
    let code = ErrorCode::CodecVersionMismatch;
    let json = serde_json::to_value(code).unwrap();
    assert_eq!(
        json.as_str().unwrap(),
        "codec_version_mismatch",
        "ErrorCode::CodecVersionMismatch must serialize to 'codec_version_mismatch'"
    );
}

/// Bridge convert output fields must be camelCase (schema, codec stay as-is since single-word).
#[test]
fn test_convert_bridge_camel_case_codec_fields() {
    let schema =
        r#"{"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}"#;
    let options = r#"{"target": "openai-strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#;

    let result = convert_json(schema, options).expect("convert_json should succeed");

    // The codec inside the bridge output must use camelCase
    assert!(
        result.contains("\"droppedConstraints\""),
        "Codec in bridge output must use camelCase 'droppedConstraints', got: {}",
        result
    );
    assert!(
        !result.contains("\"dropped_constraints\""),
        "Codec in bridge output must NOT use snake_case 'dropped_constraints'"
    );
}

// ===========================================================================
// Edge-Case Coverage — FFI Hardening (#37 epic closure)
// ===========================================================================

/// Default options via JSON bridge — `"{}"` should fail because `target` is required.
/// This documents the contract: consumers MUST provide at least `target`.
#[test]
fn test_convert_json_default_options_requires_target() {
    let schema =
        r#"{"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}"#;
    // ConvertOptions requires `target` — empty object should fail deserialization
    let result = convert_json(schema, "{}");
    assert!(
        result.is_err(),
        "Empty options must fail (target is required)"
    );

    let err: serde_json::Value = serde_json::from_str(&result.unwrap_err()).unwrap();
    assert_eq!(err["code"].as_str().unwrap(), "json_parse_error");
}

/// Panic-safety: empty schema `{}` (valid Draft 2020-12, means "accept anything").
/// Contract: no panic; if Ok, must have apiVersion; if Err, must be structured JSON.
/// This does NOT assert conversion succeeds — empty schema support is best-effort.
#[test]
fn test_convert_json_empty_schema_no_panic() {
    let options = r#"{"target": "openai-strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#;
    let result = convert_json("{}", options);
    // Either Ok or a structured error — never panic
    match result {
        Ok(json_str) => {
            let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
            assert!(parsed.get("apiVersion").is_some());
        }
        Err(err_str) => {
            let err: serde_json::Value = serde_json::from_str(&err_str)
                .unwrap_or_else(|_| panic!("Error must be valid JSON: {}", err_str));
            assert!(err.get("code").is_some());
        }
    }
}

/// Panic-safety: boolean schemas (`true`/`false`) are valid Draft 2020-12.
/// Contract: no panic; if Ok, must have apiVersion; if Err, must be structured JSON.
/// This does NOT assert conversion succeeds — boolean schema support is best-effort.
#[test]
fn test_convert_json_boolean_schemas_no_panic() {
    let options = r#"{"target": "openai-strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#;

    for schema in &["true", "false"] {
        let result = convert_json(schema, options);
        // Either Ok or a structured error — never panic
        match result {
            Ok(json_str) => {
                let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
                assert!(parsed.get("apiVersion").is_some());
            }
            Err(err_str) => {
                let err: serde_json::Value = serde_json::from_str(&err_str).unwrap_or_else(|_| {
                    panic!(
                        "Error must be valid JSON for schema={}: {}",
                        schema, err_str
                    )
                });
                assert!(err.get("code").is_some());
            }
        }
    }
}

/// Complex roundtrip through JSON bridge — map→array transform exercised via strings.
#[test]
fn test_roundtrip_json_bridge_with_map_transform() {
    let schema = r#"{
        "type": "object",
        "properties": {
            "tags": {
                "type": "object",
                "additionalProperties": { "type": "string" }
            }
        },
        "required": ["tags"]
    }"#;
    let options = r#"{"target": "openai-strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#;

    // Step 1: Convert
    let convert_result = convert_json(schema, options).expect("convert_json should succeed");
    let convert_parsed: serde_json::Value = serde_json::from_str(&convert_result).unwrap();

    // Verify tags was transpiled to array
    assert_eq!(
        convert_parsed["schema"]["properties"]["tags"]["type"], "array",
        "tags should be transpiled to array for OpenAI strict"
    );

    // Step 2: Extract codec
    let codec_json = serde_json::to_string(&convert_parsed["codec"]).unwrap();

    // Step 3: Simulate LLM output with array-of-kv format
    let llm_output =
        r#"{"tags": [{"key": "env", "value": "prod"}, {"key": "team", "value": "backend"}]}"#;

    // Step 4: Rehydrate
    let rehydrate_result =
        rehydrate_json(llm_output, &codec_json).expect("rehydrate_json should succeed");
    let rehydrate_parsed: serde_json::Value = serde_json::from_str(&rehydrate_result).unwrap();

    // Verify map was restored
    assert_eq!(rehydrate_parsed["data"]["tags"]["env"], "prod");
    assert_eq!(rehydrate_parsed["data"]["tags"]["team"], "backend");
}

/// ConvertResult must be deserializable from its own serialized JSON.
#[test]
fn test_convert_result_deserialize_roundtrip() {
    use jsonschema_llm_core::{convert, ConvertOptions, ConvertResult};

    let schema = serde_json::json!({
        "type": "object",
        "properties": { "name": { "type": "string" } },
        "required": ["name"]
    });
    let result = convert(&schema, &ConvertOptions::default()).unwrap();

    // Serialize to JSON string
    let json_str = serde_json::to_string(&result).unwrap();

    // Deserialize back to ConvertResult
    let deserialized: ConvertResult = serde_json::from_str(&json_str)
        .expect("ConvertResult must be deserializable from its own serialized output");

    assert_eq!(deserialized.schema, result.schema);
    assert_eq!(
        serde_json::to_value(&deserialized.codec).unwrap(),
        serde_json::to_value(&result.codec).unwrap()
    );
}

/// Every ConvertError variant must produce valid structured JSON with {code, message, path}.
#[test]
fn test_error_json_all_variants_shape() {
    let errors: Vec<ConvertError> = vec![
        ConvertError::JsonError(serde_json::from_str::<serde_json::Value>("{{").unwrap_err()),
        ConvertError::SchemaError {
            path: "#/test".to_string(),
            message: "test error".to_string(),
        },
        ConvertError::RecursionDepthExceeded {
            path: "#/test".to_string(),
            max_depth: 50,
        },
        ConvertError::UnsupportedFeature {
            path: "#/test".to_string(),
            feature: "testFeature".to_string(),
        },
        ConvertError::UnresolvableRef {
            path: "#/test".to_string(),
            reference: "#/missing".to_string(),
        },
        ConvertError::RehydrationError("test rehydration error".to_string()),
        ConvertError::CodecVersionMismatch {
            found: "v99".to_string(),
            expected: "v1".to_string(),
        },
    ];

    for err in &errors {
        let json = err.to_json();
        assert!(
            matches!(json.get("code"), Some(v) if v.is_string()),
            "Error {:?} missing string 'code' field, got: {:?}",
            err.error_code(),
            json.get("code")
        );
        assert!(
            matches!(json.get("message"), Some(v) if v.is_string()),
            "Error {:?} missing string 'message' field, got: {:?}",
            err.error_code(),
            json.get("message")
        );
        assert!(
            json.get("path").is_some(),
            "Error {:?} missing 'path' field (should be string or null)",
            err.error_code()
        );
    }
}

/// Thread safety — multiple independent convert_json calls must not interfere.
#[test]
fn test_convert_json_concurrent_calls() {
    use std::thread;

    let handles: Vec<_> = (0..4)
        .map(|i| {
            thread::spawn(move || {
                let schema = format!(
                    r#"{{"type": "object", "properties": {{"field{}": {{"type": "string"}}}}, "required": ["field{}"]}}"#,
                    i, i
                );
                let options = r#"{"target": "openai-strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#;
                let result = convert_json(&schema, options);
                assert!(result.is_ok(), "Thread {} failed: {:?}", i, result.err());

                let parsed: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
                assert_eq!(parsed["apiVersion"].as_str().unwrap(), "1.0");
                let prop_key = format!("field{}", i);
                assert!(
                    parsed["schema"]["properties"].get(&prop_key).is_some(),
                    "Thread {} output missing property {}",
                    i,
                    prop_key
                );
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread panicked");
    }
}

/// Bridge error codes must match typed API error codes for the same input.
#[test]
fn test_bridge_error_matches_typed_error() {
    // Trigger a JsonParseError through both APIs
    let bridge_err = convert_json(
        "not valid json",
        r#"{"target": "openai-strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#,
    );
    assert!(bridge_err.is_err());

    let bridge_code: serde_json::Value = serde_json::from_str(&bridge_err.unwrap_err()).unwrap();
    let bridge_code_str = bridge_code["code"].as_str().unwrap();

    // Same error through typed API
    let typed_err = serde_json::from_str::<serde_json::Value>("not valid json").unwrap_err();
    let typed_error = ConvertError::JsonError(typed_err);
    let typed_code = serde_json::to_value(typed_error.error_code()).unwrap();

    assert_eq!(
        bridge_code_str,
        typed_code.as_str().unwrap(),
        "Bridge and typed API must produce identical error codes"
    );
}
