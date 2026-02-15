use jsonschema_llm_java::{
    jsonschema_llm_convert, jsonschema_llm_free_string, jsonschema_llm_last_error,
    jsonschema_llm_rehydrate,
};
use std::ffi::{CStr, CString};

#[test]
fn convert_produces_valid_json_with_api_version() {
    let schema =
        CString::new(r#"{"type": "object", "properties": {"name": {"type": "string"}}}"#).unwrap();
    let options = CString::new(
        r#"{"target": "openai-strict", "mode": "strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#,
    )
    .unwrap();

    unsafe {
        let ptr = jsonschema_llm_convert(schema.as_ptr(), options.as_ptr());
        assert!(!ptr.is_null(), "convert should return non-null on success");

        let result = CStr::from_ptr(ptr).to_str().unwrap();
        let json: serde_json::Value = serde_json::from_str(result).unwrap();
        assert!(json.get("apiVersion").is_some());
        assert!(json.get("schema").is_some());
        assert!(json.get("codec").is_some());

        // last_error should be null after success
        assert!(jsonschema_llm_last_error().is_null());

        jsonschema_llm_free_string(ptr);
    }
}

#[test]
fn convert_rehydrate_roundtrip() {
    let schema =
        CString::new(r#"{"type": "object", "properties": {"name": {"type": "string"}}}"#).unwrap();
    let options = CString::new(
        r#"{"target": "openai-strict", "mode": "strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#,
    )
    .unwrap();

    unsafe {
        // Convert
        let convert_ptr = jsonschema_llm_convert(schema.as_ptr(), options.as_ptr());
        assert!(!convert_ptr.is_null());

        let convert_str = CStr::from_ptr(convert_ptr).to_str().unwrap();
        let convert_val: serde_json::Value = serde_json::from_str(convert_str).unwrap();
        let codec = convert_val.get("codec").unwrap();
        let codec_str = CString::new(serde_json::to_string(codec).unwrap()).unwrap();
        jsonschema_llm_free_string(convert_ptr);

        // Rehydrate
        let data = CString::new(r#"{"name": "Alice"}"#).unwrap();
        let rehydrate_ptr =
            jsonschema_llm_rehydrate(data.as_ptr(), codec_str.as_ptr(), schema.as_ptr());
        assert!(!rehydrate_ptr.is_null());

        let result = CStr::from_ptr(rehydrate_ptr).to_str().unwrap();
        assert!(result.contains("Alice"));

        jsonschema_llm_free_string(rehydrate_ptr);
    }
}

#[test]
fn null_pointer_returns_null_and_sets_last_error() {
    unsafe {
        let ptr = jsonschema_llm_convert(std::ptr::null(), std::ptr::null());
        assert!(ptr.is_null(), "null args should return null");

        let err = jsonschema_llm_last_error();
        assert!(!err.is_null(), "last_error should be set");

        let err_str = CStr::from_ptr(err).to_str().unwrap();
        let err_json: serde_json::Value = serde_json::from_str(err_str).unwrap();
        assert_eq!(err_json["code"], "null_pointer");
        assert!(err_json["message"]
            .as_str()
            .unwrap()
            .contains("Null pointer"));
    }
}

#[test]
fn invalid_json_returns_null_and_sets_last_error() {
    let schema = CString::new("not valid json").unwrap();
    let options = CString::new("{}").unwrap();

    unsafe {
        let ptr = jsonschema_llm_convert(schema.as_ptr(), options.as_ptr());
        assert!(ptr.is_null(), "invalid JSON should return null");

        let err = jsonschema_llm_last_error();
        assert!(!err.is_null());
    }
}

#[test]
fn free_string_null_is_noop() {
    unsafe {
        // Should not crash or panic
        jsonschema_llm_free_string(std::ptr::null_mut());
    }
}

#[test]
fn last_error_is_null_after_successful_call() {
    let schema =
        CString::new(r#"{"type": "object", "properties": {"age": {"type": "integer"}}}"#).unwrap();
    let options = CString::new(
        r#"{"target": "openai-strict", "mode": "strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#,
    )
    .unwrap();

    unsafe {
        // First, trigger an error
        let _ = jsonschema_llm_convert(std::ptr::null(), std::ptr::null());
        assert!(!jsonschema_llm_last_error().is_null());

        // Then, a successful call should clear it
        let ptr = jsonschema_llm_convert(schema.as_ptr(), options.as_ptr());
        assert!(!ptr.is_null());
        assert!(
            jsonschema_llm_last_error().is_null(),
            "last_error should be cleared after success"
        );

        jsonschema_llm_free_string(ptr);
    }
}

#[test]
fn rehydrate_null_pointer_returns_null() {
    unsafe {
        let ptr = jsonschema_llm_rehydrate(std::ptr::null(), std::ptr::null(), std::ptr::null());
        assert!(ptr.is_null());

        let err = jsonschema_llm_last_error();
        assert!(!err.is_null());
    }
}
