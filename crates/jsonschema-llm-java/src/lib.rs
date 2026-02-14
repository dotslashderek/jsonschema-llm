use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;
use jsonschema_llm_core::{convert_json, rehydrate_json};
use libc::c_char;
use std::ffi::{CStr, CString};
use std::ptr;

// ---------------------------------------------------------------------------
// C API (Panama FFM)
// ---------------------------------------------------------------------------

/// Convert a JSON Schema string to an LLM-compatible JSON Schema string.
///
/// # Safety
///
/// * `schema_json` and `options_json` must be valid, null-terminated UTF-8 C strings.
/// * The returned pointer must be freed by `jsonschema_llm_free_string`.
#[no_mangle]
pub unsafe extern "C" fn jsonschema_llm_convert(
    schema_json: *const c_char,
    options_json: *const c_char,
) -> *mut c_char {
    let result = std::panic::catch_unwind(|| {
        if schema_json.is_null() || options_json.is_null() {
            return error_json("Null pointer argument");
        }

        let schema_str = match CStr::from_ptr(schema_json).to_str() {
            Ok(s) => s,
            Err(_) => return error_json("Invalid UTF-8 in schema_json"),
        };

        let options_str = match CStr::from_ptr(options_json).to_str() {
            Ok(s) => s,
            Err(_) => return error_json("Invalid UTF-8 in options_json"),
        };

        match convert_json(schema_str, options_str) {
            Ok(json) => string_to_ptr(json),
            Err(e) => string_to_ptr(e), // Error is already a JSON string
        }
    });

    match result {
        Ok(ptr) => ptr,
        Err(_) => error_json("Panic inside Rust FFI"),
    }
}

/// Rehydrate LLM output JSON string to original schema structure.
///
/// # Safety
///
/// * `data_json`, `codec_json`, and `original_schema_json` must be valid, null-terminated UTF-8 C strings.
/// * The returned pointer must be freed by `jsonschema_llm_free_string`.
#[no_mangle]
pub unsafe extern "C" fn jsonschema_llm_rehydrate(
    data_json: *const c_char,
    codec_json: *const c_char,
    original_schema_json: *const c_char,
) -> *mut c_char {
    let result = std::panic::catch_unwind(|| {
        if data_json.is_null() || codec_json.is_null() || original_schema_json.is_null() {
            return error_json("Null pointer argument");
        }

        let data_str = match CStr::from_ptr(data_json).to_str() {
            Ok(s) => s,
            Err(_) => return error_json("Invalid UTF-8 in data_json"),
        };

        let codec_str = match CStr::from_ptr(codec_json).to_str() {
            Ok(s) => s,
            Err(_) => return error_json("Invalid UTF-8 in codec_json"),
        };

        let schema_str = match CStr::from_ptr(original_schema_json).to_str() {
            Ok(s) => s,
            Err(_) => return error_json("Invalid UTF-8 in original_schema_json"),
        };

        match rehydrate_json(data_str, codec_str, schema_str) {
            Ok(json) => string_to_ptr(json),
            Err(e) => string_to_ptr(e), // Error is already a JSON string
        }
    });

    match result {
        Ok(ptr) => ptr,
        Err(_) => error_json("Panic inside Rust FFI"),
    }
}

/// Free a string returned by `jsonschema_llm_convert` or `jsonschema_llm_rehydrate`.
///
/// # Safety
///
/// * `ptr` must be a pointer returned by one of the library functions.
/// * `ptr` must not be used after this call.
/// * Passing a null pointer is a no-op.
#[no_mangle]
pub unsafe extern "C" fn jsonschema_llm_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        // Take ownership of the pointer and drop it
        let _ = CString::from_raw(ptr);
    }
}

// ---------------------------------------------------------------------------
// JNI API (Java 11+ Fallback)
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "system" fn Java_com_jsonschema_llm_JniBinding_convert(
    mut env: JNIEnv,
    _class: JClass,
    schema_json: JString,
    options_json: JString,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let schema: String = env
            .get_string(&schema_json)
            .expect("Couldn't get schema string")
            .into();
        let options: String = env
            .get_string(&options_json)
            .expect("Couldn't get options string")
            .into();

        match convert_json(&schema, &options) {
            Ok(json) => env.new_string(json).expect("Couldn't create java string"),
            Err(e) => env.new_string(e).expect("Couldn't create java string"),
        }
    }));

    match result {
        Ok(jstr) => jstr.into_raw(),
        Err(_) => {
            // In case of panic, we should ideally throw a Java exception,
            // but for now we return a JSON error string if possible, or null.
            // Since we are inside a panic handler, interacting with JNI might be risky if the panic was JNI related.
            // But we'll try to return a simple error string.
            if let Ok(err_str) = env.new_string(r#"{"code": "Panic", "message": "Panic in Rust JNI"}"#) {
                err_str.into_raw()
            } else {
                ptr::null_mut()
            }
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_jsonschema_llm_JniBinding_rehydrate(
    mut env: JNIEnv,
    _class: JClass,
    data_json: JString,
    codec_json: JString,
    original_schema_json: JString,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let data: String = env
            .get_string(&data_json)
            .expect("Couldn't get data string")
            .into();
        let codec: String = env
            .get_string(&codec_json)
            .expect("Couldn't get codec string")
            .into();
        let schema: String = env
            .get_string(&original_schema_json)
            .expect("Couldn't get schema string")
            .into();

        match rehydrate_json(&data, &codec, &schema) {
            Ok(json) => env.new_string(json).expect("Couldn't create java string"),
            Err(e) => env.new_string(e).expect("Couldn't create java string"),
        }
    }));

    match result {
        Ok(jstr) => jstr.into_raw(),
        Err(_) => {
             if let Ok(err_str) = env.new_string(r#"{"code": "Panic", "message": "Panic in Rust JNI"}"#) {
                err_str.into_raw()
            } else {
                ptr::null_mut()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn string_to_ptr(s: String) -> *mut c_char {
    CString::new(s).unwrap().into_raw()
}

fn error_json(msg: &str) -> *mut c_char {
    let json = format!(r#"{{"code": "FFIError", "message": "{}"}}"#, msg);
    string_to_ptr(json)
}
