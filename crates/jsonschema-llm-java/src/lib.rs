//! C FFI + JNI exports for jsonschema-llm.
//!
//! Produces a single shared library (`libjsonschema_llm_java`) consumed by:
//! - **Panama FFM** (Java 22+) via standard C symbols
//! - **JNI** (Java 11+) via `Java_com_jsonschema_llm_JniBinding_*` symbols

use jni::objects::{JClass, JString};
use jni::sys::jstring;
use jni::JNIEnv;
use jsonschema_llm_core::{convert_json, rehydrate_json};
use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};
use std::ptr;

// ---------------------------------------------------------------------------
// Thread-local error storage
// ---------------------------------------------------------------------------

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

/// Store an error message in thread-local storage and return null.
fn set_last_error(msg: String) -> *mut c_char {
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = CString::new(msg).ok();
    });
    ptr::null_mut()
}

/// Clear thread-local error (called on success).
fn clear_last_error() {
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

// ---------------------------------------------------------------------------
// C API (Panama FFM)
// ---------------------------------------------------------------------------

/// Convert a JSON Schema to an LLM-compatible schema.
///
/// Returns an owned `*mut c_char` JSON string on success (caller must free
/// with `jsonschema_llm_free_string`), or `NULL` on error. On error, call
/// `jsonschema_llm_last_error` to retrieve the error JSON.
///
/// # Safety
///
/// `schema_json` and `options_json` must be valid, null-terminated UTF-8 strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jsonschema_llm_convert(
    schema_json: *const c_char,
    options_json: *const c_char,
) -> *mut c_char {
    let result = std::panic::catch_unwind(|| {
        if schema_json.is_null() || options_json.is_null() {
            return set_last_error(ffi_error_json("null_pointer", "Null pointer argument"));
        }

        let schema_str = match unsafe { CStr::from_ptr(schema_json) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                return set_last_error(ffi_error_json("utf8_error", "Invalid UTF-8 in schema_json"))
            }
        };

        let options_str = match unsafe { CStr::from_ptr(options_json) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                return set_last_error(ffi_error_json(
                    "utf8_error",
                    "Invalid UTF-8 in options_json",
                ))
            }
        };

        match convert_json(schema_str, options_str) {
            Ok(json) => {
                clear_last_error();
                string_to_ptr(json)
            }
            Err(e) => set_last_error(e),
        }
    });

    match result {
        Ok(ptr) => ptr,
        Err(_) => set_last_error(ffi_error_json("panic", "Panic inside Rust FFI")),
    }
}

/// Rehydrate LLM output back to the original schema structure.
///
/// Returns an owned `*mut c_char` JSON string on success (caller must free
/// with `jsonschema_llm_free_string`), or `NULL` on error.
///
/// # Safety
///
/// All three arguments must be valid, null-terminated UTF-8 strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jsonschema_llm_rehydrate(
    data_json: *const c_char,
    codec_json: *const c_char,
    original_schema_json: *const c_char,
) -> *mut c_char {
    let result = std::panic::catch_unwind(|| {
        if data_json.is_null() || codec_json.is_null() || original_schema_json.is_null() {
            return set_last_error(ffi_error_json("null_pointer", "Null pointer argument"));
        }

        let data_str = match unsafe { CStr::from_ptr(data_json) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                return set_last_error(ffi_error_json("utf8_error", "Invalid UTF-8 in data_json"))
            }
        };

        let codec_str = match unsafe { CStr::from_ptr(codec_json) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                return set_last_error(ffi_error_json("utf8_error", "Invalid UTF-8 in codec_json"))
            }
        };

        let schema_str = match unsafe { CStr::from_ptr(original_schema_json) }.to_str() {
            Ok(s) => s,
            Err(_) => {
                return set_last_error(ffi_error_json(
                    "utf8_error",
                    "Invalid UTF-8 in original_schema_json",
                ))
            }
        };

        match rehydrate_json(data_str, codec_str, schema_str) {
            Ok(json) => {
                clear_last_error();
                string_to_ptr(json)
            }
            Err(e) => set_last_error(e),
        }
    });

    match result {
        Ok(ptr) => ptr,
        Err(_) => set_last_error(ffi_error_json("panic", "Panic inside Rust FFI")),
    }
}

/// Return the last error as a JSON string, or `NULL` if no error.
///
/// The returned pointer is valid until the next FFI call on the same thread.
/// Do **not** free this pointer.
///
/// # Safety
///
/// This function is safe to call from any thread.
#[unsafe(no_mangle)]
pub extern "C" fn jsonschema_llm_last_error() -> *const c_char {
    LAST_ERROR.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|s| s.as_ptr())
            .unwrap_or(ptr::null())
    })
}

/// Free a string returned by `jsonschema_llm_convert` or `jsonschema_llm_rehydrate`.
///
/// Passing `NULL` is a no-op. The pointer must not be used after this call.
///
/// # Safety
///
/// `ptr` must be a pointer originally returned by one of the library functions,
/// or `NULL`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jsonschema_llm_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        let _ = unsafe { CString::from_raw(ptr) };
    }
}

// ---------------------------------------------------------------------------
// JNI API (Java 11+ fallback)
// ---------------------------------------------------------------------------

/// JNI entry point for schema conversion.
///
/// Throws `com.jsonschema.llm.JsonSchemaLlmException` on error.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jsonschema_llm_JniBinding_convert(
    mut env: JNIEnv,
    _class: JClass,
    schema_json: JString,
    options_json: JString,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let schema: String = match env.get_string(&schema_json) {
            Ok(s) => s.into(),
            Err(e) => {
                let _ = env.throw_new(
                    "com/jsonschema/llm/JsonSchemaLlmException",
                    format!("Failed to get schema string: {e}"),
                );
                return JString::default();
            }
        };
        let options: String = match env.get_string(&options_json) {
            Ok(s) => s.into(),
            Err(e) => {
                let _ = env.throw_new(
                    "com/jsonschema/llm/JsonSchemaLlmException",
                    format!("Failed to get options string: {e}"),
                );
                return JString::default();
            }
        };

        match convert_json(&schema, &options) {
            Ok(json) => match env.new_string(json) {
                Ok(jstr) => jstr,
                Err(e) => {
                    let _ = env.throw_new(
                        "com/jsonschema/llm/JsonSchemaLlmException",
                        format!("Failed to create Java string: {e}"),
                    );
                    JString::default()
                }
            },
            Err(e) => {
                let _ = env.throw_new("com/jsonschema/llm/JsonSchemaLlmException", &e);
                JString::default()
            }
        }
    }));

    match result {
        Ok(jstr) => jstr.into_raw(),
        Err(_) => {
            let _ = env.throw_new(
                "com/jsonschema/llm/JsonSchemaLlmException",
                "Panic in Rust JNI",
            );
            ptr::null_mut()
        }
    }
}

/// JNI entry point for data rehydration.
///
/// Throws `com.jsonschema.llm.JsonSchemaLlmException` on error.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_jsonschema_llm_JniBinding_rehydrate(
    mut env: JNIEnv,
    _class: JClass,
    data_json: JString,
    codec_json: JString,
    original_schema_json: JString,
) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let data: String = match env.get_string(&data_json) {
            Ok(s) => s.into(),
            Err(e) => {
                let _ = env.throw_new(
                    "com/jsonschema/llm/JsonSchemaLlmException",
                    format!("Failed to get data string: {e}"),
                );
                return JString::default();
            }
        };
        let codec: String = match env.get_string(&codec_json) {
            Ok(s) => s.into(),
            Err(e) => {
                let _ = env.throw_new(
                    "com/jsonschema/llm/JsonSchemaLlmException",
                    format!("Failed to get codec string: {e}"),
                );
                return JString::default();
            }
        };
        let schema: String = match env.get_string(&original_schema_json) {
            Ok(s) => s.into(),
            Err(e) => {
                let _ = env.throw_new(
                    "com/jsonschema/llm/JsonSchemaLlmException",
                    format!("Failed to get schema string: {e}"),
                );
                return JString::default();
            }
        };

        match rehydrate_json(&data, &codec, &schema) {
            Ok(json) => match env.new_string(json) {
                Ok(jstr) => jstr,
                Err(e) => {
                    let _ = env.throw_new(
                        "com/jsonschema/llm/JsonSchemaLlmException",
                        format!("Failed to create Java string: {e}"),
                    );
                    JString::default()
                }
            },
            Err(e) => {
                let _ = env.throw_new("com/jsonschema/llm/JsonSchemaLlmException", &e);
                JString::default()
            }
        }
    }));

    match result {
        Ok(jstr) => jstr.into_raw(),
        Err(_) => {
            let _ = env.throw_new(
                "com/jsonschema/llm/JsonSchemaLlmException",
                "Panic in Rust JNI",
            );
            ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn string_to_ptr(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(cstr) => cstr.into_raw(),
        Err(_) => set_last_error(ffi_error_json(
            "internal_error",
            "String contained null byte",
        )),
    }
}

fn ffi_error_json(code: &str, message: &str) -> String {
    serde_json::json!({
        "code": code,
        "message": message,
    })
    .to_string()
}
