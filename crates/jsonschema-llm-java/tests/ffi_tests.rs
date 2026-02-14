use std::ffi::{CStr, CString};
use jsonschema_llm_java::{jsonschema_llm_convert, jsonschema_llm_free_string, jsonschema_llm_rehydrate};

#[test]
fn test_ffi_convert_roundtrip() {
    let schema = r#"{"type": "object", "properties": {"name": {"type": "string"}}}"#;
    let options = r#"{"target": "openai-strict", "mode": "strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#;

    let c_schema = CString::new(schema).unwrap();
    let c_options = CString::new(options).unwrap();

    unsafe {
        let result_ptr = jsonschema_llm_convert(c_schema.as_ptr(), c_options.as_ptr());
        assert!(!result_ptr.is_null());

        let result_str = CStr::from_ptr(result_ptr).to_str().unwrap();
        // println!("Convert result: {}", result_str);

        // Check for success (apiVersion)
        assert!(result_str.contains("apiVersion"));
        assert!(result_str.contains("\"schema\""));

        jsonschema_llm_free_string(result_ptr);
    }
}

#[test]
fn test_ffi_rehydrate_roundtrip() {
    let schema = r#"{"type": "object", "properties": {"name": {"type": "string"}}}"#;
    let options = r#"{"target": "openai-strict", "mode": "strict", "max-depth": 50, "recursion-limit": 3, "polymorphism": "any-of"}"#;
    
    // First convert to get codec
    let c_schema = CString::new(schema).unwrap();
    let c_options = CString::new(options).unwrap();
    
    unsafe {
        let convert_ptr = jsonschema_llm_convert(c_schema.as_ptr(), c_options.as_ptr());
        let convert_json = CStr::from_ptr(convert_ptr).to_str().unwrap();
        
        let convert_val: serde_json::Value = serde_json::from_str(convert_json).unwrap();
        let codec = convert_val.get("codec").unwrap();
        let codec_str = serde_json::to_string(codec).unwrap();
        
        jsonschema_llm_free_string(convert_ptr);

        // Now rehydrate
        let data = r#"{"name": "Alice"}"#;
        let c_data = CString::new(data).unwrap();
        let c_codec = CString::new(codec_str).unwrap();
        // original schema is same as input
        
        let result_ptr = jsonschema_llm_rehydrate(c_data.as_ptr(), c_codec.as_ptr(), c_schema.as_ptr());
        assert!(!result_ptr.is_null());
        
        let result_str = CStr::from_ptr(result_ptr).to_str().unwrap();
        // println!("Rehydrate result: {}", result_str);
        
        assert!(result_str.contains("Alice"));
        
        jsonschema_llm_free_string(result_ptr);
    }
}

#[test]
fn test_ffi_null_safety() {
    unsafe {
        let ptr = jsonschema_llm_convert(std::ptr::null(), std::ptr::null());
        assert!(!ptr.is_null());
        let result = CStr::from_ptr(ptr).to_str().unwrap();
        assert!(result.contains("Null pointer argument"));
        jsonschema_llm_free_string(ptr);
    }
}
